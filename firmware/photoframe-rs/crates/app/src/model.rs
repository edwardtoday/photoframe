use photoframe_contracts::{
    DeviceConfigPayload, RemoteConfigPatch, ReportedConfig, ReportedWifiProfile,
};
use photoframe_domain::RetryPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiCredential {
    pub ssid: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageFormat {
    Bmp,
    Jpeg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageArtifact {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFetchPlan {
    pub url: String,
    pub previous_sha256: String,
    pub photo_token: String,
    pub orchestrator_token: String,
    pub previous_etag: Option<String>,
    pub previous_last_modified: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFetchOutcome {
    pub ok: bool,
    pub status_code: i32,
    pub error: String,
    pub image_changed: bool,
    pub sha256: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub artifact: Option<ImageArtifact>,
}

impl ImageFetchOutcome {
    pub fn failed(status_code: i32, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            status_code,
            error: error.into(),
            image_changed: false,
            sha256: String::new(),
            etag: None,
            last_modified: None,
            artifact: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerSample {
    pub battery_mv: i32,
    pub battery_percent: i32,
    pub charging: i32,
    pub vbus_good: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerCache {
    pub battery_mv: i32,
    pub battery_percent: i32,
    pub charging: i32,
    pub vbus_good: i32,
    pub cached_epoch: i64,
}

impl Default for PowerSample {
    fn default() -> Self {
        Self {
            battery_mv: -1,
            battery_percent: -1,
            charging: -1,
            vbus_good: -1,
        }
    }
}

impl Default for PowerCache {
    fn default() -> Self {
        Self {
            battery_mv: -1,
            battery_percent: -1,
            charging: -1,
            vbus_good: -1,
            cached_epoch: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NormalizePowerOutcome {
    pub sample: PowerSample,
    pub cache: PowerCache,
    pub used_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApplyRemoteConfigOutcome {
    pub display_config_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApplyLocalConfigOutcome {
    pub wifi_changed: bool,
    pub display_config_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalConfigPatch {
    pub wifi_ssid: Option<String>,
    pub wifi_password: Option<String>,
    pub image_url_template: Option<String>,
    pub orchestrator_enabled: Option<bool>,
    pub orchestrator_base_url: Option<String>,
    pub device_id: Option<String>,
    pub orchestrator_token: Option<String>,
    pub photo_token: Option<String>,
    pub timezone: Option<String>,
    pub interval_minutes: Option<u32>,
    pub retry_base_minutes: Option<u32>,
    pub retry_max_minutes: Option<u32>,
    pub max_failure_before_long_sleep: Option<u32>,
    pub display_rotation: Option<i32>,
    pub color_process_mode: Option<i32>,
    pub dither_mode: Option<i32>,
    pub six_color_tolerance: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRuntimeConfig {
    pub primary_wifi_ssid: String,
    pub primary_wifi_password: String,
    pub wifi_profiles: Vec<WifiCredential>,
    pub last_connected_wifi_index: Option<usize>,
    pub image_url_template: String,
    pub photo_token: String,
    pub orchestrator_enabled: bool,
    pub orchestrator_base_url: String,
    pub device_id: String,
    pub orchestrator_token: String,
    pub timezone: String,
    pub interval_minutes: u32,
    pub retry_base_minutes: u32,
    pub retry_max_minutes: u32,
    pub max_failure_before_long_sleep: u32,
    pub display_rotation: i32,
    pub color_process_mode: i32,
    pub dither_mode: i32,
    pub six_color_tolerance: i32,
    pub last_image_sha256: String,
    pub last_image_etag: String,
    pub last_image_last_modified: String,
    pub preferred_image_origin: String,
    pub last_success_epoch: i64,
    pub last_time_sync_epoch: i64,
    pub failure_count: u32,
    pub remote_config_version: i32,
}

const DEFAULT_IMAGE_URL_TEMPLATE: &str = "http://192.168.58.113:8000/image/480x800?date=%DATE%";
const DEFAULT_ORCHESTRATOR_BASE_URL: &str = "http://192.168.58.113:18081";
const FIRMWARE_BUILD_VERSION: &str = match option_env!("PHOTOFRAME_FIRMWARE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

impl Default for DeviceRuntimeConfig {
    fn default() -> Self {
        Self {
            primary_wifi_ssid: String::new(),
            primary_wifi_password: String::new(),
            wifi_profiles: Vec::new(),
            last_connected_wifi_index: None,
            image_url_template: DEFAULT_IMAGE_URL_TEMPLATE.into(),
            photo_token: String::new(),
            orchestrator_enabled: true,
            orchestrator_base_url: DEFAULT_ORCHESTRATOR_BASE_URL.into(),
            device_id: String::new(),
            orchestrator_token: String::new(),
            timezone: "UTC".into(),
            interval_minutes: 60,
            retry_base_minutes: 5,
            retry_max_minutes: 240,
            max_failure_before_long_sleep: 24,
            display_rotation: 0,
            color_process_mode: 0,
            dither_mode: 1,
            six_color_tolerance: 0,
            last_image_sha256: String::new(),
            last_image_etag: String::new(),
            last_image_last_modified: String::new(),
            preferred_image_origin: String::new(),
            last_success_epoch: 0,
            last_time_sync_epoch: 0,
            failure_count: 0,
            remote_config_version: 0,
        }
    }
}

impl DeviceRuntimeConfig {
    pub const MAX_WIFI_PROFILES: usize = 8;

    pub fn firmware_version(&self) -> &'static str {
        FIRMWARE_BUILD_VERSION
    }

    pub fn should_apply_bootstrap_recovery(&self) -> bool {
        if self.remote_config_version > 0 {
            return false;
        }

        self.orchestrator_base_url.is_empty()
            || self.orchestrator_base_url == DEFAULT_ORCHESTRATOR_BASE_URL
            || self.image_url_template.is_empty()
            || self.image_url_template == DEFAULT_IMAGE_URL_TEMPLATE
            || self.orchestrator_token.is_empty()
            || self.photo_token.is_empty()
    }

    pub fn apply_bootstrap_payload(
        &mut self,
        payload: &DeviceConfigPayload,
    ) -> ApplyRemoteConfigOutcome {
        self.apply_remote_config_patch(&RemoteConfigPatch {
            orchestrator_enabled: payload.orchestrator_enabled,
            orchestrator_base_url: payload.orchestrator_base_url.clone(),
            orchestrator_token: payload.orchestrator_token.clone(),
            image_url_template: payload.image_url_template.clone(),
            photo_token: payload.photo_token.clone(),
            wifi_profiles: payload.wifi_profiles.clone(),
            interval_minutes: payload.interval_minutes,
            retry_base_minutes: payload.retry_base_minutes,
            retry_max_minutes: payload.retry_max_minutes,
            max_failure_before_long_sleep: payload.max_failure_before_long_sleep,
            display_rotation: payload.display_rotation,
            color_process_mode: payload.color_process_mode,
            dither_mode: payload.dither_mode,
            six_color_tolerance: payload.six_color_tolerance,
            timezone: payload.timezone.clone(),
        })
    }

    pub fn has_wifi_credentials(&self) -> bool {
        !self.primary_wifi_ssid.is_empty()
            || self.wifi_profiles.iter().any(|item| !item.ssid.is_empty())
    }

    pub fn clear_wifi_credentials(&mut self) {
        self.primary_wifi_ssid.clear();
        self.primary_wifi_password.clear();
        self.wifi_profiles.clear();
        self.last_connected_wifi_index = None;
    }

    pub fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy {
            interval_minutes: self.interval_minutes,
            retry_base_minutes: self.retry_base_minutes,
            retry_max_minutes: self.retry_max_minutes,
            max_failure_before_long_sleep: self.max_failure_before_long_sleep,
        }
    }

    pub fn ensure_primary_wifi_in_profiles(&mut self) {
        if self.primary_wifi_ssid.is_empty() {
            return;
        }

        if let Some(existing) = self
            .wifi_profiles
            .iter_mut()
            .find(|item| item.ssid == self.primary_wifi_ssid)
        {
            if !self.primary_wifi_password.is_empty() {
                existing.password = self.primary_wifi_password.clone();
            }
            return;
        }

        if self.wifi_profiles.len() < Self::MAX_WIFI_PROFILES {
            self.wifi_profiles.push(WifiCredential {
                ssid: self.primary_wifi_ssid.clone(),
                password: self.primary_wifi_password.clone(),
            });
            return;
        }

        self.wifi_profiles.rotate_left(1);
        if let Some(last) = self.wifi_profiles.last_mut() {
            *last = WifiCredential {
                ssid: self.primary_wifi_ssid.clone(),
                password: self.primary_wifi_password.clone(),
            };
        }
        if let Some(index) = self.last_connected_wifi_index.as_mut()
            && *index > 0
        {
            *index -= 1;
        }
    }

    pub fn wifi_connection_order(&self) -> Vec<usize> {
        let mut indexes = Vec::new();
        let push = |items: &mut Vec<usize>, idx: usize, this: &DeviceRuntimeConfig| {
            if idx >= this.wifi_profiles.len() || this.wifi_profiles[idx].ssid.is_empty() {
                return;
            }
            if !items.contains(&idx) {
                items.push(idx);
            }
        };

        if let Some(index) = self.last_connected_wifi_index {
            push(&mut indexes, index, self);
        }
        for idx in 0..self.wifi_profiles.len() {
            push(&mut indexes, idx, self);
        }
        indexes
    }

    /// 远端配置补丁只允许修改白名单字段，并且严格对齐当前 C++ 固件的 clamp 语义。
    pub fn apply_remote_config_patch(
        &mut self,
        patch: &RemoteConfigPatch,
    ) -> ApplyRemoteConfigOutcome {
        let mut display_config_changed = false;

        if let Some(value) = patch.orchestrator_enabled {
            self.orchestrator_enabled = value != 0;
        }
        if let Some(value) = &patch.orchestrator_base_url {
            self.orchestrator_base_url = value.clone();
        }
        if let Some(value) = &patch.orchestrator_token {
            self.orchestrator_token = value.clone();
        }
        if let Some(value) = &patch.image_url_template {
            self.image_url_template = value.clone();
        }
        if let Some(value) = &patch.photo_token {
            self.photo_token = value.clone();
        }
        if let Some(value) = &patch.timezone {
            self.timezone = value.clone();
        }

        if let Some(wifi_profiles) = &patch.wifi_profiles {
            let previous_last_connected_ssid = self
                .last_connected_wifi_index
                .and_then(|index| self.wifi_profiles.get(index))
                .map(|item| item.ssid.clone());

            let mut next_profiles = Vec::new();
            for item in wifi_profiles.iter().take(Self::MAX_WIFI_PROFILES) {
                let ssid = item.ssid.trim();
                if ssid.is_empty() {
                    continue;
                }
                if next_profiles
                    .iter()
                    .any(|profile: &WifiCredential| profile.ssid == ssid)
                {
                    continue;
                }
                let password = item
                    .password
                    .as_deref()
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        self.wifi_profiles
                            .iter()
                            .find(|profile| profile.ssid == ssid)
                            .map(|profile| profile.password.clone())
                            .unwrap_or_default()
                    });
                next_profiles.push(WifiCredential {
                    ssid: ssid.to_string(),
                    password,
                });
            }

            self.wifi_profiles = next_profiles;
            self.last_connected_wifi_index = previous_last_connected_ssid.and_then(|ssid| {
                self.wifi_profiles
                    .iter()
                    .position(|profile| profile.ssid == ssid)
            });

            if self.wifi_profiles.is_empty() {
                self.primary_wifi_ssid.clear();
                self.primary_wifi_password.clear();
                self.last_connected_wifi_index = None;
            } else if let Some(profile) = self
                .wifi_profiles
                .iter()
                .find(|profile| profile.ssid == self.primary_wifi_ssid)
            {
                self.primary_wifi_password = profile.password.clone();
            } else if let Some(first) = self.wifi_profiles.first() {
                self.primary_wifi_ssid = first.ssid.clone();
                self.primary_wifi_password = first.password.clone();
            }
        }

        if let Some(value) = patch.interval_minutes {
            self.interval_minutes = value.max(1);
        }
        if let Some(value) = patch.retry_base_minutes {
            self.retry_base_minutes = value.max(1);
        }
        if let Some(value) = patch.retry_max_minutes {
            self.retry_max_minutes = value.max(self.retry_base_minutes);
        }
        if let Some(value) = patch.max_failure_before_long_sleep {
            self.max_failure_before_long_sleep = value.max(1);
        }
        if let Some(value) = patch.display_rotation {
            let next = if value == 0 { 0 } else { 2 };
            display_config_changed |= self.display_rotation != next;
            self.display_rotation = next;
        }
        if let Some(value) = patch.color_process_mode {
            let next = value.clamp(0, 2);
            display_config_changed |= self.color_process_mode != next;
            self.color_process_mode = next;
        }
        if let Some(value) = patch.dither_mode {
            let next = value.clamp(0, 1);
            display_config_changed |= self.dither_mode != next;
            self.dither_mode = next;
        }
        if let Some(value) = patch.six_color_tolerance {
            let next = value.clamp(0, 64);
            display_config_changed |= self.six_color_tolerance != next;
            self.six_color_tolerance = next;
        }

        if display_config_changed {
            self.last_image_sha256.clear();
        }

        ApplyRemoteConfigOutcome {
            display_config_changed,
        }
    }

    /// 本地 Portal 配置与当前 C++ 固件保持一致：空 SSID 不清网，空密码不覆盖旧密码。
    pub fn apply_local_config_patch(
        &mut self,
        patch: &LocalConfigPatch,
    ) -> ApplyLocalConfigOutcome {
        let mut wifi_changed = false;
        let mut display_config_changed = false;

        if let Some(ssid) = &patch.wifi_ssid {
            if !(ssid.is_empty() && !self.primary_wifi_ssid.is_empty())
                && self.primary_wifi_ssid != *ssid
            {
                self.primary_wifi_ssid = ssid.clone();
                wifi_changed = true;
            }
        }

        if let Some(password) = &patch.wifi_password
            && !password.is_empty()
            && self.primary_wifi_password != *password
        {
            self.primary_wifi_password = password.clone();
            wifi_changed = true;
        }

        if wifi_changed {
            self.ensure_primary_wifi_in_profiles();
        }

        if let Some(value) = &patch.image_url_template {
            self.image_url_template = value.clone();
        }
        if let Some(value) = patch.orchestrator_enabled {
            self.orchestrator_enabled = value;
        }
        if let Some(value) = &patch.orchestrator_base_url {
            self.orchestrator_base_url = value.clone();
        }
        if let Some(value) = &patch.device_id {
            self.device_id = value.clone();
        }
        if let Some(value) = &patch.orchestrator_token {
            self.orchestrator_token = value.clone();
        }
        if let Some(value) = &patch.photo_token {
            self.photo_token = value.clone();
        }
        if let Some(value) = &patch.timezone {
            self.timezone = value.clone();
        }
        if let Some(value) = patch.interval_minutes {
            self.interval_minutes = value.max(1);
        }
        if let Some(value) = patch.retry_base_minutes {
            self.retry_base_minutes = value.max(1);
        }
        if let Some(value) = patch.retry_max_minutes {
            self.retry_max_minutes = value.max(self.retry_base_minutes);
        }
        if let Some(value) = patch.max_failure_before_long_sleep {
            self.max_failure_before_long_sleep = value.max(1);
        }
        if let Some(value) = patch.display_rotation {
            let next = if value == 0 { 0 } else { 2 };
            display_config_changed |= self.display_rotation != next;
            self.display_rotation = next;
        }
        if let Some(value) = patch.color_process_mode {
            let next = value.clamp(0, 2);
            display_config_changed |= self.color_process_mode != next;
            self.color_process_mode = next;
        }
        if let Some(value) = patch.dither_mode {
            let next = value.clamp(0, 1);
            display_config_changed |= self.dither_mode != next;
            self.dither_mode = next;
        }
        if let Some(value) = patch.six_color_tolerance {
            let next = value.clamp(0, 64);
            display_config_changed |= self.six_color_tolerance != next;
            self.six_color_tolerance = next;
        }

        if display_config_changed {
            self.last_image_sha256.clear();
        }

        ApplyLocalConfigOutcome {
            wifi_changed,
            display_config_changed,
        }
    }

    pub fn to_reported_config(&self) -> ReportedConfig {
        ReportedConfig {
            firmware_version: self.firmware_version().to_string(),
            orchestrator_enabled: i32::from(self.orchestrator_enabled),
            orchestrator_base_url: self.orchestrator_base_url.clone(),
            orchestrator_token: self.orchestrator_token.clone(),
            image_url_template: self.image_url_template.clone(),
            photo_token: self.photo_token.clone(),
            interval_minutes: self.interval_minutes.max(1),
            retry_base_minutes: self.retry_base_minutes.max(1),
            retry_max_minutes: self.retry_max_minutes.max(self.retry_base_minutes.max(1)),
            max_failure_before_long_sleep: self.max_failure_before_long_sleep.max(1),
            display_rotation: if self.display_rotation == 0 { 0 } else { 2 },
            color_process_mode: self.color_process_mode.clamp(0, 2),
            dither_mode: self.dither_mode.clamp(0, 1),
            six_color_tolerance: self.six_color_tolerance.clamp(0, 64),
            timezone: self.timezone.clone(),
            wifi_profiles: self
                .wifi_profiles
                .iter()
                .take(Self::MAX_WIFI_PROFILES)
                .map(|item| ReportedWifiProfile {
                    ssid: item.ssid.clone(),
                    password_set: !item.password.is_empty(),
                })
                .collect(),
        }
    }
}

pub fn normalize_power_sample(
    sample: PowerSample,
    cache: Option<PowerCache>,
) -> NormalizePowerOutcome {
    let mut out = sample;
    let mut next_cache = cache.unwrap_or_default();
    let mut used_cache = false;

    let missing_live_data = out.battery_mv <= 0
        && out.battery_percent < 0
        && !(out.charging == 0 || out.charging == 1)
        && !(out.vbus_good == 0 || out.vbus_good == 1);

    if missing_live_data {
        if next_cache.battery_mv > 0 {
            out.battery_mv = next_cache.battery_mv;
        }
        if next_cache.battery_percent >= 0 {
            out.battery_percent = next_cache.battery_percent;
        }
        if next_cache.charging == 0 || next_cache.charging == 1 {
            out.charging = next_cache.charging;
        }
        if next_cache.vbus_good == 0 || next_cache.vbus_good == 1 {
            out.vbus_good = next_cache.vbus_good;
        }
        used_cache = true;
    }

    let estimated_percent = estimate_battery_percent_from_mv(out.battery_mv);
    let on_battery = out.vbus_good == 0 && out.charging == 0;
    let suspect_percent_stuck_full =
        out.battery_percent >= 100 && out.battery_mv > 0 && out.battery_mv <= 4185;
    let missing_percent = out.battery_percent < 0;
    let suspect_percent_too_high = out.battery_percent >= 0
        && estimated_percent >= 0
        && (out.battery_percent - estimated_percent) >= 8;
    let suspect_high_zone_drift =
        out.battery_percent >= 98 && out.battery_mv > 0 && out.battery_mv <= 4160;
    if on_battery
        && estimated_percent >= 0
        && (suspect_percent_stuck_full
            || missing_percent
            || suspect_percent_too_high
            || suspect_high_zone_drift)
    {
        out.battery_percent = estimated_percent;
    }

    if out.battery_mv > 0 {
        next_cache.battery_mv = out.battery_mv;
    }
    if out.battery_percent >= 0 {
        next_cache.battery_percent = out.battery_percent;
    }
    if out.charging == 0 || out.charging == 1 {
        next_cache.charging = out.charging;
    }
    if out.vbus_good == 0 || out.vbus_good == 1 {
        next_cache.vbus_good = out.vbus_good;
    }

    NormalizePowerOutcome {
        sample: out,
        cache: next_cache,
        used_cache,
    }
}

fn estimate_battery_percent_from_mv(battery_mv: i32) -> i32 {
    const CURVE: &[(i32, i32)] = &[
        (4200, 100),
        (4160, 95),
        (4120, 88),
        (4080, 80),
        (4040, 72),
        (4000, 64),
        (3960, 56),
        (3920, 48),
        (3880, 40),
        (3840, 32),
        (3800, 24),
        (3760, 16),
        (3720, 10),
        (3680, 6),
        (3600, 3),
        (3500, 1),
        (3300, 0),
    ];

    if battery_mv <= CURVE[CURVE.len() - 1].0 {
        return 0;
    }
    if battery_mv >= CURVE[0].0 {
        return 100;
    }

    for pair in CURVE.windows(2) {
        let (high_mv, high_percent) = pair[0];
        let (low_mv, low_percent) = pair[1];
        if battery_mv <= high_mv && battery_mv >= low_mv {
            let span_mv = (high_mv - low_mv).max(1);
            let offset_mv = battery_mv - low_mv;
            let span_percent = high_percent - low_percent;
            let percent = low_percent + (offset_mv * span_percent) / span_mv;
            return percent.clamp(0, 100);
        }
    }

    -1
}
