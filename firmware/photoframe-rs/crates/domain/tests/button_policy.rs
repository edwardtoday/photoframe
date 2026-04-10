use photoframe_domain::{LongPressAction, decide_long_press_action};

#[test]
fn long_press_key_shows_current_photo() {
    let action = decide_long_press_action(false, true, 3_000);
    assert_eq!(action, LongPressAction::ShowCurrentPhoto);
}

#[test]
fn long_press_boot_enters_ap_portal_without_touching_wifi() {
    let action = decide_long_press_action(true, false, 3_000);
    assert_eq!(action, LongPressAction::EnterApPortal);
}

#[test]
fn short_press_key_does_not_trigger_long_press_action() {
    let action = decide_long_press_action(false, true, 2_999);
    assert_eq!(action, LongPressAction::None);
}
