use photoframe_domain::{LongPressAction, WakeSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwakeButtonAction {
    CycleHistory,
    ShowCurrentPhoto,
    EnterApPortal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonFeedback {
    KeyShort,
    KeyLong,
    BootShort,
    BootLong,
}

pub fn wake_source_from_ext1_state(
    boot_pin: bool,
    key_pin: bool,
    boot_seen_low: bool,
    key_seen_low: bool,
) -> WakeSource {
    if key_pin && !boot_pin {
        return WakeSource::Key;
    }
    if boot_pin && !key_pin {
        return WakeSource::Boot;
    }
    if boot_pin && boot_seen_low {
        return WakeSource::Boot;
    }
    if key_pin && key_seen_low {
        return WakeSource::Key;
    }
    if boot_pin || key_pin {
        return WakeSource::SpuriousExt1;
    }
    WakeSource::Other
}

// 把按键语义收敛成纯函数，避免后续改动睡眠/USB/电源路径时再次破坏行为契约。
pub fn desired_awake_button_action(
    wake_source: WakeSource,
    long_press_action: LongPressAction,
) -> Option<AwakeButtonAction> {
    if matches!(long_press_action, LongPressAction::EnterApPortal) {
        return Some(AwakeButtonAction::EnterApPortal);
    }
    if matches!(long_press_action, LongPressAction::ShowCurrentPhoto) {
        return Some(AwakeButtonAction::ShowCurrentPhoto);
    }
    if matches!(wake_source, WakeSource::Key) {
        return Some(AwakeButtonAction::CycleHistory);
    }
    None
}

pub fn feedback_for_wake_source(
    wake_source: WakeSource,
    long_press_action: LongPressAction,
) -> Option<ButtonFeedback> {
    if matches!(long_press_action, LongPressAction::EnterApPortal) {
        return Some(ButtonFeedback::BootLong);
    }
    if matches!(long_press_action, LongPressAction::ShowCurrentPhoto) {
        return Some(ButtonFeedback::KeyLong);
    }
    match wake_source {
        WakeSource::Key => Some(ButtonFeedback::KeyShort),
        WakeSource::Boot => Some(ButtonFeedback::BootShort),
        _ => None,
    }
}

pub fn feedback_for_awake_action(action: AwakeButtonAction) -> ButtonFeedback {
    match action {
        AwakeButtonAction::CycleHistory => ButtonFeedback::KeyShort,
        AwakeButtonAction::ShowCurrentPhoto => ButtonFeedback::KeyLong,
        AwakeButtonAction::EnterApPortal => ButtonFeedback::BootLong,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AwakeButtonAction, ButtonFeedback, desired_awake_button_action, feedback_for_awake_action,
        feedback_for_wake_source, wake_source_from_ext1_state,
    };
    use photoframe_domain::{LongPressAction, WakeSource};

    #[test]
    fn ext1_boot_press_maps_to_boot_wakeup() {
        assert_eq!(
            wake_source_from_ext1_state(true, false, true, false),
            WakeSource::Boot
        );
    }

    #[test]
    fn ext1_key_press_maps_to_key_wakeup() {
        assert_eq!(
            wake_source_from_ext1_state(false, true, false, true),
            WakeSource::Key
        );
    }

    #[test]
    fn ext1_key_latched_after_release_still_maps_to_key_wakeup() {
        assert_eq!(
            wake_source_from_ext1_state(false, true, false, false),
            WakeSource::Key
        );
    }

    #[test]
    fn ext1_boot_latched_after_release_still_maps_to_boot_wakeup() {
        assert_eq!(
            wake_source_from_ext1_state(true, false, false, false),
            WakeSource::Boot
        );
    }

    #[test]
    fn ext1_without_observed_press_is_spurious() {
        assert_eq!(
            wake_source_from_ext1_state(true, true, false, false),
            WakeSource::SpuriousExt1
        );
    }

    #[test]
    fn key_wakeup_defaults_to_cycle_history() {
        assert_eq!(
            desired_awake_button_action(WakeSource::Key, LongPressAction::None),
            Some(AwakeButtonAction::CycleHistory)
        );
    }

    #[test]
    fn boot_wakeup_defaults_to_normal_sync() {
        assert_eq!(
            desired_awake_button_action(WakeSource::Boot, LongPressAction::None),
            None
        );
    }

    #[test]
    fn long_key_overrides_cycle_to_show_current_photo() {
        assert_eq!(
            desired_awake_button_action(WakeSource::Key, LongPressAction::ShowCurrentPhoto),
            Some(AwakeButtonAction::ShowCurrentPhoto)
        );
    }

    #[test]
    fn long_boot_enters_ap_portal() {
        assert_eq!(
            desired_awake_button_action(WakeSource::Boot, LongPressAction::EnterApPortal),
            Some(AwakeButtonAction::EnterApPortal)
        );
    }

    #[test]
    fn boot_wakeup_emits_boot_short_feedback() {
        assert_eq!(
            feedback_for_wake_source(WakeSource::Boot, LongPressAction::None),
            Some(ButtonFeedback::BootShort)
        );
    }

    #[test]
    fn key_wakeup_emits_key_short_feedback() {
        assert_eq!(
            feedback_for_wake_source(WakeSource::Key, LongPressAction::None),
            Some(ButtonFeedback::KeyShort)
        );
    }

    #[test]
    fn long_key_emits_key_long_feedback() {
        assert_eq!(
            feedback_for_wake_source(WakeSource::Key, LongPressAction::ShowCurrentPhoto),
            Some(ButtonFeedback::KeyLong)
        );
    }

    #[test]
    fn long_boot_emits_boot_long_feedback() {
        assert_eq!(
            feedback_for_wake_source(WakeSource::Boot, LongPressAction::EnterApPortal),
            Some(ButtonFeedback::BootLong)
        );
    }

    #[test]
    fn awake_cycle_history_uses_key_short_feedback() {
        assert_eq!(
            feedback_for_awake_action(AwakeButtonAction::CycleHistory),
            ButtonFeedback::KeyShort
        );
    }
}
