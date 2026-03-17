use photoframe_domain::sleep_seconds_until_next_beijing_sync;

#[test]
fn next_sync_waits_until_same_day_4pm_beijing() {
    let now_epoch = 1_773_646_200; // 2026-03-16 07:30:00 UTC => 15:30:00 CST

    let sleep_seconds = sleep_seconds_until_next_beijing_sync(now_epoch);

    assert_eq!(sleep_seconds, Some(30 * 60));
}

#[test]
fn next_sync_rolls_to_next_day_5am_beijing_after_4pm() {
    let now_epoch = 1_773_648_000; // 2026-03-16 08:00:00 UTC => 16:00:00 CST

    let sleep_seconds = sleep_seconds_until_next_beijing_sync(now_epoch);

    assert_eq!(sleep_seconds, Some(13 * 3600));
}

#[test]
fn invalid_clock_falls_back_to_legacy_interval_logic() {
    let sleep_seconds = sleep_seconds_until_next_beijing_sync(1_000);

    assert_eq!(sleep_seconds, None);
}
