#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use std::{
    ptr,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

#[cfg(target_os = "espidf")]
use esp_idf_sys as sys;
#[cfg(target_os = "espidf")]
use photoframe_app::{NormalizePowerOutcome, PowerCache, PowerSample, normalize_power_sample};

#[cfg(target_os = "espidf")]
const I2C_PORT: sys::i2c_port_num_t = sys::i2c_port_t_I2C_NUM_0 as sys::i2c_port_num_t;
#[cfg(target_os = "espidf")]
const I2C_SCL_PIN: i32 = 48;
#[cfg(target_os = "espidf")]
const I2C_SDA_PIN: i32 = 47;
#[cfg(target_os = "espidf")]
const AXP_IRQ_PIN: i32 = sys::gpio_num_t_GPIO_NUM_21;
#[cfg(target_os = "espidf")]
const I2C_FREQ_HZ: u32 = 100_000;
#[cfg(target_os = "espidf")]
const I2C_TIMEOUT_MS: i32 = 1_000;
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
const REG_ADC_CHANNEL_CTRL: u8 = 0x30;
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
const LDO_CODE_3300: u8 = 0x1C;

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
#[unsafe(link_section = ".rtc.data")]
static mut RTC_POWER_CACHE: PowerCache = PowerCache {
    battery_mv: -1,
    battery_percent: -1,
    charging: -1,
    vbus_good: -1,
    cached_epoch: 0,
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
    println!("photoframe-rs/power: {message}");
}

#[cfg(target_os = "espidf")]
fn sample_i2c_line_levels() -> Option<(i32, i32)> {
    unsafe {
        let _ = sys::gpio_reset_pin(I2C_SCL_PIN);
        let _ = sys::gpio_reset_pin(I2C_SDA_PIN);
        let _ = sys::gpio_set_direction(I2C_SCL_PIN, sys::gpio_mode_t_GPIO_MODE_INPUT);
        let _ = sys::gpio_set_direction(I2C_SDA_PIN, sys::gpio_mode_t_GPIO_MODE_INPUT);
        let _ = sys::gpio_pullup_en(I2C_SCL_PIN);
        let _ = sys::gpio_pullup_en(I2C_SDA_PIN);
        Some((
            sys::gpio_get_level(I2C_SCL_PIN),
            sys::gpio_get_level(I2C_SDA_PIN),
        ))
    }
}

#[cfg(target_os = "espidf")]
fn pulse_axp_irq_pin() {
    unsafe {
        let cfg = sys::gpio_config_t {
            pin_bit_mask: 1u64 << AXP_IRQ_PIN,
            mode: sys::gpio_mode_t_GPIO_MODE_OUTPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        if sys::gpio_config(&cfg) != 0 {
            return;
        }
        let _ = sys::gpio_set_level(AXP_IRQ_PIN, 0);
    }
    sleep_ms(100);
    unsafe {
        let _ = sys::gpio_set_level(AXP_IRQ_PIN, 1);
    }
    sleep_ms(200);
}

#[cfg(target_os = "espidf")]
fn recover_i2c_bus_by_bitbang() -> bool {
    unsafe {
        let cfg = sys::gpio_config_t {
            pin_bit_mask: (1u64 << I2C_SCL_PIN) | (1u64 << I2C_SDA_PIN),
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let _ = sys::gpio_reset_pin(I2C_SCL_PIN);
        let _ = sys::gpio_reset_pin(I2C_SDA_PIN);
        if sys::gpio_config(&cfg) != 0 {
            return false;
        }
        let _ = sys::gpio_set_level(I2C_SDA_PIN, 1);
        let _ = sys::gpio_set_level(I2C_SCL_PIN, 1);
    }
    sleep_ms(2);

    let (mut scl, mut sda) = sample_i2c_line_levels().unwrap_or((-1, -1));
    if scl == 0 {
        pulse_axp_irq_pin();
        (scl, sda) = sample_i2c_line_levels().unwrap_or((-1, -1));
        if scl == 0 {
            log("i2c recover abort: scl still stuck low");
            return false;
        }
    }

    for _ in 0..9 {
        if sda != 0 {
            break;
        }
        unsafe {
            let _ = sys::gpio_set_level(I2C_SCL_PIN, 0);
        }
        sleep_ms(1);
        unsafe {
            let _ = sys::gpio_set_level(I2C_SCL_PIN, 1);
        }
        sleep_ms(1);
        (_, sda) = sample_i2c_line_levels().unwrap_or((-1, -1));
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

    let (scl, sda) = sample_i2c_line_levels().unwrap_or((-1, -1));
    log(&format!("i2c recover done: scl={scl} sda={sda}"));
    scl == 1 && sda == 1
}

#[cfg(target_os = "espidf")]
fn wait_bus_idle(state: &PowerRuntime) {
    if !state.bus.is_null() {
        unsafe {
            let _ = sys::i2c_master_bus_wait_all_done(state.bus, 1000);
        }
    }
}

#[cfg(target_os = "espidf")]
fn reset_i2c_bus(state: &mut PowerRuntime) {
    unsafe {
        if !state.dev.is_null() {
            let _ = sys::i2c_master_bus_rm_device(state.dev);
            state.dev = ptr::null_mut();
        }
        if !state.bus.is_null() {
            let _ = sys::i2c_del_master_bus(state.bus);
            state.bus = ptr::null_mut();
        }
    }
    state.ready = false;
}

#[cfg(target_os = "espidf")]
fn read_reg(state: &mut PowerRuntime, reg: u8) -> Option<u8> {
    if state.dev.is_null() {
        return None;
    }
    wait_bus_idle(state);

    let mut value = 0u8;
    for attempt in 0..3 {
        let err = unsafe {
            sys::i2c_master_transmit_receive(state.dev, &reg, 1, &mut value, 1, I2C_TIMEOUT_MS)
        };
        if err == 0 {
            return Some(value);
        }
        if !state.bus.is_null() {
            unsafe {
                let _ = sys::i2c_master_bus_reset(state.bus);
            }
        }
        if attempt < 2 {
            sleep_ms(20);
        }
    }
    None
}

#[cfg(target_os = "espidf")]
fn write_reg(state: &mut PowerRuntime, reg: u8, value: u8) -> bool {
    if state.dev.is_null() {
        return false;
    }
    wait_bus_idle(state);

    let payload = [reg, value];
    for attempt in 0..3 {
        let err = unsafe {
            sys::i2c_master_transmit(state.dev, payload.as_ptr(), payload.len(), I2C_TIMEOUT_MS)
        };
        if err == 0 {
            return true;
        }
        if !state.bus.is_null() {
            unsafe {
                let _ = sys::i2c_master_bus_reset(state.bus);
            }
        }
        if attempt < 2 {
            sleep_ms(20);
        }
    }
    false
}

#[cfg(target_os = "espidf")]
fn update_reg_bits(state: &mut PowerRuntime, reg: u8, mask: u8, value: u8) -> bool {
    let current = match read_reg(state, reg) {
        Some(current) => current,
        None => return false,
    };
    let next = (current & !mask) | (value & mask);
    if next == current {
        return true;
    }
    write_reg(state, reg, next)
}

#[cfg(target_os = "espidf")]
fn enable_reg_bits(state: &mut PowerRuntime, reg: u8, bits: u8) -> bool {
    let current = match read_reg(state, reg) {
        Some(current) => current,
        None => return false,
    };
    let next = current | bits;
    if next == current {
        return true;
    }
    write_reg(state, reg, next)
}

#[cfg(target_os = "espidf")]
fn disable_reg_bits(state: &mut PowerRuntime, reg: u8, bits: u8) -> bool {
    let current = match read_reg(state, reg) {
        Some(current) => current,
        None => return false,
    };
    let next = current & !bits;
    if next == current {
        return true;
    }
    write_reg(state, reg, next)
}

#[cfg(target_os = "espidf")]
fn configure_display_power_rails(state: &mut PowerRuntime) -> bool {
    let ext_en_ok = update_reg_bits(state, REG_EXTEN_CFG, 0x07, 0x05);
    let dcdc1_ok = update_reg_bits(state, REG_DCDC1_VOLT_CTRL, 0x3F, 0x12);
    let ldo_ok = [
        REG_LDO_VOL0_CTRL,
        REG_LDO_VOL1_CTRL,
        REG_LDO_VOL2_CTRL,
        REG_LDO_VOL3_CTRL,
    ]
    .into_iter()
    .all(|reg| update_reg_bits(state, reg, 0x1F, LDO_CODE_3300));
    let rail_enable_ok = enable_reg_bits(state, REG_DCDC_ON_OFF_CTRL, 1u8 << 0)
        && enable_reg_bits(state, REG_LDO_ON_OFF_CTRL0, 0x0F);
    ext_en_ok && dcdc1_ok && ldo_ok && rail_enable_ok
}

#[cfg(target_os = "espidf")]
pub fn ensure_ready_for_render() -> bool {
    init_power().is_ok()
}

#[cfg(target_os = "espidf")]
fn init_power() -> Result<(), String> {
    let mutex = runtime();
    let mut state = mutex.lock().unwrap();
    if state.ready {
        return Ok(());
    }

    if state.bus.is_null() {
        let _ = sample_i2c_line_levels();
        let recovered = recover_i2c_bus_by_bitbang();
        if !recovered {
            if let Some((scl, _)) = sample_i2c_line_levels() {
                if scl == 0 {
                    return Err("i2c scl still low after recover".into());
                }
            }
        }

        let mut bus_cfg = sys::i2c_master_bus_config_t::default();
        bus_cfg.i2c_port = I2C_PORT;
        bus_cfg.scl_io_num = I2C_SCL_PIN;
        bus_cfg.sda_io_num = I2C_SDA_PIN;
        bus_cfg.__bindgen_anon_1 = sys::i2c_master_bus_config_t__bindgen_ty_1 {
            clk_source: sys::soc_periph_i2c_clk_src_t_I2C_CLK_SRC_DEFAULT,
        };
        bus_cfg.glitch_ignore_cnt = 7;
        bus_cfg.flags.set_enable_internal_pullup(1);
        let err = unsafe { sys::i2c_new_master_bus(&bus_cfg, &mut state.bus) };
        if err != 0 {
            return Err(format!("i2c_new_master_bus failed: {err}"));
        }
    }

    if state.dev.is_null() {
        let mut dev_cfg = sys::i2c_device_config_t::default();
        dev_cfg.dev_addr_length = sys::i2c_addr_bit_len_t_I2C_ADDR_BIT_LEN_7;
        dev_cfg.device_address = AXP2101_ADDR;
        dev_cfg.scl_speed_hz = I2C_FREQ_HZ;
        let err = unsafe { sys::i2c_master_bus_add_device(state.bus, &dev_cfg, &mut state.dev) };
        if err != 0 {
            reset_i2c_bus(&mut state);
            return Err(format!("i2c_master_bus_add_device failed: {err}"));
        }
    }

    if !state.bus.is_null() {
        unsafe {
            let _ = sys::i2c_master_bus_reset(state.bus);
        }
    }

    let mut chip_id = None;
    for attempt in 0..5 {
        chip_id = read_reg(&mut state, REG_CHIP_ID);
        if chip_id.is_some() {
            break;
        }
        if attempt < 4 {
            sleep_ms(50);
        }
    }
    let chip_id = match chip_id {
        Some(chip_id) => chip_id,
        None => {
            reset_i2c_bus(&mut state);
            return Err("read chip id failed".into());
        }
    };
    if chip_id != EXPECTED_CHIP_ID {
        log(&format!(
            "unexpected pmic chip id=0x{chip_id:02x} expect=0x{EXPECTED_CHIP_ID:02x}"
        ));
    }

    let ok = configure_display_power_rails(&mut state)
        && enable_reg_bits(&mut state, REG_ADC_CHANNEL_CTRL, 0x01)
        && enable_reg_bits(&mut state, REG_BATT_DET_CTRL, 0x01);
    if !ok {
        reset_i2c_bus(&mut state);
        return Err("pmic register init failed".into());
    }

    let dcdc_on = read_reg(&mut state, REG_DCDC_ON_OFF_CTRL).unwrap_or_default();
    let ldo_on = read_reg(&mut state, REG_LDO_ON_OFF_CTRL0).unwrap_or_default();
    let dcdc1 = read_reg(&mut state, REG_DCDC1_VOLT_CTRL).unwrap_or_default();
    log(&format!(
        "render rails ready: dcdc_on=0x{dcdc_on:02x} ldo_on=0x{ldo_on:02x} dcdc1=0x{dcdc1:02x}"
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
    let mut state = mutex.lock().unwrap();
    if !state.ready {
        return None;
    }

    let status1 = read_reg(&mut state, REG_STATUS1)?;
    let status2 = match read_reg(&mut state, REG_STATUS2) {
        Some(value) => value,
        None => {
            reset_i2c_bus(&mut state);
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

    if let Some(percent) = read_reg(&mut state, REG_BATTERY_PERCENT)
        && percent <= 100
    {
        sample.battery_percent = percent as i32;
    }
    if let (Some(h), Some(l)) = (
        read_reg(&mut state, REG_ADC_BATT_H),
        read_reg(&mut state, REG_ADC_BATT_L),
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
    let mutex = runtime();
    let mut state = mutex.lock().unwrap();
    if !state.ready {
        return;
    }
    let _ = disable_reg_bits(&mut state, REG_ADC_CHANNEL_CTRL, 0x01);
    let _ = disable_reg_bits(&mut state, REG_BATT_DET_CTRL, 0x01);
}

#[cfg(not(target_os = "espidf"))]
pub fn ensure_ready_for_render() -> bool {
    false
}

#[cfg(not(target_os = "espidf"))]
pub fn read_power_sample() -> Option<()> {
    None
}

#[cfg(not(target_os = "espidf"))]
pub fn prepare_for_sleep() {}
