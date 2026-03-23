use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceNextResponse {
    pub image_url: String,
    pub source: Option<String>,
    pub poll_after_seconds: Option<u32>,
    pub valid_until_epoch: Option<i64>,
    pub server_epoch: Option<i64>,
    pub device_epoch: Option<i64>,
    pub device_clock_ok: Option<bool>,
    pub effective_epoch: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RemoteWifiProfile {
    pub ssid: String,
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RemoteConfigPatch {
    pub orchestrator_enabled: Option<i32>,
    pub orchestrator_base_url: Option<String>,
    pub orchestrator_token: Option<String>,
    pub image_url_template: Option<String>,
    pub photo_token: Option<String>,
    pub wifi_profiles: Option<Vec<RemoteWifiProfile>>,
    pub interval_minutes: Option<u32>,
    pub retry_base_minutes: Option<u32>,
    pub retry_max_minutes: Option<u32>,
    pub max_failure_before_long_sleep: Option<u32>,
    pub display_rotation: Option<i32>,
    pub color_process_mode: Option<i32>,
    pub dither_mode: Option<i32>,
    pub six_color_tolerance: Option<i32>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeviceConfigPayload {
    pub orchestrator_enabled: Option<i32>,
    pub orchestrator_base_url: Option<String>,
    pub orchestrator_token: Option<String>,
    pub image_url_template: Option<String>,
    pub photo_token: Option<String>,
    pub wifi_profiles: Option<Vec<RemoteWifiProfile>>,
    pub interval_minutes: Option<u32>,
    pub retry_base_minutes: Option<u32>,
    pub retry_max_minutes: Option<u32>,
    pub max_failure_before_long_sleep: Option<u32>,
    pub display_rotation: Option<i32>,
    pub color_process_mode: Option<i32>,
    pub dither_mode: Option<i32>,
    pub six_color_tolerance: Option<i32>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConfigResponse {
    pub device_id: String,
    pub server_epoch: Option<i64>,
    pub device_epoch: Option<i64>,
    pub device_clock_ok: Option<bool>,
    pub effective_epoch: Option<i64>,
    pub config_version: i32,
    pub config: DeviceConfigPayload,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedWifiProfile {
    pub ssid: String,
    pub password_set: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedConfig {
    pub firmware_version: String,
    pub orchestrator_enabled: i32,
    pub orchestrator_base_url: String,
    pub orchestrator_token: String,
    pub image_url_template: String,
    pub photo_token: String,
    pub interval_minutes: u32,
    pub retry_base_minutes: u32,
    pub retry_max_minutes: u32,
    pub max_failure_before_long_sleep: u32,
    pub display_rotation: i32,
    pub color_process_mode: i32,
    pub dither_mode: i32,
    pub six_color_tolerance: i32,
    pub timezone: String,
    pub wifi_profiles: Vec<ReportedWifiProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCheckinRequest {
    pub device_id: String,
    pub checkin_epoch: i64,
    pub next_wakeup_epoch: i64,
    pub sleep_seconds: u64,
    pub poll_interval_seconds: u32,
    pub failure_count: u32,
    pub last_http_status: i32,
    pub fetch_ok: bool,
    pub image_changed: bool,
    pub image_source: String,
    pub last_error: String,
    pub sta_ip: Option<String>,
    pub battery_mv: i32,
    pub battery_percent: i32,
    pub charging: i32,
    pub vbus_good: i32,
    pub reported_config: ReportedConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConfigAppliedRequest {
    pub device_id: String,
    pub config_version: i32,
    pub applied: bool,
    pub error: String,
    pub applied_epoch: i64,
}
