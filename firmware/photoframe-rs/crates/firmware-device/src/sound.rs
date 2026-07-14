#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

use photoframe_firmware_device::button_logic::ButtonFeedback;

#[cfg(target_os = "espidf")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "espidf")]
static SOUND_DISABLED_LOGGED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "espidf")]
pub fn emit(_feedback: ButtonFeedback) {
    if !SOUND_DISABLED_LOGGED.swap(true, Ordering::SeqCst) {
        crate::device_log!(
            "WARN",
            "photoframe-rs/sound: audio feedback temporarily disabled while PMIC uses legacy i2c"
        );
    }
}

#[cfg(not(target_os = "espidf"))]
pub fn emit(_feedback: ButtonFeedback) {}
