//! A refusal changes nothing; an honoured repeat changes nothing either.

use super::*;
use matryona_machines::testing::{refuses, unchanged};
use periscope_machines::SEQUENCE_MAX;

/// One record that expired at 1 (key 9), one live fingerprinted pending
/// record in partition 4 (key 1), one live completed record (key 2), a free
/// slot, a partition cap of 1 and the clock at 2.
fn busy() -> (Dedup<u64, 4, u8>, ClaimId, ClaimId, ClaimId) {
    let mut ledger = Dedup::<u64, 4, u8>::new().with_partition_cap(1);
    let expired = acquired(ledger.claim(9, at(0), at(1)));
    let attempt = Attempt::new(1).fingerprint(0xA).partition(4);
    let pending = acquired(ledger.admit(attempt, at(0), at(20)));
    let done = acquired(ledger.claim(2, at(0), at(20)));
    assert_eq!(
        ledger.complete_with(done, 7, at(2)),
        Ok(Completion::Completed)
    );
    (ledger, expired, pending, done)
}

#[test]
fn a_refused_claim_changes_nothing() {
    let (mut ledger, ..) = busy();
    let key = |key: u64| Attempt::new(key);
    let refused = [
        (key(5), at(1), at(5), Refusal::TimeRegression),
        (key(5), at(3), Duration::ZERO, Refusal::InvalidDuration),
        (key(5), Duration::MAX, at(1), Refusal::Overflow),
        // Arguments are checked before the key is looked up.
        (key(1), at(1), Duration::ZERO, Refusal::TimeRegression),
        (key(1), at(3), Duration::ZERO, Refusal::InvalidDuration),
        (key(1), at(3), at(5), Refusal::Conflict),
        (key(1).fingerprint(0xB), at(3), at(5), Refusal::Conflict),
        (key(2).fingerprint(0xB), at(3), at(5), Refusal::Conflict),
        (key(5).partition(4), at(3), at(5), Refusal::PartitionFull),
    ];
    for (attempt, now, ttl, why) in refused {
        refuses(&mut ledger, why, |l| l.admit(attempt, now, ttl).map(drop));
    }
    // The expired record is still held: no refusal removed it.
    assert_eq!(ledger.store().iter().count(), 3);
}

#[test]
fn a_full_table_refuses_without_removing_or_observing() {
    let mut ledger = Dedup::<u64, 2>::new();
    acquired(ledger.claim(1, at(0), at(5)));
    acquired(ledger.claim(2, at(0), at(9)));
    refuses(&mut ledger, Refusal::Full, |l| {
        l.claim(3, at(4), at(9)).map(drop)
    });
    assert_eq!(
        Dedup::<u64, 0>::new().claim(0, at(0), at(1)),
        Err(Refusal::Full)
    );
    // Once key 1 has expired the same call is honoured, in its slot.
    acquired(ledger.claim(3, at(5), at(9)));
    assert_eq!(ledger.claim(1, at(5), at(9)), Err(Refusal::Full));
}

#[test]
fn an_exhausted_scope_refuses_without_removing_or_observing() {
    let store = Fixed::<u64, (), 2>::new();
    let Some(mut ledger) = Ledger::resumed(store, 3, SEQUENCE_MAX - 1, at(0)) else {
        panic!("the last id is still to issue");
    };
    let last = acquired(ledger.claim(9, at(0), at(1)));
    assert_eq!(last.raw(), periscope_machines::scoped(3, SEQUENCE_MAX));
    // Key 9 has expired and its slot is free to take, but no id is left.
    refuses(&mut ledger, Refusal::Exhausted, |l| {
        l.claim(1, at(2), at(9)).map(drop)
    });
    assert!(Ledger::resumed(Fixed::<u64, (), 2>::new(), 3, SEQUENCE_MAX + 1, at(0)).is_none());
}

#[test]
fn a_refused_completion_changes_nothing_and_returns_the_value() {
    let (mut ledger, expired, pending, done) = busy();
    let never = id(ledger.issued() + 1);
    let refused = [
        (pending, 1, at(1), Refusal::TimeRegression),
        (done, 7, at(1), Refusal::TimeRegression),
        // An id never issued is refused before the clock is read.
        (never, 1, at(1), Refusal::Stale),
        (expired, 1, at(3), Refusal::Stale),
        (pending, 1, at(20), Refusal::Stale),
        (done, 7, at(20), Refusal::Stale),
        (done, 8, at(3), Refusal::Mismatch),
    ];
    for (claim, value, now, refusal) in refused {
        refuses(&mut ledger, Refused { refusal, value }, |l| {
            l.complete_with(claim, value, now)
        });
    }
    // `complete` is `complete_with` the default value: 0 is not the 7 recorded.
    refuses(&mut ledger, Refusal::Mismatch, |l| l.complete(done, at(3)));
}

#[test]
fn a_repeated_completion_is_honoured_and_changes_nothing() {
    let (mut ledger, _, pending, done) = busy();
    // Same claim, same value, any acceptable instant while the record lives.
    for now in [2, 3, 19] {
        let got = unchanged(&mut ledger, |l| l.complete_with(done, 7, at(now)));
        assert_eq!(got, Ok(Completion::AlreadyCompleted), "a repeat at {now}");
    }
    assert_eq!(ledger.complete(pending, at(4)), Ok(Completion::Completed));
    assert_eq!(
        unchanged(&mut ledger, |l| l.complete(pending, at(9))),
        Ok(Completion::AlreadyCompleted)
    );
    assert_eq!(
        unchanged(&mut ledger, |l| l.complete_with(pending, 0, at(9))),
        Ok(Completion::AlreadyCompleted)
    );
    assert_eq!(ledger.observed(), at(4));
}

#[test]
fn a_refused_abandon_changes_nothing() {
    let (mut ledger, expired, pending, done) = busy();
    let never = id(ledger.issued() + 1);
    let refused = [
        (never, Refusal::Stale),
        (expired, Refusal::Stale),
        (done, Refusal::Concluded),
    ];
    for (claim, why) in refused {
        refuses(&mut ledger, why, |l| l.abandon(claim));
    }
    assert_eq!(ledger.abandon(pending), Ok(()));
    // A repeat is not provable: the slot is free and no history is kept.
    refuses(&mut ledger, Refusal::Stale, |l| l.abandon(pending));
    refuses(&mut ledger, Refusal::Stale, |l| l.complete(pending, at(3)));
}

#[test]
fn abandon_reads_expiry_from_the_observed_clock() {
    let mut ledger = Dedup::<u64, 2>::new();
    let slow = acquired(ledger.claim(1, at(0), at(5)));
    // The clock the ledger has seen says the claim is live: honoured.
    let mut early = ledger.clone();
    assert_eq!(early.abandon(slow), Ok(()));
    // Once an honoured call has shown it instant 5, the claim is gone.
    acquired(ledger.claim(2, at(5), at(5)));
    assert_eq!(ledger.abandon(slow), Err(Refusal::Stale));
}

/// `Stale` for an expired claim is an answer about the instant supplied: the
/// refusal records no clock, so the same completion at an earlier instant,
/// not behind the latest honoured one, is honoured. An honoured claim of the
/// key is what ends the old id for good.
#[test]
fn a_refused_expiry_is_not_remembered() {
    let mut ledger = Dedup::<u64, 2, u8>::new();
    let id = acquired(ledger.claim(1, at(0), at(10)));
    let refused = Refused {
        refusal: Refusal::Stale,
        value: 7,
    };
    refuses(&mut ledger, refused, |l| l.complete_with(id, 7, at(10)));
    assert_eq!(ledger.observed(), at(0));
    assert_eq!(
        ledger.complete_with(id, 7, at(5)),
        Ok(Completion::Completed)
    );
    assert_eq!(ledger.observed(), at(5));
    assert_eq!(ledger.claim(1, at(6), at(10)), Ok(Claim::Completed(&7)));

    // The other order: the key is claimed again at 10, and that is recorded.
    let mut ledger = Dedup::<u64, 2, u8>::new();
    let old = acquired(ledger.claim(1, at(0), at(10)));
    assert_eq!(ledger.complete(old, at(10)), Err(Refusal::Stale));
    // `abandon` carries no clock and reads the observed one, still 0.
    let mut abandoning = ledger.clone();
    assert_eq!(abandoning.abandon(old), Ok(()));
    let fresh = acquired(ledger.claim(1, at(10), at(10)));
    refuses(&mut ledger, Refusal::TimeRegression, |l| {
        l.complete(old, at(5))
    });
    refuses(&mut ledger, Refusal::Stale, |l| l.complete(old, at(10)));
    refuses(&mut ledger, Refusal::Stale, |l| l.abandon(old));
    assert_eq!(ledger.complete(fresh, at(10)), Ok(Completion::Completed));
}
