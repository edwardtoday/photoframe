use photoframe_domain::{BackoffDecision, FailureKind, RetryPolicy, apply_cycle_outcome};

#[test]
fn general_failure_uses_exponential_backoff() {
    let policy = RetryPolicy {
        interval_minutes: 60,
        retry_base_minutes: 5,
        retry_max_minutes: 240,
        max_failure_before_long_sleep: 24,
    };

    let decision = apply_cycle_outcome(&policy, 0, FailureKind::GeneralFailure);

    assert_eq!(
        decision,
        BackoffDecision {
            next_failure_count: 1,
            sleep_seconds: 300,
        }
    );
}

#[test]
fn failure_backoff_is_clamped_to_maximum() {
    let policy = RetryPolicy {
        interval_minutes: 60,
        retry_base_minutes: 5,
        retry_max_minutes: 20,
        max_failure_before_long_sleep: 24,
    };

    let decision = apply_cycle_outcome(&policy, 4, FailureKind::GeneralFailure);

    assert_eq!(
        decision,
        BackoffDecision {
            next_failure_count: 5,
            sleep_seconds: 1_200,
        }
    );
}

#[test]
fn pmic_soft_failure_keeps_regular_interval_and_resets_failure_count() {
    let policy = RetryPolicy {
        interval_minutes: 60,
        retry_base_minutes: 5,
        retry_max_minutes: 240,
        max_failure_before_long_sleep: 24,
    };

    let decision = apply_cycle_outcome(&policy, 7, FailureKind::PmicSoftFailure);

    assert_eq!(
        decision,
        BackoffDecision {
            next_failure_count: 0,
            sleep_seconds: 3_600,
        }
    );
}

#[test]
fn success_keeps_regular_interval_and_clears_failure_count() {
    let policy = RetryPolicy {
        interval_minutes: 30,
        retry_base_minutes: 5,
        retry_max_minutes: 240,
        max_failure_before_long_sleep: 24,
    };

    let decision = apply_cycle_outcome(&policy, 3, FailureKind::Success);

    assert_eq!(
        decision,
        BackoffDecision {
            next_failure_count: 0,
            sleep_seconds: 1_800,
        }
    );
}
