#![no_std]
#![doc = include_str!("../README.md")]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
mod hashed;

use core::time::Duration;

#[cfg(feature = "alloc")]
pub use hashed::{HashDedup, Hashed};

/// Opaque generation-checked claim, scoped to one ledger lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimId {
    slot: usize,
    generation: u64,
}

/// Claim outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Claim<'a, V> {
    /// Caller owns a new pending claim.
    Acquired(ClaimId),
    /// Another live attempt owns this key.
    InProgress,
    /// This key completed within its retention interval.
    Completed(&'a V),
    /// A live or completed record for this key carries a different fingerprint.
    Conflict,
    /// All slots contain unexpired records.
    Full,
    /// The attempt's partition holds its quota of unexpired records.
    PartitionFull,
    /// Invalid clock/TTL or exhausted generation space.
    Rejected,
}

/// A key plus the optional fingerprint and partition it is claimed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt<K> {
    key: K,
    fingerprint: Option<u64>,
    partition: Option<u64>,
}

impl<K> Attempt<K> {
    /// Plain attempt: no fingerprint, no partition.
    #[must_use]
    pub const fn new(key: K) -> Self {
        Self {
            key,
            fingerprint: None,
            partition: None,
        }
    }

    /// Payload identity; a record under the same key with another fingerprint conflicts.
    #[must_use]
    pub const fn fingerprint(mut self, fingerprint: u64) -> Self {
        self.fingerprint = Some(fingerprint);
        self
    }

    /// Quota bucket counted against the ledger's partition cap.
    #[must_use]
    pub const fn partition(mut self, partition: u64) -> Self {
        self.partition = Some(partition);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State<V> {
    Pending,
    Completed(V),
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<K, V> {
    key: K,
    fingerprint: Option<u64>,
    partition: Option<u64>,
    generation: u64,
    expires: Duration,
    state: State<V>,
}

mod sealed {
    pub trait Sealed {}
}

#[doc(hidden)]
pub trait Storage: sealed::Sealed {
    type Key: Eq;
    type Value;

    fn get(&self, slot: usize) -> Option<&Entry<Self::Key, Self::Value>>;
    fn get_mut(&mut self, slot: usize) -> Option<&mut Entry<Self::Key, Self::Value>>;
    fn find(&self, key: &Self::Key) -> Option<usize>;
    fn insert(&mut self, entry: Entry<Self::Key, Self::Value>) -> Option<usize>;
    fn remove(&mut self, slot: usize) -> bool;
    fn retain(&mut self, keep: impl FnMut(&Entry<Self::Key, Self::Value>) -> bool) -> usize;
    fn iter(&self) -> impl Iterator<Item = &Entry<Self::Key, Self::Value>>;
}

/// Fixed-capacity inline storage.
#[derive(Debug, PartialEq, Eq)]
pub struct Fixed<K, V, const N: usize> {
    entries: [Option<Entry<K, V>>; N],
}

impl<K, V, const N: usize> sealed::Sealed for Fixed<K, V, N> {}

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

    fn insert(&mut self, entry: Entry<K, V>) -> Option<usize> {
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

    fn iter(&self) -> impl Iterator<Item = &Entry<K, V>> {
        self.entries.iter().flatten()
    }
}

/// Exact-key claim ledger over a storage backend.
#[derive(Debug, PartialEq, Eq)]
pub struct Ledger<S> {
    store: S,
    generation: u64,
    observed: Duration,
    partition_cap: Option<usize>,
}

/// Fixed-capacity exact-key ledger.
pub type Dedup<K, const N: usize, V = ()> = Ledger<Fixed<K, V, N>>;

impl<K: Eq, V, const N: usize> Default for Dedup<K, N, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq, V, const N: usize> Dedup<K, N, V> {
    /// Start an empty ledger with a fresh handle namespace.
    #[must_use]
    pub const fn new() -> Self {
        Self::over(Fixed {
            entries: [const { None }; N],
        })
    }
}

impl<S: Storage> Ledger<S> {
    pub(crate) const fn over(store: S) -> Self {
        Self {
            store,
            generation: 0,
            observed: Duration::ZERO,
            partition_cap: None,
        }
    }

    /// Cap unexpired records per partition; attempts without a partition are uncounted.
    #[must_use]
    pub const fn with_partition_cap(mut self, cap: usize) -> Self {
        self.partition_cap = Some(cap);
        self
    }

    /// Claim a key without evicting any unexpired entry.
    #[must_use]
    pub fn claim(&mut self, key: S::Key, now: Duration, ttl: Duration) -> Claim<'_, S::Value> {
        self.admit(Attempt::new(key), now, ttl)
    }

    /// Claim a key under a payload fingerprint; a mismatching record conflicts.
    #[must_use]
    pub fn claim_with(
        &mut self,
        key: S::Key,
        fingerprint: u64,
        now: Duration,
        ttl: Duration,
    ) -> Claim<'_, S::Value> {
        self.admit(Attempt::new(key).fingerprint(fingerprint), now, ttl)
    }

    /// Claim an attempt without evicting any unexpired entry.
    #[must_use]
    pub fn admit(
        &mut self,
        attempt: Attempt<S::Key>,
        now: Duration,
        ttl: Duration,
    ) -> Claim<'_, S::Value> {
        if !self.observe(now) || ttl.is_zero() {
            return Claim::Rejected;
        }
        let Some(expires) = now.checked_add(ttl) else {
            return Claim::Rejected;
        };
        self.store.retain(|entry| now < entry.expires);
        let found = self.store.find(&attempt.key);
        if let Some(slot) = found {
            return self.revisit(slot, attempt.fingerprint);
        }
        if let (Some(cap), Some(partition)) = (self.partition_cap, attempt.partition)
            && self
                .store
                .iter()
                .filter(|entry| entry.partition == Some(partition))
                .count()
                >= cap
        {
            return Claim::PartitionFull;
        }
        let Some(generation) = self.generation.checked_add(1) else {
            return Claim::Rejected;
        };
        let Some(slot) = self.store.insert(Entry {
            key: attempt.key,
            fingerprint: attempt.fingerprint,
            partition: attempt.partition,
            generation,
            expires,
            state: State::Pending,
        }) else {
            return Claim::Full;
        };
        self.generation = generation;
        Claim::Acquired(ClaimId { slot, generation })
    }

    /// Complete a live pending claim. Repeated or stale completions return `false`.
    #[must_use]
    pub fn complete(&mut self, claim: ClaimId, now: Duration) -> bool
    where
        S::Value: Default,
    {
        self.complete_with(claim, S::Value::default(), now).is_ok()
    }

    /// Complete a live pending claim, storing `value` for duplicate attempts.
    /// Repeated or stale completions hand the value back.
    pub fn complete_with(
        &mut self,
        claim: ClaimId,
        value: S::Value,
        now: Duration,
    ) -> Result<(), S::Value> {
        if !self.observe(now) {
            return Err(value);
        }
        let Some(entry) = self.store.get_mut(claim.slot) else {
            return Err(value);
        };
        if entry.generation != claim.generation
            || !matches!(entry.state, State::Pending)
            || now >= entry.expires
        {
            return Err(value);
        }
        entry.state = State::Completed(value);
        Ok(())
    }

    /// Free a matching pending claim after a known failure, not an uncertain effect.
    #[must_use]
    pub fn abandon(&mut self, claim: ClaimId) -> bool {
        let pending = self.store.get(claim.slot).is_some_and(|entry| {
            entry.generation == claim.generation && matches!(entry.state, State::Pending)
        });
        pending && self.store.remove(claim.slot)
    }

    /// Drop every record, pending or completed, whose key `dead` marks as ownerless.
    /// Returns how many were dropped.
    pub fn expire_where(&mut self, mut dead: impl FnMut(&S::Key) -> bool) -> usize {
        self.store.retain(|entry| !dead(&entry.key))
    }

    fn revisit(&self, slot: usize, fingerprint: Option<u64>) -> Claim<'_, S::Value> {
        let Some(entry) = self.store.get(slot) else {
            return Claim::Rejected;
        };
        if entry.fingerprint != fingerprint {
            return Claim::Conflict;
        }
        match &entry.state {
            State::Pending => Claim::InProgress,
            State::Completed(value) => Claim::Completed(value),
        }
    }

    fn observe(&mut self, now: Duration) -> bool {
        if now < self.observed {
            return false;
        }
        self.observed = now;
        true
    }
}

#[cfg(test)]
mod tests;
