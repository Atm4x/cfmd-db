use std::collections::{BTreeMap, BTreeSet};

use crate::OrderDirection;

const DENSE_MARGIN: i64 = 64;
const DENSE_MAX_SLOTS: usize = 4096;
const RADIX_SHIFT: u32 = 8;
const RADIX_SLOTS: usize = 1 << RADIX_SHIFT;
const RADIX_WORDS: usize = RADIX_SLOTS / 64;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum I64TopKPhysicalKind {
    DenseUnit,
    DenseCounted,
    PagedRadix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DenseUnit {
    base: i64,
    slots: usize,
    occupied: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DenseCounted {
    base: i64,
    counts: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RadixPage {
    counts: Box<[usize; RADIX_SLOTS]>,
    occupied: [u64; RADIX_WORDS],
}

impl Default for RadixPage {
    fn default() -> Self {
        Self {
            counts: Box::new([0; RADIX_SLOTS]),
            occupied: [0; RADIX_WORDS],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PagedRadix {
    pages: BTreeMap<u64, RadixPage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PhysicalCounts {
    DenseUnit(DenseUnit),
    DenseCounted(DenseCounted),
    PagedRadix(PagedRadix),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct I64TopKState {
    counts: PhysicalCounts,
    threshold: Option<i64>,
    threshold_count: usize,
    better_rows: usize,
    total_rows: usize,
    k: usize,
    direction: OrderDirection,
    is_set: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum I64TopKPatch {
    Noop,
    UnitReplace(I64TopKUnitPatch),
    Replace(I64TopKState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct I64TopKUnitPatch {
    removed: i64,
    removed_after: usize,
    inserted: i64,
    inserted_after: usize,
    threshold: Option<i64>,
    threshold_count: usize,
    better_rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct I64TopKPlan {
    pub(crate) patch: I64TopKPatch,
    pub(crate) effect: Vec<(i64, i64)>,
    pub(crate) threshold_steps: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum I64TopKError {
    MissingKey,
    DuplicateSetKey,
    CountOverflow,
    InvalidWeight,
}

impl DenseUnit {
    fn build(counts: &BTreeMap<i64, usize>) -> Option<Self> {
        if counts.is_empty() || counts.values().any(|count| *count != 1) {
            return None;
        }
        let (&min, _) = counts.first_key_value()?;
        let (&max, _) = counts.last_key_value()?;
        let lower = min.saturating_sub(DENSE_MARGIN);
        let upper = max.saturating_add(DENSE_MARGIN);
        let span = i128::from(upper) - i128::from(lower) + 1;
        let slots = usize::try_from(span).ok()?;
        if slots == 0 || slots > DENSE_MAX_SLOTS {
            return None;
        }
        let mut state = Self {
            base: lower,
            slots,
            occupied: vec![0; slots.div_ceil(64)],
        };
        for &key in counts.keys() {
            state.set(key, 1)?;
        }
        Some(state)
    }

    fn index(&self, key: i64) -> Option<usize> {
        let index = usize::try_from(i128::from(key) - i128::from(self.base)).ok()?;
        (index < self.slots).then_some(index)
    }

    fn count(&self, key: i64) -> usize {
        self.index(key)
            .is_some_and(|index| self.occupied[index >> 6] & (1_u64 << (index & 63)) != 0)
            .into()
    }

    fn set(&mut self, key: i64, count: usize) -> Option<()> {
        if count > 1 {
            return None;
        }
        let index = self.index(key)?;
        let bit = 1_u64 << (index & 63);
        if count == 0 {
            self.occupied[index >> 6] &= !bit;
        } else {
            self.occupied[index >> 6] |= bit;
        }
        Some(())
    }

    fn previous_key(&self, key: i64) -> Option<i64> {
        let index = self.index(key)?;
        (0..index).rev().find_map(|candidate| {
            let bit = 1_u64 << (candidate & 63);
            (self.occupied[candidate >> 6] & bit != 0)
                .then(|| self.base + i64::try_from(candidate).expect("dense slot fits i64"))
        })
    }

    fn next_key(&self, key: i64) -> Option<i64> {
        let index = self.index(key)?;
        ((index + 1)..self.slots).find_map(|candidate| {
            let bit = 1_u64 << (candidate & 63);
            (self.occupied[candidate >> 6] & bit != 0)
                .then(|| self.base + i64::try_from(candidate).expect("dense slot fits i64"))
        })
    }

    fn visit(&self, mut visitor: impl FnMut(i64, usize)) {
        for (word_index, &word) in self.occupied.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                let index = (word_index << 6) + bit;
                if index < self.slots {
                    visitor(
                        self.base + i64::try_from(index).expect("dense slot fits i64"),
                        1,
                    );
                }
                bits &= bits - 1;
            }
        }
    }
}

impl DenseCounted {
    fn build(counts: &BTreeMap<i64, usize>) -> Option<Self> {
        if counts.is_empty() {
            return None;
        }
        let (&min, _) = counts.first_key_value()?;
        let (&max, _) = counts.last_key_value()?;
        let lower = min.saturating_sub(DENSE_MARGIN);
        let upper = max.saturating_add(DENSE_MARGIN);
        let span = i128::from(upper) - i128::from(lower) + 1;
        let slots = usize::try_from(span).ok()?;
        if slots == 0 || slots > DENSE_MAX_SLOTS {
            return None;
        }
        let mut state = Self {
            base: lower,
            counts: vec![0; slots],
        };
        for (&key, &count) in counts {
            state.set(key, count)?;
        }
        Some(state)
    }

    fn index(&self, key: i64) -> Option<usize> {
        let index = usize::try_from(i128::from(key) - i128::from(self.base)).ok()?;
        (index < self.counts.len()).then_some(index)
    }

    fn count(&self, key: i64) -> usize {
        self.index(key).map_or(0, |index| self.counts[index])
    }

    fn set(&mut self, key: i64, count: usize) -> Option<()> {
        let index = self.index(key)?;
        self.counts[index] = count;
        Some(())
    }

    fn previous_key(&self, key: i64) -> Option<i64> {
        let index = self.index(key)?;
        (0..index).rev().find_map(|candidate| {
            (self.counts[candidate] != 0)
                .then(|| self.base + i64::try_from(candidate).expect("dense slot fits i64"))
        })
    }

    fn next_key(&self, key: i64) -> Option<i64> {
        let index = self.index(key)?;
        ((index + 1)..self.counts.len()).find_map(|candidate| {
            (self.counts[candidate] != 0)
                .then(|| self.base + i64::try_from(candidate).expect("dense slot fits i64"))
        })
    }

    fn visit(&self, mut visitor: impl FnMut(i64, usize)) {
        for (index, &count) in self.counts.iter().enumerate() {
            if count != 0 {
                visitor(
                    self.base + i64::try_from(index).expect("dense slot fits i64"),
                    count,
                );
            }
        }
    }
}

impl PagedRadix {
    #[inline]
    fn rank(key: i64) -> u64 {
        key.cast_unsigned() ^ (1_u64 << 63)
    }

    #[inline]
    fn key(rank: u64) -> i64 {
        (rank ^ (1_u64 << 63)).cast_signed()
    }

    #[inline]
    fn split(key: i64) -> (u64, usize) {
        let rank = Self::rank(key);
        (
            rank >> RADIX_SHIFT,
            usize::try_from(rank & ((1_u64 << RADIX_SHIFT) - 1)).expect("radix slot fits usize"),
        )
    }

    fn build(counts: &BTreeMap<i64, usize>) -> Self {
        let mut state = Self::default();
        for (&key, &count) in counts {
            state.set(key, count);
        }
        state
    }

    fn count(&self, key: i64) -> usize {
        let (page, slot) = Self::split(key);
        self.pages.get(&page).map_or(0, |page| page.counts[slot])
    }

    fn set(&mut self, key: i64, count: usize) {
        let (page_key, slot) = Self::split(key);
        if count == 0 {
            let mut remove_page = false;
            if let Some(page) = self.pages.get_mut(&page_key) {
                page.counts[slot] = 0;
                page.occupied[slot >> 6] &= !(1_u64 << (slot & 63));
                remove_page = page.occupied.iter().all(|word| *word == 0);
            }
            if remove_page {
                self.pages.remove(&page_key);
            }
            return;
        }
        let page = self.pages.entry(page_key).or_default();
        page.counts[slot] = count;
        page.occupied[slot >> 6] |= 1_u64 << (slot & 63);
    }

    fn previous_key(&self, key: i64) -> Option<i64> {
        let (page_key, slot) = Self::split(key);
        if let Some(page) = self.pages.get(&page_key) {
            for candidate in (0..slot).rev() {
                if page.counts[candidate] != 0 {
                    let rank = (page_key << RADIX_SHIFT)
                        | u64::try_from(candidate).expect("radix slot fits u64");
                    return Some(Self::key(rank));
                }
            }
        }
        let (&previous_page_key, page) = self.pages.range(..page_key).next_back()?;
        for candidate in (0..RADIX_SLOTS).rev() {
            if page.counts[candidate] != 0 {
                let rank = (previous_page_key << RADIX_SHIFT)
                    | u64::try_from(candidate).expect("radix slot fits u64");
                return Some(Self::key(rank));
            }
        }
        None
    }

    fn next_key(&self, key: i64) -> Option<i64> {
        let (page_key, slot) = Self::split(key);
        if let Some(page) = self.pages.get(&page_key) {
            for candidate in (slot + 1)..RADIX_SLOTS {
                if page.counts[candidate] != 0 {
                    let rank = (page_key << RADIX_SHIFT)
                        | u64::try_from(candidate).expect("radix slot fits u64");
                    return Some(Self::key(rank));
                }
            }
        }
        let (&next_page_key, page) = self.pages.range((page_key + 1)..).next()?;
        for candidate in 0..RADIX_SLOTS {
            if page.counts[candidate] != 0 {
                let rank = (next_page_key << RADIX_SHIFT)
                    | u64::try_from(candidate).expect("radix slot fits u64");
                return Some(Self::key(rank));
            }
        }
        None
    }

    fn visit(&self, mut visitor: impl FnMut(i64, usize)) {
        for (&page_key, page) in &self.pages {
            for word_index in 0..RADIX_WORDS {
                let mut bits = page.occupied[word_index];
                while bits != 0 {
                    let bit = bits.trailing_zeros() as usize;
                    let slot = (word_index << 6) + bit;
                    let rank = (page_key << RADIX_SHIFT) | u64::try_from(slot).expect("slot fits");
                    visitor(Self::key(rank), page.counts[slot]);
                    bits &= bits - 1;
                }
            }
        }
    }
}

impl PhysicalCounts {
    fn build(counts: &BTreeMap<i64, usize>) -> Self {
        if let Some(unit) = DenseUnit::build(counts) {
            return Self::DenseUnit(unit);
        }
        if let Some(counted) = DenseCounted::build(counts) {
            return Self::DenseCounted(counted);
        }
        Self::PagedRadix(PagedRadix::build(counts))
    }

    #[cfg(test)]
    fn kind(&self) -> I64TopKPhysicalKind {
        match self {
            Self::DenseUnit(_) => I64TopKPhysicalKind::DenseUnit,
            Self::DenseCounted(_) => I64TopKPhysicalKind::DenseCounted,
            Self::PagedRadix(_) => I64TopKPhysicalKind::PagedRadix,
        }
    }

    fn count(&self, key: i64) -> usize {
        match self {
            Self::DenseUnit(state) => state.count(key),
            Self::DenseCounted(state) => state.count(key),
            Self::PagedRadix(state) => state.count(key),
        }
    }

    fn set(&mut self, key: i64, count: usize) {
        let admitted = match self {
            Self::DenseUnit(state) => state.set(key, count).is_some(),
            Self::DenseCounted(state) => state.set(key, count).is_some(),
            Self::PagedRadix(state) => {
                state.set(key, count);
                true
            }
        };
        if admitted {
            return;
        }
        let mut map = self.to_map();
        if count == 0 {
            map.remove(&key);
        } else {
            map.insert(key, count);
        }
        *self = match self {
            Self::DenseUnit(_) if count <= 1 => DenseUnit::build(&map)
                .map(Self::DenseUnit)
                .or_else(|| DenseCounted::build(&map).map(Self::DenseCounted))
                .unwrap_or_else(|| Self::PagedRadix(PagedRadix::build(&map))),
            Self::DenseUnit(_) | Self::DenseCounted(_) => DenseCounted::build(&map).map_or_else(
                || Self::PagedRadix(PagedRadix::build(&map)),
                Self::DenseCounted,
            ),
            Self::PagedRadix(_) => Self::PagedRadix(PagedRadix::build(&map)),
        };
    }

    fn ensure_admitted(&mut self, changes: &BTreeMap<i64, i64>) {
        let mut needs_promotion = false;
        let mut unit_duplicate = false;
        match self {
            Self::DenseUnit(state) => {
                for (&key, &change) in changes {
                    let current = state.count(key);
                    let next =
                        i128::try_from(current).expect("usize fits i128") + i128::from(change);
                    if state.index(key).is_none() {
                        needs_promotion = true;
                    }
                    if next > 1 {
                        unit_duplicate = true;
                    }
                }
            }
            Self::DenseCounted(state) => {
                needs_promotion = changes.keys().any(|key| state.index(*key).is_none());
            }
            Self::PagedRadix(_) => return,
        }
        if !needs_promotion && !unit_duplicate {
            return;
        }
        let map = self.to_map();
        if !needs_promotion
            && unit_duplicate
            && let Some(counted) = DenseCounted::build(&map)
        {
            *self = Self::DenseCounted(counted);
            return;
        }
        *self = Self::PagedRadix(PagedRadix::build(&map));
    }

    fn visit(&self, visitor: impl FnMut(i64, usize)) {
        match self {
            Self::DenseUnit(state) => state.visit(visitor),
            Self::DenseCounted(state) => state.visit(visitor),
            Self::PagedRadix(state) => state.visit(visitor),
        }
    }

    fn to_map(&self) -> BTreeMap<i64, usize> {
        let mut map = BTreeMap::new();
        self.visit(|key, count| {
            map.insert(key, count);
        });
        map
    }

    fn ordered_entries(&self, direction: OrderDirection) -> Vec<(i64, usize)> {
        let mut entries = Vec::new();
        self.visit(|key, count| entries.push((key, count)));
        if matches!(direction, OrderDirection::Descending) {
            entries.reverse();
        }
        entries
    }

    fn previous_numeric(&self, key: i64) -> Option<i64> {
        match self {
            Self::DenseUnit(state) => state.previous_key(key),
            Self::DenseCounted(state) => state.previous_key(key),
            Self::PagedRadix(state) => state.previous_key(key),
        }
    }

    fn next_numeric(&self, key: i64) -> Option<i64> {
        match self {
            Self::DenseUnit(state) => state.next_key(key),
            Self::DenseCounted(state) => state.next_key(key),
            Self::PagedRadix(state) => state.next_key(key),
        }
    }

    fn predecessor(&self, key: i64, direction: OrderDirection) -> Option<i64> {
        match direction {
            OrderDirection::Ascending => self.previous_numeric(key),
            OrderDirection::Descending => self.next_numeric(key),
        }
    }

    fn successor(&self, key: i64, direction: OrderDirection) -> Option<i64> {
        match direction {
            OrderDirection::Ascending => self.next_numeric(key),
            OrderDirection::Descending => self.previous_numeric(key),
        }
    }
}

impl I64TopKState {
    pub(crate) fn build(
        counts: &BTreeMap<i64, usize>,
        k: usize,
        direction: OrderDirection,
        is_set: bool,
    ) -> Result<Self, I64TopKError> {
        if is_set && counts.values().any(|count| *count > 1) {
            return Err(I64TopKError::DuplicateSetKey);
        }
        let total_rows = counts.values().try_fold(0_usize, |total, count| {
            total.checked_add(*count).ok_or(I64TopKError::CountOverflow)
        })?;
        let mut state = Self {
            counts: PhysicalCounts::build(counts),
            threshold: None,
            threshold_count: 0,
            better_rows: 0,
            total_rows,
            k,
            direction,
            is_set,
        };
        state.recompute_boundary();
        Ok(state)
    }

    #[cfg(test)]
    pub(crate) fn kind(&self) -> I64TopKPhysicalKind {
        self.counts.kind()
    }

    pub(crate) const fn total_rows(&self) -> usize {
        self.total_rows
    }

    pub(crate) fn selected_counts(&self) -> BTreeMap<i64, usize> {
        let Some(threshold) = self.threshold else {
            return BTreeMap::new();
        };
        let mut selected = BTreeMap::new();
        self.counts.visit(|key, count| {
            if self.selected_by_threshold(key, threshold) {
                selected.insert(key, count);
            }
        });
        selected
    }

    pub(crate) fn plan_signed(
        &self,
        changes: &BTreeMap<i64, i64>,
    ) -> Result<I64TopKPlan, I64TopKError> {
        if changes.values().any(|weight| *weight == 0) {
            return Err(I64TopKError::InvalidWeight);
        }
        if changes.is_empty() {
            return Ok(I64TopKPlan {
                patch: I64TopKPatch::Noop,
                effect: Vec::new(),
                threshold_steps: 0,
            });
        }
        let unit_replace = changes.len() == 2
            && changes.values().filter(|weight| **weight == -1).count() == 1
            && changes.values().filter(|weight| **weight == 1).count() == 1;
        if unit_replace {
            let removed = *changes
                .iter()
                .find_map(|(key, weight)| (*weight == -1).then_some(key))
                .expect("unit replacement has removal");
            let inserted = *changes
                .iter()
                .find_map(|(key, weight)| (*weight == 1).then_some(key))
                .expect("unit replacement has insertion");
            if self.unit_replace_is_in_place_compatible(removed, inserted)? {
                return self.plan_unit_replace_in_place(removed, inserted);
            }
        }

        let mut next = self.clone();
        next.counts.ensure_admitted(changes);
        if unit_replace {
            let removed = *changes
                .iter()
                .find_map(|(key, weight)| (*weight == -1).then_some(key))
                .expect("unit replacement has removal");
            let inserted = *changes
                .iter()
                .find_map(|(key, weight)| (*weight == 1).then_some(key))
                .expect("unit replacement has insertion");
            let (effect, threshold_steps) = next.apply_unit_replace(removed, inserted)?;
            return Ok(I64TopKPlan {
                patch: I64TopKPatch::Replace(next),
                effect,
                threshold_steps,
            });
        }

        let before = self.selected_counts();
        next.apply_general(changes)?;
        let after = next.selected_counts();
        Ok(I64TopKPlan {
            patch: I64TopKPatch::Replace(next),
            effect: selected_effect(&before, &after)?,
            threshold_steps: 0,
        })
    }

    pub(crate) fn commit_patch(&mut self, patch: I64TopKPatch) {
        match patch {
            I64TopKPatch::Noop => {}
            I64TopKPatch::UnitReplace(patch) => {
                self.counts.set(patch.removed, patch.removed_after);
                self.counts.set(patch.inserted, patch.inserted_after);
                self.threshold = patch.threshold;
                self.threshold_count = patch.threshold_count;
                self.better_rows = patch.better_rows;
            }
            I64TopKPatch::Replace(next) => *self = next,
        }
    }

    fn unit_replace_is_in_place_compatible(
        &self,
        removed: i64,
        inserted: i64,
    ) -> Result<bool, I64TopKError> {
        let removed_before = self.counts.count(removed);
        if removed_before == 0 {
            return Err(I64TopKError::MissingKey);
        }
        let inserted_before = self.counts.count(inserted);
        if self.is_set && inserted_before != 0 {
            return Err(I64TopKError::DuplicateSetKey);
        }
        let inserted_after = inserted_before
            .checked_add(1)
            .ok_or(I64TopKError::CountOverflow)?;
        Ok(match &self.counts {
            PhysicalCounts::DenseUnit(state) => {
                state.index(removed).is_some()
                    && state.index(inserted).is_some()
                    && removed_before <= 1
                    && inserted_after <= 1
            }
            PhysicalCounts::DenseCounted(state) => {
                state.index(removed).is_some() && state.index(inserted).is_some()
            }
            PhysicalCounts::PagedRadix(_) => true,
        })
    }

    fn plan_unit_replace_in_place(
        &self,
        removed: i64,
        inserted: i64,
    ) -> Result<I64TopKPlan, I64TopKError> {
        let removed_after = self
            .counts
            .count(removed)
            .checked_sub(1)
            .ok_or(I64TopKError::MissingKey)?;
        let inserted_after = self
            .counts
            .count(inserted)
            .checked_add(1)
            .ok_or(I64TopKError::CountOverflow)?;
        let (threshold, threshold_count, better_rows, threshold_steps) =
            self.plan_unit_boundary(removed, removed_after, inserted, inserted_after)?;
        let effect = match threshold {
            Some(new_threshold) => self.unit_replace_effect_planned(
                removed,
                removed_after,
                inserted,
                inserted_after,
                self.threshold,
                new_threshold,
            )?,
            None => Vec::new(),
        };
        Ok(I64TopKPlan {
            patch: I64TopKPatch::UnitReplace(I64TopKUnitPatch {
                removed,
                removed_after,
                inserted,
                inserted_after,
                threshold,
                threshold_count,
                better_rows,
            }),
            effect,
            threshold_steps,
        })
    }

    fn plan_unit_boundary(
        &self,
        removed: i64,
        removed_after: usize,
        inserted: i64,
        inserted_after: usize,
    ) -> Result<(Option<i64>, usize, usize, usize), I64TopKError> {
        let effective_k = self.k.min(self.total_rows);
        if effective_k == 0 {
            return Ok((None, 0, 0, 0));
        }
        let old_threshold = self.threshold.ok_or(I64TopKError::CountOverflow)?;
        let mut better_rows = self.better_rows;
        if self.is_better(removed, old_threshold) {
            better_rows = better_rows
                .checked_sub(1)
                .ok_or(I64TopKError::CountOverflow)?;
        }
        if self.is_better(inserted, old_threshold) {
            better_rows = better_rows
                .checked_add(1)
                .ok_or(I64TopKError::CountOverflow)?;
        }
        let mut threshold = old_threshold;
        let mut threshold_count = self.count_after_unit_replace(
            threshold,
            removed,
            removed_after,
            inserted,
            inserted_after,
        );
        let mut threshold_steps = 0;
        if better_rows >= effective_k {
            threshold = self
                .neighbor_after_unit_replace(
                    threshold,
                    true,
                    removed,
                    removed_after,
                    inserted,
                    inserted_after,
                )
                .ok_or(I64TopKError::CountOverflow)?;
            threshold_count = self.count_after_unit_replace(
                threshold,
                removed,
                removed_after,
                inserted,
                inserted_after,
            );
            better_rows = better_rows
                .checked_sub(threshold_count)
                .ok_or(I64TopKError::CountOverflow)?;
            threshold_steps = 1;
        } else if better_rows + threshold_count < effective_k {
            better_rows = better_rows
                .checked_add(threshold_count)
                .ok_or(I64TopKError::CountOverflow)?;
            threshold = self
                .neighbor_after_unit_replace(
                    threshold,
                    false,
                    removed,
                    removed_after,
                    inserted,
                    inserted_after,
                )
                .ok_or(I64TopKError::CountOverflow)?;
            threshold_count = self.count_after_unit_replace(
                threshold,
                removed,
                removed_after,
                inserted,
                inserted_after,
            );
            threshold_steps = 1;
        }
        Ok((
            Some(threshold),
            threshold_count,
            better_rows,
            threshold_steps,
        ))
    }

    fn count_after_unit_replace(
        &self,
        key: i64,
        removed: i64,
        removed_after: usize,
        inserted: i64,
        inserted_after: usize,
    ) -> usize {
        if key == removed {
            removed_after
        } else if key == inserted {
            inserted_after
        } else {
            self.counts.count(key)
        }
    }

    fn neighbor_after_unit_replace(
        &self,
        pivot: i64,
        predecessor: bool,
        removed: i64,
        removed_after: usize,
        inserted: i64,
        inserted_after: usize,
    ) -> Option<i64> {
        let step = |key| {
            if predecessor {
                self.counts.predecessor(key, self.direction)
            } else {
                self.counts.successor(key, self.direction)
            }
        };
        let mut existing = step(pivot);
        if existing == Some(removed) && removed_after == 0 {
            existing = step(removed);
        }
        let inserted_candidate = (inserted_after != 0
            && if predecessor {
                self.is_better(inserted, pivot)
            } else {
                self.is_worse(inserted, pivot)
            })
        .then_some(inserted);
        match (existing, inserted_candidate) {
            (None, candidate) | (candidate, None) => candidate,
            (Some(left), Some(right)) => Some(match (self.direction, predecessor) {
                (OrderDirection::Ascending, true) | (OrderDirection::Descending, false) => {
                    left.max(right)
                }
                (OrderDirection::Ascending, false) | (OrderDirection::Descending, true) => {
                    left.min(right)
                }
            }),
        }
    }

    fn unit_replace_effect_planned(
        &self,
        removed: i64,
        removed_after: usize,
        inserted: i64,
        inserted_after: usize,
        old_threshold: Option<i64>,
        new_threshold: i64,
    ) -> Result<Vec<(i64, i64)>, I64TopKError> {
        let mut candidates = [
            removed,
            inserted,
            old_threshold.unwrap_or(new_threshold),
            new_threshold,
        ];
        candidates.sort_unstable();
        let mut effect = Vec::with_capacity(candidates.len());
        let mut previous = None;
        for key in candidates {
            if previous == Some(key) {
                continue;
            }
            previous = Some(key);
            let old_count = self.counts.count(key);
            let new_count = self.count_after_unit_replace(
                key,
                removed,
                removed_after,
                inserted,
                inserted_after,
            );
            let before = if old_threshold
                .is_some_and(|threshold| self.selected_by_threshold(key, threshold))
            {
                i128::try_from(old_count).expect("usize fits i128")
            } else {
                0
            };
            let after = if self.selected_by_threshold(key, new_threshold) {
                i128::try_from(new_count).expect("usize fits i128")
            } else {
                0
            };
            let delta = after - before;
            if delta != 0 {
                effect.push((
                    key,
                    i64::try_from(delta).map_err(|_| I64TopKError::CountOverflow)?,
                ));
            }
        }
        Ok(effect)
    }

    fn apply_general(&mut self, changes: &BTreeMap<i64, i64>) -> Result<(), I64TopKError> {
        let mut next_total = i128::try_from(self.total_rows).expect("usize fits i128");
        for (&key, &change) in changes {
            let current = self.counts.count(key);
            let next = i128::try_from(current).expect("usize fits i128") + i128::from(change);
            if next < 0 {
                return Err(I64TopKError::MissingKey);
            }
            let next = usize::try_from(next).map_err(|_| I64TopKError::CountOverflow)?;
            if self.is_set && next > 1 {
                return Err(I64TopKError::DuplicateSetKey);
            }
            next_total = next_total
                .checked_add(i128::from(change))
                .ok_or(I64TopKError::CountOverflow)?;
            self.counts.set(key, next);
        }
        self.total_rows = usize::try_from(next_total).map_err(|_| I64TopKError::CountOverflow)?;
        self.recompute_boundary();
        Ok(())
    }

    fn apply_unit_replace(
        &mut self,
        removed: i64,
        inserted: i64,
    ) -> Result<(Vec<(i64, i64)>, usize), I64TopKError> {
        let removed_before = self.counts.count(removed);
        if removed_before == 0 {
            return Err(I64TopKError::MissingKey);
        }
        if removed == inserted {
            return Ok((Vec::new(), 0));
        }
        let inserted_before = self.counts.count(inserted);
        if self.is_set && inserted_before != 0 {
            return Err(I64TopKError::DuplicateSetKey);
        }
        let old_threshold = self.threshold;
        let Some(old_threshold_key) = old_threshold else {
            self.counts.set(removed, removed_before - 1);
            self.counts.set(inserted, inserted_before + 1);
            self.recompute_boundary();
            return Ok((Vec::new(), 0));
        };

        if self.is_better(removed, old_threshold_key) {
            self.better_rows = self
                .better_rows
                .checked_sub(1)
                .ok_or(I64TopKError::CountOverflow)?;
        }
        if self.is_better(inserted, old_threshold_key) {
            self.better_rows = self
                .better_rows
                .checked_add(1)
                .ok_or(I64TopKError::CountOverflow)?;
        }
        self.counts.set(removed, removed_before - 1);
        self.counts.set(
            inserted,
            inserted_before
                .checked_add(1)
                .ok_or(I64TopKError::CountOverflow)?,
        );

        let effective_k = self.k.min(self.total_rows);
        if effective_k == 0 {
            self.threshold = None;
            self.threshold_count = 0;
            self.better_rows = 0;
            return Ok((Vec::new(), 0));
        }
        let mut threshold = old_threshold_key;
        let mut threshold_count = self.counts.count(threshold);
        let mut steps = 0;
        if self.better_rows >= effective_k {
            let predecessor = self
                .counts
                .predecessor(threshold, self.direction)
                .ok_or(I64TopKError::CountOverflow)?;
            let predecessor_count = self.counts.count(predecessor);
            self.better_rows = self
                .better_rows
                .checked_sub(predecessor_count)
                .ok_or(I64TopKError::CountOverflow)?;
            threshold = predecessor;
            threshold_count = predecessor_count;
            steps = 1;
        } else if self.better_rows + threshold_count < effective_k {
            let successor = self
                .counts
                .successor(threshold, self.direction)
                .ok_or(I64TopKError::CountOverflow)?;
            self.better_rows = self
                .better_rows
                .checked_add(threshold_count)
                .ok_or(I64TopKError::CountOverflow)?;
            threshold = successor;
            threshold_count = self.counts.count(successor);
            steps = 1;
        }
        self.threshold = Some(threshold);
        self.threshold_count = threshold_count;

        Ok((
            self.unit_replace_effect(removed, inserted, old_threshold, threshold)?,
            steps,
        ))
    }

    fn unit_replace_effect(
        &self,
        removed: i64,
        inserted: i64,
        old_threshold: Option<i64>,
        threshold: i64,
    ) -> Result<Vec<(i64, i64)>, I64TopKError> {
        let candidates = BTreeSet::from([
            removed,
            inserted,
            old_threshold.unwrap_or(threshold),
            threshold,
        ]);
        let mut effect = Vec::with_capacity(candidates.len());
        for key in candidates {
            let new_count = self.counts.count(key);
            let signed =
                i128::from(i64::from(key == inserted)) - i128::from(i64::from(key == removed));
            let old_count = i128::try_from(new_count).expect("usize fits i128") - signed;
            let old_selected =
                old_threshold.is_some_and(|old| self.selected_by_threshold(key, old));
            let new_selected = self.selected_by_threshold(key, threshold);
            let before = if old_selected { old_count } else { 0 };
            let after = if new_selected {
                i128::try_from(new_count).expect("usize fits i128")
            } else {
                0
            };
            let delta = after - before;
            if delta != 0 {
                effect.push((
                    key,
                    i64::try_from(delta).map_err(|_| I64TopKError::CountOverflow)?,
                ));
            }
        }
        Ok(effect)
    }

    fn recompute_boundary(&mut self) {
        let effective_k = self.k.min(self.total_rows);
        if effective_k == 0 {
            self.threshold = None;
            self.threshold_count = 0;
            self.better_rows = 0;
            return;
        }
        let mut better_rows = 0_usize;
        for (key, count) in self.counts.ordered_entries(self.direction) {
            if better_rows.saturating_add(count) >= effective_k {
                self.threshold = Some(key);
                self.threshold_count = count;
                self.better_rows = better_rows;
                return;
            }
            better_rows = better_rows.saturating_add(count);
        }
        unreachable!("positive effective k must have a threshold");
    }

    fn selected_by_threshold(&self, key: i64, threshold: i64) -> bool {
        match self.direction {
            OrderDirection::Ascending => key <= threshold,
            OrderDirection::Descending => key >= threshold,
        }
    }

    fn is_better(&self, key: i64, threshold: i64) -> bool {
        match self.direction {
            OrderDirection::Ascending => key < threshold,
            OrderDirection::Descending => key > threshold,
        }
    }

    fn is_worse(&self, key: i64, threshold: i64) -> bool {
        match self.direction {
            OrderDirection::Ascending => key > threshold,
            OrderDirection::Descending => key < threshold,
        }
    }
}

fn selected_effect(
    before: &BTreeMap<i64, usize>,
    after: &BTreeMap<i64, usize>,
) -> Result<Vec<(i64, i64)>, I64TopKError> {
    let mut keys = BTreeSet::new();
    keys.extend(before.keys().copied());
    keys.extend(after.keys().copied());
    let mut effect = Vec::new();
    for key in keys {
        let old = i128::try_from(before.get(&key).copied().unwrap_or(0)).expect("usize fits i128");
        let new = i128::try_from(after.get(&key).copied().unwrap_or(0)).expect("usize fits i128");
        let delta = new - old;
        if delta != 0 {
            effect.push((
                key,
                i64::try_from(delta).map_err(|_| I64TopKError::CountOverflow)?,
            ));
        }
    }
    Ok(effect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oracle(
        counts: &BTreeMap<i64, usize>,
        k: usize,
        direction: OrderDirection,
    ) -> BTreeMap<i64, usize> {
        let mut out = BTreeMap::new();
        let mut rows = 0;
        let iter: Box<dyn Iterator<Item = (&i64, &usize)>> = match direction {
            OrderDirection::Ascending => Box::new(counts.iter()),
            OrderDirection::Descending => Box::new(counts.iter().rev()),
        };
        for (&key, &count) in iter {
            out.insert(key, count);
            rows += count;
            if rows >= k {
                break;
            }
        }
        out
    }

    #[test]
    fn dense_unit_promotes_and_window_escape_falls_back_atomically() {
        let counts = (-32..32).map(|key| (key, 1)).collect::<BTreeMap<_, _>>();
        let state = I64TopKState::build(&counts, 7, OrderDirection::Ascending, false).unwrap();
        assert_eq!(state.kind(), I64TopKPhysicalKind::DenseUnit);
        let duplicate = BTreeMap::from([(-32, -1), (0, 1)]);
        let plan = state.plan_signed(&duplicate).unwrap();
        assert_eq!(state.kind(), I64TopKPhysicalKind::DenseUnit);
        let mut promoted = state.clone();
        promoted.commit_patch(plan.patch);
        assert_eq!(promoted.kind(), I64TopKPhysicalKind::DenseCounted);
        let outlier = BTreeMap::from([(-32, -1), (10_000, 1)]);
        let plan = state.plan_signed(&outlier).unwrap();
        assert_eq!(state.kind(), I64TopKPhysicalKind::DenseUnit);
        let mut promoted = state.clone();
        promoted.commit_patch(plan.patch);
        assert_eq!(promoted.kind(), I64TopKPhysicalKind::PagedRadix);
    }

    #[test]
    fn unit_threshold_repair_matches_oracle_in_both_directions() {
        for direction in [OrderDirection::Ascending, OrderDirection::Descending] {
            let mut counts = (-48..48).map(|key| (key, 1)).collect::<BTreeMap<_, _>>();
            let mut state = I64TopKState::build(&counts, 11, direction, false).unwrap();
            for step in 0..2000_i64 {
                let removed = *counts
                    .keys()
                    .nth((usize::try_from(step).expect("non-negative step") * 17) % counts.len())
                    .unwrap();
                let mut inserted = ((step * 37) % 160) - 80;
                while counts.contains_key(&inserted) && inserted != removed {
                    inserted += 1;
                }
                let mut change = BTreeMap::new();
                *change.entry(removed).or_default() -= 1;
                *change.entry(inserted).or_default() += 1;
                change.retain(|_, weight| *weight != 0);
                let plan = state.plan_signed(&change).unwrap();
                assert!(plan.threshold_steps <= 1);
                counts.remove(&removed);
                *counts.entry(inserted).or_default() += 1;
                state.commit_patch(plan.patch);
                assert_eq!(state.selected_counts(), oracle(&counts, 11, direction));
            }
        }
    }
}
