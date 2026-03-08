use photoframe_domain::{LongPressAction, decide_long_press_action};

#[test]
fn long_press_key_opens_sta_portal() {
    let action = decide_long_press_action(false, true, 3_000);
    assert_eq!(action, LongPressAction::OpenStaPortalWindow);
}

#[test]
fn long_press_boot_clears_wifi_and_enters_ap_portal() {
    let action = decide_long_press_action(true, false, 3_000);
    assert_eq!(action, LongPressAction::ClearWifiAndEnterPortal);
}

#[test]
fn short_press_key_does_not_open_sta_portal() {
    let action = decide_long_press_action(false, true, 2_999);
    assert_eq!(action, LongPressAction::None);
}
