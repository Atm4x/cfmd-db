use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock, Weak},
};

use super::{BRANCH, PAGE};

#[derive(Debug, Clone)]
enum Node<T> {
    Branch(Vec<Option<Arc<Node<T>>>>),
    Leaf(Vec<T>),
}

/// Persistent, page-copying vector with stable O(1) snapshots.
///
/// Mutation copies only the radix path and the touched fixed-size page. The
/// contiguous slice view is a compatibility projection materialized lazily.
#[derive(Debug)]
pub struct PersistentVec<T> {
    root: Arc<Node<T>>,
    levels: usize,
    len: usize,
    contiguous_cache: OnceLock<Vec<T>>,
}

/// Weak diagnostic probe for persistent-vector radix nodes unique to one
/// snapshot relative to another. The probe itself does not retain storage.
#[derive(Debug)]
pub struct PersistentVecStorageProbe<T> {
    nodes: Vec<Weak<Node<T>>>,
}

impl<T> PersistentVecStorageProbe<T> {
    #[must_use]
    pub fn total_nodes(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn live_nodes(&self) -> usize {
        self.nodes.iter().filter(|node| node.strong_count() != 0).count()
    }

    #[must_use]
    pub fn is_fully_reclaimed(&self) -> bool {
        self.live_nodes() == 0
    }
}

impl<T> Clone for PersistentVec<T> {
    fn clone(&self) -> Self {
        Self {
            root: Arc::clone(&self.root),
            levels: self.levels,
            len: self.len,
            contiguous_cache: OnceLock::new(),
        }
    }
}

impl<T> Default for PersistentVec<T> {
    fn default() -> Self {
        Self {
            root: Arc::new(Node::Branch(vec![None; BRANCH])),
            levels: 1,
            len: 0,
            contiguous_cache: OnceLock::new(),
        }
    }
}

impl<T: Clone> From<Vec<T>> for PersistentVec<T> {
    fn from(values: Vec<T>) -> Self {
        Self::from_vec(values)
    }
}

impl<T: Clone> FromIterator<T> for PersistentVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from_vec(iter.into_iter().collect())
    }
}

impl<T: Clone> PersistentVec<T> {
    #[must_use]
    pub fn from_vec(values: Vec<T>) -> Self {
        let mut out = Self::default();
        let mut page = Vec::with_capacity(PAGE);
        for value in values {
            page.push(value);
            if page.len() == PAGE {
                out.append_page(std::mem::take(&mut page));
                page = Vec::with_capacity(PAGE);
            }
        }
        if !page.is_empty() {
            out.append_page(page);
        }
        out
    }

    pub fn push(&mut self, value: T) {
        self.clear_cache();
        let page_index = self.len / PAGE;
        let offset = self.len % PAGE;
        self.ensure_page_capacity(page_index);
        let mut page = if offset == 0 {
            Vec::with_capacity(PAGE)
        } else {
            self.page(page_index)
                .expect("existing persistent page")
                .clone()
        };
        page.push(value);
        self.root = set_page(&self.root, self.levels, page_index, Some(page));
        self.len += 1;
    }

    pub fn remove(&mut self, index: usize) -> T {
        self.clear_cache();
        assert!(index < self.len, "persistent vector index out of bounds");

        let removed = self[index].clone();
        let first_page = index / PAGE;
        let first_offset = index % PAGE;
        let last_page = (self.len - 1) / PAGE;

        for page_index in first_page..=last_page {
            let mut page = self
                .page(page_index)
                .expect("existing persistent page")
                .clone();
            if page_index == first_page {
                page.remove(first_offset);
            } else {
                page.remove(0);
            }
            if page_index < last_page {
                let successor = self
                    .page(page_index + 1)
                    .and_then(|next| next.first())
                    .expect("successor persistent page")
                    .clone();
                page.push(successor);
            }
            self.root = set_page(
                &self.root,
                self.levels,
                page_index,
                (!page.is_empty()).then_some(page),
            );
        }
        self.len -= 1;
        removed
    }

    pub fn swap(&mut self, left: usize, right: usize) {
        assert!(
            left < self.len && right < self.len,
            "persistent vector index out of bounds"
        );
        if left == right {
            return;
        }
        self.clear_cache();

        let left_page_index = left / PAGE;
        let right_page_index = right / PAGE;
        let left_offset = left % PAGE;
        let right_offset = right % PAGE;
        if left_page_index == right_page_index {
            let mut page = self
                .page(left_page_index)
                .expect("existing persistent page")
                .clone();
            page.swap(left_offset, right_offset);
            self.root = set_page(&self.root, self.levels, left_page_index, Some(page));
            return;
        }

        let mut left_page = self
            .page(left_page_index)
            .expect("existing persistent page")
            .clone();
        let mut right_page = self
            .page(right_page_index)
            .expect("existing persistent page")
            .clone();
        std::mem::swap(&mut left_page[left_offset], &mut right_page[right_offset]);
        self.root = set_page(&self.root, self.levels, left_page_index, Some(left_page));
        self.root = set_page(&self.root, self.levels, right_page_index, Some(right_page));
    }

    pub fn swap_remove(&mut self, index: usize) -> T {
        self.clear_cache();
        assert!(index < self.len, "persistent vector index out of bounds");
        let last_index = self.len - 1;
        if index == last_index {
            return self.pop_last();
        }

        let removed = self[index].clone();
        let page_index = index / PAGE;
        let last_page_index = last_index / PAGE;
        if page_index == last_page_index {
            let mut page = self
                .page(page_index)
                .expect("existing persistent page")
                .clone();
            let last = page.pop().expect("non-empty persistent page");
            page[index % PAGE] = last;
            self.root = set_page(&self.root, self.levels, page_index, Some(page));
            self.len -= 1;
            return removed;
        }

        let last = self[last_index].clone();
        self.set(index, last);
        self.pop_last();
        removed
    }

    pub fn set(&mut self, index: usize, value: T) {
        self.clear_cache();
        assert!(index < self.len, "persistent vector index out of bounds");
        let page_index = index / PAGE;
        let offset = index % PAGE;
        let mut page = self
            .page(page_index)
            .expect("existing persistent page")
            .clone();
        page[offset] = value;
        self.root = set_page(&self.root, self.levels, page_index, Some(page));
    }

    pub fn pop(&mut self) -> Option<T> {
        (!self.is_empty()).then(|| self.pop_last())
    }

    pub fn resize(&mut self, new_len: usize, value: T) {
        self.resize_with(new_len, || value.clone());
    }

    pub fn resize_with<F>(&mut self, new_len: usize, mut f: F)
    where
        F: FnMut() -> T,
    {
        if new_len <= self.len {
            self.truncate(new_len);
            return;
        }

        self.clear_cache();
        if !self.len.is_multiple_of(PAGE) {
            let page_index = self.len / PAGE;
            let mut page = self
                .page(page_index)
                .expect("existing persistent page")
                .clone();
            let added = (PAGE - page.len()).min(new_len - self.len);
            page.extend((0..added).map(|_| f()));
            self.root = set_page(&self.root, self.levels, page_index, Some(page));
            self.len += added;
        }

        while self.len < new_len {
            let added = PAGE.min(new_len - self.len);
            let mut page = Vec::with_capacity(PAGE);
            page.extend((0..added).map(|_| f()));
            self.append_page(page);
        }
    }

    pub fn as_slice(&self) -> &[T] {
        self.contiguous_cache
            .get_or_init(|| self.iter().cloned().collect())
            .as_slice()
    }

    fn truncate(&mut self, new_len: usize) {
        if new_len >= self.len {
            return;
        }
        self.clear_cache();
        if new_len == 0 {
            self.root = Arc::new(Node::Branch(vec![None; BRANCH]));
            self.levels = 1;
            self.len = 0;
            return;
        }

        let old_last_page = (self.len - 1) / PAGE;
        let kept_last_page = (new_len - 1) / PAGE;
        let kept_tail_len = new_len % PAGE;
        if kept_tail_len != 0 {
            let mut page = self
                .page(kept_last_page)
                .expect("existing persistent page")
                .clone();
            page.truncate(kept_tail_len);
            self.root = set_page(&self.root, self.levels, kept_last_page, Some(page));
        }
        for page_index in kept_last_page + 1..=old_last_page {
            self.root = set_page(&self.root, self.levels, page_index, None);
        }
        self.len = new_len;
    }

    fn pop_last(&mut self) -> T {
        self.clear_cache();
        assert!(self.len > 0, "cannot pop empty persistent vector");
        let last_index = self.len - 1;
        let page_index = last_index / PAGE;
        let mut page = self
            .page(page_index)
            .expect("existing persistent page")
            .clone();
        let value = page.pop().expect("non-empty persistent page");
        self.root = set_page(
            &self.root,
            self.levels,
            page_index,
            (!page.is_empty()).then_some(page),
        );
        self.len -= 1;
        value
    }

    fn append_page(&mut self, page: Vec<T>) {
        self.clear_cache();
        debug_assert!(!page.is_empty() && page.len() <= PAGE);
        debug_assert_eq!(self.len % PAGE, 0);
        let page_index = self.len / PAGE;
        self.ensure_page_capacity(page_index);
        self.len += page.len();
        self.root = set_page(&self.root, self.levels, page_index, Some(page));
    }

    fn ensure_page_capacity(&mut self, page_index: usize) {
        while page_index >= page_capacity(self.levels) {
            let mut children = vec![None; BRANCH];
            children[0] = Some(Arc::clone(&self.root));
            self.root = Arc::new(Node::Branch(children));
            self.levels += 1;
        }
    }

    fn clear_cache(&mut self) {
        self.contiguous_cache = OnceLock::new();
    }
}

impl<T> PersistentVec<T> {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.len.div_ceil(PAGE) * PAGE
    }

    #[must_use]
    pub fn first(&self) -> Option<&T> {
        self.get(0)
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        self.page(index / PAGE)
            .and_then(|page| page.get(index % PAGE))
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T>
    where
        T: Clone,
    {
        if index >= self.len {
            return None;
        }
        self.clear_cache();
        value_mut(
            Arc::make_mut(&mut self.root),
            self.levels,
            index / PAGE,
            index % PAGE,
        )
    }

    #[must_use]
    pub fn iter(&self) -> PersistentVecIter<'_, T> {
        PersistentVecIter {
            values: self,
            index: 0,
        }
    }

    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.root, &other.root)
    }

    #[must_use]
    pub fn shares_page_with(&self, other: &Self, index: usize) -> bool {
        if index >= self.len || index >= other.len {
            return false;
        }
        let page_index = index / PAGE;
        match (
            leaf_arc(&self.root, self.levels, page_index),
            leaf_arc(&other.root, other.levels, page_index),
        ) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Number of radix/leaf nodes structurally reachable from this root.
    #[must_use]
    pub fn structural_node_count(&self) -> usize {
        fn count<T>(node: &Arc<Node<T>>) -> usize {
            match node.as_ref() {
                Node::Leaf(_) => 1,
                Node::Branch(children) => {
                    1 + children
                        .iter()
                        .filter_map(Option::as_ref)
                        .map(count)
                        .sum::<usize>()
                }
            }
        }
        count(&self.root)
    }

    /// Exact pointer-identity sharing count for diagnostic retention tests.
    #[must_use]
    pub fn shared_structural_node_count_with(&self, other: &Self) -> usize {
        fn collect<T>(node: &Arc<Node<T>>, ids: &mut BTreeSet<usize>) {
            ids.insert(Arc::as_ptr(node) as usize);
            if let Node::Branch(children) = node.as_ref() {
                for child in children.iter().filter_map(Option::as_ref) {
                    collect(child, ids);
                }
            }
        }
        fn count_shared<T>(node: &Arc<Node<T>>, ids: &BTreeSet<usize>) -> usize {
            let own = usize::from(ids.contains(&(Arc::as_ptr(node) as usize)));
            own + match node.as_ref() {
                Node::Leaf(_) => 0,
                Node::Branch(children) => children
                    .iter()
                    .filter_map(Option::as_ref)
                    .map(|child| count_shared(child, ids))
                    .sum(),
            }
        }

        let mut ids = BTreeSet::new();
        collect(&self.root, &mut ids);
        count_shared(&other.root, &ids)
    }

    /// Weakly probes radix/leaf nodes owned by this snapshot but not shared
    /// with `other`.
    #[must_use]
    pub fn unique_storage_probe_against(&self, other: &Self) -> PersistentVecStorageProbe<T> {
        fn collect_ids<T>(node: &Arc<Node<T>>, ids: &mut BTreeSet<usize>) {
            ids.insert(Arc::as_ptr(node) as usize);
            if let Node::Branch(children) = node.as_ref() {
                for child in children.iter().filter_map(Option::as_ref) {
                    collect_ids(child, ids);
                }
            }
        }
        fn collect_unique<T>(
            node: &Arc<Node<T>>,
            shared: &BTreeSet<usize>,
            into: &mut Vec<Weak<Node<T>>>,
        ) {
            if !shared.contains(&(Arc::as_ptr(node) as usize)) {
                into.push(Arc::downgrade(node));
            }
            if let Node::Branch(children) = node.as_ref() {
                for child in children.iter().filter_map(Option::as_ref) {
                    collect_unique(child, shared, into);
                }
            }
        }

        let mut shared = BTreeSet::new();
        collect_ids(&other.root, &mut shared);
        let mut nodes = Vec::new();
        collect_unique(&self.root, &shared, &mut nodes);
        PersistentVecStorageProbe { nodes }
    }

    /// Returns the logical indexes whose values differ between two snapshots.
    /// Shared radix subtrees are skipped by pointer identity, so a sparse
    /// path-copy mutation only visits the changed branches/pages.
    #[must_use]
    pub fn changed_indices(&self, other: &Self) -> Vec<usize>
    where
        T: PartialEq,
    {
        let common_len = self.len.min(other.len);
        let mut changed = Vec::new();
        let aligned = match self.levels.cmp(&other.levels) {
            std::cmp::Ordering::Equal => Some((&self.root, &other.root, self.levels)),
            std::cmp::Ordering::Less => zero_subtree(&other.root, other.levels, self.levels)
                .map(|right| (&self.root, right, self.levels)),
            std::cmp::Ordering::Greater => zero_subtree(&self.root, self.levels, other.levels)
                .map(|left| (left, &other.root, other.levels)),
        };
        if let Some((left, right, levels)) = aligned {
            collect_changed_indices(left, right, levels, 0, common_len, &mut changed);
        } else {
            changed.extend((0..common_len).filter(|&index| self[index] != other[index]));
        }
        changed.extend(common_len..self.len.max(other.len));
        changed
    }

    #[must_use]
    pub fn estimated_heap_bytes(&self) -> usize {
        let element_bytes = self.len.saturating_mul(std::mem::size_of::<T>());
        let pages = self.len.div_ceil(PAGE);
        element_bytes.saturating_add(pages.saturating_mul(std::mem::size_of::<Arc<Node<T>>>() * 2))
    }

    fn page(&self, page_index: usize) -> Option<&Vec<T>> {
        page(&self.root, self.levels, page_index)
    }
}

fn zero_subtree<T>(
    root: &Arc<Node<T>>,
    mut levels: usize,
    target_levels: usize,
) -> Option<&Arc<Node<T>>> {
    let mut current = root;
    while levels > target_levels {
        let Node::Branch(children) = current.as_ref() else {
            return None;
        };
        current = children.first()?.as_ref()?;
        levels -= 1;
    }
    Some(current)
}

fn collect_changed_indices<T: PartialEq>(
    left: &Arc<Node<T>>,
    right: &Arc<Node<T>>,
    levels: usize,
    base_page: usize,
    common_len: usize,
    changed: &mut Vec<usize>,
) {
    if Arc::ptr_eq(left, right) {
        return;
    }
    let (Node::Branch(left_children), Node::Branch(right_children)) =
        (left.as_ref(), right.as_ref())
    else {
        return;
    };
    let pages_per_child = page_capacity(levels.saturating_sub(1));
    for child_index in 0..BRANCH {
        let child_base_page = base_page + child_index * pages_per_child;
        let child_base_index = child_base_page * PAGE;
        if child_base_index >= common_len {
            break;
        }
        match (
            left_children[child_index].as_ref(),
            right_children[child_index].as_ref(),
        ) {
            (Some(left_child), Some(right_child)) if Arc::ptr_eq(left_child, right_child) => {}
            (Some(left_child), Some(right_child)) if levels == 1 => {
                let (Node::Leaf(left_page), Node::Leaf(right_page)) =
                    (left_child.as_ref(), right_child.as_ref())
                else {
                    continue;
                };
                let page_len = left_page
                    .len()
                    .min(right_page.len())
                    .min(common_len - child_base_index);
                changed.extend(
                    (0..page_len)
                        .filter(|&offset| left_page[offset] != right_page[offset])
                        .map(|offset| child_base_index + offset),
                );
            }
            (Some(left_child), Some(right_child)) => collect_changed_indices(
                left_child,
                right_child,
                levels - 1,
                child_base_page,
                common_len,
                changed,
            ),
            _ => {
                let end = common_len.min(child_base_index + pages_per_child * PAGE);
                changed.extend(child_base_index..end);
            }
        }
    }
}

impl<T: Clone> std::ops::Deref for PersistentVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

pub struct PersistentVecIter<'a, T> {
    values: &'a PersistentVec<T>,
    index: usize,
}

impl<'a, T> Iterator for PersistentVecIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.values.get(self.index)?;
        self.index += 1;
        Some(value)
    }
}

impl<'a, T> IntoIterator for &'a PersistentVec<T> {
    type Item = &'a T;
    type IntoIter = PersistentVecIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: PartialEq> PartialEq for PersistentVec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

impl<T: Eq> Eq for PersistentVec<T> {}

impl<T> std::ops::Index<usize> for PersistentVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index)
            .expect("persistent vector index out of bounds")
    }
}

impl<T: Clone> std::ops::IndexMut<usize> for PersistentVec<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index)
            .expect("persistent vector index out of bounds")
    }
}

fn page_capacity(levels: usize) -> usize {
    (0..levels).fold(1_usize, |capacity, _| capacity.saturating_mul(BRANCH))
}

fn page<T>(node: &Node<T>, levels: usize, page_index: usize) -> Option<&Vec<T>> {
    let Node::Branch(children) = node else {
        return None;
    };
    let divisor = page_capacity(levels.saturating_sub(1));
    let child_index = (page_index / divisor) % BRANCH;
    let child = children.get(child_index)?.as_deref()?;
    if levels == 1 {
        let Node::Leaf(page) = child else { return None };
        Some(page)
    } else {
        page(child, levels - 1, page_index % divisor)
    }
}

fn leaf_arc<T>(node: &Arc<Node<T>>, levels: usize, page_index: usize) -> Option<&Arc<Node<T>>> {
    let Node::Branch(children) = node.as_ref() else {
        return None;
    };
    let divisor = page_capacity(levels.saturating_sub(1));
    let child_index = (page_index / divisor) % BRANCH;
    let child = children.get(child_index)?.as_ref()?;
    if levels == 1 {
        matches!(child.as_ref(), Node::Leaf(_)).then_some(child)
    } else {
        leaf_arc(child, levels - 1, page_index % divisor)
    }
}

fn value_mut<T: Clone>(
    node: &mut Node<T>,
    levels: usize,
    page_index: usize,
    offset: usize,
) -> Option<&mut T> {
    let Node::Branch(children) = node else {
        return None;
    };
    let divisor = page_capacity(levels.saturating_sub(1));
    let child_index = (page_index / divisor) % BRANCH;
    let child = Arc::make_mut(children.get_mut(child_index)?.as_mut()?);
    if levels == 1 {
        let Node::Leaf(page) = child else { return None };
        page.get_mut(offset)
    } else {
        value_mut(child, levels - 1, page_index % divisor, offset)
    }
}

fn set_page<T>(
    node: &Arc<Node<T>>,
    levels: usize,
    page_index: usize,
    page: Option<Vec<T>>,
) -> Arc<Node<T>> {
    let Node::Branch(existing) = node.as_ref() else {
        unreachable!("persistent vector root must be a branch")
    };
    let mut children = existing.clone();
    let divisor = page_capacity(levels.saturating_sub(1));
    let child_index = (page_index / divisor) % BRANCH;
    if levels == 1 {
        children[child_index] = page.map(|page| Arc::new(Node::Leaf(page)));
    } else {
        let child = children[child_index]
            .clone()
            .unwrap_or_else(|| Arc::new(Node::Branch(vec![None; BRANCH])));
        children[child_index] = Some(set_page(&child, levels - 1, page_index % divisor, page));
    }
    Arc::new(Node::Branch(children))
}
