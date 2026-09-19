//! Opt-in live smoke test against TypeSafe's System One endpoint. Needs
//! network access and a real key, so it runs only when explicitly requested:
//!
//! ```bash
//! BUTTERFLY_LIVE_TEST=1 TYPESAFE_API_KEY=... cargo test -p butterfly-core --test live_triage -- --nocapture
//! ```
//!
//! What it proves, and why each part is worth a paid call: the wire shape we
//! send is accepted, the answer parses, the choice is inside the vocabulary we
//! offered, and the round trip is fast enough to be worth putting in the loop.
//! The latency line is the number the whole feature is judged on — a call that
//! costs more wall-clock than the wasted attempt it saves is not worth making.

use std::time::Instant;

use butterfly_core::failure::FailureKind;
use butterfly_core::triage::{
    JevClient, Triage, TriageOutcome, DEFAULT_MIN_CONFIDENCE, DEFAULT_TIMEOUT,
};

/// Real `cargo test` output from a failing assertion, with every signature the
/// rule-based classifier keys on ("panicked at", "test result: FAILED",
/// "assertion") stripped out. The rules must give up on this; a competent
/// reader should still see a failing test.
const UNNAMEABLE_TEST_FAILURE: &str = "\
running 3 tests
t1 ... ok
t2 ... ok
t3 ... not ok

t3: left side was 4, right side was 5
  --> src/tally.rs:88

summary: 2 of 3 behaved as the spec describes, 1 did not
";

/// Guards the fixture offline, on every run. A rule added to
/// `classify_command_output` could start naming this output, and the live test
/// would then quietly stop exercising triage at all while still passing.
#[test]
fn the_fixture_still_defeats_the_rule_based_classifier() {
    assert_eq!(
        butterfly_core::failure::classify_command_output(UNNAMEABLE_TEST_FAILURE),
        FailureKind::UncertainResult,
        "the rules now name this output; pick a fixture that still defeats them"
    );
}

#[tokio::test]
async fn live_triage_names_a_failure_the_rules_cannot() {
    if std::env::var("BUTTERFLY_LIVE_TEST").as_deref() != Ok("1") {
        eprintln!("skipped: set BUTTERFLY_LIVE_TEST=1 to run");
        return;
    }
    let api_key = std::env::var(butterfly_core::triage::API_KEY_ENV)
        .unwrap_or_else(|_| panic!("{} is required", butterfly_core::triage::API_KEY_ENV));

    let client = JevClient::new(api_key, None, None, DEFAULT_TIMEOUT).expect("client");
    let triage = Triage::new(std::sync::Arc::new(client), DEFAULT_MIN_CONFIDENCE);

    let started = Instant::now();
    let outcome = triage
        .refine_command_failure(
            FailureKind::UncertainResult,
            "cargo test -p tally",
            UNNAMEABLE_TEST_FAILURE,
        )
        .await
        .expect("triage is enabled, so it answers");
    let elapsed = started.elapsed();

    eprintln!("system one round trip: {} ms", elapsed.as_millis());
    eprintln!("outcome: {outcome:?}");

    match outcome {
        TriageOutcome::Refined { kind, confidence } => {
            assert!(confidence >= DEFAULT_MIN_CONFIDENCE);
            // We do not assert one specific kind: `test_failure` is the
            // reading we would defend, but a defensible neighbour is not a
            // product bug. What must hold is that the answer is inside the
            // vocabulary and carries guidance `uncertain_result` does not.
            assert_ne!(kind, FailureKind::UncertainResult);
            assert!(!kind.guidance().is_empty());
        }
        TriageOutcome::Declined { .. } | TriageOutcome::BelowFloor { .. } => {
            // A legitimate answer: the endpoint worked, it just would not
            // commit. The loop keeps the rule verdict and loses nothing.
            eprintln!("note: no confident answer; the rule verdict would stand");
        }
        TriageOutcome::Failed { error } => panic!("live endpoint failed: {error}"),
    }
}
