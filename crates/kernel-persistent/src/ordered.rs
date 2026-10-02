use std::{
    borrow::Borrow,
    cmp::Ordering,
    collections::BTreeSet,
    sync::{Arc, Weak},
};

#[derive(Debug, Clone)]
pub(super) struct MapNode<K, V> {
    key: K,
    value: V,
    pub(super) height: u16,
    left: Option<Arc<MapNode<K, V>>>,
    right: Option<Arc<MapNode<K, V>>>,
}

#[derive(Debug, Clone)]
pub struct PersistentOrdMap<K, V> {
    pub(super) root: Option<Arc<MapNode<K, V>>>,
    len: usize,
}

/// Weak diagnostic probe for structural nodes that belong exclusively to one
/// persistent-map snapshot relative to another snapshot.
///
/// The probe never keeps storage alive. It exists so kernel-level retention
/// tests can prove that pinned immutable roots retain only path-copied nodes
/// and that those nodes are reclaimed after the last owning root is dropped.
#[derive(Debug)]
pub struct PersistentOrdMapStorageProbe<K, V> {
    nodes: Vec<Weak<MapNode<K, V>>>,
}

impl<K, V> PersistentOrdMapStorageProbe<K, V> {
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

impl<K, V> Default for PersistentOrdMap<K, V> {
    fn default() -> Self {
        Self { root: None, len: 0 }
    }
}

impl<K: Ord + Clone, V: Clone> PersistentOrdMap<K, V> {
    #[must_use]
    pub fn from_sorted_unique(entries: Vec<(K, V)>) -> Option<Self> {
        if entries
            .windows(2)
            .any(|pair| pair[0].0 >= pair[1].0)
        {
            return None;
        }
        fn build<K: Clone, V: Clone>(entries: &[(K, V)]) -> Option<Arc<MapNode<K, V>>> {
            if entries.is_empty() {
                return None;
            }
            let mid = entries.len() / 2;
            let left = build(&entries[..mid]);
            let right = build(&entries[mid + 1..]);
            Some(map_node(
                entries[mid].0.clone(),
                entries[mid].1.clone(),
                left,
                right,
            ))
        }
        let len = entries.len();
        Some(Self {
            root: build(&entries),
            len,
        })
    }

    #[must_use]
    pub fn from_sorted_unique_owned(entries: Vec<(K, V)>) -> Option<Self> {
        if entries
            .windows(2)
            .any(|pair| pair[0].0 >= pair[1].0)
        {
            return None;
        }
        fn build<K, V>(
            entries: &mut std::vec::IntoIter<(K, V)>,
            len: usize,
        ) -> Option<Arc<MapNode<K, V>>> {
            if len == 0 {
                return None;
            }
            let left_len = len / 2;
            let left = build(entries, left_len);
            let (key, value) = entries.next().expect("validated owned bulk map length");
            let right = build(entries, len - left_len - 1);
            Some(map_node(key, value, left, right))
        }
        let len = entries.len();
        let mut entries = entries.into_iter();
        Some(Self {
            root: build(&mut entries, len),
            len,
        })
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let (root, replaced) = map_insert(self.root.as_ref(), key, value);
        self.root = Some(root);
        if replaced.is_none() {
            self.len = self.len.saturating_add(1);
        }
        replaced
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (root, removed) = map_remove(self.root.as_ref(), key);
        if removed.is_some() {
            self.root = root;
            self.len = self.len.saturating_sub(1);
        }
        removed
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        let (root, removed) = map_retain(self.root.as_ref(), &mut keep);
        if removed == 0 {
            return;
        }
        self.root = root;
        self.len = self.len.saturating_sub(removed);
    }

    #[must_use]
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let mut current = self.root.as_deref();
        while let Some(node) = current {
            match key.cmp(node.key.borrow()) {
                Ordering::Less => current = node.left.as_deref(),
                Ordering::Greater => current = node.right.as_deref(),
                Ordering::Equal => return Some(&node.value),
            }
        }
        None
    }

    #[must_use]
    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &V)>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let mut current = self.root.as_deref();
        while let Some(node) = current {
            match key.cmp(node.key.borrow()) {
                Ordering::Less => current = node.left.as_deref(),
                Ordering::Greater => current = node.right.as_deref(),
                Ordering::Equal => return Some((&node.key, &node.value)),
            }
        }
        None
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        map_get_mut(&mut self.root, key)
    }

    #[must_use]
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.get(key).is_some()
    }

    #[must_use]
    pub fn predecessor(&self, key: &K) -> Option<(&K, &V)> {
        let mut current = self.root.as_deref();
        let mut candidate = None;
        while let Some(node) = current {
            match key.cmp(&node.key) {
                Ordering::Less | Ordering::Equal => current = node.left.as_deref(),
                Ordering::Greater => {
                    candidate = Some((&node.key, &node.value));
                    current = node.right.as_deref();
                }
            }
        }
        candidate
    }

    #[must_use]
    pub fn successor(&self, key: &K) -> Option<(&K, &V)> {
        let mut current = self.root.as_deref();
        let mut candidate = None;
        while let Some(node) = current {
            match key.cmp(&node.key) {
                Ordering::Less => {
                    candidate = Some((&node.key, &node.value));
                    current = node.left.as_deref();
                }
                Ordering::Equal | Ordering::Greater => current = node.right.as_deref(),
            }
        }
        candidate
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn estimated_heap_bytes(&self) -> usize {
        self.len
            .saturating_mul(std::mem::size_of::<MapNode<K, V>>())
    }

    #[must_use]
    pub fn iter(&self) -> PersistentOrdMapIter<'_, K, V> {
        PersistentOrdMapIter::new(self.root.as_deref(), self.len)
    }

    #[must_use]
    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &K> {
        self.iter().map(|(key, _)| key)
    }

    #[must_use]
    pub fn values(&self) -> impl DoubleEndedIterator<Item = &V> {
        self.iter().map(|(_, value)| value)
    }

    #[must_use]
    pub fn shares_root_with(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Number of structural AVL nodes reachable from this immutable root.
    /// This intentionally excludes heap owned by `K`/`V`; it is a path-copy
    /// retention metric rather than a general allocator estimate.
    #[must_use]
    pub fn structural_node_count(&self) -> usize {
        fn count<K, V>(node: Option<&Arc<MapNode<K, V>>>) -> usize {
            let Some(node) = node else { return 0 };
            1 + count(node.left.as_ref()) + count(node.right.as_ref())
        }
        count(self.root.as_ref())
    }

    /// Exact count of structural nodes shared by pointer identity with
    /// `other`. The traversal is diagnostic-only and therefore deliberately
    /// O(N); hot-path code must use ordinary persistent operations instead.
    #[must_use]
    pub fn shared_structural_node_count_with(&self, other: &Self) -> usize {
        fn collect<K, V>(node: Option<&Arc<MapNode<K, V>>>, ids: &mut BTreeSet<usize>) {
            let Some(node) = node else { return };
            ids.insert(Arc::as_ptr(node) as usize);
            collect(node.left.as_ref(), ids);
            collect(node.right.as_ref(), ids);
        }
        fn count_shared<K, V>(
            node: Option<&Arc<MapNode<K, V>>>,
            ids: &BTreeSet<usize>,
        ) -> usize {
            let Some(node) = node else { return 0 };
            usize::from(ids.contains(&(Arc::as_ptr(node) as usize)))
                + count_shared(node.left.as_ref(), ids)
                + count_shared(node.right.as_ref(), ids)
        }

        let mut ids = BTreeSet::new();
        collect(self.root.as_ref(), &mut ids);
        count_shared(other.root.as_ref(), &ids)
    }

    /// Weakly probes nodes reachable from this map but not shared with
    /// `other`. Dropping this snapshot must reclaim every probed node unless a
    /// third immutable root also owns it.
    #[must_use]
    pub fn unique_storage_probe_against(
        &self,
        other: &Self,
    ) -> PersistentOrdMapStorageProbe<K, V> {
        fn collect_ids<K, V>(node: Option<&Arc<MapNode<K, V>>>, ids: &mut BTreeSet<usize>) {
            let Some(node) = node else { return };
            ids.insert(Arc::as_ptr(node) as usize);
            collect_ids(node.left.as_ref(), ids);
            collect_ids(node.right.as_ref(), ids);
        }
        fn collect_unique<K, V>(
            node: Option<&Arc<MapNode<K, V>>>,
            shared: &BTreeSet<usize>,
            into: &mut Vec<Weak<MapNode<K, V>>>,
        ) {
            let Some(node) = node else { return };
            if !shared.contains(&(Arc::as_ptr(node) as usize)) {
                into.push(Arc::downgrade(node));
            }
            collect_unique(node.left.as_ref(), shared, into);
            collect_unique(node.right.as_ref(), shared, into);
        }

        let mut shared = BTreeSet::new();
        collect_ids(other.root.as_ref(), &mut shared);
        let mut nodes = Vec::new();
        collect_unique(self.root.as_ref(), &shared, &mut nodes);
        PersistentOrdMapStorageProbe { nodes }
    }
}

impl<K: Ord + Clone, V: Clone> FromIterator<(K, V)> for PersistentOrdMap<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut map = Self::default();
        for (key, value) in iter {
            map.insert(key, value);
        }
        map
    }
}

impl<K: Ord + Clone + PartialEq, V: Clone + PartialEq> PartialEq for PersistentOrdMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

impl<K: Ord + Clone + Eq, V: Clone + Eq> Eq for PersistentOrdMap<K, V> {}

impl<K: Ord + Clone, V: Clone> std::ops::Index<&K> for PersistentOrdMap<K, V> {
    type Output = V;

    fn index(&self, index: &K) -> &Self::Output {
        self.get(index).expect("persistent map key not found")
    }
}

impl<'a, K: Ord + Clone, V: Clone> IntoIterator for &'a PersistentOrdMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = PersistentOrdMapIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug, Clone)]
pub struct PersistentOrdSet<T> {
    entries: PersistentOrdMap<T, ()>,
}

impl<T> Default for PersistentOrdSet<T> {
    fn default() -> Self {
        Self {
            entries: PersistentOrdMap::default(),
        }
    }
}

impl<T: Ord + Clone> PersistentOrdSet<T> {
    pub fn insert(&mut self, value: T) -> bool {
        self.entries.insert(value, ()).is_none()
    }

    pub fn remove(&mut self, value: &T) -> bool {
        self.entries.remove(value).is_some()
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        self.entries.retain(|value, ()| keep(value));
    }

    #[must_use]
    pub fn contains(&self, value: &T) -> bool {
        self.entries.contains_key(value)
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.entries.keys()
    }

    #[must_use]
    pub fn estimated_heap_bytes(&self) -> usize {
        self.entries.estimated_heap_bytes()
    }

    #[must_use]
    pub fn shares_root_with(&self, other: &Self) -> bool {
        self.entries.shares_root_with(&other.entries)
    }

    #[must_use]
    pub fn structural_node_count(&self) -> usize {
        self.entries.structural_node_count()
    }

    #[must_use]
    pub fn shared_structural_node_count_with(&self, other: &Self) -> usize {
        self.entries.shared_structural_node_count_with(&other.entries)
    }

    #[must_use]
    pub fn unique_storage_probe_against(
        &self,
        other: &Self,
    ) -> PersistentOrdMapStorageProbe<T, ()> {
        self.entries.unique_storage_probe_against(&other.entries)
    }
}

impl<T: Ord + Clone + PartialEq> PartialEq for PersistentOrdSet<T> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl<T: Ord + Clone + Eq> Eq for PersistentOrdSet<T> {}

impl<T: Ord + Clone> FromIterator<T> for PersistentOrdSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut set = Self::default();
        for value in iter {
            set.insert(value);
        }
        set
    }
}

impl<T: Ord + Clone> Extend<T> for PersistentOrdSet<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for value in iter {
            self.insert(value);
        }
    }
}

impl<'a, T: Ord + Clone> IntoIterator for &'a PersistentOrdSet<T> {
    type Item = &'a T;
    type IntoIter = std::iter::Map<PersistentOrdMapIter<'a, T, ()>, fn((&'a T, &'a ())) -> &'a T>;

    fn into_iter(self) -> Self::IntoIter {
        fn key_only<'a, T>((key, ()): (&'a T, &'a ())) -> &'a T {
            key
        }
        self.entries.iter().map(key_only::<T>)
    }
}

pub struct PersistentOrdMapIter<'a, K, V> {
    front: Vec<&'a MapNode<K, V>>,
    back: Vec<&'a MapNode<K, V>>,
    remaining: usize,
}

impl<'a, K, V> PersistentOrdMapIter<'a, K, V> {
    fn new(root: Option<&'a MapNode<K, V>>, remaining: usize) -> Self {
        let mut out = Self {
            front: Vec::new(),
            back: Vec::new(),
            remaining,
        };
        out.push_left(root);
        out.push_right(root);
        out
    }

    fn push_left(&mut self, mut node: Option<&'a MapNode<K, V>>) {
        while let Some(current) = node {
            self.front.push(current);
            node = current.left.as_deref();
        }
    }

    fn push_right(&mut self, mut node: Option<&'a MapNode<K, V>>) {
        while let Some(current) = node {
            self.back.push(current);
            node = current.right.as_deref();
        }
    }
}

impl<'a, K, V> Iterator for PersistentOrdMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let node = self.front.pop()?;
        self.push_left(node.right.as_deref());
        self.remaining -= 1;
        Some((&node.key, &node.value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> DoubleEndedIterator for PersistentOrdMapIter<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let node = self.back.pop()?;
        self.push_right(node.left.as_deref());
        self.remaining -= 1;
        Some((&node.key, &node.value))
    }
}

impl<K, V> ExactSizeIterator for PersistentOrdMapIter<'_, K, V> {}

fn map_height<K, V>(node: Option<&Arc<MapNode<K, V>>>) -> u16 {
    node.map_or(0, |node| node.height)
}

fn map_node<K, V>(
    key: K,
    value: V,
    left: Option<Arc<MapNode<K, V>>>,
    right: Option<Arc<MapNode<K, V>>>,
) -> Arc<MapNode<K, V>> {
    Arc::new(MapNode {
        key,
        value,
        height: 1 + map_height(left.as_ref()).max(map_height(right.as_ref())),
        left,
        right,
    })
}

fn map_balance<K: Clone, V: Clone>(node: Arc<MapNode<K, V>>) -> Arc<MapNode<K, V>> {
    let balance =
        i32::from(map_height(node.left.as_ref())) - i32::from(map_height(node.right.as_ref()));
    if balance > 1 {
        let left = node.left.as_ref().expect("left-heavy map node");
        if map_height(left.right.as_ref()) > map_height(left.left.as_ref()) {
            let rotated = map_rotate_left(left);
            let rebuilt = map_node(
                node.key.clone(),
                node.value.clone(),
                Some(rotated),
                node.right.clone(),
            );
            return map_rotate_right(&rebuilt);
        }
        return map_rotate_right(&node);
    }
    if balance < -1 {
        let right = node.right.as_ref().expect("right-heavy map node");
        if map_height(right.left.as_ref()) > map_height(right.right.as_ref()) {
            let rotated = map_rotate_right(right);
            let rebuilt = map_node(
                node.key.clone(),
                node.value.clone(),
                node.left.clone(),
                Some(rotated),
            );
            return map_rotate_left(&rebuilt);
        }
        return map_rotate_left(&node);
    }
    node
}

fn map_rotate_left<K: Clone, V: Clone>(node: &Arc<MapNode<K, V>>) -> Arc<MapNode<K, V>> {
    let pivot = node
        .right
        .as_ref()
        .expect("left rotation requires right child");
    let new_left = map_node(
        node.key.clone(),
        node.value.clone(),
        node.left.clone(),
        pivot.left.clone(),
    );
    map_node(
        pivot.key.clone(),
        pivot.value.clone(),
        Some(new_left),
        pivot.right.clone(),
    )
}

fn map_rotate_right<K: Clone, V: Clone>(node: &Arc<MapNode<K, V>>) -> Arc<MapNode<K, V>> {
    let pivot = node
        .left
        .as_ref()
        .expect("right rotation requires left child");
    let new_right = map_node(
        node.key.clone(),
        node.value.clone(),
        pivot.right.clone(),
        node.right.clone(),
    );
    map_node(
        pivot.key.clone(),
        pivot.value.clone(),
        pivot.left.clone(),
        Some(new_right),
    )
}

fn map_get_mut<'a, K: Ord + Clone, V: Clone>(
    node: &'a mut Option<Arc<MapNode<K, V>>>,
    key: &K,
) -> Option<&'a mut V> {
    let node = Arc::make_mut(node.as_mut()?);
    match key.cmp(&node.key) {
        Ordering::Less => map_get_mut(&mut node.left, key),
        Ordering::Greater => map_get_mut(&mut node.right, key),
        Ordering::Equal => Some(&mut node.value),
    }
}

fn map_retain<K: Clone, V: Clone>(
    node: Option<&Arc<MapNode<K, V>>>,
    keep: &mut impl FnMut(&K, &V) -> bool,
) -> (Option<Arc<MapNode<K, V>>>, usize) {
    let Some(node) = node else {
        return (None, 0);
    };

    let (left, removed_left) = map_retain(node.left.as_ref(), keep);
    let keep_node = keep(&node.key, &node.value);
    let (right, removed_right) = map_retain(node.right.as_ref(), keep);
    let removed_children = removed_left.saturating_add(removed_right);

    if keep_node {
        if removed_children == 0 {
            return (Some(Arc::clone(node)), 0);
        }
        return (
            Some(map_balance(map_node(
                node.key.clone(),
                node.value.clone(),
                left,
                right,
            ))),
            removed_children,
        );
    }

    (map_join(left, right), removed_children.saturating_add(1))
}

fn map_join<K: Clone, V: Clone>(
    left: Option<Arc<MapNode<K, V>>>,
    right: Option<Arc<MapNode<K, V>>>,
) -> Option<Arc<MapNode<K, V>>> {
    let (left, right) = match (left, right) {
        (None, right) => return right,
        (left, None) => return left,
        (Some(left), Some(right)) => (left, right),
    };

    let left_height = left.height;
    let right_height = right.height;
    if left_height > right_height.saturating_add(1) {
        let joined = map_join(left.right.clone(), Some(right));
        return Some(map_balance(map_node(
            left.key.clone(),
            left.value.clone(),
            left.left.clone(),
            joined,
        )));
    }
    if right_height > left_height.saturating_add(1) {
        let joined = map_join(Some(left), right.left.clone());
        return Some(map_balance(map_node(
            right.key.clone(),
            right.value.clone(),
            joined,
            right.right.clone(),
        )));
    }

    let (key, value, next_right) = map_take_min(&right);
    Some(map_balance(map_node(key, value, Some(left), next_right)))
}

fn map_remove<K: Ord + Clone, V: Clone>(
    node: Option<&Arc<MapNode<K, V>>>,
    key: &K,
) -> (Option<Arc<MapNode<K, V>>>, Option<V>) {
    let Some(node) = node else {
        return (None, None);
    };
    match key.cmp(&node.key) {
        Ordering::Less => {
            let (left, removed) = map_remove(node.left.as_ref(), key);
            if removed.is_none() {
                return (Some(Arc::clone(node)), None);
            }
            (
                Some(map_balance(map_node(
                    node.key.clone(),
                    node.value.clone(),
                    left,
                    node.right.clone(),
                ))),
                removed,
            )
        }
        Ordering::Greater => {
            let (right, removed) = map_remove(node.right.as_ref(), key);
            if removed.is_none() {
                return (Some(Arc::clone(node)), None);
            }
            (
                Some(map_balance(map_node(
                    node.key.clone(),
                    node.value.clone(),
                    node.left.clone(),
                    right,
                ))),
                removed,
            )
        }
        Ordering::Equal => {
            let removed = node.value.clone();
            match (&node.left, &node.right) {
                (None, None) => (None, Some(removed)),
                (Some(left), None) => (Some(Arc::clone(left)), Some(removed)),
                (None, Some(right)) => (Some(Arc::clone(right)), Some(removed)),
                (Some(left), Some(right)) => {
                    let (successor_key, successor_value, next_right) = map_take_min(right);
                    (
                        Some(map_balance(map_node(
                            successor_key,
                            successor_value,
                            Some(Arc::clone(left)),
                            next_right,
                        ))),
                        Some(removed),
                    )
                }
            }
        }
    }
}

fn map_take_min<K: Clone, V: Clone>(
    node: &Arc<MapNode<K, V>>,
) -> (K, V, Option<Arc<MapNode<K, V>>>) {
    let Some(left) = node.left.as_ref() else {
        return (node.key.clone(), node.value.clone(), node.right.clone());
    };
    let (key, value, next_left) = map_take_min(left);
    (
        key,
        value,
        Some(map_balance(map_node(
            node.key.clone(),
            node.value.clone(),
            next_left,
            node.right.clone(),
        ))),
    )
}

fn map_insert<K: Ord + Clone, V: Clone>(
    node: Option<&Arc<MapNode<K, V>>>,
    key: K,
    value: V,
) -> (Arc<MapNode<K, V>>, Option<V>) {
    let Some(node) = node else {
        return (map_node(key, value, None, None), None);
    };
    match key.cmp(&node.key) {
        Ordering::Less => {
            let (left, replaced) = map_insert(node.left.as_ref(), key, value);
            (
                map_balance(map_node(
                    node.key.clone(),
                    node.value.clone(),
                    Some(left),
                    node.right.clone(),
                )),
                replaced,
            )
        }
        Ordering::Greater => {
            let (right, replaced) = map_insert(node.right.as_ref(), key, value);
            (
                map_balance(map_node(
                    node.key.clone(),
                    node.value.clone(),
                    node.left.clone(),
                    Some(right),
                )),
                replaced,
            )
        }
        Ordering::Equal => (
            map_node(key, value, node.left.clone(), node.right.clone()),
            Some(node.value.clone()),
        ),
    }
}
