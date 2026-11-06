use alloc::vec::Vec;
use core::hash::{BuildHasher, Hash};

use hashbrown::{DefaultHashBuilder, HashTable};

use crate::{Entry, Ledger, Storage};

/// Heap-backed storage with a runtime capacity and hashed key lookup.
#[derive(Debug)]
pub struct Hashed<K, V, H = DefaultHashBuilder> {
    slots: Vec<Option<Entry<K, V>>>,
    free: Vec<usize>,
    index: HashTable<usize>,
    hasher: H,
    capacity: usize,
}

/// Heap-backed exact-key ledger.
pub type HashDedup<K, V = (), H = DefaultHashBuilder> = Ledger<Hashed<K, V, H>>;

impl<K: Hash + Eq, V> HashDedup<K, V> {
    /// Start an empty ledger holding at most `capacity` unexpired records.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_hasher(capacity, DefaultHashBuilder::default())
    }
}

impl<K: Hash + Eq, V, H: BuildHasher> HashDedup<K, V, H> {
    /// Start an empty ledger with an explicit hasher.
    #[must_use]
    pub const fn with_capacity_and_hasher(capacity: usize, hasher: H) -> Self {
        Self::over(Hashed {
            slots: Vec::new(),
            free: Vec::new(),
            index: HashTable::new(),
            hasher,
            capacity,
        })
    }
}

impl<K: Hash + Eq, V, H: BuildHasher> Hashed<K, V, H> {
    fn key_at(&self, slot: usize) -> Option<&K> {
        self.slots.get(slot)?.as_ref().map(|entry| &entry.key)
    }

    fn unlink(&mut self, slot: usize) -> bool {
        let Some(key) = self.key_at(slot) else {
            return false;
        };
        let hash = self.hasher.hash_one(key);
        let Ok(occupied) = self.index.find_entry(hash, |&candidate| candidate == slot) else {
            return false;
        };
        occupied.remove();
        self.slots.get_mut(slot).and_then(Option::take);
        self.free.push(slot);
        true
    }
}

impl<K: Hash + Eq, V, H: BuildHasher> Storage for Hashed<K, V, H> {
    type Key = K;
    type Value = V;

    fn get(&self, slot: usize) -> Option<&Entry<K, V>> {
        self.slots.get(slot)?.as_ref()
    }

    fn get_mut(&mut self, slot: usize) -> Option<&mut Entry<K, V>> {
        self.slots.get_mut(slot)?.as_mut()
    }

    fn find(&self, key: &K) -> Option<usize> {
        let hash = self.hasher.hash_one(key);
        self.index
            .find(hash, |&slot| self.key_at(slot) == Some(key))
            .copied()
    }

    fn insert(&mut self, entry: Entry<K, V>) -> Option<usize> {
        if self.index.len() >= self.capacity {
            return None;
        }
        let hash = self.hasher.hash_one(&entry.key);
        let slot = match self.free.pop() {
            Some(slot) if slot < self.slots.len() => {
                self.slots[slot] = Some(entry);
                slot
            }
            _ => {
                self.slots.push(Some(entry));
                self.slots.len() - 1
            }
        };
        let (slots, hasher) = (&self.slots, &self.hasher);
        self.index.insert_unique(hash, slot, |&other| {
            slots
                .get(other)
                .and_then(Option::as_ref)
                .map_or(0, |entry| hasher.hash_one(&entry.key))
        });
        Some(slot)
    }

    fn remove(&mut self, slot: usize) -> bool {
        self.unlink(slot)
    }

    fn retain(&mut self, mut keep: impl FnMut(&Entry<K, V>) -> bool) -> usize {
        let mut removed = 0;
        for slot in 0..self.slots.len() {
            if self.get(slot).is_some_and(|entry| !keep(entry)) && self.unlink(slot) {
                removed += 1;
            }
        }
        removed
    }

    fn iter(&self) -> impl Iterator<Item = &Entry<K, V>> {
        self.slots.iter().flatten()
    }
}

#[cfg(test)]
mod tests;
