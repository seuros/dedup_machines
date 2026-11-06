use super::*;
fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}
fn acquired(claim: Claim) -> ClaimId {
    let Claim::Acquired(id) = claim else {
        panic!("expected acquired claim");
    };
    id
}

#[test]
fn lifecycle_is_exact_and_full_tables_do_not_evict() {
    let mut ledger = Dedup::<u64, 1>::new();
    let id = acquired(ledger.claim(7, at(0), at(10)));
    assert_eq!(ledger.claim(7, at(1), at(10)), Claim::InProgress);
    assert_eq!(ledger.claim(8, at(1), at(10)), Claim::Full);
    assert!(ledger.complete(id, at(2)));
    assert!(!ledger.complete(id, at(2)));
    assert_eq!(ledger.claim(7, at(9), at(1)), Claim::Completed);
    let replacement = acquired(ledger.claim(7, at(10), at(5)));
    assert!(!ledger.complete(id, at(10)));
    assert!(ledger.complete(replacement, at(10)));
}

#[test]
fn expired_pending_workers_cannot_complete_replacements() {
    let mut ledger = Dedup::<u64, 1>::new();
    let stale = acquired(ledger.claim(7, at(0), at(1)));
    let fresh = acquired(ledger.claim(7, at(1), at(1)));
    assert!(!ledger.complete(stale, at(1)));
    assert!(!ledger.abandon(stale));
    assert!(ledger.abandon(fresh));
    assert!(!ledger.abandon(fresh));
}

#[test]
fn invalid_time_and_exhaustion_fail_closed() {
    let mut ledger = Dedup::<u64, 1>::new();
    assert_eq!(ledger.claim(0, at(1), Duration::ZERO), Claim::Rejected);
    assert_eq!(ledger.claim(0, at(0), at(1)), Claim::Rejected);
    assert_eq!(ledger.claim(0, Duration::MAX, at(1)), Claim::Rejected);
    let mut exhausted = Dedup::<u64, 1>::new();
    exhausted.generation = u64::MAX;
    assert_eq!(exhausted.claim(0, at(0), at(1)), Claim::Rejected);
    assert_eq!(Dedup::<u64, 0>::new().claim(0, at(0), at(1)), Claim::Full);
}
