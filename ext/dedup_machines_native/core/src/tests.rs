use super::*;
fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}
fn acquired<V>(claim: Claim<'_, V>) -> ClaimId {
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
    assert_eq!(ledger.claim(7, at(9), at(1)), Claim::Completed(&()));
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

#[test]
fn fingerprints_conflict_across_pending_and_completed_records() {
    let mut ledger = Dedup::<u64, 2>::new();
    let id = acquired(ledger.claim_with(7, 0xA, at(0), at(10)));
    assert_eq!(ledger.claim_with(7, 0xB, at(1), at(10)), Claim::Conflict);
    assert_eq!(ledger.claim(7, at(1), at(10)), Claim::Conflict);
    assert_eq!(ledger.claim_with(7, 0xA, at(1), at(10)), Claim::InProgress);
    assert!(ledger.complete(id, at(2)));
    assert_eq!(ledger.claim_with(7, 0xB, at(3), at(10)), Claim::Conflict);
    assert_eq!(
        ledger.claim_with(7, 0xA, at(3), at(10)),
        Claim::Completed(&())
    );
    assert!(matches!(
        ledger.claim_with(7, 0xB, at(10), at(10)),
        Claim::Acquired(_)
    ));
}

#[test]
fn completion_values_are_returned_to_duplicates_and_stale_completers() {
    let mut ledger = Dedup::<u64, 1, u32>::new();
    let id = acquired(ledger.claim(7, at(0), at(10)));
    assert_eq!(ledger.complete_with(id, 42, at(1)), Ok(()));
    assert_eq!(ledger.complete_with(id, 43, at(1)), Err(43));
    assert_eq!(ledger.claim(7, at(2), at(10)), Claim::Completed(&42));
    let replacement = acquired(ledger.claim(7, at(10), at(1)));
    assert_eq!(ledger.complete_with(id, 44, at(10)), Err(44));
    assert_eq!(ledger.complete_with(replacement, 45, at(11)), Err(45));
}

#[test]
fn partition_caps_bound_each_bucket_below_the_table() {
    let mut ledger = Dedup::<u64, 3>::new().with_partition_cap(1);
    let first = acquired(ledger.admit(Attempt::new(1).partition(9), at(0), at(10)));
    assert_eq!(
        ledger.admit(Attempt::new(2).partition(9), at(0), at(10)),
        Claim::PartitionFull
    );
    assert!(matches!(
        ledger.admit(Attempt::new(2).partition(8), at(0), at(10)),
        Claim::Acquired(_)
    ));
    assert!(matches!(ledger.claim(3, at(0), at(10)), Claim::Acquired(_)));
    assert!(ledger.abandon(first));
    assert!(matches!(
        ledger.admit(Attempt::new(1).partition(9), at(1), at(10)),
        Claim::Acquired(_)
    ));
    assert_eq!(
        ledger.admit(Attempt::new(4).partition(7), at(1), at(10)),
        Claim::Full
    );
}

#[test]
fn ownerless_records_expire_ahead_of_their_ttl() {
    let mut ledger = Dedup::<(u8, u8), 3>::new();
    let dead = acquired(ledger.claim((1, 1), at(0), at(10)));
    let done = acquired(ledger.claim((1, 2), at(0), at(10)));
    let alive = acquired(ledger.claim((2, 1), at(0), at(10)));
    assert!(ledger.complete(done, at(0)));
    assert_eq!(ledger.expire_where(|key| key.0 == 1), 2);
    assert_eq!(ledger.expire_where(|key| key.0 == 1), 0);
    assert!(!ledger.abandon(dead));
    assert!(matches!(
        ledger.claim((1, 2), at(1), at(10)),
        Claim::Acquired(_)
    ));
    assert!(ledger.complete(alive, at(1)));
}
