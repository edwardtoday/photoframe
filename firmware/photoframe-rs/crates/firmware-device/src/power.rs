#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use std::{
    ptr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[cfg(target_os = "espidf")]
use esp_idf_sys as sys;
#[cfg(target_os = "espidf")]
use photoframe_app::{NormalizePowerOutcome, PowerCache, PowerSample, normalize_power_sample};

#[cfg(target_os = "espidf")]
const I2C_PORT_LEGACY: sys::i2c_port_t = sys::i2c_port_t_I2C_NUM_0;
#[cfg(target_os = "espidf")]
const I2C_SCL_PIN: i32 = 48;
#[cfg(target_os = "espidf")]
const I2C_SDA_PIN: i32 = 47;
#[cfg(target_os = "espidf")]
const AXP_IRQ_PIN: i32 = sys::gpio_num_t_GPIO_NUM_21;
#[cfg(target_os = "espidf")]
const PWR_BUTTON_PIN: i32 = sys::gpio_num_t_GPIO_NUM_5;
#[cfg(target_os = "espidf")]
const I2C_FREQ_HZ: u32 = 100_000;
#[cfg(target_os = "espidf")]
const I2C_LEGACY_TIMEOUT_TICKS: sys::TickType_t = 100;
#[cfg(target_os = "espidf")]
const AXP2101_ADDR: u16 = 0x34;
#[cfg(target_os = "espidf")]
const REG_CHIP_ID: u8 = 0x03;
#[cfg(target_os = "espidf")]
const REG_STATUS1: u8 = 0x00;
#[cfg(target_os = "espidf")]
const REG_STATUS2: u8 = 0x01;
#[cfg(target_os = "espidf")]
const REG_EXTEN_CFG: u8 = 0x16;
#[cfg(target_os = "espidf")]
const REG_SLEEP_WAKEUP_CTRL: u8 = 0x26;
#[cfg(target_os = "espidf")]
const REG_ADC_CHANNEL_CTRL: u8 = 0x30;
#[cfg(target_os = "espidf")]
const REG_INT_EN1: u8 = 0x40;
#[cfg(target_os = "espidf")]
const REG_INT_EN2: u8 = 0x41;
#[cfg(target_os = "espidf")]
const REG_INT_EN3: u8 = 0x42;
#[cfg(target_os = "espidf")]
const REG_INT_STS1: u8 = 0x48;
#[cfg(target_os = "espidf")]
const REG_INT_STS2: u8 = 0x49;
#[cfg(target_os = "espidf")]
const REG_INT_STS3: u8 = 0x4A;
#[cfg(target_os = "espidf")]
const REG_ADC_BATT_H: u8 = 0x34;
#[cfg(target_os = "espidf")]
const REG_ADC_BATT_L: u8 = 0x35;
#[cfg(target_os = "espidf")]
const REG_BATTERY_PERCENT: u8 = 0xA4;
#[cfg(target_os = "espidf")]
const REG_DCDC_ON_OFF_CTRL: u8 = 0x80;
#[cfg(target_os = "espidf")]
const REG_DCDC1_VOLT_CTRL: u8 = 0x82;
#[cfg(target_os = "espidf")]
const REG_BATT_DET_CTRL: u8 = 0x68;
#[cfg(target_os = "espidf")]
const REG_LDO_ON_OFF_CTRL0: u8 = 0x90;
#[cfg(target_os = "espidf")]
const REG_LDO_ON_OFF_CTRL1: u8 = 0x91;
#[cfg(target_os = "espidf")]
const REG_LDO_VOL0_CTRL: u8 = 0x92;
#[cfg(target_os = "espidf")]
const REG_LDO_VOL1_CTRL: u8 = 0x93;
#[cfg(target_os = "espidf")]
const REG_LDO_VOL2_CTRL: u8 = 0x94;
#[cfg(target_os = "espidf")]
const REG_LDO_VOL3_CTRL: u8 = 0x95;
#[cfg(target_os = "espidf")]
const EXPECTED_CHIP_ID: u8 = 0x4A;
#[cfg(target_os = "espidf")]
const DCDC_ENABLE_DCDC1: u8 = 1u8 << 0;
#[cfg(target_os = "espidf")]
const SLEEP_DCDC_DISABLE_MASK: u8 = (1u8 << 1) | (1u8 << 2) | (1u8 << 3) | (1u8 << 4);
#[cfg(target_os = "espidf")]
const LDO_CODE_3300: u8 = 0x1C;
#[cfg(target_os = "espidf")]
const LDO_ENABLE_ALDO3: u8 = 1u8 << 2;
#[cfg(target_os = "espidf")]
const LDO_ENABLE_ALDO4: u8 = 1u8 << 3;
#[cfg(target_os = "espidf")]
const LDO_ENABLE_ALL: u8 = 0x0F;
#[cfg(target_os = "espidf")]
const UNUSED_LDO_CTRL0_MASK: u8 = 0xF0;
#[cfg(target_os = "espidf")]
const UNUSED_LDO_CTRL1_MASK: u8 = 0x01;
#[cfg(target_os = "espidf")]
const SLEEP_LDO_DISABLE_MASK0: u8 = LDO_ENABLE_ALL | UNUSED_LDO_CTRL0_MASK;
#[cfg(target_os = "espidf")]
const SLEEP_LDO_DISABLE_MASK1: u8 = UNUSED_LDO_CTRL1_MASK;
#[cfg(target_os = "espidf")]
const SLEEP_CTRL_SLEEP_ENABLE: u8 = 1u8 << 0;
#[cfg(target_os = "espidf")]
const SLEEP_CTRL_WAKEUP_ENABLE: u8 = 1u8 << 1;
#[cfg(target_os = "espidf")]
const SLEEP_CTRL_DC_DLO_SELECT: u8 = 1u8 << 2;
#[cfg(target_os = "espidf")]
const SLEEP_CTRL_PWROK_TO_LOW: u8 = 1u8 << 3;
#[cfg(target_os = "espidf")]
const SLEEP_CTRL_IRQ_PIN_TO_LOW: u8 = 1u8 << 4;
#[cfg(target_os = "espidf")]
const POWER_INIT_MAX_ATTEMPTS: usize = 2;
#[cfg(target_os = "espidf")]
const POWER_INIT_RETRY_DELAY_MS: u64 = 250;
#[cfg(target_os = "espidf")]
const AXP_WAKE_SETTLE_MS: u64 = 2_000;
#[cfg(target_os = "espidf")]
const AXP_WAKE_MAX_ATTEMPTS: usize = 2;
#[cfg(target_os = "espidf")]
const AXP_HARD_WAKE_LOW_MS: u64 = 1_500;
#[cfg(target_os = "espidf")]
const AXP_HARD_WAKE_SETTLE_MS: u64 = 3_000;
#[cfg(target_os = "espidf")]
const SDCARD_POWER_CYCLE_OFF_MS: u64 = 80;
#[cfg(target_os = "espidf")]
const SDCARD_POWER_CYCLE_ON_MS: u64 = 250;
#[cfg(target_os = "espidf")]
const I2C_LINE_PROBE_SETTLE_MS: u64 = 2;
#[cfg(target_os = "espidf")]
const I2C_LINE_PROBE_LOW_MS: u64 = 1;
#[cfg(target_os = "espidf")]
const I2C_FORCED_RECOVERY_PULSES: usize = 16;
#[cfg(target_os = "espidf")]
const I2C_FORCED_RECOVERY_LOW_MS: u64 = 1;
#[cfg(target_os = "espidf")]
const I2C_FORCED_RECOVERY_HIGH_MS: u64 = 2;
#[cfg(target_os = "espidf")]
const I2C_FORCED_RECOVERY_PROBE_DELAY_MS: [u64; 3] = [0, 1, 5];
#[cfg(target_os = "espidf")]
const POWER_DIAG_PINS: [(&str, i32); 4] = [
    ("scl", I2C_SCL_PIN),
    ("sda", I2C_SDA_PIN),
    ("irq", AXP_IRQ_PIN),
    ("pwr_btn", PWR_BUTTON_PIN),
];

#[cfg(target_os = "espidf")]
struct PowerRuntime {
    bus: sys::i2c_master_bus_handle_t,
    dev: sys::i2c_master_dev_handle_t,
    ready: bool,
}

#[cfg(target_os = "espidf")]
unsafe impl Send for PowerRuntime {}

#[cfg(target_os = "espidf")]
impl Default for PowerRuntime {
    fn default() -> Self {
        Self {
            bus: ptr::null_mut(),
            dev: ptr::null_mut(),
            ready: false,
        }
    }
}

#[cfg(target_os = "espidf")]
static POWER_RUNTIME: OnceLock<Mutex<PowerRuntime>> = OnceLock::new();
#[cfg(target_os = "espidf")]
static I2C_LINE_PROBE_RAN: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "espidf")]
static AXP_IRQ_PROBE_RAN: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "espidf")]
static BOOT_POWER_PREPARED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "espidf")]
#[unsafe(link_section = ".rtc.data")]
static mut RTC_POWER_CACHE: PowerCache = PowerCache {
    battery_mv: -1,
    battery_percent: -1,
    charging: -1,
    vbus_good: -1,
    cached_epoch: 0,
};

#[cfg(target_os = "espidf")]
const RTC_SLEEP_DIAG_MAGIC: u32 = 0x5046_5344;

#[cfg(target_os = "espidf")]
#[repr(C)]
#[derive(Clone, Copy)]
struct SleepDiagSnapshot {
    magic: u32,
    power_ready: u32,
    pre_scl: i32,
    pre_sda: i32,
    pre_irq: i32,
    pre_pwr: i32,
    final_scl: i32,
    final_sda: i32,
    final_irq: i32,
    final_pwr: i32,
    reg26: i32,
    reg80: i32,
    reg90: i32,
    reg91: i32,
}

#[cfg(target_os = "espidf")]
#[unsafe(link_section = ".rtc.data")]
static mut RTC_SLEEP_DIAG: SleepDiagSnapshot = SleepDiagSnapshot {
    magic: 0,
    power_ready: 0,
    pre_scl: -1,
    pre_sda: -1,
    pre_irq: -1,
    pre_pwr: -1,
    final_scl: -1,
    final_sda: -1,
    final_irq: -1,
    final_pwr: -1,
    reg26: -1,
    reg80: -1,
    reg90: -1,
    reg91: -1,
};

#[cfg(target_os = "espidf")]
fn runtime() -> &'static Mutex<PowerRuntime> {
    POWER_RUNTIME.get_or_init(|| Mutex::new(PowerRuntime::default()))
}

#[cfg(target_os = "espidf")]
fn sleep_ms(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}

#[cfg(target_os = "espidf")]
fn log(message: &str) {
    let rendered = format!("photoframe-rs/power: {message}");
    println!("{}", rendered);
    crate::diag::append("INFO", &rendered);
}

#[cfg(target_os = "espidf")]
fn err_name(err: i32) -> String {
    unsafe {
        std::ffi::CStr::from_ptr(sys::esp_err_to_name(err))
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(target_os = "espidf")]
fn read_bus_levels_raw() -> (i32, i32, i32, i32) {
    unsafe {
        (
            sys::gpio_get_level(I2C_SCL_PIN),
            sys::gpio_get_level(I2C_SDA_PIN),
            sys::gpio_get_level(AXP_IRQ_PIN),
            sys::gpio_get_level(PWR_BUTTON_PIN),
        )
    }
}

#[cfg(target_os = "espidf")]
fn log_bus_levels_raw(stage: &str) {
    let (scl, sda, irq, pwr) = read_bus_levels_raw();
    log(&format!(
        "{stage}: scl={scl} sda={sda} irq={irq} pwr_btn={pwr}"
    ));
}

#[cfg(target_os = "espidf")]
fn format_diag_reg(value: i32) -> String {
    if value < 0 {
        "n/a".to_string()
    } else {
        format!("0x{value:02x}")
    }
}

#[cfg(target_os = "espidf")]
fn format_diag_opt_u8(value: Option<u8>) -> String {
    value
        .map(|value| format!("0x{value:02x}"))
        .unwrap_or_else(|| "n/a".to_string())
}

#[cfg(target_os = "espidf")]
fn store_sleep_diag_snapshot(
    power_ready: bool,
    pre_levels: (i32, i32, i32, i32),
    final_levels: (i32, i32, i32, i32),
    reg26: Option<u8>,
    reg80: Option<u8>,
    reg90: Option<u8>,
    reg91: Option<u8>,
) {
    unsafe {
        RTC_SLEEP_DIAG = SleepDiagSnapshot {
            magic: RTC_SLEEP_DIAG_MAGIC,
            power_ready: u32::from(power_ready),
            pre_scl: pre_levels.0,
            pre_sda: pre_levels.1,
            pre_irq: pre_levels.2,
            pre_pwr: pre_levels.3,
            final_scl: final_levels.0,
            final_sda: final_levels.1,
            final_irq: final_levels.2,
            final_pwr: final_levels.3,
            reg26: reg26.map(i32::from).unwrap_or(-1),
            reg80: reg80.map(i32::from).unwrap_or(-1),
            reg90: reg90.map(i32::from).unwrap_or(-1),
            reg91: reg91.map(i32::from).unwrap_or(-1),
        };
    }
}

#[cfg(target_os = "espidf")]
fn take_sleep_diag_snapshot() -> Option<SleepDiagSnapshot> {
    unsafe {
        if RTC_SLEEP_DIAG.magic != RTC_SLEEP_DIAG_MAGIC {
            return None;
        }
        let snapshot = RTC_SLEEP_DIAG;
        RTC_SLEEP_DIAG.magic = 0;
        Some(snapshot)
    }
}

#[cfg(target_os = "espidf")]
fn log_sleep_diag_snapshot() {
    let Some(snapshot) = take_sleep_diag_snapshot() else {
        return;
    };
    log(&format!(
        "boot rtc sleep snapshot: power_ready={} pre[scl={} sda={} irq={} pwr_btn={}] final[scl={} sda={} irq={} pwr_btn={}] reg26={} reg80={} reg90={} reg91={}",
        snapshot.power_ready,
        snapshot.pre_scl,
        snapshot.pre_sda,
        snapshot.pre_irq,
        snapshot.pre_pwr,
        snapshot.final_scl,
        snapshot.final_sda,
        snapshot.final_irq,
        snapshot.final_pwr,
        format_diag_reg(snapshot.reg26),
        format_diag_reg(snapshot.reg80),
        format_diag_reg(snapshot.reg90),
        format_diag_reg(snapshot.reg91),
    ));
}

#[cfg(target_os = "espidf")]
fn release_gpio_pad_state(pin: i32, label: &str, reset_to_input_pullup: bool) {
    unsafe {
        let sleep_sel_dis = sys::gpio_sleep_sel_dis(pin);
        let hold_dis = sys::gpio_hold_dis(pin);
        let rtc_hold_dis = sys::rtc_gpio_hold_dis(pin);
        let rtc_deinit = if pin == AXP_IRQ_PIN {
            sys::rtc_gpio_deinit(pin)
        } else {
            0
        };
        let level_before = sys::gpio_get_level(pin);
        if reset_to_input_pullup {
            let reset = sys::gpio_reset_pin(pin);
            let input = sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT);
            let pullup = sys::gpio_pullup_en(pin);
            let level_after = sys::gpio_get_level(pin);
            log(&format!(
                "pad release {label}: pin={pin} mode=input_pullup sleep_sel_dis={sleep_sel_dis} hold_dis={hold_dis} rtc_hold_dis={rtc_hold_dis} rtc_deinit={rtc_deinit} reset={reset} input={input} pullup={pullup} level_before={level_before} level_after={level_after}"
            ));
        } else {
            log(&format!(
                "pad release {label}: pin={pin} mode=hold_only sleep_sel_dis={sleep_sel_dis} hold_dis={hold_dis} rtc_hold_dis={rtc_hold_dis} rtc_deinit={rtc_deinit} level_before={level_before}"
            ));
        }
    }
}

#[cfg(target_os = "espidf")]
fn release_power_diag_pins(stage: &str, preserve_i2c_state: bool) {
    log_bus_levels_raw(&format!("{stage} raw before"));
    if preserve_i2c_state {
        log(&format!(
            "{stage}: leave i2c pads untouched, skip gpio_deep_sleep_hold_dis"
        ));
        for (label, pin) in POWER_DIAG_PINS {
            if pin == I2C_SCL_PIN || pin == I2C_SDA_PIN {
                log(&format!(
                    "pad release {label}: pin={pin} mode=preserve_i2c level={}",
                    read_pin_level(pin)
                ));
                continue;
            }
            release_gpio_pad_state(pin, label, true);
        }
        log_bus_levels_raw(&format!("{stage} raw after preserve"));
        configure_axp_irq_pin_input_pullup();
        configure_pwr_button_pin_input_pullup();
        log_bus_levels_raw(&format!("{stage} raw after non-i2c pullup"));
        return;
    }

    unsafe {
        sys::gpio_deep_sleep_hold_dis();
    }
    log(&format!("{stage}: gpio_deep_sleep_hold_dis called"));
    for (label, pin) in POWER_DIAG_PINS {
        release_gpio_pad_state(pin, label, true);
    }
    log_bus_levels_raw(&format!("{stage} raw after release"));
    configure_i2c_lines_input_pullup();
    configure_axp_irq_pin_input_pullup();
    configure_pwr_button_pin_input_pullup();
    log_bus_levels(&format!("{stage} sampled after pullup"));
}

#[cfg(target_os = "espidf")]
fn release_power_diag_pins_for_boot(stage: &str) {
    // 官方固件启动时不保留上次深睡的 GPIO/I2C pad 状态。先解除 hold 并拉成输入上拉，
    // 避免旧固件或失败睡眠路径把 PMIC I2C 线保持在低电平。
    release_power_diag_pins(stage, false);
}

#[cfg(target_os = "espidf")]
fn configure_i2c_lines_input_pullup() {
    unsafe {
        let _ = sys::gpio_sleep_sel_dis(I2C_SCL_PIN);
        let _ = sys::gpio_sleep_sel_dis(I2C_SDA_PIN);
        let _ = sys::gpio_hold_dis(I2C_SCL_PIN);
        let _ = sys::gpio_hold_dis(I2C_SDA_PIN);
        let _ = sys::gpio_set_direction(I2C_SCL_PIN, sys::gpio_mode_t_GPIO_MODE_INPUT);
        let _ = sys::gpio_set_direction(I2C_SDA_PIN, sys::gpio_mode_t_GPIO_MODE_INPUT);
        let _ = sys::gpio_pullup_en(I2C_SCL_PIN);
        let _ = sys::gpio_pullup_en(I2C_SDA_PIN);
        let _ = sys::gpio_pulldown_dis(I2C_SCL_PIN);
        let _ = sys::gpio_pulldown_dis(I2C_SDA_PIN);
    }
}

#[cfg(target_os = "espidf")]
fn read_i2c_line_levels() -> (i32, i32) {
    unsafe {
        (
            sys::gpio_get_level(I2C_SCL_PIN),
            sys::gpio_get_level(I2C_SDA_PIN),
        )
    }
}

#[cfg(target_os = "espidf")]
fn read_pin_level(pin: i32) -> i32 {
    unsafe { sys::gpio_get_level(pin) }
}

#[cfg(target_os = "espidf")]
fn configure_single_input_pull(pin: i32, pullup: bool, pulldown: bool) -> (i32, i32, i32, i32) {
    unsafe {
        let reset = sys::gpio_reset_pin(pin);
        let dir = sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT);
        let pullup_rc = if pullup {
            sys::gpio_pullup_en(pin)
        } else {
            sys::gpio_pullup_dis(pin)
        };
        let pulldown_rc = if pulldown {
            sys::gpio_pulldown_en(pin)
        } else {
            sys::gpio_pulldown_dis(pin)
        };
        (reset, dir, pullup_rc, pulldown_rc)
    }
}

#[cfg(target_os = "espidf")]
fn configure_single_open_drain_release(pin: i32) -> i32 {
    unsafe {
        let _ = sys::gpio_hold_dis(pin);
        let _ = sys::gpio_reset_pin(pin);
        let cfg = sys::gpio_config_t {
            pin_bit_mask: 1u64 << pin,
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let cfg_rc = sys::gpio_config(&cfg);
        let level_rc = sys::gpio_set_level(pin, 1);
        if cfg_rc != 0 { cfg_rc } else { level_rc }
    }
}

#[cfg(target_os = "espidf")]
fn configure_single_weak_push_pull(pin: i32, level: u32) -> (i32, i32, i32, i32) {
    unsafe {
        let _ = sys::gpio_hold_dis(pin);
        let _ = sys::gpio_reset_pin(pin);
        let drive_rc = sys::gpio_set_drive_capability(pin, sys::gpio_drive_cap_t_GPIO_DRIVE_CAP_0);
        let dir_rc = sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT);
        let level_rc = sys::gpio_set_level(pin, level);
        (0, drive_rc, dir_rc, level_rc)
    }
}

#[cfg(target_os = "espidf")]
fn probe_i2c_line(stage: &str, label: &str, pin: i32, peer_label: &str, peer_pin: i32) {
    configure_i2c_lines_input_pullup();
    sleep_ms(I2C_LINE_PROBE_SETTLE_MS);
    let start = read_pin_level(pin);
    let peer_start = read_pin_level(peer_pin);

    let (pullup_reset, pullup_dir, pullup_rc, pullup_pd_rc) =
        configure_single_input_pull(pin, true, false);
    sleep_ms(I2C_LINE_PROBE_SETTLE_MS);
    let input_pullup = read_pin_level(pin);
    let peer_after_pullup = read_pin_level(peer_pin);

    let (pulldown_reset, pulldown_dir, pulldown_pu_rc, pulldown_rc) =
        configure_single_input_pull(pin, false, true);
    sleep_ms(I2C_LINE_PROBE_SETTLE_MS);
    let input_pulldown = read_pin_level(pin);

    let od_rc = configure_single_open_drain_release(pin);
    sleep_ms(I2C_LINE_PROBE_SETTLE_MS);
    let od_release = read_pin_level(pin);
    let peer_after_od = read_pin_level(peer_pin);

    let (_, high_drive_rc, high_dir_rc, high_level_rc) = configure_single_weak_push_pull(pin, 1);
    sleep_ms(I2C_LINE_PROBE_SETTLE_MS);
    let weak_high = read_pin_level(pin);
    let peer_after_high = read_pin_level(peer_pin);

    let (_, low_drive_rc, low_dir_rc, low_level_rc) = configure_single_weak_push_pull(pin, 0);
    sleep_ms(I2C_LINE_PROBE_LOW_MS);
    let weak_low = read_pin_level(pin);

    let (final_reset, final_dir, final_pullup_rc, final_pulldown_rc) =
        configure_single_input_pull(pin, true, false);
    sleep_ms(I2C_LINE_PROBE_SETTLE_MS);
    let final_pullup = read_pin_level(pin);
    let peer_final = read_pin_level(peer_pin);

    let diagnosis = if input_pullup == 0 && od_release == 0 && weak_high == 0 {
        "external_or_strong_low"
    } else if input_pullup == 0 && weak_high == 1 {
        "weak_pullup_or_capacitance"
    } else if input_pullup == 1 && input_pulldown == 0 {
        "line_released"
    } else {
        "mixed"
    };
    log(&format!(
        "{stage}: line probe {label} pin={pin} diagnosis={diagnosis} start={start} {peer_label}_start={peer_start} input_pullup={input_pullup} peer_after_pullup={peer_after_pullup} input_pulldown={input_pulldown} od_release={od_release} peer_after_od={peer_after_od} weak_high={weak_high} peer_after_high={peer_after_high} weak_low={weak_low} final_pullup={final_pullup} peer_final={peer_final} rc_pullup={pullup_reset}/{pullup_dir}/{pullup_rc}/{pullup_pd_rc} rc_pulldown={pulldown_reset}/{pulldown_dir}/{pulldown_pu_rc}/{pulldown_rc} rc_od={od_rc} rc_high={high_drive_rc}/{high_dir_rc}/{high_level_rc} rc_low={low_drive_rc}/{low_dir_rc}/{low_level_rc} rc_final={final_reset}/{final_dir}/{final_pullup_rc}/{final_pulldown_rc}"
    ));
}

#[cfg(target_os = "espidf")]
fn run_i2c_stuck_line_probe_once(stage: &str) {
    if I2C_LINE_PROBE_RAN.load(Ordering::SeqCst) {
        return;
    }
    configure_i2c_lines_input_pullup();
    sleep_ms(I2C_LINE_PROBE_SETTLE_MS);
    let (scl, sda) = read_i2c_line_levels();
    if scl != 0 && sda != 0 {
        return;
    }
    if I2C_LINE_PROBE_RAN.swap(true, Ordering::SeqCst) {
        return;
    }
    log(&format!(
        "{stage}: i2c stuck-line probe begin scl={scl} sda={sda} irq={} pwr_btn={}",
        read_axp_irq_level(),
        read_pwr_button_level()
    ));
    probe_i2c_line(stage, "scl", I2C_SCL_PIN, "sda", I2C_SDA_PIN);
    probe_i2c_line(stage, "sda", I2C_SDA_PIN, "scl", I2C_SCL_PIN);
    configure_i2c_lines_input_pullup();
    sleep_ms(I2C_LINE_PROBE_SETTLE_MS);
    let (final_scl, final_sda) = read_i2c_line_levels();
    log(&format!(
        "{stage}: i2c stuck-line probe end scl={final_scl} sda={final_sda} irq={} pwr_btn={}",
        read_axp_irq_level(),
        read_pwr_button_level()
    ));
}

#[cfg(target_os = "espidf")]
fn force_release_i2c_bus_lines(stage: &str) -> bool {
    configure_i2c_lines_input_pullup();
    sleep_ms(I2C_LINE_PROBE_SETTLE_MS);
    let (start_scl, start_sda) = read_i2c_line_levels();
    log(&format!(
        "{stage}: forced bus release start scl={start_scl} sda={start_sda} irq={}",
        read_axp_irq_level()
    ));

    unsafe {
        for pin in [I2C_SCL_PIN, I2C_SDA_PIN] {
            let _ = sys::gpio_sleep_sel_dis(pin);
            let _ = sys::gpio_hold_dis(pin);
            let _ = sys::gpio_reset_pin(pin);
            let _ = sys::gpio_set_drive_capability(pin, sys::gpio_drive_cap_t_GPIO_DRIVE_CAP_0);
            let _ = sys::gpio_pulldown_dis(pin);
        }
        let _ = sys::gpio_pullup_en(I2C_SDA_PIN);
        let _ = sys::gpio_set_direction(I2C_SDA_PIN, sys::gpio_mode_t_GPIO_MODE_INPUT);
        let _ = sys::gpio_set_level(I2C_SCL_PIN, 1);
        let _ = sys::gpio_set_direction(I2C_SCL_PIN, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT);
        let _ = sys::gpio_set_level(I2C_SCL_PIN, 1);
    }
    sleep_ms(I2C_FORCED_RECOVERY_HIGH_MS);

    for pulse in 0..I2C_FORCED_RECOVERY_PULSES {
        unsafe {
            let _ = sys::gpio_set_level(I2C_SCL_PIN, 0);
        }
        sleep_ms(I2C_FORCED_RECOVERY_LOW_MS);
        unsafe {
            let _ = sys::gpio_set_level(I2C_SCL_PIN, 1);
        }
        sleep_ms(I2C_FORCED_RECOVERY_HIGH_MS);
        if pulse == 0 || pulse + 1 == 8 || pulse + 1 == I2C_FORCED_RECOVERY_PULSES {
            let (scl, sda) = read_i2c_line_levels();
            log(&format!(
                "{stage}: forced pulse {}/{} scl={} sda={} irq={}",
                pulse + 1,
                I2C_FORCED_RECOVERY_PULSES,
                scl,
                sda,
                read_axp_irq_level()
            ));
        }
    }

    // Generate a STOP condition with the weakest GPIO drive. This is a last-resort
    // recovery path for a bus that remains low even before the I2C peripheral owns it.
    unsafe {
        let _ = sys::gpio_set_level(I2C_SDA_PIN, 0);
        let _ = sys::gpio_set_direction(I2C_SDA_PIN, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT);
    }
    sleep_ms(I2C_FORCED_RECOVERY_LOW_MS);
    unsafe {
        let _ = sys::gpio_set_level(I2C_SCL_PIN, 1);
    }
    sleep_ms(I2C_FORCED_RECOVERY_HIGH_MS);
    unsafe {
        let _ = sys::gpio_set_level(I2C_SDA_PIN, 1);
    }
    sleep_ms(I2C_FORCED_RECOVERY_HIGH_MS);

    configure_i2c_lines_input_pullup();
    let mut sampled = Vec::with_capacity(I2C_FORCED_RECOVERY_PROBE_DELAY_MS.len());
    for delay_ms in I2C_FORCED_RECOVERY_PROBE_DELAY_MS {
        if delay_ms > 0 {
            sleep_ms(delay_ms);
        }
        let (scl, sda) = read_i2c_line_levels();
        sampled.push((delay_ms, scl, sda));
    }

    let pmic_probe_ok = false;
    let chip_id = None;
    let reg26 = None;
    let reg80 = None;
    let reg90 = None;
    let reg91 = None;

    let final_sample =
        sampled
            .last()
            .copied()
            .unwrap_or((0, read_i2c_line_levels().0, read_i2c_line_levels().1));
    let sample_summary = sampled
        .iter()
        .map(|(delay_ms, scl, sda)| format!("t+{delay_ms}ms={scl}/{sda}"))
        .collect::<Vec<_>>()
        .join(" ");
    log(&format!(
        "{stage}: forced bus release end {} irq={} pmic_probe_ok={} chip_id={} reg26={} reg80={} reg90={} reg91={}",
        sample_summary,
        read_axp_irq_level(),
        i32::from(pmic_probe_ok),
        format_diag_opt_u8(chip_id),
        format_diag_opt_u8(reg26),
        format_diag_opt_u8(reg80),
        format_diag_opt_u8(reg90),
        format_diag_opt_u8(reg91),
    ));
    (final_sample.1 == 1 && final_sample.2 == 1) || pmic_probe_ok
}

#[cfg(target_os = "espidf")]
fn sample_i2c_line_levels() -> Option<(i32, i32)> {
    configure_i2c_lines_input_pullup();
    Some(read_i2c_line_levels())
}

#[cfg(target_os = "espidf")]
fn configure_i2c_lines_open_drain_pullup() -> bool {
    unsafe {
        let cfg = sys::gpio_config_t {
            pin_bit_mask: (1u64 << I2C_SCL_PIN) | (1u64 << I2C_SDA_PIN),
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let _ = sys::gpio_sleep_sel_dis(I2C_SCL_PIN);
        let _ = sys::gpio_sleep_sel_dis(I2C_SDA_PIN);
        let _ = sys::gpio_hold_dis(I2C_SCL_PIN);
        let _ = sys::gpio_hold_dis(I2C_SDA_PIN);
        if sys::gpio_config(&cfg) != 0 {
            return false;
        }
        let _ = sys::gpio_set_level(I2C_SDA_PIN, 1);
        let _ = sys::gpio_set_level(I2C_SCL_PIN, 1);
    }
    true
}

#[cfg(target_os = "espidf")]
fn configure_axp_irq_pin_input_pullup() {
    unsafe {
        let _ = sys::gpio_sleep_sel_dis(AXP_IRQ_PIN);
        let _ = sys::gpio_hold_dis(AXP_IRQ_PIN);
        let _ = sys::rtc_gpio_hold_dis(AXP_IRQ_PIN);
        let _ = sys::rtc_gpio_deinit(AXP_IRQ_PIN);
        let cfg = sys::gpio_config_t {
            pin_bit_mask: 1u64 << AXP_IRQ_PIN,
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let _ = sys::gpio_config(&cfg);
    }
}

#[cfg(target_os = "espidf")]
fn configure_axp_irq_pin_output_high(stage: &str) {
    unsafe {
        let _ = sys::gpio_sleep_sel_dis(AXP_IRQ_PIN);
        let _ = sys::gpio_hold_dis(AXP_IRQ_PIN);
        let _ = sys::rtc_gpio_hold_dis(AXP_IRQ_PIN);
        let _ = sys::rtc_gpio_deinit(AXP_IRQ_PIN);
        let _ = sys::gpio_reset_pin(AXP_IRQ_PIN);
        let cfg = sys::gpio_config_t {
            pin_bit_mask: 1u64 << AXP_IRQ_PIN,
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let cfg_rc = sys::gpio_config(&cfg);
        let set_rc = if cfg_rc == 0 {
            sys::gpio_set_level(AXP_IRQ_PIN, 1)
        } else {
            cfg_rc
        };
        sleep_ms(2);
        log(&format!(
            "{stage}: irq input_output high cfg={cfg_rc} set={set_rc} level={}",
            sys::gpio_get_level(AXP_IRQ_PIN)
        ));
    }
}

#[cfg(target_os = "espidf")]
fn probe_axp_irq_pin(stage: &str) {
    unsafe {
        let _ = sys::gpio_sleep_sel_dis(AXP_IRQ_PIN);
        let _ = sys::gpio_hold_dis(AXP_IRQ_PIN);
        let _ = sys::rtc_gpio_hold_dis(AXP_IRQ_PIN);
        let _ = sys::rtc_gpio_deinit(AXP_IRQ_PIN);
        let start = sys::gpio_get_level(AXP_IRQ_PIN);

        let _ = sys::gpio_reset_pin(AXP_IRQ_PIN);
        let _ = sys::gpio_set_direction(AXP_IRQ_PIN, sys::gpio_mode_t_GPIO_MODE_INPUT);
        let _ = sys::gpio_pullup_en(AXP_IRQ_PIN);
        let _ = sys::gpio_pulldown_dis(AXP_IRQ_PIN);
        sleep_ms(2);
        let input_pullup = sys::gpio_get_level(AXP_IRQ_PIN);

        let _ = sys::gpio_pullup_dis(AXP_IRQ_PIN);
        let _ = sys::gpio_pulldown_dis(AXP_IRQ_PIN);
        sleep_ms(2);
        let input_floating = sys::gpio_get_level(AXP_IRQ_PIN);

        let _ = sys::gpio_pulldown_en(AXP_IRQ_PIN);
        sleep_ms(2);
        let input_pulldown = sys::gpio_get_level(AXP_IRQ_PIN);

        let cfg = sys::gpio_config_t {
            pin_bit_mask: 1u64 << AXP_IRQ_PIN,
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let cfg_rc = sys::gpio_config(&cfg);
        let high_rc = sys::gpio_set_level(AXP_IRQ_PIN, 1);
        sleep_ms(2);
        let high_level = sys::gpio_get_level(AXP_IRQ_PIN);
        let low_rc = sys::gpio_set_level(AXP_IRQ_PIN, 0);
        sleep_ms(2);
        let low_level = sys::gpio_get_level(AXP_IRQ_PIN);
        let final_high_rc = sys::gpio_set_level(AXP_IRQ_PIN, 1);
        sleep_ms(2);
        let final_high_level = sys::gpio_get_level(AXP_IRQ_PIN);

        let diagnosis = if input_pullup == 0 && high_level == 0 && final_high_level == 0 {
            "external_or_strong_low"
        } else if high_level == 1 && low_level == 0 && final_high_level == 1 {
            "drive_ok"
        } else {
            "mixed"
        };
        log(&format!(
            "{stage}: irq probe diagnosis={diagnosis} start={start} input_pullup={input_pullup} input_floating={input_floating} input_pulldown={input_pulldown} input_output_high={high_level} input_output_low={low_level} final_high={final_high_level} rc_cfg={cfg_rc} rc_high={high_rc} rc_low={low_rc} rc_final_high={final_high_rc}"
        ));
    }
}

#[cfg(target_os = "espidf")]
fn run_axp_irq_probe_once(stage: &str) {
    if AXP_IRQ_PROBE_RAN.swap(true, Ordering::SeqCst) {
        return;
    }
    probe_axp_irq_pin(stage);
}

#[cfg(target_os = "espidf")]
fn configure_pwr_button_pin_input_pullup() {
    unsafe {
        let cfg = sys::gpio_config_t {
            pin_bit_mask: 1u64 << PWR_BUTTON_PIN,
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let _ = sys::gpio_config(&cfg);
    }
}

#[cfg(target_os = "espidf")]
fn configure_pwr_button_pin_input_floating() {
    unsafe {
        let cfg = sys::gpio_config_t {
            pin_bit_mask: 1u64 << PWR_BUTTON_PIN,
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_DISABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let _ = sys::gpio_config(&cfg);
    }
}

#[cfg(target_os = "espidf")]
fn read_axp_irq_level() -> i32 {
    unsafe { sys::gpio_get_level(AXP_IRQ_PIN) }
}

#[cfg(target_os = "espidf")]
fn read_pwr_button_level() -> i32 {
    unsafe { sys::gpio_get_level(PWR_BUTTON_PIN) }
}

#[cfg(target_os = "espidf")]
fn log_bus_levels(stage: &str) {
    let (scl, sda) = sample_i2c_line_levels().unwrap_or((-1, -1));
    let irq = read_axp_irq_level();
    let pwr = read_pwr_button_level();
    log(&format!(
        "{stage}: scl={scl} sda={sda} irq={irq} pwr_btn={pwr}"
    ));
}

#[cfg(target_os = "espidf")]
fn pulse_axp_irq_pin(stage: &str, low_ms: u64, high_ms: u64) {
    log_bus_levels_raw(&format!("{stage} raw before"));
    unsafe {
        let _ = sys::gpio_sleep_sel_dis(AXP_IRQ_PIN);
        let _ = sys::gpio_hold_dis(AXP_IRQ_PIN);
        let _ = sys::rtc_gpio_hold_dis(AXP_IRQ_PIN);
        let _ = sys::rtc_gpio_deinit(AXP_IRQ_PIN);
        let _ = sys::gpio_reset_pin(AXP_IRQ_PIN);
        let cfg = sys::gpio_config_t {
            pin_bit_mask: 1u64 << AXP_IRQ_PIN,
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let cfg_rc = sys::gpio_config(&cfg);
        if cfg_rc != 0 {
            log(&format!(
                "{stage}: gpio_config input_output failed err={cfg_rc}"
            ));
            return;
        }
        let high1_rc = sys::gpio_set_level(AXP_IRQ_PIN, 1);
        sleep_ms(2);
        let high1_level = sys::gpio_get_level(AXP_IRQ_PIN);
        let low_rc = sys::gpio_set_level(AXP_IRQ_PIN, 0);
        let low_level = sys::gpio_get_level(AXP_IRQ_PIN);
        log(&format!(
            "{stage}: irq pushpull prehigh_rc={high1_rc} prehigh_level={high1_level} low_rc={low_rc} low_level={low_level}"
        ));
    }
    sleep_ms(low_ms);
    unsafe {
        let high2_rc = sys::gpio_set_level(AXP_IRQ_PIN, 1);
        sleep_ms(2);
        let high2_level = sys::gpio_get_level(AXP_IRQ_PIN);
        log(&format!(
            "{stage}: irq pushpull high_rc={high2_rc} high_level={high2_level}"
        ));
    }
    sleep_ms(high_ms);
    // 保持 GPIO21 高输出，和官方 PhotoPainter / 刘霞版本一致；不要立即切回浮空输入。
    log(&format!(
        "{stage}: irq held output high level={}",
        read_axp_irq_level()
    ));
    log_bus_levels_raw(&format!("{stage} raw after"));
}

#[cfg(target_os = "espidf")]
fn hard_wake_axp_irq_pin() {
    pulse_axp_irq_pin(
        "axp hard wake",
        AXP_HARD_WAKE_LOW_MS,
        AXP_HARD_WAKE_SETTLE_MS,
    );
}

#[cfg(target_os = "espidf")]
fn recover_i2c_bus_by_bitbang() -> bool {
    if !configure_i2c_lines_open_drain_pullup() {
        return false;
    }
    sleep_ms(2);

    let (mut scl, mut sda) = read_i2c_line_levels();
    log(&format!(
        "i2c recover start: scl={scl} sda={sda} irq={}",
        read_axp_irq_level()
    ));
    if scl == 0 {
        run_axp_irq_probe_once("i2c recover irq");
        for attempt in 0..AXP_WAKE_MAX_ATTEMPTS {
            log(&format!(
                "i2c recover: scl low, pulse axp wake attempt={}/{}",
                attempt + 1,
                AXP_WAKE_MAX_ATTEMPTS
            ));
            pulse_axp_irq_pin("i2c recover pulse", 100, 200);
            sleep_ms(AXP_WAKE_SETTLE_MS);
            if !configure_i2c_lines_open_drain_pullup() {
                return false;
            }
            sleep_ms(2);
            (scl, sda) = read_i2c_line_levels();
            log(&format!(
                "i2c recover: after pulse attempt={}/{} scl={} sda={} irq={}",
                attempt + 1,
                AXP_WAKE_MAX_ATTEMPTS,
                scl,
                sda,
                read_axp_irq_level()
            ));
            if scl != 0 {
                break;
            }
        }
        if scl == 0 {
            log("i2c recover: try hard axp wake cycle");
            hard_wake_axp_irq_pin();
            if !configure_i2c_lines_open_drain_pullup() {
                return false;
            }
            sleep_ms(2);
            (scl, sda) = read_i2c_line_levels();
            log(&format!(
                "i2c recover: after hard wake scl={} sda={} irq={}",
                scl,
                sda,
                read_axp_irq_level()
            ));
        }
        if scl == 0 {
            log("i2c recover: try forced bus release while scl is stuck low");
            let forced = force_release_i2c_bus_lines("i2c recover forced");
            (scl, sda) = read_i2c_line_levels();
            log(&format!(
                "i2c recover: after forced bus release ok={} scl={} sda={} irq={}",
                i32::from(forced),
                scl,
                sda,
                read_axp_irq_level()
            ));
        }
        if scl == 0 {
            log("i2c recover abort: scl still stuck low");
            configure_i2c_lines_input_pullup();
            return false;
        }
    }

    for _ in 0..9 {
        if sda != 0 {
            break;
        }
        if !configure_i2c_lines_open_drain_pullup() {
            return false;
        }
        unsafe {
            let _ = sys::gpio_set_level(I2C_SCL_PIN, 0);
        }
        sleep_ms(1);
        unsafe {
            let _ = sys::gpio_set_level(I2C_SCL_PIN, 1);
        }
        sleep_ms(1);
        let (_scl_after_pulse, next_sda) = read_i2c_line_levels();
        sda = next_sda;
    }

    if !configure_i2c_lines_open_drain_pullup() {
        return false;
    }
    unsafe {
        let _ = sys::gpio_set_level(I2C_SDA_PIN, 0);
    }
    sleep_ms(1);
    unsafe {
        let _ = sys::gpio_set_level(I2C_SCL_PIN, 1);
    }
    sleep_ms(1);
    unsafe {
        let _ = sys::gpio_set_level(I2C_SDA_PIN, 1);
    }
    sleep_ms(2);

    configure_i2c_lines_input_pullup();
    let (scl, sda) = read_i2c_line_levels();
    log(&format!(
        "i2c recover done: scl={scl} sda={sda} irq={}",
        read_axp_irq_level()
    ));
    scl == 1 && sda == 1
}

#[cfg(target_os = "espidf")]
fn legacy_i2c_delete_driver() {
    unsafe {
        let _ = sys::i2c_driver_delete(I2C_PORT_LEGACY);
    }
    configure_i2c_lines_input_pullup();
}

#[cfg(target_os = "espidf")]
fn legacy_i2c_install_driver() -> Result<(), String> {
    legacy_i2c_delete_driver();
    configure_i2c_lines_input_pullup();

    let mut cfg = sys::i2c_config_t::default();
    cfg.mode = sys::i2c_mode_t_I2C_MODE_MASTER;
    cfg.sda_io_num = I2C_SDA_PIN;
    cfg.scl_io_num = I2C_SCL_PIN;
    cfg.sda_pullup_en = true;
    cfg.scl_pullup_en = true;
    cfg.__bindgen_anon_1 = sys::i2c_config_t__bindgen_ty_1 {
        master: sys::i2c_config_t__bindgen_ty_1__bindgen_ty_1 {
            clk_speed: I2C_FREQ_HZ,
        },
    };
    cfg.clk_flags = 0;

    let param_err = unsafe { sys::i2c_param_config(I2C_PORT_LEGACY, &cfg) };
    if param_err != 0 {
        return Err(format!(
            "legacy i2c_param_config failed: {} ({param_err})",
            err_name(param_err)
        ));
    }
    let install_err = unsafe { sys::i2c_driver_install(I2C_PORT_LEGACY, cfg.mode, 0, 0, 0) };
    if install_err != 0 {
        legacy_i2c_delete_driver();
        return Err(format!(
            "legacy i2c_driver_install failed: {} ({install_err})",
            err_name(install_err)
        ));
    }
    Ok(())
}

#[cfg(target_os = "espidf")]
fn legacy_read_reg(reg: u8) -> Result<u8, String> {
    let mut value = 0u8;
    let err = unsafe {
        sys::i2c_master_write_read_device(
            I2C_PORT_LEGACY,
            AXP2101_ADDR as u8,
            &reg,
            1,
            &mut value,
            1,
            I2C_LEGACY_TIMEOUT_TICKS,
        )
    };
    if err == 0 {
        Ok(value)
    } else {
        Err(format!(
            "legacy read reg 0x{reg:02x} failed: {} ({err})",
            err_name(err)
        ))
    }
}

#[cfg(target_os = "espidf")]
fn legacy_write_reg(reg: u8, value: u8) -> Result<(), String> {
    let payload = [reg, value];
    let err = unsafe {
        sys::i2c_master_write_to_device(
            I2C_PORT_LEGACY,
            AXP2101_ADDR as u8,
            payload.as_ptr(),
            payload.len(),
            I2C_LEGACY_TIMEOUT_TICKS,
        )
    };
    if err == 0 {
        Ok(())
    } else {
        Err(format!(
            "legacy write reg 0x{reg:02x} failed: {} ({err})",
            err_name(err)
        ))
    }
}

#[cfg(target_os = "espidf")]
fn legacy_update_reg_bits(reg: u8, mask: u8, value: u8) -> Result<(), String> {
    let current = legacy_read_reg(reg)?;
    let next = (current & !mask) | (value & mask);
    if next == current {
        return Ok(());
    }
    legacy_write_reg(reg, next)
}

#[cfg(target_os = "espidf")]
fn legacy_enable_reg_bits(reg: u8, bits: u8) -> Result<(), String> {
    let current = legacy_read_reg(reg)?;
    let next = current | bits;
    if next == current {
        return Ok(());
    }
    legacy_write_reg(reg, next)
}

#[cfg(target_os = "espidf")]
fn legacy_disable_reg_bits(reg: u8, bits: u8) -> Result<(), String> {
    let current = legacy_read_reg(reg)?;
    let next = current & !bits;
    if next == current {
        return Ok(());
    }
    legacy_write_reg(reg, next)
}

#[cfg(target_os = "espidf")]
fn legacy_clear_pmic_irq_state() -> Result<(), String> {
    legacy_write_reg(REG_INT_EN1, 0)?;
    legacy_write_reg(REG_INT_EN2, 0)?;
    legacy_write_reg(REG_INT_EN3, 0)?;
    legacy_write_reg(REG_INT_STS1, 0xFF)?;
    legacy_write_reg(REG_INT_STS2, 0xFF)?;
    legacy_write_reg(REG_INT_STS3, 0xFF)?;
    Ok(())
}

#[cfg(target_os = "espidf")]
fn legacy_configure_sleep_wakeup_control() -> Result<(), String> {
    let current = legacy_read_reg(REG_SLEEP_WAKEUP_CTRL)?;
    let mut next = current | SLEEP_CTRL_DC_DLO_SELECT | SLEEP_CTRL_IRQ_PIN_TO_LOW;
    next &= !SLEEP_CTRL_WAKEUP_ENABLE;
    next &= !SLEEP_CTRL_PWROK_TO_LOW;
    next |= SLEEP_CTRL_SLEEP_ENABLE;
    if next == current {
        return Ok(());
    }
    legacy_write_reg(REG_SLEEP_WAKEUP_CTRL, next)
}

#[cfg(target_os = "espidf")]
fn legacy_prime_display_power_rails() -> Result<(u8, u8, u8, u8, u8), String> {
    let chip_id = legacy_read_reg(REG_CHIP_ID)?;
    if chip_id != EXPECTED_CHIP_ID {
        return Err(format!(
            "legacy chip id mismatch: got 0x{chip_id:02x}, expect 0x{EXPECTED_CHIP_ID:02x}"
        ));
    }

    legacy_clear_pmic_irq_state()?;
    legacy_update_reg_bits(REG_EXTEN_CFG, 0x07, 0x05)?;
    legacy_update_reg_bits(REG_DCDC1_VOLT_CTRL, 0x3F, 0x12)?;
    for reg in [
        REG_LDO_VOL0_CTRL,
        REG_LDO_VOL1_CTRL,
        REG_LDO_VOL2_CTRL,
        REG_LDO_VOL3_CTRL,
    ] {
        legacy_update_reg_bits(reg, 0x1F, LDO_CODE_3300)?;
    }
    legacy_enable_reg_bits(REG_DCDC_ON_OFF_CTRL, DCDC_ENABLE_DCDC1)?;
    legacy_enable_reg_bits(REG_LDO_ON_OFF_CTRL0, LDO_ENABLE_ALL)?;
    legacy_disable_reg_bits(REG_LDO_ON_OFF_CTRL0, UNUSED_LDO_CTRL0_MASK)?;
    legacy_disable_reg_bits(REG_LDO_ON_OFF_CTRL1, UNUSED_LDO_CTRL1_MASK)?;
    legacy_enable_reg_bits(REG_ADC_CHANNEL_CTRL, 0x01)?;
    legacy_enable_reg_bits(REG_BATT_DET_CTRL, 0x01)?;

    Ok((
        chip_id,
        legacy_read_reg(REG_DCDC_ON_OFF_CTRL)?,
        legacy_read_reg(REG_LDO_ON_OFF_CTRL0)?,
        legacy_read_reg(REG_LDO_ON_OFF_CTRL1)?,
        legacy_read_reg(REG_SLEEP_WAKEUP_CTRL)?,
    ))
}

#[cfg(target_os = "espidf")]
fn reset_i2c_bus(state: &mut PowerRuntime) {
    state.dev = ptr::null_mut();
    state.bus = ptr::null_mut();
    legacy_i2c_delete_driver();
    state.ready = false;
}

#[cfg(target_os = "espidf")]
pub fn ensure_ready_for_render() -> bool {
    for attempt in 0..POWER_INIT_MAX_ATTEMPTS {
        match init_power() {
            Ok(()) => {
                if attempt > 0 {
                    log(&format!(
                        "render power init recovered attempt={}/{}",
                        attempt + 1,
                        POWER_INIT_MAX_ATTEMPTS
                    ));
                }
                return true;
            }
            Err(err) => {
                log(&format!(
                    "render power init failed attempt={}/{} reason={err}",
                    attempt + 1,
                    POWER_INIT_MAX_ATTEMPTS
                ));
                if attempt + 1 >= POWER_INIT_MAX_ATTEMPTS {
                    log_bus_levels_raw("render power init terminal failure raw");
                    run_i2c_stuck_line_probe_once("render power init terminal failure");
                    return false;
                }
                reset_power_runtime();
                sleep_ms(POWER_INIT_RETRY_DELAY_MS);
            }
        }
    }
    false
}

#[cfg(target_os = "espidf")]
pub fn prepare_for_boot() {
    I2C_LINE_PROBE_RAN.store(false, Ordering::SeqCst);
    AXP_IRQ_PROBE_RAN.store(false, Ordering::SeqCst);
    log_sleep_diag_snapshot();
    // 参考官方与刘霞版本：启动早期先解除旧 pad/hold 状态，再用 GPIO21 输出低/高踢醒 PMIC。
    release_power_diag_pins_for_boot("boot release");
    run_axp_irq_probe_once("boot irq");
    pulse_axp_irq_pin("boot wake", 100, 200);
    sleep_ms(AXP_WAKE_SETTLE_MS);
    configure_i2c_lines_input_pullup();
    log_bus_levels_raw("boot wake settled raw");
    BOOT_POWER_PREPARED.store(true, Ordering::SeqCst);
}

#[cfg(target_os = "espidf")]
pub fn ensure_ready_for_sdcard() -> bool {
    ensure_ready_for_render()
}

#[cfg(target_os = "espidf")]
pub fn i2c_bus_handle() -> Result<sys::i2c_master_bus_handle_t, String> {
    Err("power i2c bus unavailable: PMIC is running on legacy i2c only".into())
}

#[cfg(target_os = "espidf")]
pub fn recover_sdcard_power() -> bool {
    if !ensure_ready_for_render() {
        return false;
    }

    let mutex = runtime();
    let state = mutex.lock().unwrap();
    if !state.ready {
        return false;
    }
    drop(state);

    let disable_ok =
        legacy_disable_reg_bits(REG_LDO_ON_OFF_CTRL0, LDO_ENABLE_ALDO3 | LDO_ENABLE_ALDO4).is_ok();
    sleep_ms(SDCARD_POWER_CYCLE_OFF_MS);

    let enable_ok =
        legacy_enable_reg_bits(REG_LDO_ON_OFF_CTRL0, LDO_ENABLE_ALDO3 | LDO_ENABLE_ALDO4).is_ok();
    sleep_ms(SDCARD_POWER_CYCLE_ON_MS);

    let ok = disable_ok && enable_ok;
    if ok {
        log("sdcard rails power-cycled for mount retry");
    } else {
        log(&format!(
            "sdcard rail power-cycle failed disable_ok={} enable_ok={}",
            i32::from(disable_ok),
            i32::from(enable_ok)
        ));
    }
    ok
}

#[cfg(target_os = "espidf")]
fn reset_power_runtime() {
    let mutex = runtime();
    let mut state = mutex.lock().unwrap();
    reset_i2c_bus(&mut state);
}

#[cfg(target_os = "espidf")]
fn init_power() -> Result<(), String> {
    let mutex = runtime();
    let mut state = mutex.lock().unwrap();
    if state.ready {
        return Ok(());
    }

    if !BOOT_POWER_PREPARED.load(Ordering::SeqCst) {
        log("init_power: boot wake state missing, run fallback bootstrap");
        release_power_diag_pins_for_boot("init_power bootstrap");
        pulse_axp_irq_pin("init_power bootstrap wake", 100, 200);
        sleep_ms(AXP_WAKE_SETTLE_MS);
        configure_i2c_lines_input_pullup();
        log_bus_levels_raw("init_power bootstrap settled raw");
        BOOT_POWER_PREPARED.store(true, Ordering::SeqCst);
    }
    log_bus_levels_raw("init_power before recover raw");
    let recovered = recover_i2c_bus_by_bitbang();
    let (scl_after_recover, sda_after_recover) = sample_i2c_line_levels().unwrap_or((-1, -1));
    if !recovered {
        log(&format!(
            "init_power: gpio recover incomplete, continue with legacy i2c probe scl={scl_after_recover} sda={sda_after_recover} irq={}",
            read_axp_irq_level()
        ));
    } else {
        log(&format!(
            "init_power: gpio recover sampled scl={scl_after_recover} sda={sda_after_recover} irq={}",
            read_axp_irq_level()
        ));
    }

    legacy_i2c_install_driver()?;
    let (chip_id, dcdc_on, ldo_on0, ldo_on1, sleep_ctrl) = match legacy_prime_display_power_rails()
    {
        Ok(snapshot) => snapshot,
        Err(err) => {
            legacy_i2c_delete_driver();
            let (scl, sda) = sample_i2c_line_levels().unwrap_or((-1, -1));
            return Err(format!(
                "legacy pmic init failed scl={scl} sda={sda} irq={} reason={err}",
                read_axp_irq_level()
            ));
        }
    };
    if chip_id != EXPECTED_CHIP_ID {
        log(&format!(
            "unexpected pmic chip id=0x{chip_id:02x} expect=0x{EXPECTED_CHIP_ID:02x}"
        ));
    }
    configure_axp_irq_pin_output_high("init_power irq hold");

    let dcdc1 = legacy_read_reg(REG_DCDC1_VOLT_CTRL).unwrap_or_default();
    log(&format!(
        "render rails ready: chip_id=0x{chip_id:02x} dcdc_on=0x{dcdc_on:02x} ldo_on0=0x{ldo_on0:02x} ldo_on1=0x{ldo_on1:02x} dcdc1=0x{dcdc1:02x} sleep_ctrl=0x{sleep_ctrl:02x}"
    ));
    sleep_ms(200);

    state.ready = true;
    Ok(())
}

#[cfg(target_os = "espidf")]
fn cached_power() -> Option<PowerCache> {
    unsafe {
        if RTC_POWER_CACHE.battery_mv > 0
            || RTC_POWER_CACHE.battery_percent >= 0
            || RTC_POWER_CACHE.charging >= 0
            || RTC_POWER_CACHE.vbus_good >= 0
        {
            Some(RTC_POWER_CACHE)
        } else {
            None
        }
    }
}

#[cfg(target_os = "espidf")]
fn store_cache(cache: PowerCache) {
    unsafe {
        RTC_POWER_CACHE = cache;
    }
}

#[cfg(target_os = "espidf")]
pub fn read_power_sample() -> Option<PowerSample> {
    if init_power().is_err() {
        if let Some(cache) = cached_power() {
            return Some(normalize_power_sample(PowerSample::default(), Some(cache)).sample);
        }
        return None;
    }

    let mutex = runtime();
    let state = mutex.lock().unwrap();
    if !state.ready {
        return None;
    }
    drop(state);

    let status1 = legacy_read_reg(REG_STATUS1).ok()?;
    let status2 = match legacy_read_reg(REG_STATUS2) {
        Ok(value) => value,
        Err(_) => {
            reset_power_runtime();
            if let Some(cache) = cached_power() {
                return Some(normalize_power_sample(PowerSample::default(), Some(cache)).sample);
            }
            return None;
        }
    };

    let mut sample = PowerSample {
        battery_mv: -1,
        battery_percent: -1,
        charging: if ((status2 >> 5) & 0x03) == 0x01 {
            1
        } else {
            0
        },
        vbus_good: if (status1 & (1u8 << 5)) != 0 { 1 } else { 0 },
    };

    if let Ok(percent) = legacy_read_reg(REG_BATTERY_PERCENT)
        && percent <= 100
    {
        sample.battery_percent = percent as i32;
    }
    if let (Ok(h), Ok(l)) = (
        legacy_read_reg(REG_ADC_BATT_H),
        legacy_read_reg(REG_ADC_BATT_L),
    ) {
        let mv = (((h & 0x1F) as i32) << 8) | l as i32;
        if mv > 0 {
            sample.battery_mv = mv;
        }
    }

    let cache = cached_power();
    let NormalizePowerOutcome { sample, cache, .. } = normalize_power_sample(sample, cache);
    store_cache(cache);
    Some(sample)
}

#[cfg(target_os = "espidf")]
pub fn prepare_for_sleep() {
    let pre_levels = read_bus_levels_raw();
    let power_ready;
    let mut reg26 = None;
    let mut reg80 = None;
    let mut reg90 = None;
    let mut reg91 = None;

    {
        let mutex = runtime();
        let mut state = mutex.lock().unwrap();
        let pmic_runtime_ready = state.ready;
        power_ready = pmic_runtime_ready;
        if pmic_runtime_ready {
            let sleep_ctrl_before = legacy_read_reg(REG_SLEEP_WAKEUP_CTRL).unwrap_or_default();
            let dcdc_on_before = legacy_read_reg(REG_DCDC_ON_OFF_CTRL).unwrap_or_default();
            let ldo_on0_before = legacy_read_reg(REG_LDO_ON_OFF_CTRL0).unwrap_or_default();
            let ldo_on1_before = legacy_read_reg(REG_LDO_ON_OFF_CTRL1).unwrap_or_default();
            log(&format!(
                "prepare sleep: reg26 before=0x{sleep_ctrl_before:02x} dcdc_on=0x{dcdc_on_before:02x} ldo_on0=0x{ldo_on0_before:02x} ldo_on1=0x{ldo_on1_before:02x}"
            ));
            if let Err(err) = legacy_clear_pmic_irq_state() {
                log(&format!(
                    "prepare sleep: clear pmic irq state failed: {err}"
                ));
            }
            // 与官方 PhotoPainter 低功耗示例保持同向配置：
            // 由 AXP2101 记录睡前电源状态、IRQ 低电平参与唤醒，且唤醒时不额外拉低 PWROK。
            if let Err(err) = legacy_configure_sleep_wakeup_control() {
                log(&format!("sleep wakeup control configure failed: {err}"));
            }
            // 共享在 PMIC I2C 上的外围在深睡后可能把总线一直拖低。
            // 这里按官方低功耗路径保留 DCDC1，其余 DCDC/LDO 在睡前统一断电。
            let _ = legacy_disable_reg_bits(REG_DCDC_ON_OFF_CTRL, SLEEP_DCDC_DISABLE_MASK);
            let _ = legacy_disable_reg_bits(REG_LDO_ON_OFF_CTRL0, SLEEP_LDO_DISABLE_MASK0);
            let _ = legacy_disable_reg_bits(REG_LDO_ON_OFF_CTRL1, SLEEP_LDO_DISABLE_MASK1);
            let _ = legacy_disable_reg_bits(REG_ADC_CHANNEL_CTRL, 0x01);
            let _ = legacy_disable_reg_bits(REG_BATT_DET_CTRL, 0x01);
            reg26 = legacy_read_reg(REG_SLEEP_WAKEUP_CTRL).ok();
            reg80 = legacy_read_reg(REG_DCDC_ON_OFF_CTRL).ok();
            reg90 = legacy_read_reg(REG_LDO_ON_OFF_CTRL0).ok();
            reg91 = legacy_read_reg(REG_LDO_ON_OFF_CTRL1).ok();
            log(&format!(
                "prepare sleep: reg26 after={} dcdc_on={} ldo_on0={} ldo_on1={}",
                format_diag_reg(reg26.map(i32::from).unwrap_or(-1)),
                format_diag_reg(reg80.map(i32::from).unwrap_or(-1)),
                format_diag_reg(reg90.map(i32::from).unwrap_or(-1)),
                format_diag_reg(reg91.map(i32::from).unwrap_or(-1)),
            ));
        } else {
            log("prepare sleep: pmic runtime unavailable, fallback to gpio-only release");
        }
        reset_i2c_bus(&mut state);
    }

    // 即使本轮 PMIC/I2C 初始化失败，也要把共享总线和唤醒脚整理成下一轮可恢复的状态。
    // GPIO21 按官方路径保持高电平，不再切成 open-drain；坏状态下也避免让它浮空成低。
    configure_i2c_lines_input_pullup();
    configure_axp_irq_pin_output_high("prepare sleep irq hold");
    configure_pwr_button_pin_input_floating();
    let final_levels = read_bus_levels_raw();
    store_sleep_diag_snapshot(
        power_ready,
        pre_levels,
        final_levels,
        reg26,
        reg80,
        reg90,
        reg91,
    );
    log_bus_levels_raw("prepare sleep final raw");
}

#[cfg(not(target_os = "espidf"))]
pub fn ensure_ready_for_render() -> bool {
    false
}

#[cfg(not(target_os = "espidf"))]
pub fn prepare_for_boot() {}

#[cfg(not(target_os = "espidf"))]
pub fn ensure_ready_for_sdcard() -> bool {
    false
}

#[cfg(not(target_os = "espidf"))]
pub fn i2c_bus_handle() -> Result<(), String> {
    Err("power i2c bus unavailable".into())
}

#[cfg(not(target_os = "espidf"))]
pub fn recover_sdcard_power() -> bool {
    false
}

#[cfg(not(target_os = "espidf"))]
pub fn read_power_sample() -> Option<()> {
    None
}

#[cfg(not(target_os = "espidf"))]
pub fn prepare_for_sleep() {}
