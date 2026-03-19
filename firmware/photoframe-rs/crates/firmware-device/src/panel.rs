#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use std::{
    ptr,
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "espidf")]
use esp_idf_sys as sys;

#[cfg(target_os = "espidf")]
const PANEL_WIDTH: usize = 800;
#[cfg(target_os = "espidf")]
const PANEL_HEIGHT: usize = 480;
#[cfg(target_os = "espidf")]
const DISPLAY_LEN: usize = (PANEL_WIDTH * PANEL_HEIGHT) / 2;
#[cfg(target_os = "espidf")]
const PIN_MOSI: i32 = 11;
#[cfg(target_os = "espidf")]
const PIN_CLK: i32 = 10;
#[cfg(target_os = "espidf")]
const PIN_DC: i32 = 8;
#[cfg(target_os = "espidf")]
const PIN_CS: i32 = 9;
#[cfg(target_os = "espidf")]
const PIN_RST: i32 = 12;
#[cfg(target_os = "espidf")]
const PIN_BUSY: i32 = 13;
#[cfg(target_os = "espidf")]
#[cfg(target_os = "espidf")]
const FLUSH_MAX_RETRIES: usize = 3;
#[cfg(target_os = "espidf")]
const FLUSH_RETRY_DELAY_MS: u64 = 500;
#[cfg(target_os = "espidf")]
const DEBUG_PANEL_WRITE_CHUNK_BYTES: usize = 256;
#[cfg(target_os = "espidf")]
const DEBUG_PANEL_WRITE_CHUNK_DELAY_MS: u64 = 1;
#[cfg(target_os = "espidf")]
#[cfg(target_os = "espidf")]
struct PanelRuntime {
    spi_handle: sys::spi_device_handle_t,
    initialized: bool,
}

#[cfg(target_os = "espidf")]
unsafe impl Send for PanelRuntime {}

#[cfg(target_os = "espidf")]
impl Default for PanelRuntime {
    fn default() -> Self {
        Self {
            spi_handle: ptr::null_mut(),
            initialized: false,
        }
    }
}

#[cfg(target_os = "espidf")]
static PANEL_RUNTIME: OnceLock<Mutex<PanelRuntime>> = OnceLock::new();

#[cfg(target_os = "espidf")]
fn runtime() -> &'static Mutex<PanelRuntime> {
    PANEL_RUNTIME.get_or_init(|| Mutex::new(PanelRuntime::default()))
}

#[cfg(target_os = "espidf")]
fn sleep_ms(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}

#[cfg(target_os = "espidf")]
fn init_bus(state: &mut PanelRuntime) -> Result<(), String> {
    let mut bus_cfg = sys::spi_bus_config_t::default();
    bus_cfg.__bindgen_anon_1 = sys::spi_bus_config_t__bindgen_ty_1 {
        mosi_io_num: PIN_MOSI,
    };
    bus_cfg.__bindgen_anon_2 = sys::spi_bus_config_t__bindgen_ty_2 { miso_io_num: -1 };
    bus_cfg.sclk_io_num = PIN_CLK;
    bus_cfg.__bindgen_anon_3 = sys::spi_bus_config_t__bindgen_ty_3 { quadwp_io_num: -1 };
    bus_cfg.__bindgen_anon_4 = sys::spi_bus_config_t__bindgen_ty_4 { quadhd_io_num: -1 };
    bus_cfg.max_transfer_sz = DISPLAY_LEN as i32;

    let err = unsafe {
        sys::spi_bus_initialize(
            sys::spi_host_device_t_SPI3_HOST,
            &bus_cfg,
            sys::spi_common_dma_t_SPI_DMA_CH_AUTO,
        )
    };
    if err != 0 && err != sys::ESP_ERR_INVALID_STATE {
        return Err(format!("spi_bus_initialize failed: {err}"));
    }

    let mut dev_cfg = sys::spi_device_interface_config_t::default();
    dev_cfg.spics_io_num = -1;
    dev_cfg.clock_speed_hz = 40 * 1000 * 1000;
    dev_cfg.mode = 0;
    dev_cfg.queue_size = 7;
    dev_cfg.flags = sys::SPI_DEVICE_HALFDUPLEX;

    let err = unsafe {
        sys::spi_bus_add_device(
            sys::spi_host_device_t_SPI3_HOST,
            &dev_cfg,
            &mut state.spi_handle,
        )
    };
    if err != 0 {
        return Err(format!("spi_bus_add_device failed: {err}"));
    }

    unsafe {
        let out_cfg = sys::gpio_config_t {
            pin_bit_mask: (1u64 << PIN_RST) | (1u64 << PIN_DC) | (1u64 << PIN_CS),
            mode: sys::gpio_mode_t_GPIO_MODE_OUTPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        if sys::gpio_config(&out_cfg) != 0 {
            return Err("gpio_config(out) failed".into());
        }
        let in_cfg = sys::gpio_config_t {
            pin_bit_mask: 1u64 << PIN_BUSY,
            mode: sys::gpio_mode_t_GPIO_MODE_INPUT,
            pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        if sys::gpio_config(&in_cfg) != 0 {
            return Err("gpio_config(in) failed".into());
        }
        let _ = sys::gpio_set_level(PIN_RST, 1);
        let _ = sys::gpio_set_level(PIN_CS, 1);
        let _ = sys::gpio_set_level(PIN_DC, 1);
    }
    Ok(())
}

#[cfg(target_os = "espidf")]
fn reset() {
    unsafe {
        let _ = sys::gpio_set_level(PIN_RST, 1);
    }
    sleep_ms(50);
    unsafe {
        let _ = sys::gpio_set_level(PIN_RST, 0);
    }
    sleep_ms(20);
    unsafe {
        let _ = sys::gpio_set_level(PIN_RST, 1);
    }
    sleep_ms(50);
}

#[cfg(target_os = "espidf")]
fn wait_busy(stage: &str, timeout_ms: i32) -> Result<(), String> {
    let start_us = unsafe { sys::esp_timer_get_time() };
    let mut waited_ms = 0;
    while unsafe { sys::gpio_get_level(PIN_BUSY) } == 0 {
        sleep_ms(10);
        waited_ms += 10;
        if waited_ms >= timeout_ms {
            println!(
                "photoframe-rs/panel: busy timeout tolerated stage={stage} waited={waited_ms}ms"
            );
            break;
        }
    }
    let cost_ms = (unsafe { sys::esp_timer_get_time() } - start_us) / 1000;
    println!("photoframe-rs/timing: panel_busy stage={stage} cost={cost_ms}ms");
    Ok(())
}

#[cfg(target_os = "espidf")]
fn write_byte(spi_handle: sys::spi_device_handle_t, value: u8) -> Result<(), String> {
    let mut transaction = sys::spi_transaction_t {
        length: 8,
        __bindgen_anon_1: sys::spi_transaction_t__bindgen_ty_1 {
            tx_buffer: &value as *const _ as *const _,
        },
        ..Default::default()
    };
    let err = unsafe { sys::spi_device_polling_transmit(spi_handle, &mut transaction) };
    if err != 0 {
        return Err(format!("spi tx byte failed: {err}"));
    }
    Ok(())
}

#[cfg(target_os = "espidf")]
fn write_command(spi_handle: sys::spi_device_handle_t, cmd: u8) -> Result<(), String> {
    unsafe {
        let _ = sys::gpio_set_level(PIN_DC, 0);
        let _ = sys::gpio_set_level(PIN_CS, 0);
    }
    let result = write_byte(spi_handle, cmd);
    unsafe {
        let _ = sys::gpio_set_level(PIN_CS, 1);
    }
    result
}

#[cfg(target_os = "espidf")]
fn write_data(spi_handle: sys::spi_device_handle_t, data: u8) -> Result<(), String> {
    unsafe {
        let _ = sys::gpio_set_level(PIN_DC, 1);
        let _ = sys::gpio_set_level(PIN_CS, 0);
    }
    let result = write_byte(spi_handle, data);
    unsafe {
        let _ = sys::gpio_set_level(PIN_CS, 1);
    }
    result
}

#[cfg(target_os = "espidf")]
fn write_buffer(spi_handle: sys::spi_device_handle_t, data: &[u8]) -> Result<(), String> {
    unsafe {
        let _ = sys::gpio_set_level(PIN_DC, 1);
        let _ = sys::gpio_set_level(PIN_CS, 0);
    }
    let mut offset = 0usize;
    while offset < data.len() {
        let chunk = (data.len() - offset).min(DEBUG_PANEL_WRITE_CHUNK_BYTES);
        let mut transaction = sys::spi_transaction_t {
            length: chunk * 8,
            __bindgen_anon_1: sys::spi_transaction_t__bindgen_ty_1 {
                tx_buffer: data[offset..].as_ptr() as *const _,
            },
            ..Default::default()
        };
        let err = unsafe { sys::spi_device_polling_transmit(spi_handle, &mut transaction) };
        if err != 0 {
            unsafe {
                let _ = sys::gpio_set_level(PIN_CS, 1);
            }
            return Err(format!("spi tx buffer failed at offset={offset}: {err}"));
        }
        offset += chunk;
        if offset < data.len() {
            sleep_ms(DEBUG_PANEL_WRITE_CHUNK_DELAY_MS);
        }
    }
    unsafe {
        let _ = sys::gpio_set_level(PIN_CS, 1);
    }
    Ok(())
}

#[cfg(target_os = "espidf")]
fn turn_on_display(spi_handle: sys::spi_device_handle_t) -> Result<(), String> {
    crate::runtime_bridge::record_render_trace(22);
    write_command(spi_handle, 0x04)?;
    wait_busy("turn_on/0x04", 45_000)?;

    write_command(spi_handle, 0x06)?;
    for value in [0x6F, 0x1F, 0x17, 0x49] {
        write_data(spi_handle, value)?;
    }

    crate::runtime_bridge::record_render_trace(23);
    write_command(spi_handle, 0x12)?;
    write_data(spi_handle, 0x00)?;
    wait_busy("turn_on/0x12", 45_000)?;

    crate::runtime_bridge::record_render_trace(24);
    write_command(spi_handle, 0x02)?;
    write_data(spi_handle, 0x00)?;
    wait_busy("turn_on/0x02", 45_000)?;
    Ok(())
}

#[cfg(target_os = "espidf")]
fn flush_raw(spi_handle: sys::spi_device_handle_t, data: &[u8]) -> Result<(), String> {
    let start = Instant::now();
    write_command(spi_handle, 0x10)?;
    let transfer_start = Instant::now();
    write_buffer(spi_handle, data)?;
    let transfer_ms = transfer_start.elapsed().as_millis();
    let trigger_start = Instant::now();
    turn_on_display(spi_handle)?;
    let trigger_ms = trigger_start.elapsed().as_millis();
    println!(
        "photoframe-rs/timing: panel_flush transfer={}ms trigger={}ms total={}ms bytes={}",
        transfer_ms,
        trigger_ms,
        start.elapsed().as_millis(),
        data.len()
    );
    Ok(())
}

#[cfg(target_os = "espidf")]
fn apply_panel_init_sequence(spi_handle: sys::spi_device_handle_t) -> Result<(), String> {
    crate::runtime_bridge::record_render_trace(20);
    reset();
    wait_busy("panel_init/reset", 45_000)?;
    sleep_ms(50);

    write_command(spi_handle, 0xAA)?;
    for value in [0x49, 0x55, 0x20, 0x08, 0x09, 0x18] {
        write_data(spi_handle, value)?;
    }
    write_command(spi_handle, 0x01)?;
    write_data(spi_handle, 0x3F)?;
    write_command(spi_handle, 0x00)?;
    for value in [0x5F, 0x69] {
        write_data(spi_handle, value)?;
    }
    write_command(spi_handle, 0x03)?;
    for value in [0x00, 0x54, 0x00, 0x44] {
        write_data(spi_handle, value)?;
    }
    write_command(spi_handle, 0x05)?;
    for value in [0x40, 0x1F, 0x1F, 0x2C] {
        write_data(spi_handle, value)?;
    }
    write_command(spi_handle, 0x06)?;
    for value in [0x6F, 0x1F, 0x17, 0x49] {
        write_data(spi_handle, value)?;
    }
    write_command(spi_handle, 0x08)?;
    for value in [0x6F, 0x1F, 0x1F, 0x22] {
        write_data(spi_handle, value)?;
    }
    write_command(spi_handle, 0x30)?;
    write_data(spi_handle, 0x03)?;
    write_command(spi_handle, 0x50)?;
    write_data(spi_handle, 0x3F)?;
    write_command(spi_handle, 0x60)?;
    for value in [0x02, 0x00] {
        write_data(spi_handle, value)?;
    }
    write_command(spi_handle, 0x61)?;
    for value in [0x03, 0x20, 0x01, 0xE0] {
        write_data(spi_handle, value)?;
    }
    write_command(spi_handle, 0x84)?;
    write_data(spi_handle, 0x01)?;
    write_command(spi_handle, 0xE3)?;
    write_data(spi_handle, 0x2F)?;
    crate::runtime_bridge::record_render_trace(21);
    Ok(())
}

#[cfg(target_os = "espidf")]
fn ensure_initialized_locked(state: &mut PanelRuntime) -> Result<(), String> {
    if state.initialized {
        return Ok(());
    }
    if state.spi_handle.is_null() {
        init_bus(state)?;
    }
    apply_panel_init_sequence(state.spi_handle)?;
    state.initialized = true;
    Ok(())
}

#[cfg(target_os = "espidf")]
fn reset_runtime_after_failure(state: &mut PanelRuntime) {
    state.initialized = false;
    if !state.spi_handle.is_null() {
        let err = unsafe { sys::spi_bus_remove_device(state.spi_handle) };
        if err != 0 {
            println!("photoframe-rs/panel: spi_bus_remove_device failed: {err}");
        }
        state.spi_handle = ptr::null_mut();
    }
    let err = unsafe { sys::spi_bus_free(sys::spi_host_device_t_SPI3_HOST) };
    if err != 0 && err != sys::ESP_ERR_INVALID_STATE {
        println!("photoframe-rs/panel: spi_bus_free failed: {err}");
    }
}

#[cfg(target_os = "espidf")]
pub fn flush_packed_image(data: &[u8]) -> Result<(), String> {
    if data.len() != DISPLAY_LEN {
        return Err(format!("packed image len mismatch: {}", data.len()));
    }
    let mutex = runtime();
    let mut state = mutex
        .lock()
        .map_err(|_| "panel runtime mutex poisoned".to_string())?;
    let mut last_error = String::from("packed flush failed");

    for attempt in 0..FLUSH_MAX_RETRIES {
        if attempt > 0 {
            sleep_ms(FLUSH_RETRY_DELAY_MS);
        }
        if let Err(err) = ensure_initialized_locked(&mut state) {
            last_error = format!("panel init failed: {err}");
            println!(
                "photoframe-rs/panel: init attempt={}/{} err={}",
                attempt + 1,
                FLUSH_MAX_RETRIES,
                err
            );
            reset_runtime_after_failure(&mut state);
            continue;
        }

        match flush_raw(state.spi_handle, data) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_error = format!("flush attempt {} failed: {err}", attempt + 1);
                println!(
                    "photoframe-rs/panel: flush attempt={}/{} err={}",
                    attempt + 1,
                    FLUSH_MAX_RETRIES,
                    err
                );
                reset_runtime_after_failure(&mut state);
            }
        }
    }

    Err(last_error)
}

#[cfg(target_os = "espidf")]
#[allow(dead_code)]
pub fn warmup_panel() -> Result<(), String> {
    let mutex = runtime();
    let mut state = mutex
        .lock()
        .map_err(|_| "panel runtime mutex poisoned".to_string())?;
    ensure_initialized_locked(&mut state)
}

#[cfg(not(target_os = "espidf"))]
pub fn warmup_panel() -> Result<(), String> {
    Err("panel only works on espidf target".into())
}

#[cfg(not(target_os = "espidf"))]
pub fn flush_packed_image(_data: &[u8]) -> Result<(), String> {
    Err("panel only works on espidf target".into())
}
