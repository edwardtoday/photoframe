#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use std::{thread, time::Duration};

use photoframe_firmware_device::button_logic::ButtonFeedback;

#[cfg(target_os = "espidf")]
use esp_idf_sys as sys;

#[cfg(target_os = "espidf")]
const LED_RED_PIN: i32 = sys::gpio_num_t_GPIO_NUM_45;
#[cfg(target_os = "espidf")]
const LED_GREEN_PIN: i32 = sys::gpio_num_t_GPIO_NUM_42;
#[cfg(target_os = "espidf")]
const LED_ON_LEVEL: u32 = 0;
#[cfg(target_os = "espidf")]
const LED_OFF_LEVEL: u32 = 1;
#[cfg(target_os = "espidf")]
const SHORT_FEEDBACK_MS: u64 = 200;
#[cfg(target_os = "espidf")]
const LONG_FEEDBACK_MS: u64 = 500;

#[cfg(target_os = "espidf")]
fn configure_led_pins_once() {
    unsafe {
        let cfg = sys::gpio_config_t {
            pin_bit_mask: (1u64 << LED_RED_PIN) | (1u64 << LED_GREEN_PIN),
            mode: sys::gpio_mode_t_GPIO_MODE_OUTPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let _ = sys::gpio_config(&cfg);
        let _ = sys::gpio_set_level(LED_RED_PIN, LED_OFF_LEVEL);
        let _ = sys::gpio_set_level(LED_GREEN_PIN, LED_OFF_LEVEL);
    }
}

#[cfg(target_os = "espidf")]
fn emit_single(pin: i32, duration_ms: u64) {
    unsafe {
        let _ = sys::gpio_set_level(LED_RED_PIN, LED_OFF_LEVEL);
        let _ = sys::gpio_set_level(LED_GREEN_PIN, LED_OFF_LEVEL);
        let _ = sys::gpio_set_level(pin, LED_ON_LEVEL);
    }
    thread::sleep(Duration::from_millis(duration_ms));
    unsafe {
        let _ = sys::gpio_set_level(pin, LED_OFF_LEVEL);
    }
}

#[cfg(target_os = "espidf")]
pub fn init() {
    configure_led_pins_once();
}

#[cfg(target_os = "espidf")]
pub fn emit(feedback: ButtonFeedback) {
    crate::sound::emit(feedback);
    configure_led_pins_once();
    match feedback {
        ButtonFeedback::KeyShort => emit_single(LED_RED_PIN, SHORT_FEEDBACK_MS),
        ButtonFeedback::KeyLong => emit_single(LED_RED_PIN, LONG_FEEDBACK_MS),
        ButtonFeedback::BootShort => emit_single(LED_GREEN_PIN, SHORT_FEEDBACK_MS),
        ButtonFeedback::BootLong => emit_single(LED_GREEN_PIN, LONG_FEEDBACK_MS),
    }
}

#[cfg(not(target_os = "espidf"))]
pub fn init() {}

#[cfg(not(target_os = "espidf"))]
pub fn emit(_feedback: ButtonFeedback) {}
