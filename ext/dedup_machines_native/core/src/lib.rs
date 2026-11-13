#![no_std]
#![doc = include_str!("../README.md")]

#[cfg(feature = "alloc")]
extern crate alloc;

mod claim;
#[cfg(feature = "alloc")]
mod hashed;
mod refusal;
mod storage;

use core::time::Duration;

use periscope_machines::Issuer;

use claim::State;
pub use claim::{Attempt, Claim, ClaimId, Completion, Entry};
#[cfg(feature = "alloc")]
pub use hashed::{HashDedup, Hashed};
pub use refusal::{Refusal, Refused};
pub use storage::{Fixed, Storage};

/// Exact-key claim ledger over a storage backend.
///
/// Equality compares the whole value: records, issued ids, observed clock and
/// partition cap. A refused call leaves the ledger equal to a clone taken
/// before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger<S> {
    store: S,
    /// Issues the claim ids; it also holds the ledger's scope.
    claims: Issuer,
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
    /// Empty ledger in scope zero.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_scope(0)
    }

    /// Empty ledger whose claim ids carry `scope`.
    ///
    /// A claim id is a plain integer, so two ledgers in the same scope issue
    /// the same ones. Give every ledger whose claim ids could meet its own
    /// scope: an id of another scope is then always [`Refusal::Stale`]. The
    /// ledger cannot pick a scope itself; keeping them distinct is the
    /// caller's job.
    #[must_use]
    pub const fn with_scope(scope: u16) -> Self {
        Self::fresh(Fixed::new(), Issuer::new(scope), Duration::ZERO)
    }
}

impl<S> Ledger<S> {
    pub(crate) const fn fresh(store: S, claims: Issuer, observed: Duration) -> Self {
        Self {
            store,
            claims,
            observed,
            partition_cap: None,
        }
    }
}

impl<S: Storage> Ledger<S> {
    /// Empty ledger in scope zero over `store`.
    ///
    /// `None` when `store` already holds a record: a ledger that has issued
    /// nothing owns none of them. Continue a persisted ledger with
    /// [`Ledger::resumed`].
    #[must_use]
    pub fn over(store: S) -> Option<Self> {
        Self::over_with_scope(store, 0)
    }

    /// Empty ledger over `store` whose claim ids carry `scope`, see
    /// [`Dedup::with_scope`]. `None` when `store` already holds a record.
    #[must_use]
    pub fn over_with_scope(store: S, scope: u16) -> Option<Self> {
        Self::resumed(store, scope, 0, Duration::ZERO)
    }

    /// Continue the ledger of `scope` whose records live in `store`, after
    /// `issued` claim ids ([`Ledger::issued`]) and from the last clock value
    /// it `observed` ([`Ledger::observed`]).
    ///
    /// The next claim carries sequence `issued + 1`, so a claim id that
    /// outlived the restart is never issued again. All three must come from
    /// the same committed state as the records: resuming from a lower count
    /// reissues ids. A ledger that cannot save its count takes a scope no
    /// earlier incarnation used instead.
    ///
    /// `None`, refusing the saved state, when `issued` is beyond
    /// `SEQUENCE_MAX` (2^48 - 1); when a record's claim id is not one this
    /// scope issued within `issued` (another scope, sequence zero, or beyond
    /// the count); or when two slots carry the same key or the same claim
    /// id, two identical records included. Each record must be the one its
    /// key and its claim id lead to: `find` and `locate` answer its slot.
    ///
    /// The partition cap is configuration, not saved state: set it again with
    /// [`Ledger::with_partition_cap`].
    #[must_use]
    pub fn resumed(store: S, scope: u16, issued: u64, observed: Duration) -> Option<Self> {
        let claims = Issuer::resumed(scope, issued)?;
        // A lookup answers one slot, so two slots under one key, or under
        // one claim id, cannot both be the slot their lookup answers.
        let sound = store.iter().all(|(slot, entry)| {
            claims.owns(entry.claim.raw())
                && store.find(&entry.key) == Some(slot)
                && store.locate(entry.claim) == Some(slot)
        });
        sound.then(|| Self::fresh(store, claims, observed))
    }

    /// The scope carried by every claim id this ledger issues.
    #[must_use]
    pub const fn scope(&self) -> u16 {
        self.claims.scope()
    }

    /// How many claim ids this ledger has issued; also the sequence number of
    /// the latest one. Persist it with the records.
    #[must_use]
    pub const fn issued(&self) -> u64 {
        self.claims.issued()
    }

    /// Latest clock value recorded by an honoured call; persist it with the
    /// records.
    #[must_use]
    pub const fn observed(&self) -> Duration {
        self.observed
    }

    /// The partition cap, if one is set.
    #[must_use]
    pub const fn partition_cap(&self) -> Option<usize> {
        self.partition_cap
    }

    /// The storage backend, for a persisting adapter to read through.
    ///
    /// It may still hold records whose TTL has elapsed: every call reads
    /// those as absent, and the next acquiring claim removes them.
    #[must_use]
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Give the storage back, to persist or hand to [`Ledger::resumed`].
    #[must_use]
    pub fn into_store(self) -> S {
        self.store
    }

    /// Cap live records per partition; attempts without a partition are uncounted.
    #[must_use]
    pub const fn with_partition_cap(mut self, cap: usize) -> Self {
        self.partition_cap = Some(cap);
        self
    }

    /// Claim a key without evicting any live record. See [`Ledger::admit`].
    ///
    /// # Errors
    /// As [`Ledger::admit`], without [`Refusal::PartitionFull`].
    pub fn claim(
        &mut self,
        key: S::Key,
        now: Duration,
        ttl: Duration,
    ) -> Result<Claim<'_, S::Value>, Refusal> {
        self.admit(Attempt::new(key), now, ttl)
    }

    /// Claim a key under a payload fingerprint. See [`Ledger::admit`].
    ///
    /// # Errors
    /// As [`Ledger::admit`], without [`Refusal::PartitionFull`].
    pub fn claim_with(
        &mut self,
        key: S::Key,
        fingerprint: u64,
        now: Duration,
        ttl: Duration,
    ) -> Result<Claim<'_, S::Value>, Refusal> {
        self.admit(Attempt::new(key).fingerprint(fingerprint), now, ttl)
    }

    /// Ask whether the attempt may run, without evicting any live record.
    ///
    /// A record counts until `now` reaches its expiry and is read as absent
    /// from then on. Every honoured answer records `now` as observed.
    /// [`Claim::InProgress`] and [`Claim::Completed`] change nothing else.
    /// [`Claim::Acquired`] also removes every record that expired by `now`,
    /// stores the new pending record and issues its id.
    ///
    /// # Errors
    /// In this order: [`Refusal::TimeRegression`], [`Refusal::InvalidDuration`]
    /// for a zero TTL, [`Refusal::Overflow`] when `now + ttl` does not fit,
    /// [`Refusal::Conflict`] when the key is live under another fingerprint,
    /// [`Refusal::PartitionFull`], [`Refusal::Exhausted`] when the scope's
    /// claim ids are spent, [`Refusal::Full`]. A refusal changes nothing, and
    /// in particular removes no expired record.
    pub fn admit(
        &mut self,
        attempt: Attempt<S::Key>,
        now: Duration,
        ttl: Duration,
    ) -> Result<Claim<'_, S::Value>, Refusal> {
        self.check_time(now)?;
        if ttl.is_zero() {
            return Err(Refusal::InvalidDuration);
        }
        let expires = now.checked_add(ttl).ok_or(Refusal::Overflow)?;
        let held = self.store.find(&attempt.key).filter(|&slot| {
            let entry = self.store.get(slot);
            entry.is_some_and(|entry| entry.live(now))
        });
        if let Some(slot) = held {
            return self.revisit(slot, attempt.fingerprint, now);
        }
        if let (Some(cap), Some(partition)) = (self.partition_cap, attempt.partition) {
            let counted = |(_, entry): &(usize, &Entry<_, _>)| {
                entry.live(now) && entry.partition == Some(partition)
            };
            if self.store.iter().filter(counted).count() >= cap {
                return Err(Refusal::PartitionFull);
            }
        }
        // Issue on a copy: the id is consumed only once the record is stored.
        let mut claims = self.claims;
        let claim = match claims.issue() {
            Ok(raw) => ClaimId::from_raw(raw),
            Err(periscope_machines::Refusal::Exhausted) => return Err(Refusal::Exhausted),
        };
        // Removing the expired is an effect of an honoured claim only. With
        // none to remove, a full backend refuses the insert and stores nothing;
        // with one removed, the insert has room (the `Storage` contract).
        if self.store.iter().any(|(_, entry)| !entry.live(now)) {
            self.store.retain(|entry| entry.live(now));
        }
        self.store
            .insert(Entry {
                key: attempt.key,
                fingerprint: attempt.fingerprint,
                partition: attempt.partition,
                claim,
                expires,
                state: State::Pending,
            })
            .ok_or(Refusal::Full)?;
        self.claims = claims;
        self.observed = now;
        Ok(Claim::Acquired(claim))
    }

    /// Complete a live pending claim with `S::Value::default()`. See
    /// [`Ledger::complete_with`].
    ///
    /// # Errors
    /// As [`Ledger::complete_with`]; there is no value to hand back.
    pub fn complete(&mut self, claim: ClaimId, now: Duration) -> Result<Completion, Refusal>
    where
        S::Value: Default + PartialEq,
    {
        self.complete_with(claim, S::Value::default(), now)
            .map_err(|refused| refused.refusal)
    }

    /// Complete a live pending claim, storing `value` for duplicate attempts.
    ///
    /// [`Completion::Completed`] records the value and `now` as observed; the
    /// record keeps its original expiry. Completing the same live claim again
    /// with an equal value is the idempotent repeat
    /// [`Completion::AlreadyCompleted`]: the value is dropped, being equal to
    /// the one recorded, and nothing changes.
    ///
    /// # Errors
    /// A [`Refused`] carrying `value` back with the reason, in this order:
    /// [`Refusal::Stale`] for an id this ledger never issued (another scope,
    /// or a sequence it has not reached), [`Refusal::TimeRegression`],
    /// [`Refusal::Stale`] for a claim that is not live at `now` (expired by
    /// then, abandoned, dropped), [`Refusal::Mismatch`] for a completed
    /// claim whose recorded value is not `value`.
    ///
    /// `Stale` for an expired claim is an answer about `now`, not a recorded
    /// fact: like every refusal it leaves the observed clock alone, so the
    /// same completion at an earlier instant, not behind the latest honoured
    /// call, is honoured. An honoured `Acquired` of the key is what ends the
    /// old id for good.
    pub fn complete_with(
        &mut self,
        claim: ClaimId,
        value: S::Value,
        now: Duration,
    ) -> Result<Completion, Refused<S::Value>>
    where
        S::Value: PartialEq,
    {
        let slot = match self.completable(claim, &value, now) {
            Ok(Some(slot)) => slot,
            Ok(None) => return Ok(Completion::AlreadyCompleted),
            Err(refusal) => return Err(Refused { refusal, value }),
        };
        let Some(entry) = self.store.get_mut(slot) else {
            let refusal = Refusal::Stale;
            return Err(Refused { refusal, value });
        };
        entry.state = State::Completed(value);
        self.observed = now;
        Ok(Completion::Completed)
    }

    /// Free a live pending claim after a known failure, not an uncertain
    /// effect. The key is claimable again at once.
    ///
    /// The call carries no clock: a claim is live here until the *observed*
    /// clock reaches its expiry. A repeat is not honoured: the slot is freed
    /// and no history is kept, so the ledger cannot tell an abandoned claim
    /// from an expired or dropped one.
    ///
    /// # Errors
    /// [`Refusal::Stale`] for an id this ledger never issued or one that is
    /// no longer live; [`Refusal::Concluded`] for a completed claim.
    pub fn abandon(&mut self, claim: ClaimId) -> Result<(), Refusal> {
        if !self.claims.owns(claim.raw()) {
            return Err(Refusal::Stale);
        }
        let slot = self
            .live_claim(claim, self.observed)
            .ok_or(Refusal::Stale)?;
        let pending = self.store.get(slot).is_some_and(Entry::is_pending);
        if !pending {
            return Err(Refusal::Concluded);
        }
        if self.store.remove(slot) {
            Ok(())
        } else {
            Err(Refusal::Stale)
        }
    }

    /// Drain: drop every live record, pending or completed, whose key `dead`
    /// marks as ownerless, and return how many were dropped.
    ///
    /// Records that expired by the observed clock are removed along the way,
    /// uncounted and without being shown to `dead`: they were already absent.
    pub fn expire_where(&mut self, mut dead: impl FnMut(&S::Key) -> bool) -> usize {
        let observed = self.observed;
        let mut dropped = 0;
        self.store.retain(|entry| {
            if !entry.live(observed) {
                return false;
            }
            let drop = dead(&entry.key);
            dropped += usize::from(drop);
            !drop
        });
        dropped
    }

    /// The slot of a pending claim to complete, `None` for a provable repeat.
    fn completable(
        &self,
        claim: ClaimId,
        value: &S::Value,
        now: Duration,
    ) -> Result<Option<usize>, Refusal>
    where
        S::Value: PartialEq,
    {
        if !self.claims.owns(claim.raw()) {
            return Err(Refusal::Stale);
        }
        self.check_time(now)?;
        let slot = self.live_claim(claim, now).ok_or(Refusal::Stale)?;
        match self.store.get(slot).map(|entry| &entry.state) {
            Some(State::Pending) => Ok(Some(slot)),
            Some(State::Completed(recorded)) if recorded == value => Ok(None),
            Some(State::Completed(_)) => Err(Refusal::Mismatch),
            None => Err(Refusal::Stale),
        }
    }

    /// The slot of the record claimed under `claim`, if it is live at `now`.
    fn live_claim(&self, claim: ClaimId, now: Duration) -> Option<usize> {
        self.store.locate(claim).filter(|&slot| {
            let entry = self.store.get(slot);
            entry.is_some_and(|entry| entry.claim == claim && entry.live(now))
        })
    }

    fn revisit(
        &mut self,
        slot: usize,
        fingerprint: Option<u64>,
        now: Duration,
    ) -> Result<Claim<'_, S::Value>, Refusal> {
        let entry = self.store.get(slot);
        if entry.is_none_or(|entry| entry.fingerprint != fingerprint) {
            return Err(Refusal::Conflict);
        }
        self.observed = now;
        match self.store.get(slot).map(|entry| &entry.state) {
            Some(State::Completed(value)) => Ok(Claim::Completed(value)),
            // `None` cannot happen: the slot was read a line above.
            Some(State::Pending) | None => Ok(Claim::InProgress),
        }
    }

    fn check_time(&self, now: Duration) -> Result<(), Refusal> {
        if now < self.observed {
            Err(Refusal::TimeRegression)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
