#![cfg_attr(not(target_os = "espidf"), allow(dead_code, unused_imports))]
mod jpeg;
mod panel;
mod portal;
mod power;
mod render_core;
mod runtime_bridge;
mod wifi;

#[cfg(target_os = "espidf")]
use std::{
    ffi::CString,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "espidf")]
use photoframe_app::{
    BootContext, CycleExit, CycleRunner, DeviceRuntimeConfig, Display, PowerSample, Storage,
};
use photoframe_domain::WakeSource;

#[cfg(target_os = "espidf")]
use photoframe_domain::{
    FailureKind, LongPressAction, apply_cycle_outcome, seconds_to_microseconds, should_sync_time,
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
const LEGACY_IMAGE_URL_TEMPLATE: &str = "http://192.168.58.113:8000/image/480x800?date=%DATE%";
#[cfg(target_os = "espidf")]
const LEGACY_ORCHESTRATOR_BASE_URL: &str = "http://192.168.58.113:18081";

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
        return LongPressAction::ClearWifiAndEnterPortal;
    }
    if is_key_pressed() {
        return LongPressAction::OpenStaPortalWindow;
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

    if config.orchestrator_base_url == LEGACY_ORCHESTRATOR_BASE_URL {
        config.orchestrator_base_url = defaults.orchestrator_base_url;
        changed = true;
    }
    if config.image_url_template == LEGACY_IMAGE_URL_TEMPLATE {
        config.image_url_template = defaults.image_url_template;
        changed = true;
    }

    changed
}

#[cfg(target_os = "espidf")]
fn merge_builtin_wifi_profiles(
    config: &mut DeviceRuntimeConfig,
    long_press_action: LongPressAction,
) -> bool {
    if matches!(long_press_action, LongPressAction::ClearWifiAndEnterPortal) {
        return false;
    }

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
fn current_wake_source() -> WakeSource {
    match unsafe { esp_idf_sys::esp_sleep_get_wakeup_cause() } {
        esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TIMER => {
            println!("photoframe-rs: wakeup cause=TIMER");
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

            println!(
                "photoframe-rs: wakeup cause=EXT1 pins=0x{pins:x} key={} boot={} seen_low(key={} boot={})",
                key_level,
                boot_level,
                i32::from(key_seen_low),
                i32::from(boot_seen_low),
            );

            wake_source_from_ext1_state(boot_pin, key_pin, boot_seen_low, key_seen_low)
        }
        other => {
            println!("photoframe-rs: wakeup cause=OTHER({other})");
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
}

#[cfg(target_os = "espidf")]
fn main() {
    esp_idf_sys::link_patches();

    use crate::wifi::EspWifiManager;
    use photoframe_platform_espidf::{
        EspIdfClock, EspIdfImageFetcher, EspIdfOrchestratorApi, EspIdfStorage,
    };

    configure_button_gpio();
    let wake_source = current_wake_source();
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

    if matches!(long_press_action, LongPressAction::ClearWifiAndEnterPortal) {
        config.clear_wifi_credentials();
        if let Err(err) = storage.save_config(&config) {
            println!("photoframe-rs: clear wifi failed: {err}");
        }
    }

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

    config.ensure_primary_wifi_in_profiles();
    println!("photoframe-rs: device_id={}", config.device_id);

    if config != previous_config && let Err(err) = storage.save_config(&config) {
        println!("photoframe-rs: save bootstrap config failed: {err}");
    }

    if !config.has_wifi_credentials() {
        println!("photoframe-rs: missing wifi credentials, entering AP portal");
        enter_ap_portal_or_idle();
    }

    let hostname = if config.device_id.is_empty() {
        "photoframe-rs"
    } else {
        &config.device_id
    };

    if let Err(err) = EspWifiManager::init_once(hostname) {
        println!("photoframe-rs: wifi init failed: {err}");
        idle_forever();
    }

    let mut connected = false;
    for profile_index in config.wifi_connection_order() {
        let profile = &config.wifi_profiles[profile_index];
        println!(
            "photoframe-rs: wifi try idx={} ssid={}",
            profile_index, profile.ssid
        );
        match EspWifiManager::connect(hostname, &profile.ssid, &profile.password, 25, 5) {
            Ok(()) => {
                println!(
                    "photoframe-rs: wifi connected idx={} ssid={} ip={}",
                    profile_index,
                    profile.ssid,
                    EspWifiManager::sta_ip_string().unwrap_or_else(|| "-".into())
                );
                config.primary_wifi_ssid = profile.ssid.clone();
                config.primary_wifi_password = profile.password.clone();
                config.last_connected_wifi_index = Some(profile_index);
                if let Err(err) = storage.save_config(&config) {
                    println!("photoframe-rs: save connected profile failed: {err}");
                }
                connected = true;
                break;
            }
            Err(err) => {
                println!("photoframe-rs: wifi connect failed idx={} err={}", profile_index, err);
                EspWifiManager::stop();
            }
        }
    }

    if !connected {
        let decision = apply_cycle_outcome(
            &config.retry_policy(),
            config.failure_count,
            FailureKind::GeneralFailure,
        );
        config.failure_count = decision.next_failure_count;
        if let Err(err) = storage.save_config(&config) {
            println!("photoframe-rs: save wifi failure state failed: {err}");
        }
        println!(
            "photoframe-rs: wifi connect failed for all profiles, sleep={}s",
            decision.sleep_seconds
        );
        enter_deep_sleep(decision.sleep_seconds, false);
    }

    apply_timezone(&config.timezone);
    let time_before_sync = system_now_epoch();
    if should_sync_time(time_before_sync, config.last_time_sync_epoch) && sync_time(&config.timezone)
    {
        config.last_time_sync_epoch = system_now_epoch();
        if let Err(err) = storage.save_config(&config) {
            println!("photoframe-rs: save time sync epoch failed: {err}");
        }
    }
    let portal_power_sample = runtime_bridge::EspRuntimeBridge::read_power_sample()
        .unwrap_or_else(|| PowerSample::default());
    if matches!(long_press_action, LongPressAction::OpenStaPortalWindow) {
        if let Err(err) = portal::run_sta_portal_window(portal_power_sample, false) {
            println!("photoframe-rs: sta portal window failed: {err}");
        }
    }

    let power_sample = runtime_bridge::EspRuntimeBridge::read_power_sample()
        .unwrap_or(portal_power_sample);
    let sta_ip = EspWifiManager::sta_ip_string();

    let clock = EspIdfClock;
    let boot = BootContext {
        wake_source,
        long_press_action,
        sta_ip,
        power_sample,
    };

    let mut runner = CycleRunner::new(
        clock,
        storage,
        EspIdfOrchestratorApi,
        EspIdfImageFetcher,
        DeviceDisplay,
    );

    match runner.run(boot) {
        Ok(report) => {
            println!(
                "photoframe-rs: cycle exit={:?} source={}",
                report.exit, report.image_source
            );
            EspWifiManager::stop();
            match report.exit {
                CycleExit::EnterApPortal => enter_ap_portal_or_idle(),
                CycleExit::RebootForConfig => unsafe {
                    esp_idf_sys::esp_restart();
                },
                CycleExit::Sleep {
                    seconds,
                    timer_only,
                } => {
                    enter_deep_sleep(seconds, timer_only);
                }
            }
        }
        Err(err) => {
            println!("photoframe-rs: cycle failed: {err}");
            EspWifiManager::stop();
            enter_deep_sleep(config.retry_base_minutes.max(1) as u64 * 60, false);
        }
    }
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
            println!("photoframe-rs: invalid timezone, skip apply: {err}");
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
            println!("photoframe-rs: time synced epoch={now}");
            unsafe {
                esp_idf_sys::esp_sntp_stop();
            }
            return true;
        }
        thread::sleep(Duration::from_millis(500));
    }

    println!("photoframe-rs: time sync timeout, continue with current rtc time");
    unsafe {
        esp_idf_sys::esp_sntp_stop();
    }
    false
}

#[cfg(target_os = "espidf")]
fn is_usb_serial_connected() -> bool {
    unsafe { esp_idf_sys::usb_serial_jtag_is_connected() }
}

#[cfg(target_os = "espidf")]
fn hold_awake_while_usb_present(planned_sleep_seconds: u64, timer_only: bool) {
    const HOLD_LOOP_SLEEP_MS: u64 = 100;
    const POWER_SAMPLE_PERIOD: Duration = Duration::from_secs(3);
    const HOLD_LOG_PERIOD: Duration = Duration::from_secs(10);
    const MAX_POWER_SAMPLE_FAILURES: usize = 3;

    let mut power_sample = runtime_bridge::EspRuntimeBridge::read_power_sample().unwrap_or_default();
    let mut power_sample_failures = 0usize;
    let mut usb_serial_connected = is_usb_serial_connected();
    let mut usb_power_present = power_sample.vbus_good == 1;
    if !usb_serial_connected && !usb_power_present {
        return;
    }

    println!(
        "photoframe-rs: usb present (serial={} vbus={}), skip {} deep sleep (planned {}s)",
        i32::from(usb_serial_connected),
        i32::from(usb_power_present),
        if timer_only { "timer-only" } else { "normal" },
        planned_sleep_seconds,
    );

    let mut last_power_sample_at = Instant::now() - POWER_SAMPLE_PERIOD;
    let mut last_log = Instant::now() - HOLD_LOG_PERIOD;
    loop {
        usb_serial_connected = is_usb_serial_connected();
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
        usb_power_present = power_sample.vbus_good == 1;
        if !usb_serial_connected
            && (!usb_power_present || power_sample_failures >= MAX_POWER_SAMPLE_FAILURES)
        {
            break;
        }

        if last_log.elapsed() >= HOLD_LOG_PERIOD {
            println!(
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

    println!("photoframe-rs: usb no longer present, resume deep sleep");
}

#[cfg(target_os = "espidf")]
fn enter_ap_portal_or_idle() -> ! {
    match portal::run_ap_portal_forever() {
        Ok(()) => idle_forever(),
        Err(err) => {
            println!("photoframe-rs: ap portal failed: {err}");
            idle_forever();
        }
    }
}

#[cfg(target_os = "espidf")]
fn enter_deep_sleep(seconds: u64, timer_only: bool) -> ! {
    hold_awake_while_usb_present(seconds, timer_only);
    runtime_bridge::EspRuntimeBridge::prepare_for_sleep();

    unsafe {
        let _ = esp_idf_sys::esp_sleep_disable_wakeup_source(
            esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_ALL,
        );
        let _ =
            esp_idf_sys::esp_sleep_enable_timer_wakeup(seconds_to_microseconds(seconds));

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
    use super::{startup_message, wake_source_from_ext1_state};
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
}
