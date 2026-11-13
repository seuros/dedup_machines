//! A backend defined outside the crate: two slots, no hashing.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pair {
    entries: [Option<Entry<u64, u8>>; 2],
}

impl Storage for Pair {
    type Key = u64;
    type Value = u8;

    fn get(&self, slot: usize) -> Option<&Entry<u64, u8>> {
        self.entries.get(slot)?.as_ref()
    }

    fn get_mut(&mut self, slot: usize) -> Option<&mut Entry<u64, u8>> {
        self.entries.get_mut(slot)?.as_mut()
    }

    fn find(&self, key: &u64) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.as_ref().is_some_and(|entry| entry.key() == key))
    }

    fn locate(&self, claim: ClaimId) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.as_ref().is_some_and(|entry| entry.claim() == claim))
    }

    fn insert(&mut self, entry: Entry<u64, u8>) -> Option<usize> {
        let slot = self.entries.iter().position(Option::is_none)?;
        self.entries[slot] = Some(entry);
        Some(slot)
    }

    fn remove(&mut self, slot: usize) -> bool {
        self.entries.get_mut(slot).and_then(Option::take).is_some()
    }

    fn retain(&mut self, mut keep: impl FnMut(&Entry<u64, u8>) -> bool) -> usize {
        let mut removed = 0;
        for slot in &mut self.entries {
            if slot.as_ref().is_some_and(|entry| !keep(entry)) {
                *slot = None;
                removed += 1;
            }
        }
        removed
    }

    fn iter(&self) -> impl Iterator<Item = (usize, &Entry<u64, u8>)> {
        let held = self.entries.iter().enumerate();
        held.filter_map(|(slot, entry)| Some((slot, entry.as_ref()?)))
    }
}

#[test]
fn a_foreign_backend_persists_and_restores_a_ledger() {
    let empty = Pair {
        entries: [None, None],
    };
    let Some(mut ledger) = Ledger::over_with_scope(empty, 2) else {
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
                ClaimId::from_raw(entry.claim().raw()),
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
        let empty = Pair {
            entries: [None, None],
        };
        Ledger::over_with_scope(empty, scope).unwrap_or_else(|| panic!("empty"))
    });
}

/// A backend whose `insert` checks nothing can be handed the same record
/// twice. `resumed` is what refuses it: a lookup answers one slot.
#[test]
fn a_foreign_backend_holding_one_record_twice_is_refused() {
    let entry = || Entry::<u64, u8>::new(1, None, None, ClaimId::from_raw(1), at(100), None);
    let mut twice = Pair {
        entries: [None, None],
    };
    assert_eq!(twice.insert(entry()), Some(0));
    assert_eq!(twice.insert(entry()), Some(1));
    assert!(Ledger::resumed(twice, 0, 1, at(0)).is_none());
}
