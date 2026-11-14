use alloc::string::{String, ToString};

use super::*;
use crate::test_support::{
    acquired, an_unwinding_predicate_drops_nothing, at, holds_the_storage_contract,
};
use crate::{Claim, Completion, Refusal, Refused};

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
    let mut empty = HashDedup::<u64>::with_capacity(0);
    assert_eq!(empty.claim(0, at(0), at(1)), Err(Refusal::Full));
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

/// Completing is the one change a stored record sees, so the indexes by key
/// and by claim id never go stale.
#[test]
fn hashed_storage_keeps_the_storage_contract() {
    holds_the_storage_contract(Hashed::<u64, u8>::with_capacity(3));
    an_unwinding_predicate_drops_nothing(HashDedup::<u64>::with_capacity(3));
}
