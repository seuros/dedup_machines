use core::fmt;

/// Why the ledger did not honour a call. A refused call changed nothing: no
/// record, no observed clock, no issued id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `now` is earlier than the latest instant the ledger observed.
    TimeRegression,
    /// The TTL is zero: the claim would be dead on arrival.
    InvalidDuration,
    /// `now + ttl` does not fit a `Duration`.
    Overflow,
    /// The ledger has issued every claim id of its scope.
    Exhausted,
    /// A live record for this key carries a different fingerprint (or none
    /// where this attempt has one): the key is being reused for another
    /// payload.
    Conflict,
    /// Every slot holds a live record; nothing is evicted.
    Full,
    /// The attempt's partition holds its quota of live records.
    PartitionFull,
    /// The claim id names no live claim of this ledger: it is expired at the
    /// instant the call is evaluated at, was abandoned or dropped, belongs to
    /// another scope, or was never issued. For an expired claim that is an
    /// answer about the instant, not a recorded fact.
    Stale,
    /// The claim is completed; a completed claim cannot be abandoned.
    Concluded,
    /// The claim is already completed with a value other than the one this
    /// call carries; the recorded value stands.
    Mismatch,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TimeRegression => "now is earlier than the observed clock",
            Self::InvalidDuration => "the ttl is zero",
            Self::Overflow => "now + ttl overflows",
            Self::Exhausted => "every claim id of this scope has been issued",
            Self::Conflict => "the key is live under a different fingerprint",
            Self::Full => "every slot holds a live record",
            Self::PartitionFull => "the partition holds its quota of live records",
            Self::Stale => "the claim id names no live claim of this ledger",
            Self::Concluded => "the claim is completed",
            Self::Mismatch => "the claim is completed with a different value",
        })
    }
}

impl core::error::Error for Refusal {}

/// A refused [`Ledger::complete_with`](crate::Ledger::complete_with): the
/// reason, and the value the call carried, handed back untouched.
///
/// This is not a second refusal type: the reason is the crate's one
/// [`Refusal`]. It exists because the call takes the value by move, and a
/// refusal must not swallow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused<V> {
    /// Why the completion was not honoured.
    pub refusal: Refusal,
    /// The value the call carried.
    pub value: V,
}

impl<V> fmt::Display for Refused<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.refusal.fmt(f)
    }
}

impl<V: fmt::Debug> core::error::Error for Refused<V> {}
