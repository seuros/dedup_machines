use crate::{ClaimId, Entry};

/// Where a [`Ledger`](crate::Ledger) keeps its records.
///
/// [`Fixed`] and [`Hashed`](crate::Hashed) are in-memory. Implement this to
/// put the ledger over something else, such as a database table or a
/// shared-memory arena; the ledger only ever addresses records by the slot
/// `insert` returned.
///
/// Slots are opaque: a backend may reuse one after `remove`, because a
/// [`ClaimId`] is never reissued and so tells a stale handle from a fresh
/// record.
///
/// The ledger's guarantee that a refusal changes nothing rests on two rules:
/// an `insert` that answers `None` stored nothing, and an `insert` right
/// after a `retain` that dropped a record finds room.
///
/// No two held records share a key or a claim id. A ledger never asks for
/// that; [`Fixed`] and [`Hashed`](crate::Hashed) refuse an `insert` that
/// would, and [`Ledger::resumed`](crate::Ledger::resumed) refuses a store
/// where `find` or `locate` does not lead back to the slot `iter` reported.
pub trait Storage {
    /// Claimed key type.
    type Key: Eq;
    /// Value stored on completion.
    type Value;

    /// The record at `slot`, if one is held.
    fn get(&self, slot: usize) -> Option<&Entry<Self::Key, Self::Value>>;
    /// Mutable access to the record at `slot`, if one is held.
    fn get_mut(&mut self, slot: usize) -> Option<&mut Entry<Self::Key, Self::Value>>;
    /// The slot holding `key`, if any.
    fn find(&self, key: &Self::Key) -> Option<usize>;
    /// The slot holding the record claimed under `claim`, if any.
    fn locate(&self, claim: ClaimId) -> Option<usize>;
    /// Store `entry` in a free slot, or `None`, storing nothing, when the
    /// backend is full or already holds a record under that key or that
    /// claim id. A ledger only inserts a key and an id it found absent, so
    /// to a ledger `None` means full.
    fn insert(&mut self, entry: Entry<Self::Key, Self::Value>) -> Option<usize>;
    /// Drop the record at `slot`; `false` when there was none.
    fn remove(&mut self, slot: usize) -> bool;
    /// Drop every record `keep` rejects, returning how many were dropped.
    fn retain(&mut self, keep: impl FnMut(&Entry<Self::Key, Self::Value>) -> bool) -> usize;
    /// Every record held with the slot holding it, in no particular order.
    fn iter(&self) -> impl Iterator<Item = (usize, &Entry<Self::Key, Self::Value>)>;
}

/// Fixed-capacity inline storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixed<K, V, const N: usize> {
    entries: [Option<Entry<K, V>>; N],
}

impl<K, V, const N: usize> Fixed<K, V, N> {
    /// Empty storage with `N` slots.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [const { None }; N],
        }
    }
}

impl<K, V, const N: usize> Default for Fixed<K, V, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq, V, const N: usize> Storage for Fixed<K, V, N> {
    type Key = K;
    type Value = V;

    fn get(&self, slot: usize) -> Option<&Entry<K, V>> {
        self.entries.get(slot)?.as_ref()
    }

    fn get_mut(&mut self, slot: usize) -> Option<&mut Entry<K, V>> {
        self.entries.get_mut(slot)?.as_mut()
    }

    fn find(&self, key: &K) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.as_ref().is_some_and(|entry| entry.key == *key))
    }

    fn locate(&self, claim: ClaimId) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.as_ref().is_some_and(|entry| entry.claim == claim))
    }

    fn insert(&mut self, entry: Entry<K, V>) -> Option<usize> {
        if self.find(&entry.key).is_some() || self.locate(entry.claim).is_some() {
            return None;
        }
        let slot = self.entries.iter().position(Option::is_none)?;
        self.entries[slot] = Some(entry);
        Some(slot)
    }

    fn remove(&mut self, slot: usize) -> bool {
        self.entries.get_mut(slot).and_then(Option::take).is_some()
    }

    fn retain(&mut self, mut keep: impl FnMut(&Entry<K, V>) -> bool) -> usize {
        let mut removed = 0;
        for slot in &mut self.entries {
            if slot.as_ref().is_some_and(|entry| !keep(entry)) {
                *slot = None;
                removed += 1;
            }
        }
        removed
    }

    fn iter(&self) -> impl Iterator<Item = (usize, &Entry<K, V>)> {
        let held = self.entries.iter().enumerate();
        held.filter_map(|(slot, entry)| Some((slot, entry.as_ref()?)))
    }
}
