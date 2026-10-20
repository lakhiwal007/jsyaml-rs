//! The JS containers the built-in tags build, with JS semantics: `{}` with string keys
//! and `Object.keys` order, and `Map` / `Set` with `SameValueZero` keys (`NaN` equals
//! `NaN`, `+0` equals `-0`, objects by identity).

use std::collections::{HashMap, HashSet};

use super::{NodeId, V, js_number_to_string};

/// Linear scans beat hashing for small collections; index above this size.
const INDEX_THRESHOLD: usize = 8;

/// `String(value)` for a primitive.
pub(crate) fn js_string(value: &V) -> String {
    match value {
        V::Null => "null".to_owned(),
        V::Bool(b) => b.to_string(),
        V::Num(n) => js_number_to_string(*n),
        V::Str(s) => s.clone(),
        V::Node(_) => "[object Object]".to_owned(),
    }
}

/// A `SameValueZero` key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SzKey {
    Null,
    Bool(bool),
    Num(u64),
    Str(String),
    Node(NodeId),
}

impl SzKey {
    pub(crate) fn of(value: &V) -> SzKey {
        match value {
            V::Null => SzKey::Null,
            V::Bool(b) => SzKey::Bool(*b),
            V::Num(n) if n.is_nan() => SzKey::Num(f64::NAN.to_bits()),
            V::Num(n) if *n == 0.0 => SzKey::Num(0),
            V::Num(n) => SzKey::Num(n.to_bits()),
            V::Str(s) => SzKey::Str(s.clone()),
            V::Node(id) => SzKey::Node(*id),
        }
    }
}

#[allow(clippy::float_cmp)] // SameValueZero is exact equality
fn same_value_zero(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Num(x), V::Num(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}

/// An unordered `Set` (merge bookkeeping, `omapTag`'s `seen`).
#[derive(Debug, Default)]
pub(crate) struct SzSet(HashSet<SzKey>);

impl SzSet {
    pub(crate) fn insert(&mut self, value: &V) {
        self.0.insert(SzKey::of(value));
    }

    pub(crate) fn contains(&self, value: &V) -> bool {
        self.0.contains(&SzKey::of(value))
    }

    pub(crate) fn remove(&mut self, value: &V) {
        self.0.remove(&SzKey::of(value));
    }
}

/// A JS `Set`: insertion-ordered, `SameValueZero`.
#[derive(Debug, Default)]
pub(crate) struct KeySet {
    pub(crate) items: Vec<V>,
    index: Option<HashSet<SzKey>>,
}

impl KeySet {
    pub(crate) fn contains(&self, value: &V) -> bool {
        match &self.index {
            Some(index) => index.contains(&SzKey::of(value)),
            None => self.items.iter().any(|item| same_value_zero(item, value)),
        }
    }

    pub(crate) fn insert(&mut self, value: V) {
        if self.contains(&value) {
            return;
        }
        if let Some(index) = &mut self.index {
            index.insert(SzKey::of(&value));
        } else if self.items.len() >= INDEX_THRESHOLD {
            let mut index: HashSet<SzKey> = self.items.iter().map(SzKey::of).collect();
            index.insert(SzKey::of(&value));
            self.index = Some(index);
        }
        self.items.push(value);
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &V> {
        self.items.iter()
    }
}

/// A JS `Map`: insertion-ordered, `SameValueZero`; setting an existing key keeps its
/// position.
#[derive(Debug, Default)]
pub(crate) struct SzMap {
    entries: Vec<(V, V)>,
    index: Option<HashMap<SzKey, usize>>,
}

impl SzMap {
    fn find(&self, key: &V) -> Option<usize> {
        match &self.index {
            Some(index) => index.get(&SzKey::of(key)).copied(),
            None => self
                .entries
                .iter()
                .position(|(k, _)| same_value_zero(k, key)),
        }
    }

    pub(crate) fn contains(&self, key: &V) -> bool {
        self.find(key).is_some()
    }

    pub(crate) fn get(&self, key: &V) -> Option<&V> {
        self.find(key).map(|i| &self.entries[i].1)
    }

    pub(crate) fn insert(&mut self, key: V, value: V) {
        if let Some(i) = self.find(&key) {
            self.entries[i].1 = value;
            return;
        }
        if let Some(index) = &mut self.index {
            index.insert(SzKey::of(&key), self.entries.len());
        } else if self.entries.len() >= INDEX_THRESHOLD {
            let mut index: HashMap<SzKey, usize> = self
                .entries
                .iter()
                .enumerate()
                .map(|(i, (k, _))| (SzKey::of(k), i))
                .collect();
            index.insert(SzKey::of(&key), self.entries.len());
            self.index = Some(index);
        }
        self.entries.push((key, value));
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &V> {
        self.entries.iter().map(|(k, _)| k)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&V, &V)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub(crate) fn into_entries(self) -> impl Iterator<Item = (V, V)> {
        self.entries.into_iter()
    }
}

/// A plain `{}`: string keys; assigning an existing key keeps its position.
#[derive(Debug, Default)]
pub(crate) struct ObjectMap {
    entries: Vec<(String, V)>,
    index: Option<HashMap<String, usize>>,
}

/// An "array index" key, which `Object.keys` lists first, in numeric order.
fn array_index(key: &str) -> Option<u32> {
    let b = key.as_bytes();
    if b.is_empty()
        || b.len() > 10
        || (b.len() > 1 && b[0] == b'0')
        || !b.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    key.parse::<u32>().ok().filter(|&n| n != u32::MAX)
}

impl ObjectMap {
    fn find(&self, key: &str) -> Option<usize> {
        match &self.index {
            Some(index) => index.get(key).copied(),
            None => self.entries.iter().position(|(k, _)| k == key),
        }
    }

    pub(crate) fn contains(&self, key: &str) -> bool {
        self.find(key).is_some()
    }

    pub(crate) fn get(&self, key: &str) -> Option<&V> {
        self.find(key).map(|i| &self.entries[i].1)
    }

    pub(crate) fn insert(&mut self, key: String, value: V) {
        if let Some(i) = self.find(&key) {
            self.entries[i].1 = value;
            return;
        }
        if let Some(index) = &mut self.index {
            index.insert(key.clone(), self.entries.len());
        } else if self.entries.len() >= INDEX_THRESHOLD {
            let mut index: HashMap<String, usize> = self
                .entries
                .iter()
                .enumerate()
                .map(|(i, (k, _))| (k.clone(), i))
                .collect();
            index.insert(key.clone(), self.entries.len());
            self.index = Some(index);
        }
        self.entries.push((key, value));
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Entries in insertion order (JS reorders integer keys itself when building the object).
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    /// `Object.keys` order: array-index keys ascending, then the rest in insertion order.
    pub(crate) fn js_keys(&self) -> impl Iterator<Item = &str> {
        let mut indexed: Vec<(u32, &str)> = self
            .entries
            .iter()
            .filter_map(|(k, _)| array_index(k).map(|n| (n, k.as_str())))
            .collect();
        indexed.sort_unstable_by_key(|&(n, _)| n);
        let rest = self
            .entries
            .iter()
            .map(|(k, _)| k.as_str())
            .filter(|k| array_index(k).is_none());
        indexed.into_iter().map(|(_, k)| k).chain(rest)
    }

    pub(crate) fn into_entries(self) -> impl Iterator<Item = (String, V)> {
        self.entries.into_iter()
    }
}
