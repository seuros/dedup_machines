use super::*;
use crate::test_support::{acquired, at, plant};

mod backend;
pub mod model;
mod refusals;
mod scope;

#[test]
fn lifecycle_is_exact_and_full_tables_do_not_evict() {
    let mut ledger = Dedup::<u64, 1>::new();
    let id = acquired(ledger.claim(7, at(0), at(10)));
    assert_eq!((id.raw(), id.scope()), (1, 0));
    assert_eq!(ledger.claim(7, at(1), at(10)), Ok(Claim::InProgress));
    assert_eq!(ledger.claim(8, at(1), at(10)), Err(Refusal::Full));
    assert_eq!(ledger.complete(id, at(2)), Ok(Completion::Completed));
    assert_eq!(ledger.complete(id, at(2)), Ok(Completion::AlreadyCompleted));
    assert_eq!(ledger.claim(7, at(9), at(1)), Ok(Claim::Completed(&())));
    let replacement = acquired(ledger.claim(7, at(10), at(5)));
    assert_eq!(replacement.raw(), 2);
    assert_eq!(ledger.complete(id, at(10)), Err(Refusal::Stale));
    assert_eq!(
        ledger.complete(replacement, at(10)),
        Ok(Completion::Completed)
    );
}

#[test]
fn expired_pending_workers_cannot_complete_replacements() {
    let mut ledger = Dedup::<u64, 1>::new();
    let stale = acquired(ledger.claim(7, at(0), at(1)));
    let fresh = acquired(ledger.claim(7, at(1), at(1)));
    assert_eq!(ledger.complete(stale, at(1)), Err(Refusal::Stale));
    assert_eq!(ledger.abandon(stale), Err(Refusal::Stale));
    assert_eq!(ledger.abandon(fresh), Ok(()));
    assert_eq!(ledger.abandon(fresh), Err(Refusal::Stale));
}

#[test]
fn fingerprints_conflict_across_pending_and_completed_records() {
    let mut ledger = Dedup::<u64, 2>::new();
    let id = acquired(ledger.claim_with(7, 0xA, at(0), at(10)));
    assert_eq!(
        ledger.claim_with(7, 0xB, at(1), at(10)),
        Err(Refusal::Conflict)
    );
    assert_eq!(ledger.claim(7, at(1), at(10)), Err(Refusal::Conflict));
    assert_eq!(
        ledger.claim_with(7, 0xA, at(1), at(10)),
        Ok(Claim::InProgress)
    );
    assert_eq!(ledger.complete(id, at(2)), Ok(Completion::Completed));
    assert_eq!(
        ledger.claim_with(7, 0xB, at(3), at(10)),
        Err(Refusal::Conflict)
    );
    assert_eq!(
        ledger.claim_with(7, 0xA, at(3), at(10)),
        Ok(Claim::Completed(&()))
    );
    // The record expired: the key is free for another payload.
    acquired(ledger.claim_with(7, 0xB, at(10), at(10)));
}

#[test]
fn completion_values_are_returned_to_duplicates_and_stale_completers() {
    let mut ledger = Dedup::<u64, 1, u32>::new();
    let id = acquired(ledger.claim(7, at(0), at(10)));
    assert_eq!(
        ledger.complete_with(id, 42, at(1)),
        Ok(Completion::Completed)
    );
    let refused = |refusal, value| Err(Refused { refusal, value });
    assert_eq!(
        ledger.complete_with(id, 43, at(1)),
        refused(Refusal::Mismatch, 43)
    );
    assert_eq!(ledger.claim(7, at(2), at(10)), Ok(Claim::Completed(&42)));
    let replacement = acquired(ledger.claim(7, at(10), at(1)));
    assert_eq!(
        ledger.complete_with(id, 44, at(10)),
        refused(Refusal::Stale, 44)
    );
    assert_eq!(
        ledger.complete_with(replacement, 45, at(11)),
        refused(Refusal::Stale, 45)
    );
}

#[test]
fn partition_caps_bound_each_bucket_below_the_table() {
    let mut ledger = Dedup::<u64, 3>::new().with_partition_cap(1);
    assert_eq!(ledger.partition_cap(), Some(1));
    let first = acquired(ledger.admit(Attempt::new(1).partition(9), at(0), at(10)));
    assert_eq!(
        ledger.admit(Attempt::new(2).partition(9), at(0), at(10)),
        Err(Refusal::PartitionFull)
    );
    acquired(ledger.admit(Attempt::new(2).partition(8), at(0), at(10)));
    acquired(ledger.claim(3, at(0), at(10)));
    assert_eq!(ledger.abandon(first), Ok(()));
    acquired(ledger.admit(Attempt::new(1).partition(9), at(1), at(10)));
    assert_eq!(
        ledger.admit(Attempt::new(4).partition(7), at(1), at(10)),
        Err(Refusal::Full)
    );
}

#[test]
fn ownerless_records_expire_ahead_of_their_ttl() {
    let mut ledger = Dedup::<(u8, u8), 3>::new();
    let dead = acquired(ledger.claim((1, 1), at(0), at(10)));
    let done = acquired(ledger.claim((1, 2), at(0), at(10)));
    let alive = acquired(ledger.claim((2, 1), at(0), at(10)));
    assert_eq!(ledger.complete(done, at(0)), Ok(Completion::Completed));
    assert_eq!(ledger.expire_where(|key| key.0 == 1), 2);
    assert_eq!(ledger.expire_where(|key| key.0 == 1), 0);
    assert_eq!(ledger.abandon(dead), Err(Refusal::Stale));
    acquired(ledger.claim((1, 2), at(1), at(10)));
    assert_eq!(ledger.complete(alive, at(1)), Ok(Completion::Completed));
}

#[test]
fn expiry_is_read_lazily_and_applied_by_acquiring_claims() {
    let mut ledger = Dedup::<u64, 2, u8>::new();
    let short = acquired(ledger.claim(1, at(0), at(2)));
    let long = acquired(ledger.claim(2, at(0), at(9)));
    assert_eq!(
        ledger.complete_with(long, 5, at(1)),
        Ok(Completion::Completed)
    );
    // Honoured non-acquiring answers move the observed clock and nothing else.
    let before = ledger.clone();
    assert_eq!(ledger.claim(2, at(3), at(9)), Ok(Claim::Completed(&5)));
    assert_eq!(ledger.observed(), at(3));
    assert_eq!(ledger.store(), before.store());
    assert_eq!(ledger.issued(), before.issued());
    // Key 1 expired at 2: still held, read as absent by every call.
    assert_eq!(ledger.store().iter().count(), 2);
    assert_eq!(ledger.abandon(short), Err(Refusal::Stale));
    assert_eq!(
        ledger.complete_with(short, 0, at(3)),
        Err(Refused {
            refusal: Refusal::Stale,
            value: 0,
        })
    );
    // The drain removes it without counting it or showing it to the caller.
    let mut drained = ledger.clone();
    assert_eq!(
        drained.expire_where(|key| {
            assert_ne!(*key, 1, "an expired record was offered");
            false
        }),
        0
    );
    assert_eq!(drained.store().iter().count(), 1);
    // An acquiring claim removes it and takes its slot, in a full table.
    let fresh = acquired(ledger.claim(1, at(3), at(9)));
    assert_eq!(fresh.raw(), 3);
    assert_eq!(ledger.store().iter().count(), 2);
    assert_eq!(
        ledger
            .store()
            .iter()
            .filter(|(_, e)| e.claim() == short)
            .count(),
        0
    );
}

#[test]
fn refusals_format_and_are_errors() {
    fn error<E: core::error::Error>(_: &E) {}
    let all = [
        Refusal::TimeRegression,
        Refusal::InvalidDuration,
        Refusal::Overflow,
        Refusal::Exhausted,
        Refusal::Conflict,
        Refusal::Full,
        Refusal::PartitionFull,
        Refusal::Stale,
        Refusal::Concluded,
        Refusal::Mismatch,
    ];
    for (i, refusal) in all.iter().enumerate() {
        error(refusal);
        let text = std::format!("{refusal}");
        assert_ne!(text, "");
        let refused = Refused {
            refusal: *refusal,
            value: 7_u8,
        };
        error(&refused);
        assert_eq!(std::format!("{refused}"), text);
        for other in &all[i + 1..] {
            assert_ne!(std::format!("{other}"), text);
        }
    }
}
