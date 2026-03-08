use photoframe_domain::{seconds_to_microseconds, should_sync_time};

#[test]
fn invalid_rtc_time_forces_sync() {
    assert!(should_sync_time(0, 1_700_000_000));
}

#[test]
fn stale_sync_record_forces_sync() {
    assert!(should_sync_time(1_735_689_600, 0));
}

#[test]
fn recent_valid_sync_skips_network_time_sync() {
    assert!(!should_sync_time(
        1_735_689_600 + 7200,
        1_735_689_600 + 3600
    ));
}

#[test]
fn one_day_or_clock_rollback_triggers_sync() {
    let last_sync = 1_735_689_600 + 3600;
    assert!(should_sync_time(last_sync + 24 * 3600, last_sync));
    assert!(should_sync_time(last_sync, last_sync + 1));
}

#[test]
fn sleep_microseconds_use_u64_without_overflow() {
    assert_eq!(seconds_to_microseconds(3_600), 3_600_000_000);
    assert_eq!(
        seconds_to_microseconds(u32::MAX as u64),
        (u32::MAX as u64) * 1_000_000
    );
}
