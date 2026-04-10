#![cfg_attr(not(target_os = "espidf"), allow(dead_code, unused_imports))]
mod diag;
mod jpeg;
mod panel;
mod photo_history;
mod portal;
mod power;
mod render_core;
mod runtime_bridge;
mod sdcard;
mod wifi;

const _: () = photoframe_firmware_device::LIB_TARGET_PRESENT;

#[cfg(target_os = "espidf")]
use std::{
    ffi::CString,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "espidf")]
use photoframe_app::{
    BootContext, Clock, CycleExit, CycleReport, CycleRunner, DeviceRuntimeConfig, Display,
    ImageFetchPlan, ImageFetcher, Storage, date_days_behind, shift_date_string_days,
};
#[cfg(target_os = "espidf")]
use photoframe_contracts::DeviceConfigPayload;
use photoframe_domain::WakeSource;

#[cfg(target_os = "espidf")]
use photoframe_domain::{
    CycleAction, FailureKind, LongPressAction, apply_cycle_outcome, seconds_to_microseconds,
    should_sync_time, sleep_seconds_until_next_beijing_sync,
};

#[cfg(target_os = "espidf")]
const KEY_BUTTON: i32 = esp_idf_sys::gpio_num_t_GPIO_NUM_4;
#[cfg(target_os = "espidf")]
const BOOT_BUTTON: i32 = esp_idf_sys::gpio_num_t_GPIO_NUM_0;
#[cfg(target_os = "espidf")]
const EXT1_SAMPLE_ROUNDS: usize = 8;
#[cfg(target_os = "espidf")]
const BUILTIN_WIFI_PROFILES: [(&str, &str); 3] = [
    ("OpenWrt", "sansiAX3"),
    ("Qing-IoT", "jiajuzhuanyong"),
    ("Qing-AP", "64139772"),
];
#[cfg(target_os = "espidf")]
const DEPRECATED_IMAGE_URL_TEMPLATE: &str = "https://picsum.photos/480/800?date=%DATE%";
#[cfg(target_os = "espidf")]
const DEPRECATED_ORCHESTRATOR_BASE_URL: &str = "http://192.168.233.11:8081";
#[cfg(target_os = "espidf")]
const MANUAL_SYNC_SERIAL_GRACE_SECONDS: u64 = 60;

#[cfg(target_os = "espidf")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreSleepHoldMode {
    UsbOrSerial,
    ManualSyncSerialGrace { wait_seconds: u64 },
}

#[cfg(target_os = "espidf")]
enum PreparedCycle {
    Ready(BootContext),
    EnterApPortal,
    RetryLater { sleep_seconds: u64 },
}

#[cfg(target_os = "espidf")]
const USB_DEBUG_POLL_SECONDS: u64 = 5;
#[cfg(target_os = "espidf")]
const USB_DEBUG_LOG_DUMP_MAX_LINES: usize = 400;
#[cfg(target_os = "espidf")]
const USB_DEBUG_LOG_DUMP_MAX_BYTES: usize = 48 * 1024;
#[cfg(target_os = "espidf")]
const USB_DEBUG_RESUME_WAIT_MS: u64 = 3_000;
#[cfg(target_os = "espidf")]
const USB_DEBUG_RESUME_POLL_MS: u64 = 20;
#[cfg(target_os = "espidf")]
const USB_DEBUG_POST_DUMP_SETTLE_MS: u64 = 200;
#[cfg(target_os = "espidf")]
const USB_DEBUG_RESUME_TOKEN: &[u8] = b"PHOTOFRAME_USB_RESUME";

#[cfg(target_os = "espidf")]
fn configure_button_gpio() {
    unsafe {
        let cfg = esp_idf_sys::gpio_config_t {
            pin_bit_mask: (1u64 << KEY_BUTTON) | (1u64 << BOOT_BUTTON),
            mode: esp_idf_sys::gpio_mode_t_GPIO_MODE_INPUT,
            pull_up_en: esp_idf_sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
            pull_down_en: esp_idf_sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: esp_idf_sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        };
        let _ = esp_idf_sys::gpio_config(&cfg);
    }
}

#[cfg(target_os = "espidf")]
fn is_key_pressed() -> bool {
    unsafe { esp_idf_sys::gpio_get_level(KEY_BUTTON) == 0 }
}

#[cfg(target_os = "espidf")]
fn is_boot_pressed() -> bool {
    unsafe { esp_idf_sys::gpio_get_level(BOOT_BUTTON) == 0 }
}

#[cfg(target_os = "espidf")]
fn detect_long_press_action() -> LongPressAction {
    if !is_key_pressed() && !is_boot_pressed() {
        return LongPressAction::None;
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !is_key_pressed() && !is_boot_pressed() {
            return LongPressAction::None;
        }
        thread::sleep(Duration::from_millis(20));
    }

    if is_boot_pressed() {
        return LongPressAction::EnterApPortal;
    }
    if is_key_pressed() {
        return LongPressAction::ShowCurrentPhoto;
    }
    LongPressAction::None
}

fn wake_source_from_ext1_state(
    boot_pin: bool,
    key_pin: bool,
    boot_seen_low: bool,
    key_seen_low: bool,
) -> WakeSource {
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

#[cfg(target_os = "espidf")]
fn migrate_legacy_network_defaults(config: &mut DeviceRuntimeConfig) -> bool {
    if config.remote_config_version > 0 {
        return false;
    }

    let defaults = DeviceRuntimeConfig::default();
    let mut changed = false;

    if config.orchestrator_base_url == DEPRECATED_ORCHESTRATOR_BASE_URL {
        config.orchestrator_base_url = defaults.orchestrator_base_url;
        changed = true;
    }
    if config.image_url_template == DEPRECATED_IMAGE_URL_TEMPLATE {
        config.image_url_template = defaults.image_url_template;
        changed = true;
    }

    changed
}

#[cfg(target_os = "espidf")]
fn merge_builtin_wifi_profiles(
    config: &mut DeviceRuntimeConfig,
    _long_press_action: LongPressAction,
) -> bool {
    let previous_last_connected_ssid = config
        .last_connected_wifi_index
        .and_then(|index| config.wifi_profiles.get(index))
        .map(|item| item.ssid.clone());

    let mut next_profiles = Vec::new();
    for (ssid, password) in BUILTIN_WIFI_PROFILES.iter() {
        let existing_password = config
            .wifi_profiles
            .iter()
            .find(|item| item.ssid == *ssid)
            .map(|item| item.password.clone())
            .unwrap_or_default();
        next_profiles.push(photoframe_app::WifiCredential {
            ssid: (*ssid).to_string(),
            password: if existing_password.is_empty() {
                (*password).to_string()
            } else {
                existing_password
            },
        });
    }

    for profile in config.wifi_profiles.iter() {
        let ssid = profile.ssid.trim();
        if ssid.is_empty() || next_profiles.iter().any(|item| item.ssid == ssid) {
            continue;
        }
        if next_profiles.len() >= DeviceRuntimeConfig::MAX_WIFI_PROFILES {
            break;
        }
        next_profiles.push(photoframe_app::WifiCredential {
            ssid: ssid.to_string(),
            password: profile.password.clone(),
        });
    }

    let mut changed = config.wifi_profiles != next_profiles;
    config.wifi_profiles = next_profiles;

    config.last_connected_wifi_index = previous_last_connected_ssid.and_then(|ssid| {
        config
            .wifi_profiles
            .iter()
            .position(|profile| profile.ssid == ssid)
    });

    if let Some(current) = config
        .wifi_profiles
        .iter()
        .find(|profile| profile.ssid == config.primary_wifi_ssid)
    {
        if config.primary_wifi_password != current.password {
            changed = true;
            config.primary_wifi_password = current.password.clone();
        }
    } else if let Some(first) = config.wifi_profiles.first() {
        if config.primary_wifi_ssid != first.ssid || config.primary_wifi_password != first.password
        {
            changed = true;
            config.primary_wifi_ssid = first.ssid.clone();
            config.primary_wifi_password = first.password.clone();
        }
    }

    changed
}

#[cfg(target_os = "espidf")]
fn bootstrap_config_from_env() -> Option<DeviceConfigPayload> {
    let raw = option_env!("PHOTOFRAME_BOOTSTRAP_CONFIG_JSON")?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    match serde_json::from_str::<DeviceConfigPayload>(trimmed) {
        Ok(payload) => Some(payload),
        Err(err) => {
            println!("photoframe-rs: invalid bootstrap config json: {err}");
            None
        }
    }
}

#[cfg(target_os = "espidf")]
fn apply_test_power_override(sample: &mut photoframe_app::PowerSample) {
    let Some(raw) = option_env!("PHOTOFRAME_TEST_POWER_OVERRIDE_JSON") else {
        return;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(override_cfg) => {
            if let Some(value) = override_cfg.get("battery_mv").and_then(|v| v.as_i64()) {
                sample.battery_mv = value as i32;
            }
            if let Some(value) = override_cfg.get("battery_percent").and_then(|v| v.as_i64()) {
                sample.battery_percent = value as i32;
            }
            if let Some(value) = override_cfg.get("charging").and_then(|v| v.as_i64()) {
                sample.charging = value as i32;
            }
            if let Some(value) = override_cfg.get("vbus_good").and_then(|v| v.as_i64()) {
                sample.vbus_good = value as i32;
            }
            println!(
                "photoframe-rs: applied test power override battery_mv={} battery_percent={} charging={} vbus_good={}",
                sample.battery_mv, sample.battery_percent, sample.charging, sample.vbus_good
            );
        }
        Err(err) => {
            println!("photoframe-rs: invalid test power override json: {err}");
        }
    }
}

#[cfg(target_os = "espidf")]
fn current_wake_source() -> WakeSource {
    match unsafe { esp_idf_sys::esp_sleep_get_wakeup_cause() } {
        esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TIMER => {
            crate::device_log!("INFO", "photoframe-rs: wakeup cause=TIMER");
            WakeSource::Timer
        }
        esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT1 => {
            let pins = unsafe { esp_idf_sys::esp_sleep_get_ext1_wakeup_status() };
            let boot_pin = (pins & (1u64 << BOOT_BUTTON)) != 0;
            let key_pin = (pins & (1u64 << KEY_BUTTON)) != 0;
            let mut boot_level = unsafe { esp_idf_sys::gpio_get_level(BOOT_BUTTON) };
            let mut key_level = unsafe { esp_idf_sys::gpio_get_level(KEY_BUTTON) };
            let mut boot_seen_low = boot_pin && boot_level == 0;
            let mut key_seen_low = key_pin && key_level == 0;

            if !boot_seen_low && !key_seen_low && (boot_pin || key_pin) {
                for _ in 0..EXT1_SAMPLE_ROUNDS {
                    thread::sleep(Duration::from_millis(10));
                    boot_level = unsafe { esp_idf_sys::gpio_get_level(BOOT_BUTTON) };
                    key_level = unsafe { esp_idf_sys::gpio_get_level(KEY_BUTTON) };
                    if boot_pin && boot_level == 0 {
                        boot_seen_low = true;
                    }
                    if key_pin && key_level == 0 {
                        key_seen_low = true;
                    }
                    if boot_seen_low || key_seen_low {
                        break;
                    }
                }
            }

            crate::device_log!(
                "INFO",
                "photoframe-rs: wakeup cause=EXT1 pins=0x{pins:x} key={} boot={} seen_low(key={} boot={})",
                key_level,
                boot_level,
                i32::from(key_seen_low),
                i32::from(boot_seen_low),
            );

            wake_source_from_ext1_state(boot_pin, key_pin, boot_seen_low, key_seen_low)
        }
        other => {
            crate::device_log!("INFO", "photoframe-rs: wakeup cause=OTHER({other})");
            WakeSource::Other
        }
    }
}

#[cfg(target_os = "espidf")]
struct DeviceDisplay;

#[cfg(target_os = "espidf")]
impl Display for DeviceDisplay {
    /// 设备侧显示适配：当前通过 C++ bridge 复用已验证的墨水屏渲染链路。
    fn render(
        &mut self,
        artifact: &photoframe_app::ImageArtifact,
        config: &photoframe_app::DeviceRuntimeConfig,
        _force_refresh: bool,
    ) -> Result<(), FailureKind> {
        runtime_bridge::EspRuntimeBridge::render_image(artifact, config)
    }

    fn after_render_success(
        &mut self,
        artifact: &photoframe_app::ImageArtifact,
        _config: &photoframe_app::DeviceRuntimeConfig,
        image_sha256: &str,
        image_date: Option<&str>,
    ) -> Result<(), String> {
        if let Err(err) = photo_history::remember_rendered_photo(artifact, image_sha256, image_date)
        {
            crate::device_log!(
                "WARN",
                "photoframe-rs: cache rendered photo failed sha={} date={} err={}",
                image_sha256,
                image_date.unwrap_or_default(),
                err
            );
        }
        Ok(())
    }
}

#[cfg(target_os = "espidf")]
enum LocalPhotoOutcome {
    Rendered { image_source: String },
    Unavailable { image_source: String },
}

#[cfg(target_os = "espidf")]
fn regular_sleep_seconds(config: &DeviceRuntimeConfig) -> u64 {
    let fallback = u64::from(config.interval_minutes.max(1)) * 60;
    sleep_seconds_until_next_beijing_sync(system_now_epoch()).unwrap_or(fallback)
}

#[cfg(target_os = "espidf")]
fn history_fetch_output_format() -> &'static str {
    "bmp"
}

#[cfg(target_os = "espidf")]
fn render_cached_photo(
    config: &DeviceRuntimeConfig,
    sha256: &str,
    reason: &str,
) -> Result<bool, String> {
    let Some(artifact) = photo_history::load_artifact_by_sha256(sha256)? else {
        crate::device_log!(
            "WARN",
            "photoframe-rs: cached photo missing sha={} reason={}",
            sha256,
            reason
        );
        return Ok(false);
    };
    let mut display = DeviceDisplay;
    display
        .render(&artifact, config, false)
        .map_err(|err| format!("render cached photo failed: {err:?}"))?;
    crate::device_log!(
        "INFO",
        "photoframe-rs: rendered cached photo sha={} reason={}",
        sha256,
        reason
    );
    Ok(true)
}

#[cfg(target_os = "espidf")]
fn current_orchestrator_date(config: &DeviceRuntimeConfig) -> Result<String, String> {
    if !config.last_image_date.trim().is_empty() {
        return Ok(config.last_image_date.trim().to_string());
    }
    if let Some(entry) = photo_history::entry_for_sha256(&config.last_image_sha256)?
        && !entry.image_date.trim().is_empty()
    {
        return Ok(entry.image_date);
    }
    let clock = photoframe_platform_espidf::EspIdfClock;
    Ok(clock.today_date_string())
}

#[cfg(target_os = "espidf")]
fn displayed_history_date(
    config: &DeviceRuntimeConfig,
    current_date: &str,
) -> Result<String, String> {
    if !config.displayed_image_date.trim().is_empty() {
        return Ok(config.displayed_image_date.trim().to_string());
    }
    let current_sha = if config.displayed_image_sha256.is_empty() {
        config.last_image_sha256.as_str()
    } else {
        config.displayed_image_sha256.as_str()
    };
    if let Some(entry) = photo_history::entry_for_sha256(current_sha)?
        && !entry.image_date.trim().is_empty()
    {
        return Ok(entry.image_date);
    }
    Ok(current_date.to_string())
}

#[cfg(target_os = "espidf")]
fn next_history_target_date(current_date: &str, displayed_date: &str) -> Result<String, String> {
    if displayed_date == current_date {
        return shift_date_string_days(current_date, -1)
            .ok_or_else(|| format!("cannot shift history date from current={current_date}"));
    }

    let days_behind = date_days_behind(current_date, displayed_date).unwrap_or_default();
    if days_behind >= photo_history::PHOTO_HISTORY_MAX_ENTRIES as i64 {
        return Ok(current_date.to_string());
    }
    shift_date_string_days(displayed_date, -1)
        .ok_or_else(|| format!("cannot shift history date displayed={displayed_date}"))
}

#[cfg(target_os = "espidf")]
fn apply_displayed_history_state(
    config: &mut DeviceRuntimeConfig,
    sha256: &str,
    displayed_date: &str,
    current_date: &str,
    image_source: &str,
) -> LocalPhotoOutcome {
    config.displayed_image_sha256 = sha256.to_string();
    config.displayed_image_date = displayed_date.to_string();
    config.manual_history_active = displayed_date != current_date;
    crate::device_log!(
        "INFO",
        "photoframe-rs: history switched sha={} date={} browse_active={}",
        config.displayed_image_sha256,
        config.displayed_image_date,
        i32::from(config.manual_history_active)
    );
    LocalPhotoOutcome::Rendered {
        image_source: image_source.to_string(),
    }
}

#[cfg(target_os = "espidf")]
fn render_history_date_from_cache(
    config: &mut DeviceRuntimeConfig,
    displayed_date: &str,
    current_date: &str,
    reason: &str,
) -> Result<Option<LocalPhotoOutcome>, String> {
    let Some(entry) = photo_history::entry_for_date(displayed_date)? else {
        return Ok(None);
    };
    if !render_cached_photo(config, &entry.sha256, reason)? {
        return Ok(None);
    }
    let image_source = if displayed_date == current_date {
        "current_cached"
    } else {
        "history_cached"
    };
    Ok(Some(apply_displayed_history_state(
        config,
        &entry.sha256,
        displayed_date,
        current_date,
        image_source,
    )))
}

#[cfg(target_os = "espidf")]
fn connect_best_wifi<S: Storage>(
    storage: &mut S,
    config: &mut DeviceRuntimeConfig,
) -> Result<Option<String>, String> {
    use crate::wifi::EspWifiManager;

    if !config.has_wifi_credentials() {
        return Err("missing wifi credentials".into());
    }

    let hostname = if config.device_id.is_empty() {
        "photoframe-rs"
    } else {
        &config.device_id
    };

    EspWifiManager::init_once(hostname)?;

    for profile_index in config.wifi_connection_order() {
        let profile = &config.wifi_profiles[profile_index];
        crate::device_log!(
            "INFO",
            "photoframe-rs: wifi try idx={} ssid={}",
            profile_index,
            profile.ssid
        );
        match EspWifiManager::connect(hostname, &profile.ssid, &profile.password, 25, 5) {
            Ok(()) => {
                let sta_ip = EspWifiManager::sta_ip_string();
                crate::device_log!(
                    "INFO",
                    "photoframe-rs: wifi connected idx={} ssid={} ip={}",
                    profile_index,
                    profile.ssid,
                    sta_ip.clone().unwrap_or_else(|| "-".into())
                );
                config.primary_wifi_ssid = profile.ssid.clone();
                config.primary_wifi_password = profile.password.clone();
                config.last_connected_wifi_index = Some(profile_index);
                if let Err(err) = storage.save_config(config) {
                    println!("photoframe-rs: save connected profile failed: {err}");
                }
                return Ok(sta_ip);
            }
            Err(err) => {
                crate::device_log!(
                    "WARN",
                    "photoframe-rs: wifi connect failed idx={} err={}",
                    profile_index,
                    err
                );
                EspWifiManager::stop();
            }
        }
    }

    Ok(None)
}

#[cfg(target_os = "espidf")]
fn build_remote_history_url(config: &DeviceRuntimeConfig, date_text: &str) -> String {
    format!(
        "{}/api/v1/device/history/daily.{}?device_id={}&date={}",
        config.orchestrator_base_url.trim_end_matches('/'),
        history_fetch_output_format(),
        config.device_id,
        date_text
    )
}

#[cfg(target_os = "espidf")]
fn fetch_history_date_via_orchestrator<S: Storage>(
    storage: &mut S,
    config: &mut DeviceRuntimeConfig,
    displayed_date: &str,
    current_date: &str,
) -> Result<Option<LocalPhotoOutcome>, String> {
    if !config.orchestrator_enabled
        || config.orchestrator_base_url.trim().is_empty()
        || config.orchestrator_token.trim().is_empty()
    {
        crate::device_log!(
            "WARN",
            "photoframe-rs: skip remote history fetch date={} orchestrator unavailable",
            displayed_date
        );
        return Ok(LocalPhotoOutcome::Unavailable {
            image_source: "history_remote_unavailable".into(),
        })
        .map(Some);
    }

    if connect_best_wifi(storage, config)?.is_none() {
        crate::device_log!(
            "WARN",
            "photoframe-rs: skip remote history fetch date={} wifi unavailable",
            displayed_date
        );
        return Ok(Some(LocalPhotoOutcome::Unavailable {
            image_source: "history_wifi_unavailable".into(),
        }));
    }

    let url = build_remote_history_url(config, displayed_date);
    let mut fetcher = photoframe_platform_espidf::EspIdfImageFetcher;
    let result = fetcher.fetch(&ImageFetchPlan {
        device_id: config.device_id.clone(),
        url: url.clone(),
        debug_stage_base_url: config.orchestrator_base_url.clone(),
        previous_sha256: String::new(),
        photo_token: config.photo_token.clone(),
        orchestrator_token: config.orchestrator_token.clone(),
        previous_etag: None,
        previous_last_modified: None,
    });
    if !result.ok {
        crate::device_log!(
            "WARN",
            "photoframe-rs: remote history fetch failed date={} status={} err={} url={}",
            displayed_date,
            result.status_code,
            result.error,
            url
        );
        return Ok(Some(LocalPhotoOutcome::Unavailable {
            image_source: "history_remote_missing".into(),
        }));
    }
    let Some(artifact) = result.artifact.as_ref() else {
        crate::device_log!(
            "WARN",
            "photoframe-rs: remote history fetch missing artifact date={} url={}",
            displayed_date,
            url
        );
        return Ok(Some(LocalPhotoOutcome::Unavailable {
            image_source: "history_remote_missing".into(),
        }));
    };

    let mut display = DeviceDisplay;
    display
        .render(artifact, config, false)
        .map_err(|err| format!("render remote history photo failed: {err:?}"))?;
    if let Err(err) =
        photo_history::remember_rendered_photo(artifact, &result.sha256, Some(displayed_date))
    {
        crate::device_log!(
            "WARN",
            "photoframe-rs: cache remote history photo failed sha={} date={} err={}",
            result.sha256,
            displayed_date,
            err
        );
    }

    crate::device_log!(
        "INFO",
        "photoframe-rs: remote history fetched date={} sha={} url={}",
        displayed_date,
        result.sha256,
        url
    );
    Ok(Some(apply_displayed_history_state(
        config,
        &result.sha256,
        displayed_date,
        current_date,
        if displayed_date == current_date {
            "current_remote"
        } else {
            "history_remote"
        },
    )))
}

#[cfg(target_os = "espidf")]
fn show_history_date<S: Storage>(
    storage: &mut S,
    config: &mut DeviceRuntimeConfig,
    displayed_date: &str,
    current_date: &str,
    cache_reason: &str,
) -> Result<LocalPhotoOutcome, String> {
    if let Some(outcome) =
        render_history_date_from_cache(config, displayed_date, current_date, cache_reason)?
    {
        return Ok(outcome);
    }
    if let Some(outcome) =
        fetch_history_date_via_orchestrator(storage, config, displayed_date, current_date)?
    {
        return Ok(outcome);
    }
    Ok(LocalPhotoOutcome::Unavailable {
        image_source: "history_missing".into(),
    })
}

#[cfg(target_os = "espidf")]
fn cycle_cached_history_photo<S: Storage>(
    storage: &mut S,
    config: &mut DeviceRuntimeConfig,
) -> Result<LocalPhotoOutcome, String> {
    let current_date = current_orchestrator_date(config)?;
    let displayed_date = displayed_history_date(config, &current_date)?;
    let target_date = next_history_target_date(&current_date, &displayed_date)?;
    show_history_date(storage, config, &target_date, &current_date, "key_cycle")
}

#[cfg(target_os = "espidf")]
fn show_current_orchestrator_photo<S: Storage>(
    storage: &mut S,
    config: &mut DeviceRuntimeConfig,
) -> Result<LocalPhotoOutcome, String> {
    let current_date = current_orchestrator_date(config)?;
    if config.last_image_sha256.trim().is_empty() {
        crate::device_log!(
            "WARN",
            "photoframe-rs: current orchestrator sha unknown, try remote current date fetch"
        );
    }
    show_history_date(
        storage,
        config,
        &current_date,
        &current_date,
        "key_long_current",
    )
}

#[cfg(target_os = "espidf")]
fn maybe_handle_local_button_action<S: Storage>(
    storage: &mut S,
    wake_source: WakeSource,
    long_press_action: LongPressAction,
) -> Result<Option<CycleReport>, String> {
    if matches!(long_press_action, LongPressAction::EnterApPortal) {
        return Ok(Some(CycleReport {
            exit: CycleExit::EnterApPortal,
            action: CycleAction::ManualSync,
            image_source: "portal".into(),
            fetch_url_used: None,
            checkin_reported: false,
            portal_window_opened: false,
            logs_uploaded: false,
        }));
    }

    let wants_show_current = matches!(long_press_action, LongPressAction::ShowCurrentPhoto);
    let wants_cycle_history = matches!(wake_source, WakeSource::Key)
        && matches!(long_press_action, LongPressAction::None);
    if !wants_show_current && !wants_cycle_history {
        return Ok(None);
    }

    let mut config = storage.load_config()?;
    let sleep_seconds = regular_sleep_seconds(&config);
    let outcome = if wants_show_current {
        show_current_orchestrator_photo(storage, &mut config)?
    } else {
        cycle_cached_history_photo(storage, &mut config)?
    };
    storage.save_config(&config)?;

    let image_source = match outcome {
        LocalPhotoOutcome::Rendered { image_source } => image_source,
        LocalPhotoOutcome::Unavailable { image_source } => image_source,
    };

    Ok(Some(CycleReport {
        exit: CycleExit::Sleep {
            seconds: sleep_seconds,
            timer_only: false,
        },
        action: CycleAction::BrowseHistory,
        image_source,
        fetch_url_used: None,
        checkin_reported: false,
        portal_window_opened: false,
        logs_uploaded: false,
    }))
}

#[cfg(target_os = "espidf")]
fn main() {
    esp_idf_sys::link_patches();
    power::prepare_for_boot();
    let mut sd_history_ready = false;
    if !power::ensure_ready_for_sdcard() {
        println!("photoframe-rs: sdcard power prepare failed");
    }
    match sdcard::mount_if_available() {
        Ok(ready) => {
            if ready {
                println!(
                    "photoframe-rs: sdcard log storage ready at {}",
                    sdcard::mount_path()
                );
            }
            sd_history_ready = ready;
        }
        Err(first_err) => {
            println!("photoframe-rs: sdcard mount first attempt failed: {first_err}");
            if power::recover_sdcard_power() {
                match sdcard::mount_if_available() {
                    Ok(ready) => {
                        if ready {
                            println!(
                                "photoframe-rs: sdcard log storage ready at {} after power recovery",
                                sdcard::mount_path()
                            );
                        }
                        sd_history_ready = ready;
                    }
                    Err(second_err) => {
                        println!("photoframe-rs: sdcard mount unavailable: {second_err}");
                    }
                }
            } else {
                println!(
                    "photoframe-rs: sdcard mount unavailable: recovery skipped after first failure"
                );
            }
        }
    }
    diag::begin_boot_session(sd_history_ready);
    photoframe_platform_espidf::register_diag_log_sink(diag::append_external);

    use crate::wifi::EspWifiManager;
    use photoframe_platform_espidf::{
        EspIdfClock, EspIdfFirmwareUpdater, EspIdfImageFetcher, EspIdfOrchestratorApi,
        EspIdfStorage,
    };

    configure_button_gpio();
    let reset_reason = photoframe_platform_espidf::current_reset_reason_label();
    crate::device_log!("INFO", "photoframe-rs: reset reason={}", reset_reason);
    let wake_source = current_wake_source();
    if let Some(stage) = runtime_bridge::take_render_trace() {
        crate::device_log!("INFO", "photoframe-rs: previous render trace={stage}");
    }
    let long_press_action = detect_long_press_action();

    let mut storage = match EspIdfStorage::new() {
        Ok(storage) => storage,
        Err(err) => {
            println!("photoframe-rs: storage init failed: {err}");
            idle_forever();
        }
    };

    let mut config = match storage.load_config() {
        Ok(config) => config,
        Err(err) => {
            println!("photoframe-rs: config load failed: {err}");
            idle_forever();
        }
    };

    match storage.ensure_device_identity(&mut config) {
        Ok(true) => {
            if let Err(err) = storage.save_config(&config) {
                println!("photoframe-rs: save identity failed: {err}");
            }
        }
        Ok(false) => {}
        Err(err) => {
            println!("photoframe-rs: ensure identity failed: {err}");
        }
    }

    let previous_config = config.clone();
    if migrate_legacy_network_defaults(&mut config) {
        println!(
            "photoframe-rs: migrated legacy network defaults orchestrator_base_url={} image_url_template={}",
            config.orchestrator_base_url, config.image_url_template
        );
    }
    if merge_builtin_wifi_profiles(&mut config, long_press_action) {
        println!(
            "photoframe-rs: ensured built-in wifi profiles count={}",
            config.wifi_profiles.len()
        );
    }
    if config.should_apply_bootstrap_recovery()
        && let Some(payload) = bootstrap_config_from_env()
    {
        let outcome = config.apply_bootstrap_payload(&payload);
        println!(
            "photoframe-rs: applied bootstrap recovery base_url={} has_orch_token={} has_photo_token={} display_changed={}",
            config.orchestrator_base_url,
            i32::from(!config.orchestrator_token.is_empty()),
            i32::from(!config.photo_token.is_empty()),
            i32::from(outcome.display_config_changed),
        );
    }

    config.ensure_primary_wifi_in_profiles();

    crate::device_log!("INFO", "photoframe-rs: panel warmup deferred until render");
    crate::device_log!("INFO", "photoframe-rs: device_id={}", config.device_id);

    if config != previous_config
        && let Err(err) = storage.save_config(&config)
    {
        println!("photoframe-rs: save bootstrap config failed: {err}");
    }

    let clock = EspIdfClock;
    let mut runner = CycleRunner::new_with_services(
        clock,
        storage,
        EspIdfOrchestratorApi,
        EspIdfImageFetcher,
        DeviceDisplay,
        EspIdfFirmwareUpdater,
        diag::DeviceLogUploadCollector,
    );

    let mut skip_usb_dump_once = reset_reason == "sw" && is_usb_serial_connected();
    if skip_usb_dump_once {
        set_usb_console_suppressed(true);
    }
    let mut cycle_wake_source = wake_source;
    let mut cycle_long_press_action = long_press_action;
    loop {
        wait_for_usb_resume_after_log_dump(maybe_dump_logs_for_usb_serial_attach(
            &mut skip_usb_dump_once,
        ));
        let local_report = match maybe_handle_local_button_action(
            runner.storage_mut(),
            cycle_wake_source,
            cycle_long_press_action,
        ) {
            Ok(report) => report,
            Err(err) => {
                crate::device_log!("ERROR", "photoframe-rs: local button action failed: {err}");
                let fallback_sleep_seconds = 5 * 60;
                if is_usb_serial_connected() {
                    crate::device_log!(
                        "INFO",
                        "photoframe-rs: usb debug mode retry after local action failure in {}s",
                        USB_DEBUG_POLL_SECONDS
                    );
                    thread::sleep(Duration::from_secs(USB_DEBUG_POLL_SECONDS));
                    wait_for_usb_resume_after_log_dump(maybe_dump_logs_for_usb_serial_attach(
                        &mut skip_usb_dump_once,
                    ));
                    cycle_wake_source = WakeSource::Other;
                    cycle_long_press_action = LongPressAction::None;
                    continue;
                }
                enter_deep_sleep(fallback_sleep_seconds, false, PreSleepHoldMode::UsbOrSerial);
            }
        };

        let report = if let Some(report) = local_report {
            report
        } else {
            let prepared = match prepare_cycle(
                runner.storage_mut(),
                cycle_wake_source,
                cycle_long_press_action,
            ) {
                Ok(prepared) => prepared,
                Err(err) => {
                    crate::device_log!("ERROR", "photoframe-rs: prepare cycle failed: {err}");
                    let fallback_sleep_seconds = 5 * 60;
                    if is_usb_serial_connected() {
                        crate::device_log!(
                            "INFO",
                            "photoframe-rs: usb debug mode retry after prepare failure in {}s",
                            USB_DEBUG_POLL_SECONDS
                        );
                        EspWifiManager::stop();
                        thread::sleep(Duration::from_secs(USB_DEBUG_POLL_SECONDS));
                        wait_for_usb_resume_after_log_dump(maybe_dump_logs_for_usb_serial_attach(
                            &mut skip_usb_dump_once,
                        ));
                        cycle_wake_source = WakeSource::Other;
                        cycle_long_press_action = LongPressAction::None;
                        continue;
                    }
                    enter_deep_sleep(fallback_sleep_seconds, false, PreSleepHoldMode::UsbOrSerial);
                }
            };

            match prepared {
                PreparedCycle::Ready(boot) => match runner.run(boot) {
                    Ok(report) => report,
                    Err(err) => {
                        crate::device_log!("ERROR", "photoframe-rs: cycle failed: {err}");
                        EspWifiManager::stop();
                        let fallback_sleep_seconds = 5 * 60;
                        if is_usb_serial_connected() {
                            crate::device_log!(
                                "INFO",
                                "photoframe-rs: usb debug mode retry after cycle failure in {}s",
                                USB_DEBUG_POLL_SECONDS
                            );
                            thread::sleep(Duration::from_secs(USB_DEBUG_POLL_SECONDS));
                            wait_for_usb_resume_after_log_dump(
                                maybe_dump_logs_for_usb_serial_attach(&mut skip_usb_dump_once),
                            );
                            cycle_wake_source = WakeSource::Other;
                            cycle_long_press_action = LongPressAction::None;
                            continue;
                        }
                        enter_deep_sleep(
                            fallback_sleep_seconds,
                            false,
                            PreSleepHoldMode::UsbOrSerial,
                        );
                    }
                },
                PreparedCycle::EnterApPortal => enter_ap_portal_or_idle(),
                PreparedCycle::RetryLater { sleep_seconds } => {
                    EspWifiManager::stop();
                    if is_usb_serial_connected() {
                        crate::device_log!(
                            "INFO",
                            "photoframe-rs: usb debug mode retry after wifi failure in {}s",
                            USB_DEBUG_POLL_SECONDS
                        );
                        thread::sleep(Duration::from_secs(USB_DEBUG_POLL_SECONDS));
                        wait_for_usb_resume_after_log_dump(maybe_dump_logs_for_usb_serial_attach(
                            &mut skip_usb_dump_once,
                        ));
                        cycle_wake_source = WakeSource::Other;
                        cycle_long_press_action = LongPressAction::None;
                        continue;
                    }
                    enter_deep_sleep(sleep_seconds, false, PreSleepHoldMode::UsbOrSerial);
                }
            }
        };

        crate::device_log!(
            "INFO",
            "photoframe-rs: cycle exit={:?} source={} checkin_reported={} logs_uploaded={}",
            report.exit,
            report.image_source,
            report.checkin_reported,
            report.logs_uploaded
        );
        EspWifiManager::stop();
        match report.exit {
            CycleExit::EnterApPortal => enter_ap_portal_or_idle(),
            CycleExit::RebootForConfig => restart_device(),
            CycleExit::RebootForFirmwareUpdate => restart_device(),
            CycleExit::Sleep {
                seconds,
                timer_only,
            } => {
                if is_usb_serial_connected() {
                    crate::device_log!(
                        "INFO",
                        "photoframe-rs: usb debug mode active, rerun cycle in {}s (planned_sleep={}s)",
                        USB_DEBUG_POLL_SECONDS,
                        seconds
                    );
                    thread::sleep(Duration::from_secs(USB_DEBUG_POLL_SECONDS));
                    wait_for_usb_resume_after_log_dump(maybe_dump_logs_for_usb_serial_attach(
                        &mut skip_usb_dump_once,
                    ));
                    cycle_wake_source = WakeSource::Other;
                    cycle_long_press_action = LongPressAction::None;
                    continue;
                }
                let hold_mode = if matches!(report.action, CycleAction::ManualSync) {
                    PreSleepHoldMode::ManualSyncSerialGrace {
                        wait_seconds: MANUAL_SYNC_SERIAL_GRACE_SECONDS,
                    }
                } else {
                    PreSleepHoldMode::UsbOrSerial
                };
                enter_deep_sleep(seconds, timer_only, hold_mode);
            }
        }
    }
}

#[cfg(target_os = "espidf")]
fn prepare_cycle<S: Storage>(
    storage: &mut S,
    wake_source: WakeSource,
    long_press_action: LongPressAction,
) -> Result<PreparedCycle, String> {
    let mut config = storage.load_config()?;
    config.ensure_primary_wifi_in_profiles();

    if !config.has_wifi_credentials() {
        crate::device_log!(
            "WARN",
            "photoframe-rs: missing wifi credentials, entering AP portal"
        );
        return Ok(PreparedCycle::EnterApPortal);
    }

    let Some(sta_ip) = connect_best_wifi(storage, &mut config)? else {
        let decision = apply_cycle_outcome(
            &config.retry_policy(),
            config.failure_count,
            FailureKind::GeneralFailure,
        );
        config.failure_count = decision.next_failure_count;
        if let Err(err) = storage.save_config(&config) {
            println!("photoframe-rs: save wifi failure state failed: {err}");
        }
        crate::device_log!(
            "WARN",
            "photoframe-rs: wifi connect failed for all profiles, sleep={}s",
            decision.sleep_seconds
        );
        let sleep_seconds = sleep_seconds_until_next_beijing_sync(system_now_epoch())
            .unwrap_or(decision.sleep_seconds);
        return Ok(PreparedCycle::RetryLater { sleep_seconds });
    };

    apply_timezone(&config.timezone);
    let time_before_sync = system_now_epoch();
    if should_sync_time(time_before_sync, config.last_time_sync_epoch)
        && sync_time(&config.timezone)
    {
        config.last_time_sync_epoch = system_now_epoch();
        if let Err(err) = storage.save_config(&config) {
            println!("photoframe-rs: save time sync epoch failed: {err}");
        }
    }
    let mut power_sample =
        runtime_bridge::EspRuntimeBridge::read_power_sample().unwrap_or_default();
    apply_test_power_override(&mut power_sample);

    Ok(PreparedCycle::Ready(BootContext {
        wake_source,
        long_press_action,
        sta_ip: Some(sta_ip),
        power_sample,
    }))
}

#[cfg(target_os = "espidf")]
fn system_now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(target_os = "espidf")]
fn apply_timezone(timezone: &str) {
    if timezone.is_empty() {
        return;
    }
    let key = CString::new("TZ").expect("TZ env key");
    let value = match CString::new(timezone) {
        Ok(value) => value,
        Err(err) => {
            crate::device_log!("WARN", "photoframe-rs: invalid timezone, skip apply: {err}");
            return;
        }
    };
    unsafe {
        let _ = esp_idf_sys::setenv(key.as_ptr(), value.as_ptr(), 1);
        esp_idf_sys::tzset();
    }
}

#[cfg(target_os = "espidf")]
fn sync_time(timezone: &str) -> bool {
    apply_timezone(timezone);

    static POOL_NTP: &[u8] = b"pool.ntp.org\0";
    static CLOUDFLARE_NTP: &[u8] = b"time.cloudflare.com\0";

    unsafe {
        esp_idf_sys::esp_sntp_stop();
        esp_idf_sys::esp_sntp_setoperatingmode(
            esp_idf_sys::esp_sntp_operatingmode_t_ESP_SNTP_OPMODE_POLL,
        );
        esp_idf_sys::esp_sntp_setservername(0, POOL_NTP.as_ptr());
        esp_idf_sys::esp_sntp_setservername(1, CLOUDFLARE_NTP.as_ptr());
        esp_idf_sys::esp_sntp_init();
    }

    for _ in 0..20 {
        let now = system_now_epoch();
        if now > 1_735_689_600 {
            crate::device_log!("INFO", "photoframe-rs: time synced epoch={now}");
            unsafe {
                esp_idf_sys::esp_sntp_stop();
            }
            return true;
        }
        thread::sleep(Duration::from_millis(500));
    }

    crate::device_log!(
        "WARN",
        "photoframe-rs: time sync timeout, continue with current rtc time"
    );
    unsafe {
        esp_idf_sys::esp_sntp_stop();
    }
    false
}

#[cfg(target_os = "espidf")]
fn is_usb_serial_connected() -> bool {
    unsafe { esp_idf_sys::usb_serial_jtag_is_connected() }
}

fn should_hold_awake_for_usb_or_serial(usb_serial_connected: bool) -> bool {
    usb_serial_connected
}

#[cfg(target_os = "espidf")]
fn set_usb_console_suppressed(suppressed: bool) {
    static ESP_LOG_TAG_ALL: &[u8] = b"*\0";

    diag::set_console_stdout_suppressed(suppressed);
    let level = if suppressed {
        esp_idf_sys::esp_log_level_t_ESP_LOG_NONE
    } else {
        esp_idf_sys::esp_log_level_t_ESP_LOG_INFO
    };
    unsafe {
        esp_idf_sys::esp_log_level_set(ESP_LOG_TAG_ALL.as_ptr() as *const _, level);
    }
}

#[cfg(target_os = "espidf")]
fn maybe_dump_logs_for_usb_serial_attach(skip_once: &mut bool) -> bool {
    static USB_SERIAL_CONNECTED_LAST: AtomicBool = AtomicBool::new(false);

    let connected = is_usb_serial_connected();
    let was_connected = USB_SERIAL_CONNECTED_LAST.swap(connected, Ordering::SeqCst);
    if !connected || was_connected {
        return false;
    }

    if *skip_once {
        *skip_once = false;
        crate::device_log!(
            "INFO",
            "photoframe-rs: skip usb tf dump once after resume reboot"
        );
        return false;
    }

    set_usb_console_suppressed(false);
    crate::device_log!(
        "INFO",
        "photoframe-rs: usb serial attached, dumping tf history lines<={} bytes<={}",
        USB_DEBUG_LOG_DUMP_MAX_LINES,
        USB_DEBUG_LOG_DUMP_MAX_BYTES
    );
    let dumped = diag::emit_serial_dump(
        "usb_attach",
        USB_DEBUG_LOG_DUMP_MAX_LINES,
        USB_DEBUG_LOG_DUMP_MAX_BYTES,
    );
    crate::device_log!(
        "INFO",
        "photoframe-rs: usb serial tf history dump finished dumped={}",
        i32::from(dumped)
    );
    dumped
}

#[cfg(target_os = "espidf")]
fn wait_for_usb_resume_after_log_dump(dumped: bool) {
    if !dumped {
        return;
    }

    crate::device_log!(
        "INFO",
        "photoframe-rs: usb log dump complete, wait for host resume before wifi cycle timeout={}ms",
        USB_DEBUG_RESUME_WAIT_MS
    );
    let deadline = Instant::now() + Duration::from_millis(USB_DEBUG_RESUME_WAIT_MS);
    let mut read_buf = [0u8; 64];
    let mut pending = Vec::with_capacity(USB_DEBUG_RESUME_TOKEN.len() * 2);
    let mut driver_cfg = esp_idf_sys::usb_serial_jtag_driver_config_t {
        tx_buffer_size: 256,
        rx_buffer_size: 256,
    };
    let driver_installed =
        unsafe { esp_idf_sys::usb_serial_jtag_driver_install(&mut driver_cfg) } == 0;
    if driver_installed {
        unsafe {
            esp_idf_sys::esp_vfs_usb_serial_jtag_use_driver();
        }
    } else {
        crate::device_log!(
            "WARN",
            "photoframe-rs: usb serial driver install failed, fallback to timeout"
        );
    }

    let mut resumed = false;
    while driver_installed && Instant::now() < deadline {
        let read = unsafe {
            esp_idf_sys::usb_serial_jtag_read_bytes(
                read_buf.as_mut_ptr() as *mut _,
                read_buf.len() as u32,
                0,
            )
        };
        if read > 0 {
            let read = read as usize;
            pending.extend_from_slice(&read_buf[..read]);
            if pending
                .windows(USB_DEBUG_RESUME_TOKEN.len())
                .any(|window| window == USB_DEBUG_RESUME_TOKEN)
            {
                resumed = true;
                break;
            }
            if pending.len() > USB_DEBUG_RESUME_TOKEN.len() * 2 {
                let keep_from = pending.len() - USB_DEBUG_RESUME_TOKEN.len();
                pending.drain(..keep_from);
            }
        }

        thread::sleep(Duration::from_millis(USB_DEBUG_RESUME_POLL_MS));
    }

    if driver_installed {
        unsafe {
            esp_idf_sys::esp_vfs_usb_serial_jtag_use_nonblocking();
            let _ = esp_idf_sys::usb_serial_jtag_driver_uninstall();
        }
    }

    if resumed {
        crate::device_log!(
            "INFO",
            "photoframe-rs: usb host resume received, continue wifi cycle"
        );
    } else {
        crate::device_log!(
            "WARN",
            "photoframe-rs: usb host resume timeout, continue wifi cycle"
        );
    }
    set_usb_console_suppressed(true);
    thread::sleep(Duration::from_millis(USB_DEBUG_POST_DUMP_SETTLE_MS));
    crate::device_log!(
        "INFO",
        "photoframe-rs: usb dump handshake complete, reboot before wifi cycle"
    );
    restart_device();
}

#[cfg(target_os = "espidf")]
fn hold_awake_before_sleep(
    planned_sleep_seconds: u64,
    timer_only: bool,
    hold_mode: PreSleepHoldMode,
) {
    const HOLD_LOOP_SLEEP_MS: u64 = 100;
    const POWER_SAMPLE_PERIOD: Duration = Duration::from_secs(3);
    const HOLD_LOG_PERIOD: Duration = Duration::from_secs(10);
    let mut power_sample =
        runtime_bridge::EspRuntimeBridge::read_power_sample().unwrap_or_default();
    let mut power_sample_failures = 0usize;
    let mut usb_serial_connected = is_usb_serial_connected();
    let mut usb_power_present = power_sample.vbus_good == 1;
    let mut serial_seen = usb_serial_connected;
    let grace_deadline = match hold_mode {
        PreSleepHoldMode::UsbOrSerial => None,
        PreSleepHoldMode::ManualSyncSerialGrace { wait_seconds } => {
            Some(Instant::now() + Duration::from_secs(wait_seconds.max(1)))
        }
    };

    match hold_mode {
        PreSleepHoldMode::UsbOrSerial => {
            if !should_hold_awake_for_usb_or_serial(usb_serial_connected) {
                return;
            }
            crate::device_log!(
                "INFO",
                "photoframe-rs: usb serial attached (serial={} vbus={}), skip {} deep sleep (planned {}s)",
                i32::from(usb_serial_connected),
                i32::from(usb_power_present),
                if timer_only { "timer-only" } else { "normal" },
                planned_sleep_seconds,
            );
        }
        PreSleepHoldMode::ManualSyncSerialGrace { wait_seconds } => {
            crate::device_log!(
                "INFO",
                "photoframe-rs: manual sync complete, keep awake {}s for usb serial attach",
                wait_seconds.max(1),
            );
        }
    }

    let mut last_power_sample_at = Instant::now() - POWER_SAMPLE_PERIOD;
    let mut last_log = Instant::now() - HOLD_LOG_PERIOD;
    let mut skip_usb_dump_once = false;
    loop {
        usb_serial_connected = is_usb_serial_connected();
        maybe_dump_logs_for_usb_serial_attach(&mut skip_usb_dump_once);
        serial_seen |= usb_serial_connected;
        if last_power_sample_at.elapsed() >= POWER_SAMPLE_PERIOD {
            match runtime_bridge::EspRuntimeBridge::read_power_sample() {
                Some(sample) => {
                    power_sample = sample;
                    power_sample_failures = 0;
                }
                None => {
                    power_sample_failures = power_sample_failures.saturating_add(1);
                }
            }
            last_power_sample_at = Instant::now();
        }
        match hold_mode {
            PreSleepHoldMode::UsbOrSerial => {
                if !should_hold_awake_for_usb_or_serial(usb_serial_connected) {
                    break;
                }
            }
            PreSleepHoldMode::ManualSyncSerialGrace { .. } => {
                if usb_serial_connected {
                    // 串口调试已接入时持续保持唤醒，直到用户断开。
                } else if serial_seen {
                    break;
                } else if let Some(deadline) = grace_deadline
                    && Instant::now() >= deadline
                {
                    break;
                }
            }
        }

        if last_log.elapsed() >= HOLD_LOG_PERIOD {
            crate::device_log!(
                "INFO",
                "photoframe-rs: usb hold batt={}%%/{}mV charging={} vbus={} next_sleep={}s",
                power_sample.battery_percent,
                power_sample.battery_mv,
                power_sample.charging,
                power_sample.vbus_good,
                planned_sleep_seconds,
            );
            last_log = Instant::now();
        }
        thread::sleep(Duration::from_millis(HOLD_LOOP_SLEEP_MS));
    }

    match hold_mode {
        PreSleepHoldMode::UsbOrSerial => {
            crate::device_log!(
                "INFO",
                "photoframe-rs: usb no longer present, resume deep sleep"
            );
        }
        PreSleepHoldMode::ManualSyncSerialGrace { .. } => {
            crate::device_log!(
                "INFO",
                "photoframe-rs: manual sync grace finished, resume deep sleep"
            );
        }
    }
}

#[cfg(target_os = "espidf")]
fn enter_ap_portal_or_idle() -> ! {
    match portal::run_ap_portal_forever() {
        Ok(()) => idle_forever(),
        Err(err) => {
            crate::device_log!("ERROR", "photoframe-rs: ap portal failed: {err}");
            idle_forever();
        }
    }
}

#[cfg(target_os = "espidf")]
fn enter_deep_sleep(seconds: u64, timer_only: bool, hold_mode: PreSleepHoldMode) -> ! {
    hold_awake_before_sleep(seconds, timer_only, hold_mode);
    diag::persist_for_next_boot();
    runtime_bridge::EspRuntimeBridge::prepare_for_sleep();

    unsafe {
        let _ = esp_idf_sys::esp_sleep_disable_wakeup_source(
            esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_ALL,
        );
        let _ = esp_idf_sys::esp_sleep_enable_timer_wakeup(seconds_to_microseconds(seconds));

        if !timer_only {
            let wakeup_pins = (1u64 << KEY_BUTTON) | (1u64 << BOOT_BUTTON);
            let _ = esp_idf_sys::esp_sleep_pd_config(
                esp_idf_sys::esp_sleep_pd_domain_t_ESP_PD_DOMAIN_RTC_PERIPH,
                esp_idf_sys::esp_sleep_pd_option_t_ESP_PD_OPTION_ON,
            );
            let _ = esp_idf_sys::rtc_gpio_pulldown_dis(KEY_BUTTON);
            let _ = esp_idf_sys::rtc_gpio_pullup_en(KEY_BUTTON);
            let _ = esp_idf_sys::rtc_gpio_pulldown_dis(BOOT_BUTTON);
            let _ = esp_idf_sys::rtc_gpio_pullup_en(BOOT_BUTTON);
            let _ = esp_idf_sys::esp_sleep_enable_ext1_wakeup_io(
                wakeup_pins,
                esp_idf_sys::esp_sleep_ext1_wakeup_mode_t_ESP_EXT1_WAKEUP_ANY_LOW,
            );
        }

        thread::sleep(Duration::from_millis(150));
        esp_idf_sys::esp_deep_sleep_start()
    }
}

#[cfg(target_os = "espidf")]
fn restart_device() -> ! {
    diag::persist_for_next_boot();
    unsafe {
        esp_idf_sys::esp_restart();
    }
}

#[cfg(not(target_os = "espidf"))]
fn main() {
    println!("{}", startup_message());
}

#[allow(dead_code)]
fn startup_message() -> &'static str {
    #[cfg(target_os = "espidf")]
    {
        "photoframe-rs firmware entrypoint on espidf target"
    }

    #[cfg(not(target_os = "espidf"))]
    {
        "photoframe-rs firmware host stub; build this crate for the espidf target to run on device"
    }
}

#[cfg(target_os = "espidf")]
fn idle_forever() -> ! {
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(not(target_os = "espidf"))]
fn idle_forever() -> ! {
    panic!("host stub")
}

#[cfg(test)]
mod tests {
    use super::{
        should_hold_awake_for_usb_or_serial, startup_message, wake_source_from_ext1_state,
    };
    use photoframe_domain::WakeSource;

    #[test]
    fn host_message_mentions_espidf_target() {
        let message = startup_message();
        assert!(message.contains("espidf"));
        assert!(message.contains("photoframe-rs"));
    }

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
    fn ext1_without_observed_press_is_spurious() {
        assert_eq!(
            wake_source_from_ext1_state(true, true, false, false),
            WakeSource::SpuriousExt1
        );
    }

    #[test]
    fn usb_hold_requires_real_serial_connection() {
        assert!(should_hold_awake_for_usb_or_serial(true));
        assert!(!should_hold_awake_for_usb_or_serial(false));
    }
}
