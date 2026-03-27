#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use std::{
    ffi::{CStr, CString},
    ptr,
    sync::{Mutex, OnceLock},
};
#[cfg(all(not(target_os = "espidf"), test))]
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "espidf")]
use esp_idf_sys as sys;

#[cfg(target_os = "espidf")]
const SD_MOUNT_PATH: &str = "/sdcard";
#[cfg(target_os = "espidf")]
const SDMMC_HOST_FLAG_DDR: u32 = 1 << 4;
#[cfg(target_os = "espidf")]
const SDMMC_HOST_FLAG_DEINIT_ARG: u32 = 1 << 5;
#[cfg(target_os = "espidf")]
const SDMMC_SLOT_FLAG_INTERNAL_PULLUP: u32 = 1;
#[cfg(target_os = "espidf")]
const SDMMC_SLOT_NO_CD: i32 = -1;
#[cfg(target_os = "espidf")]
const SDMMC_SLOT_NO_WP: i32 = -1;
const SDMMC_SLOT_UNUSED_PIN: i32 = -1;

#[cfg(target_os = "espidf")]
#[derive(Clone, Copy)]
struct SdCardCandidate {
    name: &'static str,
    slot: i32,
    clk: i32,
    cmd: i32,
    d0: i32,
    d1: i32,
    d2: i32,
    d3: i32,
}

#[cfg(target_os = "espidf")]
const SD_CANDIDATES: [SdCardCandidate; 4] = [
    SdCardCandidate {
        name: "sdcard_bsp_slot1",
        slot: 1,
        clk: 39,
        cmd: 41,
        d0: 40,
        d1: 1,
        d2: 2,
        d3: 38,
    },
    SdCardCandidate {
        name: "sdcard_bsp_slot0",
        slot: 0,
        clk: 39,
        cmd: 41,
        d0: 40,
        d1: 1,
        d2: 2,
        d3: 38,
    },
    SdCardCandidate {
        name: "codec_board_slot1",
        slot: 1,
        clk: 43,
        cmd: 44,
        d0: 39,
        d1: 40,
        d2: 41,
        d3: 42,
    },
    SdCardCandidate {
        name: "codec_board_slot0",
        slot: 0,
        clk: 43,
        cmd: 44,
        d0: 39,
        d1: 40,
        d2: 41,
        d3: 42,
    },
];

#[cfg(target_os = "espidf")]
struct SdCardState {
    mounted: bool,
    card: *mut sys::sdmmc_card_t,
}

#[cfg(target_os = "espidf")]
unsafe impl Send for SdCardState {}

#[cfg(target_os = "espidf")]
impl Default for SdCardState {
    fn default() -> Self {
        Self {
            mounted: false,
            card: ptr::null_mut(),
        }
    }
}

#[cfg(target_os = "espidf")]
fn global_state() -> &'static Mutex<SdCardState> {
    static STATE: OnceLock<Mutex<SdCardState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(SdCardState::default()))
}

#[cfg(target_os = "espidf")]
fn check_esp(err: i32, op: &str) -> Result<(), String> {
    if err == 0 {
        return Ok(());
    }
    unsafe {
        let name = CStr::from_ptr(sys::esp_err_to_name(err))
            .to_string_lossy()
            .into_owned();
        Err(format!("{op} failed: {name} ({err})"))
    }
}

#[cfg(target_os = "espidf")]
fn build_host_config(slot: i32) -> sys::sdmmc_host_t {
    sys::sdmmc_host_t {
        flags: SDMMC_HOST_FLAG_DEINIT_ARG | SDMMC_HOST_FLAG_DDR | 0b11,
        slot,
        max_freq_khz: sys::SDMMC_FREQ_HIGHSPEED as i32,
        io_voltage: 3.3,
        driver_strength: sys::sdmmc_driver_strength_t_SDMMC_DRIVER_STRENGTH_B,
        current_limit: sys::sdmmc_current_limit_t_SDMMC_CURRENT_LIMIT_200MA,
        init: Some(sys::sdmmc_host_init),
        set_bus_width: Some(sys::sdmmc_host_set_bus_width),
        get_bus_width: Some(sys::sdmmc_host_get_slot_width),
        set_bus_ddr_mode: Some(sys::sdmmc_host_set_bus_ddr_mode),
        set_card_clk: Some(sys::sdmmc_host_set_card_clk),
        set_cclk_always_on: Some(sys::sdmmc_host_set_cclk_always_on),
        do_transaction: Some(sys::sdmmc_host_do_transaction),
        __bindgen_anon_1: sys::sdmmc_host_t__bindgen_ty_1 {
            deinit: Some(sys::sdmmc_host_deinit),
        },
        io_int_enable: Some(sys::sdmmc_host_io_int_enable),
        io_int_wait: Some(sys::sdmmc_host_io_int_wait),
        command_timeout_ms: 0,
        get_real_freq: Some(sys::sdmmc_host_get_real_freq),
        input_delay_phase: sys::sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_0,
        set_input_delay: Some(sys::sdmmc_host_set_input_delay),
        dma_aligned_buffer: ptr::null_mut(),
        pwr_ctrl_handle: ptr::null_mut(),
        get_dma_info: Some(sys::sdmmc_host_get_dma_info),
        check_buffer_alignment: Some(sys::sdmmc_host_check_buffer_alignment),
        is_slot_set_to_uhs1: None,
    }
}

#[cfg(target_os = "espidf")]
fn gpio_num(pin: i32) -> sys::gpio_num_t {
    pin as sys::gpio_num_t
}

#[cfg(target_os = "espidf")]
fn build_slot_config(candidate: SdCardCandidate) -> sys::sdmmc_slot_config_t {
    sys::sdmmc_slot_config_t {
        clk: gpio_num(candidate.clk),
        cmd: gpio_num(candidate.cmd),
        d0: gpio_num(candidate.d0),
        d1: gpio_num(candidate.d1),
        d2: gpio_num(candidate.d2),
        d3: gpio_num(candidate.d3),
        d4: gpio_num(SDMMC_SLOT_UNUSED_PIN),
        d5: gpio_num(SDMMC_SLOT_UNUSED_PIN),
        d6: gpio_num(SDMMC_SLOT_UNUSED_PIN),
        d7: gpio_num(SDMMC_SLOT_UNUSED_PIN),
        __bindgen_anon_1: sys::sdmmc_slot_config_t__bindgen_ty_1 {
            cd: gpio_num(SDMMC_SLOT_NO_CD),
        },
        __bindgen_anon_2: sys::sdmmc_slot_config_t__bindgen_ty_2 {
            wp: gpio_num(SDMMC_SLOT_NO_WP),
        },
        width: 4,
        flags: SDMMC_SLOT_FLAG_INTERNAL_PULLUP,
    }
}

#[cfg(target_os = "espidf")]
pub(crate) fn mount_if_available() -> Result<bool, String> {
    let mut guard = global_state()
        .lock()
        .map_err(|_| "sdcard state lock poisoned".to_string())?;
    if guard.mounted {
        return Ok(true);
    }

    let base_path = CString::new(SD_MOUNT_PATH).expect("sd mount path");
    let mount_config = sys::esp_vfs_fat_sdmmc_mount_config_t {
        format_if_mount_failed: false,
        max_files: 5,
        allocation_unit_size: 16 * 1024 * 3,
        disk_status_check_enable: false,
        use_one_fat: false,
    };

    let mut last_error = String::from("no sd candidate attempted");
    for candidate in SD_CANDIDATES {
        let host = build_host_config(candidate.slot);
        let slot = build_slot_config(candidate);
        let mut card: *mut sys::sdmmc_card_t = ptr::null_mut();
        let err = unsafe {
            sys::esp_vfs_fat_sdmmc_mount(
                base_path.as_ptr(),
                &host,
                &slot as *const _ as *const _,
                &mount_config,
                &mut card,
            )
        };
        if err == 0 {
            guard.mounted = true;
            guard.card = card;
            println!(
                "photoframe-rs: sdcard mounted via {} slot={} pins=clk:{} cmd:{} d0:{} d1:{} d2:{} d3:{}",
                candidate.name,
                candidate.slot,
                candidate.clk,
                candidate.cmd,
                candidate.d0,
                candidate.d1,
                candidate.d2,
                candidate.d3,
            );
            return Ok(true);
        }
        last_error = format!(
            "{}: {}",
            candidate.name,
            check_esp(err, "esp_vfs_fat_sdmmc_mount").unwrap_err()
        );
        unsafe {
            let _ = sys::sdmmc_host_deinit();
        }
    }
    Err(last_error)
}

#[cfg(not(target_os = "espidf"))]
pub(crate) fn mount_if_available() -> Result<bool, String> {
    Ok(false)
}

#[cfg(target_os = "espidf")]
pub(crate) fn is_ready() -> bool {
    global_state().lock().map(|guard| guard.mounted).unwrap_or(false)
}

#[cfg(not(target_os = "espidf"))]
#[cfg(test)]
fn test_ready_state() -> &'static Mutex<bool> {
    static READY: OnceLock<Mutex<bool>> = OnceLock::new();
    READY.get_or_init(|| Mutex::new(false))
}

#[cfg(not(target_os = "espidf"))]
#[cfg(test)]
pub(crate) fn set_test_ready(ready: bool) {
    if let Ok(mut guard) = test_ready_state().lock() {
        *guard = ready;
    }
}

#[cfg(not(target_os = "espidf"))]
#[cfg(test)]
pub(crate) fn is_ready() -> bool {
    test_ready_state().lock().map(|guard| *guard).unwrap_or(false)
}

#[cfg(not(target_os = "espidf"))]
#[cfg(not(test))]
pub(crate) fn is_ready() -> bool {
    false
}

pub(crate) fn mount_path() -> &'static str {
    "/sdcard"
}
