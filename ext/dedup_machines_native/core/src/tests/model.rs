//! Randomized runs against an independent naive model.
//!
//! The model keeps a plain vector and removes expired rows eagerly, on every
//! honoured call; the ledger reads them lazily and removes them only in an
//! acquiring claim. The two must give the same answers and hold the same live
//! records, which is what makes the laziness invisible.

use core::fmt::Debug;
use std::vec::Vec;

use periscope_machines::{scope_of, scoped, sequence_of};

use super::*;
use crate::test_support::pick;
use matryona_machines::testing::SplitMix64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    key: u64,
    fingerprint: Option<u64>,
    partition: Option<u64>,
    claim: u64,
    expires: u64,
    value: Option<u8>,
}

/// `Claim` without the borrow.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Acquired(u64),
    InProgress,
    Completed(u8),
}

#[derive(Clone)]
struct Model {
    rows: Vec<Row>,
    cap: usize,
    quota: usize,
    scope: u16,
    issued: u64,
    observed: u64,
}

impl Model {
    /// An honoured call at `now`: the clock moves and the expired rows go.
    fn honour(&mut self, now: u64) {
        self.observed = now;
        self.rows.retain(|row| row.expires > now);
    }

    fn owned(&self, claim: u64) -> bool {
        scope_of(claim) == self.scope && (1..=self.issued).contains(&sequence_of(claim))
    }

    fn admit(&mut self, attempt: &Row, now: u64, ttl: u64) -> Result<Answer, Refusal> {
        if now < self.observed {
            return Err(Refusal::TimeRegression);
        }
        if ttl == 0 {
            return Err(Refusal::InvalidDuration);
        }
        let live = |row: &&Row| row.expires > now;
        if let Some(row) = self.rows.iter().filter(live).find(|r| r.key == attempt.key) {
            if row.fingerprint != attempt.fingerprint {
                return Err(Refusal::Conflict);
            }
            let answer = row.value.map_or(Answer::InProgress, Answer::Completed);
            self.honour(now);
            return Ok(answer);
        }
        let peers = |row: &&Row| attempt.partition.is_some() && row.partition == attempt.partition;
        if self.rows.iter().filter(live).filter(peers).count() >= self.quota {
            return Err(Refusal::PartitionFull);
        }
        if self.rows.iter().filter(live).count() >= self.cap {
            return Err(Refusal::Full);
        }
        self.honour(now);
        self.issued += 1;
        let claim = scoped(self.scope, self.issued);
        self.rows.push(Row {
            claim,
            expires: now + ttl,
            value: None,
            ..attempt.clone()
        });
        Ok(Answer::Acquired(claim))
    }

    fn complete(&mut self, claim: u64, value: u8, now: u64) -> Result<Completion, Refusal> {
        if !self.owned(claim) {
            return Err(Refusal::Stale);
        }
        if now < self.observed {
            return Err(Refusal::TimeRegression);
        }
        let live = |row: &&mut Row| row.claim == claim && row.expires > now;
        let row = self.rows.iter_mut().find(live).ok_or(Refusal::Stale)?;
        match row.value {
            None => {
                row.value = Some(value);
                self.honour(now);
                Ok(Completion::Completed)
            }
            Some(recorded) if recorded == value => Ok(Completion::AlreadyCompleted),
            Some(_) => Err(Refusal::Mismatch),
        }
    }

    fn abandon(&mut self, claim: u64) -> Result<(), Refusal> {
        let observed = self.observed;
        let live = |row: &Row| row.claim == claim && row.expires > observed;
        let at = self.rows.iter().position(live).ok_or(Refusal::Stale)?;
        if self.rows[at].value.is_some() {
            return Err(Refusal::Concluded);
        }
        self.rows.remove(at);
        Ok(())
    }

    fn expire_where(&mut self, dead: impl Fn(u64) -> bool) -> usize {
        self.honour(self.observed);
        let before = self.rows.len();
        self.rows.retain(|row| !dead(row.key));
        before - self.rows.len()
    }
}

/// What the runs must have exercised, counted across every seed.
#[derive(Debug, Default)]
struct Seen {
    acquired: usize,
    in_progress: usize,
    completed_answer: usize,
    reused_expired_slot: usize,
    completed: usize,
    already_completed: usize,
    abandoned: usize,
    dropped: usize,
    restarts: usize,
    foreign: usize,
    refusals: [usize; 10],
}

impl Seen {
    fn refused(&mut self, refusal: Refusal) {
        self.refusals[match refusal {
            Refusal::TimeRegression => 0,
            Refusal::InvalidDuration => 1,
            Refusal::Overflow => 2,
            Refusal::Exhausted => 3,
            Refusal::Conflict => 4,
            Refusal::Full => 5,
            Refusal::PartitionFull => 6,
            Refusal::Stale => 7,
            Refusal::Concluded => 8,
            Refusal::Mismatch => 9,
        }] += 1;
    }
}

/// The ledger's live records, as model rows.
fn live<S: Storage<Key = u64, Value = u8>>(ledger: &Ledger<S>) -> Vec<Row> {
    let mut rows: Vec<Row> = ledger
        .store()
        .iter()
        .filter(|(_, entry)| entry.expires() > ledger.observed())
        .map(|(_, entry)| Row {
            key: *entry.key(),
            fingerprint: entry.fingerprint(),
            partition: entry.partition(),
            claim: entry.claim().raw(),
            expires: entry.expires().as_secs(),
            value: entry.value().copied(),
        })
        .collect();
    rows.sort();
    rows
}

struct Run<S> {
    ledger: Ledger<S>,
    model: Model,
    rng: SplitMix64,
    /// Every id handed out so far, live or not.
    handed: Vec<u64>,
}

impl<S: Storage<Key = u64, Value = u8> + Clone + PartialEq + Debug> Run<S> {
    fn now(&mut self) -> u64 {
        match self.rng.below(8) {
            0 => self.model.observed.saturating_sub(1 + self.rng.below(3)),
            1..=4 => self.model.observed,
            _ => self.model.observed + 1 + self.rng.below(4),
        }
    }

    /// A handed-out id, a forged one, or one from another scope.
    fn named(&mut self, seen: &mut Seen) -> u64 {
        // A sequence of zero builds no `ClaimId` at all.
        let guess = scoped(self.model.scope, 1 + self.rng.below(self.model.issued + 2));
        let named = match (self.rng.below(8), self.handed.len()) {
            (0, _) | (_, 0) => guess,
            (_, held) => self.handed[held - 1 - pick(&mut self.rng, held.min(4))],
        };
        if self.rng.below(8) == 0 {
            seen.foreign += 1;
            // Flipping a low scope bit always lands in another scope.
            return named ^ scoped(1 + u16::try_from(self.rng.below(3)).unwrap_or(0), 0);
        }
        named
    }

    fn admit(&mut self, seen: &mut Seen) {
        let attempt = Row {
            key: self.rng.below(6),
            fingerprint: [None, None, Some(1), Some(2)][pick(&mut self.rng, 4)],
            partition: [None, Some(1), Some(2)][pick(&mut self.rng, 3)],
            claim: 0,
            expires: 0,
            value: None,
        };
        let (now, ttl) = (self.now(), self.rng.below(6));
        let mut asked = Attempt::new(attempt.key);
        asked.fingerprint = attempt.fingerprint;
        asked.partition = attempt.partition;
        let before = self.ledger.clone();
        let stale = live(&before).len() < before.store().iter().count();
        let got = match self.ledger.admit(asked, at(now), at(ttl)) {
            Ok(Claim::Acquired(id)) => Ok(Answer::Acquired(id.raw())),
            Ok(Claim::InProgress) => Ok(Answer::InProgress),
            Ok(Claim::Completed(value)) => Ok(Answer::Completed(*value)),
            Err(refusal) => Err(refusal),
        };
        assert_eq!(got, self.model.admit(&attempt, now, ttl));
        match got {
            Ok(Answer::Acquired(id)) => {
                assert!(!self.handed.contains(&id), "{id} was issued twice");
                self.handed.push(id);
                seen.acquired += 1;
                seen.reused_expired_slot += usize::from(stale);
                // Every expired record is gone after an acquiring claim.
                let held = self.ledger.store().iter().count();
                assert_eq!(held, live(&self.ledger).len());
            }
            Ok(Answer::InProgress | Answer::Completed(_)) => {
                seen.in_progress += usize::from(got == Ok(Answer::InProgress));
                seen.completed_answer += usize::from(got != Ok(Answer::InProgress));
                // Only the observed clock moved.
                assert_eq!(self.ledger.observed(), at(now));
                assert_eq!(self.ledger.store(), before.store());
                assert_eq!(self.ledger.issued(), before.issued());
            }
            Err(refusal) => {
                seen.refused(refusal);
                assert_eq!(self.ledger, before, "{refusal:?} changed the ledger");
            }
        }
    }

    fn complete(&mut self, seen: &mut Seen) {
        let (claim, value, now) = (self.named(seen), self.rng.below(2), self.now());
        let value = u8::try_from(value).unwrap_or(0);
        let before = self.ledger.clone();
        let got = self.ledger.complete_with(id(claim), value, at(now));
        let want = self.model.complete(claim, value, now);
        assert_eq!(got, want.map_err(|refusal| Refused { refusal, value }));
        match got {
            Ok(Completion::Completed) => seen.completed += 1,
            Ok(Completion::AlreadyCompleted) => {
                seen.already_completed += 1;
                assert_eq!(self.ledger, before, "a repeat changed the ledger");
            }
            Err(refused) => {
                seen.refused(refused.refusal);
                assert_eq!(self.ledger, before, "{refused:?} changed the ledger");
            }
        }
    }

    fn abandon(&mut self, seen: &mut Seen) {
        let claim = self.named(seen);
        let before = self.ledger.clone();
        let got = self.ledger.abandon(id(claim));
        assert_eq!(got, self.model.abandon(claim));
        match got {
            Ok(()) => seen.abandoned += 1,
            Err(refusal) => {
                seen.refused(refusal);
                assert_eq!(self.ledger, before, "{refusal:?} changed the ledger");
            }
        }
    }

    fn drain(&mut self, seen: &mut Seen) {
        let odd = self.rng.below(2);
        let dropped = self.ledger.expire_where(|key| key % 2 == odd);
        assert_eq!(dropped, self.model.expire_where(|key| key % 2 == odd));
        seen.dropped += dropped;
    }

    /// Save, rebuild from the saved state, and carry on with the rebuilt one.
    fn restart(&mut self, seen: &mut Seen) {
        let ledger = &self.ledger;
        let (scope, issued, observed) = (ledger.scope(), ledger.issued(), ledger.observed());
        let store = ledger.clone().into_store();
        // Another scope takes the saved state only when no record ties it down.
        let elsewhere = Ledger::resumed(store.clone(), scope ^ 1, issued, observed);
        assert_eq!(elsewhere.is_none(), store.iter().next().is_some());
        let Some(restored) = Ledger::resumed(store, scope, issued, observed) else {
            panic!("a ledger's own state is sound");
        };
        let restored = restored.with_partition_cap(self.model.quota);
        assert_eq!(restored, self.ledger);
        self.ledger = restored;
        seen.restarts += 1;
    }
}

/// Run every seed over ledgers `make` builds for a scope, holding `cap` records.
pub fn run<S>(cap: usize, make: impl Fn(u16) -> Ledger<S>)
where
    S: Storage<Key = u64, Value = u8> + Clone + PartialEq + Debug,
{
    let mut seen = Seen::default();
    for seed in 0..300_u64 {
        let scope = u16::try_from(seed % 5).unwrap_or(0) * 4;
        let quota = 1 + usize::try_from(seed % 2).unwrap_or(0);
        let mut run = Run {
            ledger: make(scope).with_partition_cap(quota),
            model: Model {
                rows: Vec::new(),
                cap,
                quota,
                scope,
                issued: 0,
                observed: 0,
            },
            rng: SplitMix64::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
            handed: Vec::new(),
        };
        for _ in 0..400 {
            match run.rng.below(16) {
                0..=6 => run.admit(&mut seen),
                7..=10 => run.complete(&mut seen),
                11..=13 => run.abandon(&mut seen),
                14 => run.drain(&mut seen),
                _ => run.restart(&mut seen),
            }
            assert_eq!(run.ledger.issued(), run.model.issued);
            assert_eq!(run.ledger.observed(), at(run.model.observed));
            let mut want = run.model.rows.clone();
            want.sort();
            assert_eq!(live(&run.ledger), want);
        }
    }
    let exercised = [
        seen.acquired,
        seen.in_progress,
        seen.completed_answer,
        seen.reused_expired_slot,
        seen.completed,
        seen.already_completed,
        seen.abandoned,
        seen.dropped,
        seen.restarts,
        seen.foreign,
    ];
    assert!(exercised.iter().all(|&count| count > 50), "{seen:?}");
    // Overflow and exhaustion need instants and counts no run reaches; they
    // are pinned in `refusals`.
    for (reason, &count) in seen.refusals.iter().enumerate() {
        assert_eq!(count > 50, ![2, 3].contains(&reason), "{reason}: {seen:?}");
    }
}

#[test]
fn the_ledger_matches_the_model_on_random_operations() {
    run(3, Dedup::<u64, 3, u8>::with_scope);
}
