use alloc::string::{String, ToString};
use core::time::Duration;

use super::*;
use crate::test_support::{acquired, at, plant};
use crate::{Attempt, Claim, ClaimId, Completion, Entry, Refusal, Refused};

#[test]
fn owned_keys_follow_the_fixed_lifecycle() {
    let mut ledger = HashDedup::<String, String>::with_capacity(1);
    let id = acquired(ledger.claim("a".into(), at(0), at(10)));
    assert_eq!(
        ledger.claim("a".into(), at(1), at(10)),
        Ok(Claim::InProgress)
    );
    assert_eq!(ledger.claim("b".into(), at(1), at(10)), Err(Refusal::Full));
    let done = ledger.complete_with(id, "id-1".into(), at(2));
    assert_eq!(done, Ok(Completion::Completed));
    let before = ledger.clone();
    let repeat = ledger.complete_with(id, "id-1".into(), at(2));
    assert_eq!(repeat, Ok(Completion::AlreadyCompleted));
    assert_eq!(
        ledger.complete_with(id, "id-2".into(), at(2)),
        Err(Refused {
            refusal: Refusal::Mismatch,
            value: "id-2".to_string()
        })
    );
    assert_eq!(ledger, before);
    assert_eq!(
        ledger.claim("a".into(), at(9), at(1)),
        Ok(Claim::Completed(&"id-1".into()))
    );
    let replacement = acquired(ledger.claim("a".into(), at(10), at(5)));
    assert_eq!(ledger.complete(id, at(10)), Err(Refusal::Stale));
    let done = ledger.complete_with(replacement, String::new(), at(10));
    assert_eq!(done, Ok(Completion::Completed));
}

#[test]
fn freed_slots_are_reused_without_resurrecting_handles() {
    let mut ledger = HashDedup::<u64>::with_scope(2, 6);
    let first = acquired(ledger.claim(1, at(0), at(10)));
    assert_eq!((first.scope(), ledger.scope()), (6, 6));
    let second = acquired(ledger.claim(2, at(0), at(10)));
    assert_eq!(ledger.abandon(first), Ok(()));
    let third = acquired(ledger.claim(3, at(0), at(10)));
    assert_eq!(ledger.claim(4, at(0), at(10)), Err(Refusal::Full));
    assert_eq!(ledger.abandon(first), Err(Refusal::Stale));
    assert_eq!(ledger.complete(third, at(1)), Ok(Completion::Completed));
    assert_eq!(ledger.complete(second, at(1)), Ok(Completion::Completed));
    let before = ledger.clone();
    assert_eq!(ledger.claim(1, at(1), at(10)), Err(Refusal::Full));
    assert_eq!(ledger, before);
    assert_eq!(ledger.claim(3, at(1), at(10)), Ok(Claim::Completed(&())));
    acquired(ledger.claim(1, at(10), at(10)));
    assert_eq!(ledger.store().slots.len(), 2);
    assert_eq!(ledger.store().iter().count(), 1);
}

#[test]
fn fingerprints_conflict_on_owned_keys() {
    let mut ledger = HashDedup::<String, u8>::with_capacity(4);
    let id = acquired(ledger.claim_with("turn".into(), 1, at(0), at(10)));
    assert_eq!(
        ledger.claim_with("turn".into(), 2, at(0), at(10)),
        Err(Refusal::Conflict)
    );
    assert_eq!(
        ledger.complete_with(id, 200, at(1)),
        Ok(Completion::Completed)
    );
    assert_eq!(
        ledger.claim_with("turn".into(), 2, at(1), at(10)),
        Err(Refusal::Conflict)
    );
    assert_eq!(
        ledger.claim_with("turn".into(), 1, at(1), at(10)),
        Ok(Claim::Completed(&200))
    );
}

#[test]
fn partition_caps_and_owner_expiry_apply_to_owned_keys() {
    let mut ledger = HashDedup::<(String, String)>::with_capacity(8).with_partition_cap(1);
    let session = |name: &str| Attempt::new((name.into(), "src".into())).partition(1);
    let doomed = acquired(ledger.admit(session("s1"), at(0), at(10)));
    let before = ledger.clone();
    assert_eq!(
        ledger.admit(session("s2"), at(0), at(10)),
        Err(Refusal::PartitionFull)
    );
    assert_eq!(ledger, before);
    let other = Attempt::new(("s2".into(), "src".into())).partition(2);
    acquired(ledger.admit(other, at(0), at(10)));
    assert_eq!(ledger.expire_where(|key| key.0 == "s1"), 1);
    assert_eq!(ledger.abandon(doomed), Err(Refusal::Stale));
    acquired(ledger.admit(session("s3"), at(1), at(10)));
    assert_eq!(
        ledger.claim(("s2".into(), "src".into()), at(1), at(10)),
        Ok(Claim::InProgress)
    );
}

#[test]
fn zero_capacity_and_invalid_time_refuse_without_a_trace() {
    let mut ledger = HashDedup::<u64>::with_capacity(0);
    let before = ledger.clone();
    assert_eq!(ledger.claim(0, at(0), at(1)), Err(Refusal::Full));
    assert_eq!(
        ledger.claim(0, at(1), Duration::ZERO),
        Err(Refusal::InvalidDuration)
    );
    assert_eq!(
        ledger.claim(0, Duration::MAX, at(1)),
        Err(Refusal::Overflow)
    );
    assert_eq!(ledger, before);
    let mut ledger = HashDedup::<u64>::with_capacity(1);
    acquired(ledger.claim(0, at(1), at(1)));
    assert_eq!(ledger.claim(0, at(0), at(1)), Err(Refusal::TimeRegression));
}

#[test]
fn a_custom_hasher_and_a_restart_keep_both_indexes() {
    let hasher = hashbrown::DefaultHashBuilder::default();
    let store = Hashed::<u64, u8, _>::with_capacity_and_hasher(3, hasher);
    let Some(mut ledger) = Ledger::over_with_scope(store, 4) else {
        panic!("an empty store is always sound");
    };
    let kept = acquired(ledger.claim(1, at(0), at(9)));
    let gone = acquired(ledger.claim(2, at(0), at(9)));
    assert_eq!(ledger.abandon(gone), Ok(()));
    let saved = ledger.clone().into_store();
    assert!(Ledger::over(saved.clone()).is_none());
    assert!(Ledger::resumed(saved.clone(), 5, 2, at(0)).is_none());
    let Some(mut restored) = Ledger::resumed(saved, 4, 2, at(0)) else {
        panic!("the saved state is sound");
    };
    assert_eq!(restored, ledger);
    assert_eq!(restored.abandon(gone), Err(Refusal::Stale));
    assert_eq!(
        restored.complete_with(kept, 3, at(1)),
        Ok(Completion::Completed)
    );
    assert_eq!(
        acquired(restored.claim(2, at(1), at(9))).raw(),
        kept.raw() + 2
    );
}

#[test]
fn the_model_holds_over_hashed_storage() {
    crate::tests::model::run(3, |scope| HashDedup::<u64, u8>::with_scope(3, scope));
}

#[test]
fn hashed_storage_holds_one_record_per_key_and_per_claim() {
    let entry = |key: u64, raw: u64| {
        Entry::<u64, ()>::new(key, None, None, ClaimId::from_raw(raw), at(100), None)
    };
    let mut store = Hashed::<u64, ()>::with_capacity(3);
    assert_eq!(store.insert(entry(1, 1)), Some(0));
    let before = store.clone();
    for second in [entry(1, 1), entry(1, 2), entry(2, 1)] {
        assert_eq!(store.insert(second), None);
        assert_eq!(store, before);
    }
    assert_eq!(store.insert(entry(2, 2)), Some(1));
}

/// `resumed` checks a heap store as it checks a fixed one: two slots under
/// one key or one claim id, two identical records included, are refused.
#[test]
fn resuming_refuses_two_records_under_one_key_or_claim() {
    let entry = |key: u64, raw: u64| {
        Entry::<u64, ()>::new(key, None, None, ClaimId::from_raw(raw), at(100), None)
    };
    let resumed = |entries: [Entry<u64, ()>; 2]| {
        let mut store = Hashed::<u64, ()>::with_capacity(2);
        for entry in entries {
            plant(&mut store, entry);
        }
        assert_eq!(store.iter().count(), 2);
        Ledger::resumed(store, 0, 2, at(0)).is_some()
    };
    assert!(resumed([entry(1, 1), entry(2, 2)]));
    assert!(!resumed([entry(1, 1), entry(1, 1)]));
    assert!(!resumed([entry(1, 1), entry(1, 2)]));
    assert!(!resumed([entry(1, 1), entry(2, 1)]));
}
