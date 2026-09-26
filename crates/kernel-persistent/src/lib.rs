use std::{
    cmp::Ordering,
    sync::{Arc, OnceLock},
};

const PAGE: usize = 256;
const BRANCH: usize = 32;

#[derive(Debug, Clone)]
struct MapNode<K, V> {
    key: K,
    value: V,
    height: u16,
    left: Option<Arc<MapNode<K, V>>>,
    right: Option<Arc<MapNode<K, V>>>,
}

#[derive(Debug, Clone)]
pub struct PersistentOrdMap<K, V> {
    root: Option<Arc<MapNode<K, V>>>,
    len: usize,
}

impl<K, V> Default for PersistentOrdMap<K, V> {
    fn default() -> Self {
        Self { root: None, len: 0 }
    }
}

impl<K: Ord + Clone, V: Clone> PersistentOrdMap<K, V> {
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
        let removed = self
            .iter()
            .filter(|(key, value)| !keep(key, value))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in removed {
            self.remove(&key);
        }
    }

    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        let mut current = self.root.as_deref();
        while let Some(node) = current {
            match key.cmp(&node.key) {
                Ordering::Less => current = node.left.as_deref(),
                Ordering::Greater => current = node.right.as_deref(),
                Ordering::Equal => return Some(&node.value),
            }
        }
        None
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        map_get_mut(&mut self.root, key)
    }

    #[must_use]
    pub fn contains_key(&self, key: &K) -> bool {
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
        assert!(index < self.len, "persistent vector index out of bounds");
        let removed = self[index].clone();
        for current in index..self.len - 1 {
            let next = self[current + 1].clone();
            self.set(current, next);
        }
        self.pop_last();
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
        let left_value = self[left].clone();
        let right_value = self[right].clone();
        self.set(left, right_value);
        self.set(right, left_value);
    }

    pub fn swap_remove(&mut self, index: usize) -> T {
        self.clear_cache();
        assert!(index < self.len, "persistent vector index out of bounds");
        let last_index = self.len - 1;
        let removed = self.get(index).expect("checked index").clone();
        if index != last_index {
            let last = self.get(last_index).expect("checked last index").clone();
            self.set(index, last);
        }
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
        while self.len < new_len {
            self.push(value.clone());
        }
        while self.len > new_len {
            self.pop_last();
        }
    }

    pub fn resize_with<F>(&mut self, new_len: usize, mut f: F)
    where
        F: FnMut() -> T,
    {
        while self.len < new_len {
            self.push(f());
        }
        while self.len > new_len {
            self.pop_last();
        }
    }

    pub fn as_slice(&self) -> &[T] {
        self.contiguous_cache
            .get_or_init(|| self.iter().cloned().collect())
            .as_slice()
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

#[cfg(test)]
mod tests {
    use super::{PersistentOrdMap, PersistentVec};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct CloneCounted {
        value: u32,
        clones: Arc<AtomicUsize>,
    }

    impl Clone for CloneCounted {
        fn clone(&self) -> Self {
            self.clones.fetch_add(1, Ordering::Relaxed);
            Self {
                value: self.value,
                clones: Arc::clone(&self.clones),
            }
        }
    }

    #[test]
    fn ordered_map_novel_insert_clones_only_search_path() {
        let clones = Arc::new(AtomicUsize::new(0));
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..65_536 {
            map.insert(
                key,
                CloneCounted {
                    value: key,
                    clones: Arc::clone(&clones),
                },
            );
        }
        let snapshot = map.clone();
        clones.store(0, Ordering::Relaxed);
        map.insert(
            65_536,
            CloneCounted {
                value: 65_536,
                clones: Arc::clone(&clones),
            },
        );
        assert!(clones.load(Ordering::Relaxed) < 128);
        assert_eq!(snapshot.get(&65_536).map(|value| value.value), None);
        assert_eq!(map.get(&65_536).map(|value| value.value), Some(65_536));
    }

    #[test]
    fn ordered_map_get_mut_clones_only_search_path() {
        let clones = Arc::new(AtomicUsize::new(0));
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..65_536 {
            map.insert(
                key,
                CloneCounted {
                    value: key,
                    clones: Arc::clone(&clones),
                },
            );
        }
        let snapshot = map.clone();
        clones.store(0, Ordering::Relaxed);
        map.get_mut(&32_768).unwrap().value = 7;
        assert!(clones.load(Ordering::Relaxed) < 128);
        assert_eq!(snapshot.get(&32_768).map(|value| value.value), Some(32_768));
        assert_eq!(map.get(&32_768).map(|value| value.value), Some(7));
    }

    #[test]
    fn ordered_map_path_copy_preserves_snapshot_and_order() {
        let mut original = PersistentOrdMap::default();
        for key in 0_u32..10_000 {
            original.insert(key, key * 2);
        }
        let snapshot = original.clone();
        assert!(original.shares_root_with(&snapshot));

        original.insert(10_000, 20_000);
        original.insert(5_000, 7);

        assert_eq!(snapshot.get(&10_000), None);
        assert_eq!(snapshot.get(&5_000), Some(&10_000));
        assert_eq!(original.get(&5_000), Some(&7));
        assert_eq!(snapshot.len(), 10_000);
        assert_eq!(original.len(), 10_001);
        assert!(!original.shares_root_with(&snapshot));
        assert!(snapshot.keys().copied().eq(0_u32..10_000));
    }

    #[test]
    fn ordered_map_remove_path_copies_and_preserves_snapshot() {
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..65_536 {
            map.insert(key, key * 3);
        }
        let snapshot = map.clone();
        assert_eq!(map.remove(&32_768), Some(98_304));
        assert_eq!(map.remove(&0), Some(0));
        assert_eq!(map.remove(&65_535), Some(196_605));
        assert_eq!(map.remove(&99_999), None);
        assert_eq!(snapshot.get(&32_768), Some(&98_304));
        assert_eq!(map.get(&32_768), None);
        assert_eq!(snapshot.len(), 65_536);
        assert_eq!(map.len(), 65_533);
        assert!(map.keys().copied().is_sorted());
        let height = map.root.as_ref().map_or(0, |root| root.height);
        assert!(
            height <= 24,
            "AVL height grew unexpectedly after removals: {height}"
        );
    }

    #[test]
    fn ordered_map_bidirectional_iteration_preserves_exact_order() {
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..10_000 {
            map.insert(key, key * 2);
        }
        assert!(map.keys().copied().eq(0_u32..10_000));
        assert!(map.keys().rev().copied().eq((0_u32..10_000).rev()));
        let mut iter = map.iter();
        assert_eq!(iter.next().map(|(key, _)| *key), Some(0));
        assert_eq!(iter.next_back().map(|(key, _)| *key), Some(9_999));
        assert_eq!(iter.len(), 9_998);
    }

    #[test]
    fn ordered_map_predecessor_and_successor_are_strict_and_logical() {
        let mut map = PersistentOrdMap::default();
        for key in [10_u32, 20, 40, 80] {
            map.insert(key, key * 10);
        }
        assert_eq!(map.predecessor(&10).map(|(key, _)| *key), None);
        assert_eq!(map.predecessor(&20).map(|(key, _)| *key), Some(10));
        assert_eq!(map.predecessor(&39).map(|(key, _)| *key), Some(20));
        assert_eq!(map.predecessor(&100).map(|(key, _)| *key), Some(80));
        assert_eq!(map.successor(&80).map(|(key, _)| *key), None);
        assert_eq!(map.successor(&40).map(|(key, _)| *key), Some(80));
        assert_eq!(map.successor(&21).map(|(key, _)| *key), Some(40));
        assert_eq!(map.successor(&0).map(|(key, _)| *key), Some(10));
    }

    #[test]
    fn ordered_map_balances_monotone_insertions() {
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..65_536 {
            map.insert(key, key);
        }
        let height = map.root.as_ref().map_or(0, |root| root.height);
        assert!(height <= 24, "AVL height grew unexpectedly: {height}");
    }

    #[test]
    fn snapshot_mutation_preserves_old_root_contents() {
        let original = PersistentVec::from_vec((0_u64..10_000).collect());
        let mut next = original.clone();
        next.set(9_999, 77);
        assert_eq!(original[9_999], 9_999);
        assert_eq!(next[9_999], 77);
        assert_eq!(original[42], next[42]);
    }

    #[test]
    fn changed_indices_skips_shared_pages_and_reports_appends() {
        let original = PersistentVec::from_vec((0_u64..20_000).collect());
        let mut next = original.clone();
        next.set(17, 99);
        next.set(19_001, 101);
        next.push(20_000);
        assert_eq!(original.changed_indices(&next), vec![17, 19_001, 20_000]);
    }

    #[test]
    fn changed_indices_preserves_locality_across_radix_height_growth() {
        let original = PersistentVec::from_vec((0_u64..8_192).collect());
        let mut next = original.clone();
        next.push(8_192);
        assert_eq!(original.changed_indices(&next), vec![8_192]);
    }
}
