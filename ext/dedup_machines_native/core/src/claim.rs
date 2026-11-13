use core::time::Duration;

use periscope_machines::scope_of;

/// Identity of one acquired claim.
///
/// Issued once per ledger and never again: it carries the ledger's scope in
/// its top sixteen bits above a sequence that starts at one. It is a plain
/// integer underneath ([`ClaimId::raw`]), so it can cross a queue or a
/// database column and be rebuilt with [`ClaimId::from_raw`]; a rebuilt id
/// the ledger never issued is [`Refusal::Stale`](crate::Refusal::Stale).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClaimId(u64);

impl ClaimId {
    /// Rebuild an id from its integer form. Any integer is accepted here; the
    /// ledger decides whether it names a claim.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The integer form: scope above, sequence below.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// The scope of the ledger that issued this id.
    #[must_use]
    pub const fn scope(self) -> u16 {
        scope_of(self.0)
    }
}

/// Honoured answer to "may I run this key?".
///
/// All three are truthful answers to a well-formed question; none is a
/// refusal. Only [`Claim::Acquired`] changes the records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim<'a, V> {
    /// Yes: the caller owns a new pending claim.
    Acquired(ClaimId),
    /// No: another live attempt owns this key.
    InProgress,
    /// No: this key completed within its retention interval, with this value.
    Completed(&'a V),
}

/// Honoured outcome of [`Ledger::complete`](crate::Ledger::complete) and
/// [`Ledger::complete_with`](crate::Ledger::complete_with).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// The pending claim is now completed and its value recorded.
    Completed,
    /// An idempotent repeat: this same claim is already completed with this
    /// same value. Nothing changed, not even the observed clock.
    AlreadyCompleted,
}

/// A key plus the optional fingerprint and partition it is claimed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt<K> {
    pub(crate) key: K,
    pub(crate) fingerprint: Option<u64>,
    pub(crate) partition: Option<u64>,
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
pub enum State<V> {
    Pending,
    Completed(V),
}

/// One record in a [`Storage`](crate::Storage) backend: a claimed key, the id
/// of its claim, its expiry, and whether the claim is still pending or
/// carries a completed value.
///
/// A [`Ledger`](crate::Ledger) builds entries and hands them to its storage;
/// a backend that persists them elsewhere reads them back through the
/// accessors and rebuilds them with [`Entry::new`]. Rebuilt entries are
/// checked by [`Ledger::resumed`](crate::Ledger::resumed), not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<K, V> {
    pub(crate) key: K,
    pub(crate) fingerprint: Option<u64>,
    pub(crate) partition: Option<u64>,
    pub(crate) claim: ClaimId,
    pub(crate) expires: Duration,
    pub(crate) state: State<V>,
}

impl<K, V> Entry<K, V> {
    /// Rebuild a stored record; `value` is `None` while the claim is pending.
    #[must_use]
    pub fn new(
        key: K,
        fingerprint: Option<u64>,
        partition: Option<u64>,
        claim: ClaimId,
        expires: Duration,
        value: Option<V>,
    ) -> Self {
        Self {
            key,
            fingerprint,
            partition,
            claim,
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

    /// The id the claim was issued under.
    #[must_use]
    pub const fn claim(&self) -> ClaimId {
        self.claim
    }

    /// First instant at which the record no longer counts. Every call reads
    /// a record as absent from here on; an acquiring claim removes it.
    #[must_use]
    pub const fn expires(&self) -> Duration {
        self.expires
    }

    /// `true` while the claim is uncompleted.
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

    /// Whether the record still counts at `now`.
    pub(crate) fn live(&self, now: Duration) -> bool {
        now < self.expires
    }
}
