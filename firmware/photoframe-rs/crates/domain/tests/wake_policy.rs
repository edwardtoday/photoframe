use photoframe_domain::{CycleAction, WakeSource, decide_cycle_action};

#[test]
fn spurious_ext1_skips_network_cycle() {
    let action = decide_cycle_action(WakeSource::SpuriousExt1);
    assert_eq!(action, CycleAction::SleepTimerOnly);
}

#[test]
fn key_wake_triggers_history_browse() {
    let action = decide_cycle_action(WakeSource::Key);
    assert_eq!(action, CycleAction::BrowseHistory);
}

#[test]
fn boot_wake_triggers_manual_sync() {
    let action = decide_cycle_action(WakeSource::Boot);
    assert_eq!(action, CycleAction::ManualSync);
}
