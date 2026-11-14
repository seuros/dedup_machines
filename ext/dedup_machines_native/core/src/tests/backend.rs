//! A backend defined outside the crate: two slots, no hashing.

use super::*;
use crate::storage::slots;
use crate::test_support::{an_unwinding_predicate_drops_nothing, record};

/// Unlike `Fixed`, its `insert` checks nothing: it takes a second record
/// under a key or a claim id it already holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pair {
    entries: [Option<Entry<u64, u8>>; 2],
}

impl Pair {
    const EMPTY: Self = Self {
        entries: [None, None],
    };
}

impl Storage for Pair {
    type Key = u64;
    type Value = u8;

    fn get(&self, slot: usize) -> Option<&Entry<u64, u8>> {
        slots::get(&self.entries, slot)
    }

    fn complete(&mut self, slot: usize, value: u8) -> Result<(), u8> {
        slots::complete(&mut self.entries, slot, value)
    }

    fn find(&self, key: &u64) -> Option<usize> {
        slots::find(&self.entries, |entry| entry.key() == key)
    }

    fn locate(&self, claim: ClaimId) -> Option<usize> {
        slots::find(&self.entries, |entry| entry.claim() == claim)
    }

    fn insert(&mut self, entry: Entry<u64, u8>) -> Option<usize> {
        slots::insert(&mut self.entries, entry)
    }

    fn remove(&mut self, slot: usize) -> bool {
        slots::remove(&mut self.entries, slot)
    }

    fn retain(&mut self, keep: impl FnMut(&Entry<u64, u8>) -> bool) -> usize {
        slots::retain(&mut self.entries, keep)
    }

    fn iter(&self) -> impl Iterator<Item = (usize, &Entry<u64, u8>)> {
        slots::iter(&self.entries)
    }
}

#[test]
fn a_foreign_backend_persists_and_restores_a_ledger() {
    let Some(mut ledger) = Ledger::over_with_scope(Pair::EMPTY, 2) else {
        panic!("an empty store is always sound");
    };
    let done = acquired(ledger.claim(1, at(0), at(10)));
    assert_eq!(
        ledger.complete_with(done, 7, at(1)),
        Ok(Completion::Completed)
    );
    let pending = acquired(ledger.claim(2, at(2), at(10)));
    let (scope, issued, observed) = (ledger.scope(), ledger.issued(), ledger.observed());
    assert_eq!((scope, issued, observed), (2, 2, at(2)));

    // Round-trip every record through the accessors, as a database adapter would.
    let saved = ledger.into_store();
    let copied: [Option<Entry<u64, u8>>; 2] = [0, 1].map(|slot| {
        saved.get(slot).map(|entry| {
            assert_eq!(entry.is_pending(), entry.value().is_none());
            Entry::new(
                *entry.key(),
                entry.fingerprint(),
                entry.partition(),
                id(entry.claim().raw()),
                entry.expires(),
                entry.value().copied(),
            )
        })
    });
    assert_eq!(copied, saved.entries);

    let copy = Pair { entries: copied };
    assert!(Ledger::resumed(copy.clone(), 0, issued, observed).is_none());
    let Some(mut restored) = Ledger::resumed(copy, scope, issued, observed) else {
        panic!("the saved state is sound");
    };
    assert_eq!(restored.claim(1, at(3), at(10)), Ok(Claim::Completed(&7)));
    assert_eq!(restored.complete(pending, at(3)), Ok(Completion::Completed));
    assert_eq!(restored.abandon(pending), Err(Refusal::Concluded));
    assert_eq!(restored.claim(3, at(3), at(10)), Err(Refusal::Full));
    assert_eq!(restored.abandon(done), Err(Refusal::Concluded));
}

#[test]
fn the_model_holds_over_a_foreign_backend() {
    // `Pair` has two slots; the model run sizes itself from the capacity.
    model::run(2, |scope| {
        Ledger::over_with_scope(Pair::EMPTY, scope).unwrap_or_else(|| panic!("empty"))
    });
    let ledger = Ledger::over(Pair::EMPTY).unwrap_or_else(|| panic!("empty"));
    an_unwinding_predicate_drops_nothing(ledger);
}

/// A backend whose `insert` checks nothing can be handed two records under
/// one key or one claim id, the same record twice included. `resumed` is
/// what refuses it: a lookup answers one slot.
#[test]
fn a_foreign_backend_holding_a_key_or_a_claim_twice_is_refused() {
    let resumed = |entries: [Entry<u64, u8>; 2]| {
        let mut store = Pair::EMPTY;
        for entry in entries {
            assert!(store.insert(entry).is_some());
        }
        Ledger::resumed(store, 0, 2, at(0)).is_some()
    };
    assert!(resumed([record(1, 1), record(2, 2)]));
    assert!(!resumed([record(1, 1), record(1, 1)]));
    assert!(!resumed([record(1, 1), record(1, 2)]));
    assert!(!resumed([record(1, 1), record(2, 1)]));
}

/// One record alone is the saved state of its scope and count, and its claim
/// is abandoned once.
#[test]
fn one_saved_record_is_abandoned_once() {
    let mut store = Fixed::<u64, (), 2>::new();
    assert_eq!(store.insert(record(1, 1)), Some(0));
    let Some(mut ledger) = Ledger::resumed(store, 0, 1, at(0)) else {
        panic!("one record is sound");
    };
    assert_eq!(ledger.abandon(id(1)), Ok(()));
    assert_eq!(ledger.abandon(id(1)), Err(Refusal::Stale));
    assert_eq!(acquired(ledger.claim(1, at(0), at(5))).raw(), 2);
}
