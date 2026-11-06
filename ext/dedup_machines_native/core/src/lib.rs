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

/// One record in a [`Storage`] backend: a claimed key, its generation and
/// expiry, and whether the claim is still pending or carries a completed value.
///
/// A [`Ledger`] builds entries and hands them to its storage; a backend that
/// persists them elsewhere reads them back through the accessors and rebuilds
/// them with [`Entry::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<K, V> {
    key: K,
    fingerprint: Option<u64>,
    partition: Option<u64>,
    generation: u64,
    expires: Duration,
    state: State<V>,
}

impl<K, V> Entry<K, V> {
    /// Rebuild a stored record; `value` is `None` while the claim is pending.
    #[must_use]
    pub fn new(
        key: K,
        fingerprint: Option<u64>,
        partition: Option<u64>,
        generation: u64,
        expires: Duration,
        value: Option<V>,
    ) -> Self {
        Self {
            key,
            fingerprint,
            partition,
            generation,
            expires,
            state: value.map_or(State::Pending, State::Completed),
        }
    }

    /// The claimed key.
    #[must_use]
    pub const fn key(&self) -> &K {
        &self.key
    }

    /// Payload fingerprint the key was claimed under, if any.
    #[must_use]
    pub const fn fingerprint(&self) -> Option<u64> {
        self.fingerprint
    }

    /// Partition the record is counted against, if any.
    #[must_use]
    pub const fn partition(&self) -> Option<u64> {
        self.partition
    }

    /// Generation the claim was issued with; stale [`ClaimId`]s carry an older one.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Instant after which the record is dropped on the next observation.
    #[must_use]
    pub const fn expires(&self) -> Duration {
        self.expires
    }

    /// `true` while the claim is live and uncompleted.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self.state, State::Pending)
    }

    /// The completed value, once there is one.
    #[must_use]
    pub const fn value(&self) -> Option<&V> {
        match &self.state {
            State::Completed(value) => Some(value),
            State::Pending => None,
        }
    }
}

/// Where a [`Ledger`] keeps its records.
///
/// [`Fixed`] and [`Hashed`] are in-memory. Implement this to put the ledger
/// over something else, such as a database table or a shared-memory arena;
/// the ledger only ever addresses records by the slot `insert` returned.
///
/// Slots are opaque: a backend may reuse one after `remove`, because the
/// ledger's generation check tells a stale [`ClaimId`] from a fresh record.
pub trait Storage {
    /// Claimed key type.
    type Key: Eq;
    /// Value stored on completion.
    type Value;

    /// The record at `slot`, if one is live.
    fn get(&self, slot: usize) -> Option<&Entry<Self::Key, Self::Value>>;
    /// Mutable access to the record at `slot`, if one is live.
    fn get_mut(&mut self, slot: usize) -> Option<&mut Entry<Self::Key, Self::Value>>;
    /// The slot holding `key`, if any.
    fn find(&self, key: &Self::Key) -> Option<usize>;
    /// Store `entry` in a free slot, or `None` when the backend is full.
    fn insert(&mut self, entry: Entry<Self::Key, Self::Value>) -> Option<usize>;
    /// Drop the record at `slot`; `false` when there was none.
    fn remove(&mut self, slot: usize) -> bool;
    /// Drop every record `keep` rejects, returning how many were dropped.
    fn retain(&mut self, keep: impl FnMut(&Entry<Self::Key, Self::Value>) -> bool) -> usize;
    /// Every live record, in no particular order.
    fn iter(&self) -> impl Iterator<Item = &Entry<Self::Key, Self::Value>>;
}

/// Fixed-capacity inline storage.
#[derive(Debug, PartialEq, Eq)]
pub struct Fixed<K, V, const N: usize> {
    entries: [Option<Entry<K, V>>; N],
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
    /// Start an empty ledger over `store` with a fresh handle namespace.
    ///
    /// The store is expected to be empty: records it already holds would
    /// outrank every [`ClaimId`] this ledger issues. Use [`Ledger::restore`]
    /// to continue a persisted one.
    #[must_use]
    pub const fn over(store: S) -> Self {
        Self::restore(store, 0, Duration::ZERO)
    }

    /// Continue a ledger whose records live in `store`, from the highest
    /// `generation` it ever issued and the last clock value it `observed`.
    ///
    /// Both must come from the same committed state as the records, so no
    /// stored [`ClaimId`] can be reissued.
    #[must_use]
    pub const fn restore(store: S, generation: u64, observed: Duration) -> Self {
        Self {
            store,
            generation,
            observed,
            partition_cap: None,
        }
    }

    /// Highest generation issued so far; persist it with the records.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Latest clock value accepted; persist it with the records.
    #[must_use]
    pub const fn observed(&self) -> Duration {
        self.observed
    }

    /// The storage backend, for a persisting adapter to read through.
    #[must_use]
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Give the storage back, to persist or hand to [`Ledger::restore`].
    #[must_use]
    pub fn into_store(self) -> S {
        self.store
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
    ///
    /// # Errors
    ///
    /// Repeated, stale, or expired completions hand `value` back.
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
