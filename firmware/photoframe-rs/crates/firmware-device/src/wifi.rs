#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use std::{
    ffi::{CStr, CString, c_void},
    ptr,
    sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering},
    thread,
    time::Duration,
};

#[cfg(target_os = "espidf")]
use esp_idf_sys as sys;

#[cfg(target_os = "espidf")]
const WIFI_CONNECTED_BIT: u32 = 1 << 0;
#[cfg(target_os = "espidf")]
const WIFI_FAIL_BIT: u32 = 1 << 1;

#[cfg(target_os = "espidf")]
static WIFI_EVENTS: AtomicPtr<sys::EventGroupDef_t> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "espidf")]
static STA_NETIF: AtomicPtr<sys::esp_netif_t> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "espidf")]
static WIFI_READY: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "espidf")]
static RETRY_COUNT: AtomicI32 = AtomicI32::new(0);
#[cfg(target_os = "espidf")]
static RETRY_LIMIT: AtomicI32 = AtomicI32::new(5);
#[cfg(target_os = "espidf")]
static LAST_DISCONNECT_REASON: AtomicI32 = AtomicI32::new(0);

pub struct EspWifiManager;

#[cfg(target_os = "espidf")]
unsafe extern "C" fn wifi_event_handler(
    _arg: *mut c_void,
    event_base: sys::esp_event_base_t,
    event_id: i32,
    event_data: *mut c_void,
) {
    if event_base == unsafe { sys::WIFI_EVENT } {
        if event_id == sys::wifi_event_t_WIFI_EVENT_STA_START as i32 {
            let _ = unsafe { sys::esp_wifi_connect() };
            return;
        }

        if event_id == sys::wifi_event_t_WIFI_EVENT_STA_DISCONNECTED as i32 {
            let reason = if event_data.is_null() {
                0
            } else {
                unsafe { (*(event_data as *mut sys::wifi_event_sta_disconnected_t)).reason as i32 }
            };
            LAST_DISCONNECT_REASON.store(reason, Ordering::SeqCst);

            let retry_count = RETRY_COUNT.load(Ordering::SeqCst);
            let retry_limit = RETRY_LIMIT.load(Ordering::SeqCst);
            if retry_count < retry_limit {
                RETRY_COUNT.store(retry_count + 1, Ordering::SeqCst);
                let _ = unsafe { sys::esp_wifi_connect() };
            } else {
                let events = WIFI_EVENTS.load(Ordering::SeqCst);
                if !events.is_null() {
                    let _ = unsafe { sys::xEventGroupSetBits(events, WIFI_FAIL_BIT) };
                }
            }
            return;
        }
    }

    if event_base == unsafe { sys::IP_EVENT }
        && event_id == sys::ip_event_t_IP_EVENT_STA_GOT_IP as i32
    {
        RETRY_COUNT.store(0, Ordering::SeqCst);
        LAST_DISCONNECT_REASON.store(0, Ordering::SeqCst);
        let events = WIFI_EVENTS.load(Ordering::SeqCst);
        if !events.is_null() {
            let _ = unsafe { sys::xEventGroupSetBits(events, WIFI_CONNECTED_BIT) };
        }
    }
}

#[cfg(target_os = "espidf")]
pub(crate) fn wifi_init_config_default() -> sys::wifi_init_config_t {
    sys::wifi_init_config_t {
        osi_funcs: ptr::addr_of_mut!(sys::g_wifi_osi_funcs),
        wpa_crypto_funcs: unsafe { sys::g_wifi_default_wpa_crypto_funcs },
        static_rx_buf_num: sys::CONFIG_ESP_WIFI_STATIC_RX_BUFFER_NUM as i32,
        dynamic_rx_buf_num: sys::CONFIG_ESP_WIFI_DYNAMIC_RX_BUFFER_NUM as i32,
        tx_buf_type: sys::CONFIG_ESP_WIFI_TX_BUFFER_TYPE as i32,
        static_tx_buf_num: sys::WIFI_STATIC_TX_BUFFER_NUM as i32,
        dynamic_tx_buf_num: sys::WIFI_DYNAMIC_TX_BUFFER_NUM as i32,
        rx_mgmt_buf_type: sys::CONFIG_ESP_WIFI_STATIC_RX_MGMT_BUFFER as i32,
        rx_mgmt_buf_num: sys::WIFI_RX_MGMT_BUF_NUM_DEF as i32,
        cache_tx_buf_num: sys::WIFI_CACHE_TX_BUFFER_NUM as i32,
        csi_enable: sys::WIFI_CSI_ENABLED as i32,
        ampdu_rx_enable: sys::WIFI_AMPDU_RX_ENABLED as i32,
        ampdu_tx_enable: sys::WIFI_AMPDU_TX_ENABLED as i32,
        amsdu_tx_enable: sys::WIFI_AMSDU_TX_ENABLED as i32,
        nvs_enable: sys::WIFI_NVS_ENABLED as i32,
        nano_enable: sys::WIFI_NANO_FORMAT_ENABLED as i32,
        rx_ba_win: sys::WIFI_DEFAULT_RX_BA_WIN as i32,
        wifi_task_core_id: sys::WIFI_TASK_CORE_ID as i32,
        beacon_max_len: sys::WIFI_SOFTAP_BEACON_MAX_LEN as i32,
        mgmt_sbuf_num: sys::WIFI_MGMT_SBUF_NUM as i32,
        feature_caps: sys::WIFI_FEATURE_CAPS as u64,
        sta_disconnected_pm: sys::WIFI_STA_DISCONNECTED_PM_ENABLED != 0,
        espnow_max_encrypt_num: sys::CONFIG_ESP_WIFI_ESPNOW_MAX_ENCRYPT_NUM as i32,
        tx_hetb_queue_num: sys::WIFI_TX_HETB_QUEUE_NUM as i32,
        dump_hesigb_enable: sys::WIFI_DUMP_HESIGB_ENABLED != 0,
        magic: sys::WIFI_INIT_CONFIG_MAGIC as i32,
    }
}

#[cfg(target_os = "espidf")]
fn copy_bytes_to_array<const N: usize>(dst: &mut [u8; N], value: &str) {
    dst.fill(0);
    let bytes = value.as_bytes();
    let len = bytes.len().min(N.saturating_sub(1));
    dst[..len].copy_from_slice(&bytes[..len]);
}

#[cfg(target_os = "espidf")]
fn esp_to_result(err: i32, op: &str) -> Result<(), String> {
    if err == 0 || err == sys::ESP_ERR_INVALID_STATE {
        return Ok(());
    }
    Err(format!("{op} failed: {err}"))
}

impl EspWifiManager {
    #[cfg(target_os = "espidf")]
    pub fn init_once(hostname: &str) -> Result<(), String> {
        if WIFI_READY.load(Ordering::SeqCst) {
            if !hostname.is_empty() {
                let netif = STA_NETIF.load(Ordering::SeqCst);
                if !netif.is_null() {
                    let hostname = CString::new(hostname).map_err(|err| err.to_string())?;
                    let _ = unsafe { sys::esp_netif_set_hostname(netif, hostname.as_ptr()) };
                }
            }
            return Ok(());
        }

        let events = WIFI_EVENTS.load(Ordering::SeqCst);
        if events.is_null() {
            let created = unsafe { sys::xEventGroupCreate() };
            if created.is_null() {
                return Err("xEventGroupCreate failed".into());
            }
            WIFI_EVENTS.store(created, Ordering::SeqCst);
        }

        esp_to_result(unsafe { sys::esp_netif_init() }, "esp_netif_init")?;
        esp_to_result(
            unsafe { sys::esp_event_loop_create_default() },
            "esp_event_loop_create_default",
        )?;

        let sta_netif = unsafe { sys::esp_netif_create_default_wifi_sta() };
        if sta_netif.is_null() {
            return Err("esp_netif_create_default_wifi_sta failed".into());
        }
        STA_NETIF.store(sta_netif, Ordering::SeqCst);

        if !hostname.is_empty() {
            let hostname = CString::new(hostname).map_err(|err| err.to_string())?;
            let _ = unsafe { sys::esp_netif_set_hostname(sta_netif, hostname.as_ptr()) };
        }

        let init_config = wifi_init_config_default();
        esp_to_result(unsafe { sys::esp_wifi_init(&init_config) }, "esp_wifi_init")?;
        esp_to_result(
            unsafe { sys::esp_wifi_set_storage(sys::wifi_storage_t_WIFI_STORAGE_RAM) },
            "esp_wifi_set_storage",
        )?;
        esp_to_result(
            unsafe {
                sys::esp_event_handler_register(
                    sys::WIFI_EVENT,
                    sys::ESP_EVENT_ANY_ID,
                    Some(wifi_event_handler),
                    ptr::null_mut(),
                )
            },
            "esp_event_handler_register(WIFI_EVENT)",
        )?;
        esp_to_result(
            unsafe {
                sys::esp_event_handler_register(
                    sys::IP_EVENT,
                    sys::ip_event_t_IP_EVENT_STA_GOT_IP as i32,
                    Some(wifi_event_handler),
                    ptr::null_mut(),
                )
            },
            "esp_event_handler_register(IP_EVENT_STA_GOT_IP)",
        )?;
        esp_to_result(
            unsafe { sys::esp_wifi_set_mode(sys::wifi_mode_t_WIFI_MODE_STA) },
            "esp_wifi_set_mode(STA)",
        )?;

        WIFI_READY.store(true, Ordering::SeqCst);
        Ok(())
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn init_once(_hostname: &str) -> Result<(), String> {
        Err("EspWifiManager only works on espidf target".into())
    }

    #[cfg(target_os = "espidf")]
    pub fn connect(
        hostname: &str,
        ssid: &str,
        password: &str,
        timeout_sec: i32,
        retry_limit: i32,
    ) -> Result<(), String> {
        if ssid.is_empty() {
            return Err("wifi ssid is empty".into());
        }

        Self::init_once(hostname)?;

        let mut sta_config = sys::wifi_sta_config_t::default();
        copy_bytes_to_array(&mut sta_config.ssid, ssid);
        copy_bytes_to_array(&mut sta_config.password, password);
        sta_config.scan_method = sys::wifi_scan_method_t_WIFI_ALL_CHANNEL_SCAN;
        sta_config.sort_method = sys::wifi_sort_method_t_WIFI_CONNECT_AP_BY_SIGNAL;
        sta_config.threshold = sys::wifi_scan_threshold_t {
            rssi: 0,
            authmode: sys::wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK,
            rssi_5g_adjustment: 0,
        };
        sta_config.pmf_cfg = sys::wifi_pmf_config_t {
            capable: true,
            required: false,
        };
        sta_config.failure_retry_cnt = 1;

        let mut wifi_config = sys::wifi_config_t { sta: sta_config };

        RETRY_LIMIT.store(
            if retry_limit > 0 { retry_limit } else { 5 },
            Ordering::SeqCst,
        );
        RETRY_COUNT.store(0, Ordering::SeqCst);
        LAST_DISCONNECT_REASON.store(0, Ordering::SeqCst);

        let events = WIFI_EVENTS.load(Ordering::SeqCst);
        if events.is_null() {
            return Err("wifi event group missing".into());
        }
        unsafe {
            let _ = sys::xEventGroupClearBits(events, WIFI_CONNECTED_BIT | WIFI_FAIL_BIT);
        }

        esp_to_result(
            unsafe {
                sys::esp_wifi_set_config(sys::wifi_interface_t_WIFI_IF_STA, &mut wifi_config)
            },
            "esp_wifi_set_config(STA)",
        )?;

        let start_err = unsafe { sys::esp_wifi_start() };
        if start_err != 0 && start_err != sys::ESP_ERR_WIFI_CONN {
            return Err(format!("esp_wifi_start failed: {start_err}"));
        }

        let deadline_us = unsafe { sys::esp_timer_get_time() }
            + i64::from(if timeout_sec > 0 { timeout_sec } else { 25 }) * 1_000_000;
        loop {
            let bits = unsafe {
                sys::xEventGroupWaitBits(events, WIFI_CONNECTED_BIT | WIFI_FAIL_BIT, 0, 0, 0)
            };
            if (bits & WIFI_CONNECTED_BIT) != 0 {
                return Ok(());
            }
            if (bits & WIFI_FAIL_BIT) != 0 || unsafe { sys::esp_timer_get_time() } >= deadline_us {
                let _ = unsafe { sys::esp_wifi_stop() };
                let reason = LAST_DISCONNECT_REASON.load(Ordering::SeqCst);
                return Err(format!("wifi connect failed: reason={reason}"));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn connect(
        _hostname: &str,
        _ssid: &str,
        _password: &str,
        _timeout_sec: i32,
        _retry_limit: i32,
    ) -> Result<(), String> {
        Err("EspWifiManager only works on espidf target".into())
    }

    #[cfg(target_os = "espidf")]
    pub fn stop() {
        unsafe {
            let _ = sys::esp_wifi_stop();
        }
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn stop() {}

    #[cfg(target_os = "espidf")]
    pub fn sta_ip_string() -> Option<String> {
        let netif = STA_NETIF.load(Ordering::SeqCst);
        if netif.is_null() {
            return None;
        }

        let mut ip_info = sys::esp_netif_ip_info_t::default();
        let err = unsafe { sys::esp_netif_get_ip_info(netif, &mut ip_info) };
        if err != 0 {
            return None;
        }

        let mut buf = [0 as std::ffi::c_char; 16];
        let ptr = unsafe { sys::esp_ip4addr_ntoa(&ip_info.ip, buf.as_mut_ptr(), buf.len() as i32) };
        if ptr.is_null() {
            return None;
        }
        Some(
            unsafe { CStr::from_ptr(buf.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn sta_ip_string() -> Option<String> {
        None
    }
}
