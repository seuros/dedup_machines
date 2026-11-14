use alloc::vec::Vec;
use core::hash::{BuildHasher, Hash};
use core::time::Duration;

use hashbrown::{DefaultHashBuilder, HashTable};
use periscope_machines::Issuer;

use crate::storage::slots;
use crate::{ClaimId, Entry, Ledger, Storage};

/// Heap-backed storage with a runtime capacity and hashed lookup, by key and
/// by claim id.
#[derive(Debug, Clone)]
pub struct Hashed<K, V, H = DefaultHashBuilder> {
    slots: Vec<Option<Entry<K, V>>>,
    free: Vec<usize>,
    index: HashTable<usize>,
    claims: HashTable<usize>,
    hasher: H,
    capacity: usize,
}

/// Records, free list and capacity; the indexes and the hasher follow from
/// them.
impl<K: PartialEq, V: PartialEq, H> PartialEq for Hashed<K, V, H> {
    fn eq(&self, other: &Self) -> bool {
        self.slots == other.slots && self.free == other.free && self.capacity == other.capacity
    }
}

impl<K: Eq, V: Eq, H> Eq for Hashed<K, V, H> {}

/// Heap-backed exact-key ledger.
pub type HashDedup<K, V = (), H = DefaultHashBuilder> = Ledger<Hashed<K, V, H>>;

impl<K: Hash + Eq, V> HashDedup<K, V> {
    /// Empty ledger in scope zero holding at most `capacity` live records.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_scope(capacity, 0)
    }

    /// Empty ledger whose claim ids carry `scope`, see
    /// [`Dedup::with_scope`](crate::Dedup::with_scope).
    #[must_use]
    pub fn with_scope(capacity: usize, scope: u16) -> Self {
        Self::fresh(
            Hashed::with_capacity(capacity),
            Issuer::new(scope),
            Duration::ZERO,
        )
    }
}

impl<K: Hash + Eq, V> Hashed<K, V> {
    /// Empty storage holding at most `capacity` records.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_hasher(capacity, DefaultHashBuilder::default())
    }
}

impl<K: Hash + Eq, V, H: BuildHasher> Hashed<K, V, H> {
    /// Empty storage with an explicit hasher; hand it to
    /// [`Ledger::over_with_scope`].
    #[must_use]
    pub const fn with_capacity_and_hasher(capacity: usize, hasher: H) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            index: HashTable::new(),
            claims: HashTable::new(),
            hasher,
            capacity,
        }
    }

    fn at(&self, slot: usize) -> Option<&Entry<K, V>> {
        slots::get(&self.slots, slot)
    }

    fn unlink(&mut self, slot: usize) -> bool {
        let Some(entry) = self.at(slot) else {
            return false;
        };
        let (key, claim) = (
            self.hasher.hash_one(&entry.key),
            self.hasher.hash_one(entry.claim),
        );
        let same = |&candidate: &usize| candidate == slot;
        let (Ok(by_key), Ok(by_claim)) = (
            self.index.find_entry(key, same),
            self.claims.find_entry(claim, same),
        ) else {
            return false;
        };
        by_key.remove();
        by_claim.remove();
        self.slots.get_mut(slot).and_then(Option::take);
        self.free.push(slot);
        true
    }
}

impl<K: Hash + Eq, V, H: BuildHasher> Storage for Hashed<K, V, H> {
    type Key = K;
    type Value = V;

    fn get(&self, slot: usize) -> Option<&Entry<K, V>> {
        self.at(slot)
    }

    fn complete(&mut self, slot: usize, value: V) -> Result<(), V> {
        slots::complete(&mut self.slots, slot, value)
    }

    fn find(&self, key: &K) -> Option<usize> {
        let hash = self.hasher.hash_one(key);
        let held = |&slot: &usize| self.at(slot).is_some_and(|entry| entry.key == *key);
        self.index.find(hash, held).copied()
    }

    fn locate(&self, claim: ClaimId) -> Option<usize> {
        let hash = self.hasher.hash_one(claim);
        let held = |&slot: &usize| self.at(slot).is_some_and(|entry| entry.claim == claim);
        self.claims.find(hash, held).copied()
    }

    fn insert(&mut self, entry: Entry<K, V>) -> Option<usize> {
        let held = self.find(&entry.key).is_some() || self.locate(entry.claim).is_some();
        if held || self.index.len() >= self.capacity {
            return None;
        }
        let (key, claim) = (
            self.hasher.hash_one(&entry.key),
            self.hasher.hash_one(entry.claim),
        );
        let slot = match self.free.pop() {
            Some(slot) if slot < self.slots.len() => {
                self.slots[slot] = Some(entry);
                slot
            }
            Some(_) | None => {
                self.slots.push(Some(entry));
                self.slots.len() - 1
            }
        };
        let (slots, hasher) = (&self.slots, &self.hasher);
        let held = |other: usize| slots.get(other).and_then(Option::as_ref);
        self.index.insert_unique(key, slot, |&other| {
            held(other).map_or(0, |entry| hasher.hash_one(&entry.key))
        });
        self.claims.insert_unique(claim, slot, |&other| {
            held(other).map_or(0, |entry| hasher.hash_one(entry.claim))
        });
        Some(slot)
    }

    fn remove(&mut self, slot: usize) -> bool {
        self.unlink(slot)
    }

    /// Ask-then-drop: the doomed slots are collected before any is unlinked.
    fn retain(&mut self, mut keep: impl FnMut(&Entry<K, V>) -> bool) -> usize {
        let doomed: Vec<usize> = self
            .iter()
            .filter(|(_, entry)| !keep(entry))
            .map(|(slot, _)| slot)
            .collect();
        doomed.into_iter().filter(|&slot| self.unlink(slot)).count()
    }

    fn iter(&self) -> impl Iterator<Item = (usize, &Entry<K, V>)> {
        slots::iter(&self.slots)
    }
}

#[cfg(test)]
mod tests;
