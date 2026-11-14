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
/// after a `retain` that dropped a record finds room. Its guarantee that
/// [`Ledger::expire_where`](crate::Ledger::expire_where) drops nothing when
/// the caller's predicate unwinds rests on a third: `retain` asks about every
/// record before it drops any.
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
    /// Record `value` on the pending record at `slot`
    /// ([`Entry::complete`]). Nothing else of a stored record ever changes:
    /// not its key, not its claim id, so no lookup goes stale.
    ///
    /// # Errors
    /// `value`, handed back with nothing changed, when `slot` holds no
    /// record or a completed one.
    fn complete(&mut self, slot: usize, value: Self::Value) -> Result<(), Self::Value>;
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
    ///
    /// Ask-then-drop: `keep` sees every record before any is dropped, so a
    /// `keep` that unwinds leaves the backend as it was.
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
        slots::get(&self.entries, slot)
    }

    fn complete(&mut self, slot: usize, value: V) -> Result<(), V> {
        slots::complete(&mut self.entries, slot, value)
    }

    fn find(&self, key: &K) -> Option<usize> {
        slots::find(&self.entries, |entry| entry.key == *key)
    }

    fn locate(&self, claim: ClaimId) -> Option<usize> {
        slots::find(&self.entries, |entry| entry.claim == claim)
    }

    fn insert(&mut self, entry: Entry<K, V>) -> Option<usize> {
        if self.find(&entry.key).is_some() || self.locate(entry.claim).is_some() {
            return None;
        }
        slots::insert(&mut self.entries, entry)
    }

    fn remove(&mut self, slot: usize) -> bool {
        slots::remove(&mut self.entries, slot)
    }

    fn retain(&mut self, keep: impl FnMut(&Entry<K, V>) -> bool) -> usize {
        slots::retain(&mut self.entries, keep)
    }

    fn iter(&self) -> impl Iterator<Item = (usize, &Entry<K, V>)> {
        slots::iter(&self.entries)
    }
}

/// [`Storage`] over a slice of optional records, slot = index: what
/// [`Fixed`] is, shared with `Hashed`'s slots and the crate's test backends.
pub mod slots {
    use crate::Entry;

    pub fn get<K, V>(entries: &[Option<Entry<K, V>>], slot: usize) -> Option<&Entry<K, V>> {
        entries.get(slot)?.as_ref()
    }

    pub fn complete<K, V>(
        entries: &mut [Option<Entry<K, V>>],
        slot: usize,
        value: V,
    ) -> Result<(), V> {
        match entries.get_mut(slot).and_then(Option::as_mut) {
            Some(entry) => entry.complete(value),
            None => Err(value),
        }
    }

    /// The first slot whose record `is` matches.
    pub fn find<K, V>(
        entries: &[Option<Entry<K, V>>],
        is: impl Fn(&Entry<K, V>) -> bool,
    ) -> Option<usize> {
        entries
            .iter()
            .position(|entry| entry.as_ref().is_some_and(&is))
    }

    /// Store `entry` in the first free slot; nothing is checked.
    pub fn insert<K, V>(entries: &mut [Option<Entry<K, V>>], entry: Entry<K, V>) -> Option<usize> {
        let slot = entries.iter().position(Option::is_none)?;
        entries[slot] = Some(entry);
        Some(slot)
    }

    pub fn remove<K, V>(entries: &mut [Option<Entry<K, V>>], slot: usize) -> bool {
        entries.get_mut(slot).and_then(Option::take).is_some()
    }

    /// Ask-then-drop: every verdict lands in a fixed buffer of `N` marks
    /// before the first record is dropped. No allocation.
    pub fn retain<K, V, const N: usize>(
        entries: &mut [Option<Entry<K, V>>; N],
        mut keep: impl FnMut(&Entry<K, V>) -> bool,
    ) -> usize {
        let mut doomed = [false; N];
        for (mark, entry) in doomed.iter_mut().zip(entries.iter()) {
            *mark = entry.as_ref().is_some_and(|entry| !keep(entry));
        }
        let mut removed = 0;
        for (mark, entry) in doomed.into_iter().zip(entries.iter_mut()) {
            if mark {
                *entry = None;
                removed += 1;
            }
        }
        removed
    }

    pub fn iter<K, V>(
        entries: &[Option<Entry<K, V>>],
    ) -> impl Iterator<Item = (usize, &Entry<K, V>)> {
        let held = entries.iter().enumerate();
        held.filter_map(|(slot, entry)| Some((slot, entry.as_ref()?)))
    }
}
