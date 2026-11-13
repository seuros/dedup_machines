//! Claim ids carry the scope of the ledger that issued them.

use super::*;
use periscope_machines::scoped;

/// Key 1 pending and key 2 completed, both claimed at 0 for 20.
fn settled(scope: u16) -> (Dedup<u64, 3>, ClaimId, ClaimId) {
    let mut ledger = Dedup::<u64, 3>::with_scope(scope);
    let pending = acquired(ledger.claim(1, at(0), at(20)));
    let done = acquired(ledger.claim(2, at(0), at(20)));
    assert_eq!(ledger.complete(done, at(0)), Ok(Completion::Completed));
    (ledger, pending, done)
}

#[test]
fn ids_carry_the_scope_above_a_sequence_from_one() {
    let (ledger, pending, done) = settled(7);
    assert_eq!((ledger.scope(), ledger.issued()), (7, 2));
    assert_eq!((pending.scope(), done.scope()), (7, 7));
    assert_eq!((pending.raw(), done.raw()), (scoped(7, 1), scoped(7, 2)));
    assert_eq!(ClaimId::from_raw(pending.raw()), pending);
    assert_eq!(Dedup::<u64, 3>::new().scope(), 0);
    assert_eq!(Dedup::<u64, 3>::default(), Dedup::<u64, 3>::with_scope(0));
}

#[test]
fn a_claim_from_another_ledger_settles_nothing() {
    // The same calls on two ledgers: the same sequence numbers, the same keys.
    let (mut near, near_pending, near_done) = settled(1);
    let (mut far, far_pending, far_done) = settled(2);
    assert_ne!(near_pending, far_pending);
    let foreign = [
        (&mut near, far_pending, far_done),
        (&mut far, near_pending, near_done),
    ];
    for (ledger, pending, done) in foreign {
        let before = ledger.clone();
        assert_eq!(ledger.complete(pending, at(1)), Err(Refusal::Stale));
        assert_eq!(ledger.abandon(pending), Err(Refusal::Stale));
        // Nor is a repeat honoured: the completed claim here is not that one.
        assert_eq!(ledger.complete(done, at(1)), Err(Refusal::Stale));
        assert_eq!(ledger.abandon(done), Err(Refusal::Stale));
        // Refused before the clock is read.
        assert_eq!(
            ledger.complete(pending, Duration::ZERO),
            Err(Refusal::Stale)
        );
        assert_eq!(*ledger, before);
    }
    assert_eq!(
        near.complete(near_pending, at(1)),
        Ok(Completion::Completed)
    );
    assert_eq!(far.abandon(far_pending), Ok(()));
}

#[test]
fn a_claim_id_crosses_a_queue_as_an_integer() {
    let (mut ledger, pending, done) = settled(5);
    let wire: [u64; 2] = [pending.raw(), done.raw()];
    let [pending, done] = wire.map(ClaimId::from_raw);
    assert_eq!(
        ledger.complete(done, at(1)),
        Ok(Completion::AlreadyCompleted)
    );
    assert_eq!(ledger.abandon(pending), Ok(()));
    // A forged integer names nothing, in or out of the issued range.
    for raw in [0, scoped(5, 3), scoped(5, 1), scoped(4, 2), u64::MAX] {
        assert_eq!(ledger.abandon(ClaimId::from_raw(raw)), Err(Refusal::Stale));
    }
}

#[test]
fn ids_are_never_reissued_across_slot_reuse() {
    let mut ledger = Dedup::<u64, 1>::with_scope(9);
    let mut seen = std::vec::Vec::new();
    for round in 0..5 {
        let id = acquired(ledger.claim(1, at(round), at(1)));
        assert!(!seen.contains(&id));
        for old in &seen {
            assert_eq!(ledger.abandon(*old), Err(Refusal::Stale));
        }
        seen.push(id);
    }
    assert_eq!(ledger.issued(), 5);
}

#[test]
fn a_restart_resumes_the_sequence_in_its_scope() {
    let (mut ledger, pending, done) = settled(3);
    let gone = acquired(ledger.claim(3, at(1), at(20)));
    assert_eq!(ledger.abandon(gone), Ok(()));
    let (scope, issued, observed) = (ledger.scope(), ledger.issued(), ledger.observed());
    let saved = ledger.clone().into_store();

    let Some(mut restored) = Ledger::resumed(saved.clone(), scope, issued, observed) else {
        panic!("the saved state is sound");
    };
    assert_eq!(restored, ledger);
    assert_eq!(
        restored.claim(0, at(0), at(5)),
        Err(Refusal::TimeRegression)
    );
    // Handles from before the restart still name their claims, and only those.
    assert_eq!(
        restored.complete(done, at(1)),
        Ok(Completion::AlreadyCompleted)
    );
    assert_eq!(restored.abandon(gone), Err(Refusal::Stale));
    let fresh = acquired(restored.claim(3, at(1), at(20)));
    assert_eq!(fresh.raw(), scoped(3, 4));
    assert_eq!(restored.abandon(pending), Ok(()));

    // Saved state is refused under another scope, or a count that does not
    // cover the records it holds.
    assert!(Ledger::resumed(saved.clone(), 4, issued, observed).is_none());
    assert!(Ledger::resumed(saved.clone(), scope, 1, observed).is_none());
    assert!(Ledger::resumed(saved.clone(), scope, 0, observed).is_none());
    // `over` starts a sequence from nothing, so it takes only an empty store.
    assert!(Ledger::over(saved.clone()).is_none());
    assert!(Ledger::over_with_scope(saved, scope).is_none());
    let Some(empty) = Ledger::over_with_scope(Fixed::<u64, (), 3>::new(), 6) else {
        panic!("an empty store is always sound");
    };
    assert_eq!(empty, Dedup::<u64, 3>::with_scope(6));
    assert!(Ledger::over(Fixed::<u64, u8, 1>::default()).is_some());
}

#[test]
fn inconsistent_saved_records_are_refused() {
    let entry = |key: u64, raw: u64| {
        Entry::<u64, ()>::new(key, None, None, ClaimId::from_raw(raw), at(9), None)
    };
    let store = |entries: [Entry<u64, ()>; 2]| {
        let mut store = Fixed::<u64, (), 2>::new();
        for entry in entries {
            plant(&mut store, entry);
        }
        store
    };
    let resumed = |entries| Ledger::resumed(store(entries), 0, 5, at(0)).is_some();
    assert!(resumed([entry(1, 1), entry(2, 5)]));
    // A sequence of zero, one past the count, another scope.
    assert!(!resumed([entry(1, 1), entry(2, 0)]));
    assert!(!resumed([entry(1, 1), entry(2, 6)]));
    assert!(!resumed([entry(1, 1), entry(2, scoped(1, 2))]));
    // Two records under one key, two under one claim id, the same one twice.
    assert!(!resumed([entry(1, 1), entry(1, 2)]));
    assert!(!resumed([entry(1, 1), entry(2, 1)]));
    assert!(!resumed([entry(1, 1), entry(1, 1)]));
}

/// The defect: `resumed` asked only that each record's key led to its claim
/// id and its claim id to its key, which two identical records satisfy. The
/// ledger then abandoned claim 1 twice, the key staying held in between.
#[test]
fn two_identical_records_are_refused() {
    let entry = || Entry::<u64, ()>::new(1, None, None, ClaimId::from_raw(1), at(100), None);
    let mut store = Fixed::<u64, (), 2>::new();
    plant(&mut store, entry());
    plant(&mut store, entry());
    assert_eq!(store.iter().count(), 2);
    assert!(Ledger::resumed(store, 0, 1, at(0)).is_none());
    // One of them alone is the saved state of that scope and count.
    let mut store = Fixed::<u64, (), 2>::new();
    plant(&mut store, entry());
    let Some(mut ledger) = Ledger::resumed(store, 0, 1, at(0)) else {
        panic!("one record is sound");
    };
    assert_eq!(ledger.abandon(ClaimId::from_raw(1)), Ok(()));
    assert_eq!(ledger.abandon(ClaimId::from_raw(1)), Err(Refusal::Stale));
    assert_eq!(acquired(ledger.claim(1, at(0), at(5))).raw(), 2);
}

#[test]
fn fixed_storage_holds_one_record_per_key_and_per_claim() {
    let entry = |key: u64, raw: u64| {
        Entry::<u64, ()>::new(key, None, None, ClaimId::from_raw(raw), at(9), None)
    };
    let mut store = Fixed::<u64, (), 3>::new();
    assert_eq!(store.insert(entry(1, 1)), Some(0));
    let before = store.clone();
    // The same record, the same key, the same claim id: nothing is stored.
    for second in [entry(1, 1), entry(1, 2), entry(2, 1)] {
        assert_eq!(store.insert(second), None);
        assert_eq!(store, before);
    }
    assert_eq!(store.insert(entry(2, 2)), Some(1));
}
