#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

use photoframe_app::{
    Clock, DeviceRuntimeConfig, Display, FirmwareRuntimeStatus, FirmwareUpdater, ImageArtifact,
    ImageFetchOutcome, ImageFetchPlan, ImageFetcher, OrchestratorApi, Storage,
};
use photoframe_contracts::{
    DeviceCheckinRequest, DeviceLogUploadRequest, DeviceLogUploadRequestBody, DeviceNextResponse,
    FirmwareUpdateDirective,
};
use photoframe_domain::FailureKind;

#[cfg(target_os = "espidf")]
use photoframe_app::{ImageFormat, WifiCredential};
#[cfg(target_os = "espidf")]
use photoframe_contracts::{
    DeviceConfigAppliedRequest, DeviceConfigPayload, DeviceConfigResponse, RemoteConfigPatch,
};
#[cfg(target_os = "espidf")]
use photoframe_domain::{device_id_from_mac_suffix, token_hex_from_bytes};

#[cfg(target_os = "espidf")]
use esp_idf_sys as sys;

#[cfg(target_os = "espidf")]
use std::{
    ffi::{CStr, CString},
    os::raw::c_char,
    ptr,
    sync::OnceLock,
    time::Instant,
};

#[cfg(target_os = "espidf")]
type DiagnosticLogSink = fn(&str, &str);

#[cfg(target_os = "espidf")]
static DIAGNOSTIC_LOG_SINK: OnceLock<DiagnosticLogSink> = OnceLock::new();

#[cfg(target_os = "espidf")]
pub fn register_diag_log_sink(sink: DiagnosticLogSink) {
    let _ = DIAGNOSTIC_LOG_SINK.set(sink);
}

#[cfg(target_os = "espidf")]
fn emit_diag_log(level: &str, message: String) {
    println!("{}", message);
    if let Some(sink) = DIAGNOSTIC_LOG_SINK.get() {
        sink(level, &message);
    }
}

pub struct EspIdfClock;

impl Clock for EspIdfClock {
    fn now_epoch(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default()
    }

    fn today_date_string(&self) -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }
}

pub struct EspIdfStorage {
    #[cfg(target_os = "espidf")]
    handle: sys::nvs_handle_t,
}

impl EspIdfStorage {
    #[cfg(target_os = "espidf")]
    pub fn new() -> Result<Self, String> {
        unsafe {
            let mut err = sys::nvs_flash_init();
            if err == sys::ESP_ERR_NVS_NO_FREE_PAGES || err == sys::ESP_ERR_NVS_NEW_VERSION_FOUND {
                sys::nvs_flash_erase();
                err = sys::nvs_flash_init();
            }
            check_esp(err, "nvs_flash_init")?;

            let mut handle: sys::nvs_handle_t = 0;
            let ns = CString::new("photoframe").unwrap();
            check_esp(
                sys::nvs_open(ns.as_ptr(), sys::nvs_open_mode_t_NVS_READWRITE, &mut handle),
                "nvs_open",
            )?;
            Ok(Self { handle })
        }
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn new() -> Result<Self, String> {
        Err("EspIdfStorage only works on espidf target".into())
    }

    #[cfg(target_os = "espidf")]
    pub fn ensure_device_identity(
        &mut self,
        config: &mut DeviceRuntimeConfig,
    ) -> Result<bool, String> {
        let mut changed = false;

        if config.device_id.is_empty() {
            let mut mac = [0u8; 6];
            unsafe {
                check_esp(
                    sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_WIFI_STA),
                    "esp_read_mac",
                )?;
            }
            config.device_id = device_id_from_mac_suffix([mac[2], mac[3], mac[4], mac[5]]);
            changed = true;
        }

        if config.orchestrator_token.is_empty() {
            let mut bytes = [0u8; 16];
            unsafe { sys::esp_fill_random(bytes.as_mut_ptr() as *mut _, bytes.len()) };
            config.orchestrator_token = token_hex_from_bytes(bytes);
            changed = true;
        }

        Ok(changed)
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn ensure_device_identity(
        &mut self,
        _config: &mut DeviceRuntimeConfig,
    ) -> Result<bool, String> {
        Err("EspIdfStorage only works on espidf target".into())
    }
}

impl Storage for EspIdfStorage {
    fn load_config(&mut self) -> Result<DeviceRuntimeConfig, String> {
        #[cfg(not(target_os = "espidf"))]
        {
            Err("EspIdfStorage only works on espidf target".into())
        }

        #[cfg(target_os = "espidf")]
        {
            let mut config = DeviceRuntimeConfig::default();

            config.primary_wifi_ssid = self.get_string("wifi_ssid")?.unwrap_or_default();
            config.primary_wifi_password = self.get_string("wifi_pwd")?.unwrap_or_default();
            if !config.primary_wifi_ssid.is_empty() {
                config.wifi_profiles.push(WifiCredential {
                    ssid: config.primary_wifi_ssid.clone(),
                    password: config.primary_wifi_password.clone(),
                });
            }

            for idx in 1..DeviceRuntimeConfig::MAX_WIFI_PROFILES {
                let ssid = self
                    .get_string(&format!("wifi{idx}_ssid"))?
                    .unwrap_or_default();
                let password = self
                    .get_string(&format!("wifi{idx}_pwd"))?
                    .unwrap_or_default();
                if !ssid.is_empty() && !config.wifi_profiles.iter().any(|item| item.ssid == ssid) {
                    config.wifi_profiles.push(WifiCredential { ssid, password });
                }
            }

            let last_index = self.get_i32("last_wifi_idx")?.unwrap_or(-1);
            if last_index >= 0 {
                config.last_connected_wifi_index =
                    Some(last_index as usize).filter(|idx| *idx < config.wifi_profiles.len());
            }

            config.image_url_template = self
                .get_string("url_tpl")?
                .unwrap_or(config.image_url_template);
            config.photo_token = self.get_string("photo_tok")?.unwrap_or_default();
            config.orchestrator_enabled = self.get_i32("orch_en")?.unwrap_or(1) != 0;
            config.orchestrator_base_url = self
                .get_string("orch_url")?
                .unwrap_or(config.orchestrator_base_url);
            config.device_id = self.get_string("dev_id")?.unwrap_or_default();
            config.orchestrator_token = self.get_string("orch_tok")?.unwrap_or_default();
            config.timezone = self.get_string("tz")?.unwrap_or(config.timezone);
            config.interval_minutes = self
                .get_i32("intv_min")?
                .unwrap_or(config.interval_minutes as i32)
                .max(1) as u32;
            config.retry_base_minutes = self
                .get_i32("retry_base")?
                .unwrap_or(config.retry_base_minutes as i32)
                .max(1) as u32;
            config.retry_max_minutes =
                self.get_i32("retry_max")?
                    .unwrap_or(config.retry_max_minutes as i32)
                    .max(config.retry_base_minutes as i32) as u32;
            config.max_failure_before_long_sleep = self
                .get_i32("max_fail")?
                .unwrap_or(config.max_failure_before_long_sleep as i32)
                .max(1) as u32;
            config.display_rotation = clamp_i32(
                self.get_i32("rotation")?.unwrap_or(config.display_rotation),
                0,
                2,
            );
            if config.display_rotation != 0 {
                config.display_rotation = 2;
            }
            config.color_process_mode = clamp_i32(
                self.get_i32("clr_mode")?
                    .unwrap_or(config.color_process_mode),
                0,
                2,
            );
            config.dither_mode =
                clamp_i32(self.get_i32("dither")?.unwrap_or(config.dither_mode), 0, 1);
            config.six_color_tolerance = clamp_i32(
                self.get_i32("clr_tol")?
                    .unwrap_or(config.six_color_tolerance),
                0,
                64,
            );
            config.last_image_sha256 = self.get_string("img_sha256")?.unwrap_or_default();
            config.last_image_date = self.get_string("img_date")?.unwrap_or_default();
            config.last_image_etag = self.get_string("img_etag")?.unwrap_or_default();
            config.last_image_last_modified = self.get_string("img_lm")?.unwrap_or_default();
            config.displayed_image_sha256 = self.get_string("disp_sha")?.unwrap_or_default();
            config.displayed_image_date = self.get_string("disp_date")?.unwrap_or_default();
            config.manual_history_active = self.get_i32("hist_mode")?.unwrap_or(0) != 0;
            config.preferred_image_origin = self.get_string("img_origin")?.unwrap_or_default();
            config.last_success_epoch = self.get_i64("last_ok")?.unwrap_or(0);
            config.last_time_sync_epoch = self.get_i64("time_sync")?.unwrap_or(0);
            config.failure_count = self.get_i32("fail_cnt")?.unwrap_or(0).max(0) as u32;
            config.remote_config_version = self.get_i32("cfg_ver")?.unwrap_or(0).max(0);
            config.ota_target_version = self.get_string("ota_ver")?.unwrap_or_default();
            config.ota_last_error = self.get_string("ota_err")?.unwrap_or_default();
            config.ota_last_attempt_epoch = self.get_i64("ota_try")?.unwrap_or(0);

            Ok(config)
        }
    }

    fn save_config(&mut self, config: &DeviceRuntimeConfig) -> Result<(), String> {
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = config;
            Err("EspIdfStorage only works on espidf target".into())
        }

        #[cfg(target_os = "espidf")]
        {
            self.set_string("wifi_ssid", &config.primary_wifi_ssid)?;
            self.set_string("wifi_pwd", &config.primary_wifi_password)?;
            self.set_string("url_tpl", &config.image_url_template)?;
            self.set_string("photo_tok", &config.photo_token)?;
            self.set_i32("orch_en", i32::from(config.orchestrator_enabled))?;
            self.set_string("orch_url", &config.orchestrator_base_url)?;
            self.set_string("dev_id", &config.device_id)?;
            self.set_string("orch_tok", &config.orchestrator_token)?;
            self.set_string("tz", &config.timezone)?;
            self.set_i32("intv_min", config.interval_minutes as i32)?;
            self.set_i32("retry_base", config.retry_base_minutes as i32)?;
            self.set_i32("retry_max", config.retry_max_minutes as i32)?;
            self.set_i32("max_fail", config.max_failure_before_long_sleep as i32)?;
            self.set_i32("rotation", config.display_rotation)?;
            self.set_i32("clr_mode", config.color_process_mode)?;
            self.set_i32("dither", config.dither_mode)?;
            self.set_i32("clr_tol", config.six_color_tolerance)?;
            self.set_string("img_sha256", &config.last_image_sha256)?;
            self.set_string("img_date", &config.last_image_date)?;
            self.set_string("img_etag", &config.last_image_etag)?;
            self.set_string("img_lm", &config.last_image_last_modified)?;
            self.set_string("disp_sha", &config.displayed_image_sha256)?;
            self.set_string("disp_date", &config.displayed_image_date)?;
            self.set_i32("hist_mode", i32::from(config.manual_history_active))?;
            self.set_string("img_origin", &config.preferred_image_origin)?;
            self.set_i64("last_ok", config.last_success_epoch)?;
            self.set_i64("time_sync", config.last_time_sync_epoch)?;
            self.set_i32("fail_cnt", config.failure_count as i32)?;
            self.set_i32("cfg_ver", config.remote_config_version)?;
            self.set_string("ota_ver", &config.ota_target_version)?;
            self.set_string("ota_err", &config.ota_last_error)?;
            self.set_i64("ota_try", config.ota_last_attempt_epoch)?;
            self.set_i32(
                "last_wifi_idx",
                config
                    .last_connected_wifi_index
                    .map(|i| i as i32)
                    .unwrap_or(-1),
            )?;

            for idx in 1..DeviceRuntimeConfig::MAX_WIFI_PROFILES {
                let ssid_key = format!("wifi{idx}_ssid");
                let pwd_key = format!("wifi{idx}_pwd");
                if let Some(profile) = config.wifi_profiles.get(idx) {
                    self.set_string(&ssid_key, &profile.ssid)?;
                    self.set_string(&pwd_key, &profile.password)?;
                } else {
                    self.erase_key(&ssid_key)?;
                    self.erase_key(&pwd_key)?;
                }
            }

            unsafe { check_esp(sys::nvs_commit(self.handle), "nvs_commit") }
        }
    }
}

impl EspIdfStorage {
    #[cfg(target_os = "espidf")]
    fn get_string(&self, key: &str) -> Result<Option<String>, String> {
        unsafe {
            let key = CString::new(key).unwrap();
            let mut len: usize = 0;
            let err = sys::nvs_get_str(self.handle, key.as_ptr(), ptr::null_mut(), &mut len);
            if err == sys::ESP_ERR_NVS_NOT_FOUND {
                return Ok(None);
            }
            check_esp(err, "nvs_get_str_len")?;
            if len == 0 {
                return Ok(Some(String::new()));
            }
            let mut buf = vec![0u8; len];
            check_esp(
                sys::nvs_get_str(
                    self.handle,
                    key.as_ptr(),
                    buf.as_mut_ptr() as *mut _,
                    &mut len,
                ),
                "nvs_get_str",
            )?;
            let s = CStr::from_ptr(buf.as_ptr() as *const _)
                .to_string_lossy()
                .into_owned();
            Ok(Some(s))
        }
    }

    #[cfg(target_os = "espidf")]
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), String> {
        unsafe {
            let key = CString::new(key).unwrap();
            let value = CString::new(value).unwrap_or_else(|_| CString::new("").unwrap());
            check_esp(
                sys::nvs_set_str(self.handle, key.as_ptr(), value.as_ptr()),
                "nvs_set_str",
            )
        }
    }

    #[cfg(target_os = "espidf")]
    fn get_i32(&self, key: &str) -> Result<Option<i32>, String> {
        unsafe {
            let key = CString::new(key).unwrap();
            let mut value = 0i32;
            let err = sys::nvs_get_i32(self.handle, key.as_ptr(), &mut value);
            if err == sys::ESP_ERR_NVS_NOT_FOUND {
                return Ok(None);
            }
            check_esp(err, "nvs_get_i32")?;
            Ok(Some(value))
        }
    }

    #[cfg(target_os = "espidf")]
    fn set_i32(&mut self, key: &str, value: i32) -> Result<(), String> {
        unsafe {
            let key = CString::new(key).unwrap();
            check_esp(
                sys::nvs_set_i32(self.handle, key.as_ptr(), value),
                "nvs_set_i32",
            )
        }
    }

    #[cfg(target_os = "espidf")]
    fn get_i64(&self, key: &str) -> Result<Option<i64>, String> {
        unsafe {
            let key = CString::new(key).unwrap();
            let mut value = 0i64;
            let err = sys::nvs_get_i64(self.handle, key.as_ptr(), &mut value);
            if err == sys::ESP_ERR_NVS_NOT_FOUND {
                return Ok(None);
            }
            check_esp(err, "nvs_get_i64")?;
            Ok(Some(value))
        }
    }

    #[cfg(target_os = "espidf")]
    fn set_i64(&mut self, key: &str, value: i64) -> Result<(), String> {
        unsafe {
            let key = CString::new(key).unwrap();
            check_esp(
                sys::nvs_set_i64(self.handle, key.as_ptr(), value),
                "nvs_set_i64",
            )
        }
    }

    #[cfg(target_os = "espidf")]
    fn erase_key(&mut self, key: &str) -> Result<(), String> {
        unsafe {
            let key = CString::new(key).unwrap();
            let err = sys::nvs_erase_key(self.handle, key.as_ptr());
            if err == sys::ESP_ERR_NVS_NOT_FOUND {
                return Ok(());
            }
            check_esp(err, "nvs_erase_key")
        }
    }
}

pub struct EspIdfOrchestratorApi;
impl OrchestratorApi for EspIdfOrchestratorApi {
    fn sync_config(
        &mut self,
        config: &DeviceRuntimeConfig,
        now_epoch: i64,
    ) -> Result<Option<DeviceRuntimeConfig>, String> {
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = (config, now_epoch);
            Err("EspIdfOrchestratorApi 尚未接入 HTTP 客户端".into())
        }

        #[cfg(target_os = "espidf")]
        {
            if !config.orchestrator_enabled
                || config.orchestrator_base_url.is_empty()
                || config.device_id.is_empty()
            {
                return Ok(None);
            }

            let url = format!(
                "{}/api/v1/device/config?device_id={}&now_epoch={}&current_version={}&reset_reason={}",
                trim_trailing_slash(&config.orchestrator_base_url),
                url_encode_component(&config.device_id),
                now_epoch,
                config.remote_config_version.max(0),
                current_reset_reason_label(),
            );
            let response = http_get_json::<DeviceConfigResponse>(
                &url,
                Some((&PHOTOFRAME_TOKEN_HEADER, &config.orchestrator_token)),
            )?;
            if response.config_version <= config.remote_config_version {
                return Ok(None);
            }
            let mut next = config.clone();
            next.apply_remote_config_patch(&payload_to_patch(response.config));
            next.remote_config_version = response.config_version;
            Ok(Some(next))
        }
    }

    fn fetch_directive(
        &mut self,
        config: &DeviceRuntimeConfig,
        now_epoch: i64,
        preferred_poll_seconds: u64,
    ) -> Result<Option<DeviceNextResponse>, String> {
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = (config, now_epoch, preferred_poll_seconds);
            Err("EspIdfOrchestratorApi 尚未接入 HTTP 客户端".into())
        }

        #[cfg(target_os = "espidf")]
        {
            if !config.orchestrator_enabled {
                return Ok(None);
            }
            if config.orchestrator_base_url.is_empty() || config.device_id.is_empty() {
                return Ok(None);
            }

            let default_poll_seconds = preferred_poll_seconds.clamp(60, 86_400) as u32;
            let url = format!(
                "{}/api/v1/device/next?device_id={}&now_epoch={}&default_poll_seconds={}&failure_count={}&accept_formats=jpeg,bmp&reset_reason={}",
                trim_trailing_slash(&config.orchestrator_base_url),
                url_encode_component(&config.device_id),
                now_epoch,
                default_poll_seconds,
                config.failure_count,
                current_reset_reason_label(),
            );

            http_get_json::<DeviceNextResponse>(
                &url,
                Some((&PHOTOFRAME_TOKEN_HEADER, &config.orchestrator_token)),
            )
            .map(Some)
        }
    }

    fn report_checkin(
        &mut self,
        base_urls: &[String],
        payload: &DeviceCheckinRequest,
    ) -> Result<(), String> {
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = (base_urls, payload);
            Err("EspIdfOrchestratorApi 尚未接入 HTTP 客户端".into())
        }

        #[cfg(target_os = "espidf")]
        {
            let body = serde_json::to_vec(payload).map_err(|err| err.to_string())?;
            let mut last_error = String::from("no base url");
            for (base_index, base) in base_urls.iter().enumerate() {
                let url = format!("{}/api/v1/device/checkin", trim_trailing_slash(base));
                for attempt in 0..3 {
                    match http_post_json_status(
                        &url,
                        Some((
                            &PHOTOFRAME_TOKEN_HEADER,
                            &payload.reported_config.orchestrator_token,
                        )),
                        &body,
                    ) {
                        Ok(status) if (200..300).contains(&status) => {
                            emit_diag_log(
                                "INFO",
                                format!(
                                    "photoframe-rs/checkin: ok device_id={} base={}/{} attempt={}/3 status={} url={}",
                                    payload.device_id,
                                    base_index + 1,
                                    base_urls.len(),
                                    attempt + 1,
                                    status,
                                    url
                                ),
                            );
                            return Ok(());
                        }
                        Ok(status) => {
                            last_error = format!("non-2xx status={status}");
                            emit_diag_log(
                                "WARN",
                                format!(
                                    "photoframe-rs/checkin: non-2xx device_id={} base={}/{} attempt={}/3 status={} url={}",
                                    payload.device_id,
                                    base_index + 1,
                                    base_urls.len(),
                                    attempt + 1,
                                    status,
                                    url
                                ),
                            );
                        }
                        Err(err) => {
                            last_error = err.clone();
                            emit_diag_log(
                                "WARN",
                                format!(
                                    "photoframe-rs/checkin: error device_id={} base={}/{} attempt={}/3 err={} url={}",
                                    payload.device_id,
                                    base_index + 1,
                                    base_urls.len(),
                                    attempt + 1,
                                    err,
                                    url
                                ),
                            );
                        }
                    }
                }
                emit_diag_log(
                    "WARN",
                    format!(
                        "photoframe-rs/checkin: base failed device_id={} base={}/{} last_error={} url={}",
                        payload.device_id,
                        base_index + 1,
                        base_urls.len(),
                        last_error,
                        url
                    ),
                );
            }
            Err(last_error)
        }
    }

    fn report_config_applied(
        &mut self,
        config: &DeviceRuntimeConfig,
        config_version: i32,
        applied: bool,
        error: &str,
        applied_epoch: i64,
    ) -> Result<(), String> {
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = (config, config_version, applied, error, applied_epoch);
            Err("EspIdfOrchestratorApi 尚未接入 HTTP 客户端".into())
        }

        #[cfg(target_os = "espidf")]
        {
            if !config.orchestrator_enabled
                || config.orchestrator_base_url.is_empty()
                || config.device_id.is_empty()
            {
                return Ok(());
            }

            let payload = DeviceConfigAppliedRequest {
                device_id: config.device_id.clone(),
                config_version: config_version.max(0),
                applied,
                error: error.to_string(),
                applied_epoch,
            };
            let body = serde_json::to_vec(&payload).map_err(|err| err.to_string())?;
            let url = format!(
                "{}/api/v1/device/config/applied",
                trim_trailing_slash(&config.orchestrator_base_url)
            );
            let status = http_post_json_status(
                &url,
                Some((&PHOTOFRAME_TOKEN_HEADER, &config.orchestrator_token)),
                &body,
            )?;
            if (200..300).contains(&status) {
                return Ok(());
            }
            Err(format!("unexpected status: {status} url={url}"))
        }
    }

    fn report_debug_stage(
        &mut self,
        config: &DeviceRuntimeConfig,
        stage: &str,
    ) -> Result<(), String> {
        send_debug_stage_beacon(config, stage)
    }

    fn upload_logs(
        &mut self,
        config: &DeviceRuntimeConfig,
        request: &DeviceLogUploadRequest,
        payload: &DeviceLogUploadRequestBody,
    ) -> Result<(), String> {
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = (config, request, payload);
            Err("EspIdfOrchestratorApi 尚未接入 HTTP 客户端".into())
        }

        #[cfg(target_os = "espidf")]
        {
            if !config.orchestrator_enabled
                || config.orchestrator_base_url.is_empty()
                || config.device_id.is_empty()
            {
                return Ok(());
            }

            let body = serde_json::to_vec(payload).map_err(|err| err.to_string())?;
            let url = format!(
                "{}/api/v1/device/log-upload",
                trim_trailing_slash(&config.orchestrator_base_url)
            );
            let status = http_post_json_status(
                &url,
                Some((&PHOTOFRAME_TOKEN_HEADER, &config.orchestrator_token)),
                &body,
            )?;
            if (200..300).contains(&status) {
                emit_diag_log(
                    "INFO",
                    format!(
                        "photoframe-rs/log-upload: ok device_id={} request_id={} status={} url={}",
                        payload.device_id, request.request_id, status, url
                    ),
                );
                return Ok(());
            }
            Err(format!(
                "unexpected status: {status} url={url} request_id={}",
                request.request_id
            ))
        }
    }
}

pub struct EspIdfImageFetcher;
impl ImageFetcher for EspIdfImageFetcher {
    fn fetch(&mut self, plan: &ImageFetchPlan) -> ImageFetchOutcome {
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = plan;
            ImageFetchOutcome::failed(0, "EspIdfImageFetcher 尚未接入 HTTP 下载")
        }

        #[cfg(target_os = "espidf")]
        {
            match fetch_image_inner(plan) {
                Ok(result) => result,
                Err(error) => ImageFetchOutcome::failed(0, error),
            }
        }
    }
}

pub fn send_debug_stage_beacon(config: &DeviceRuntimeConfig, stage: &str) -> Result<(), String> {
    #[cfg(not(target_os = "espidf"))]
    {
        let _ = (config, stage);
        Ok(())
    }

    #[cfg(target_os = "espidf")]
    {
        if option_env!("PHOTOFRAME_DEBUG_STAGE_BEACON").is_none()
            || !config.orchestrator_enabled
            || config.orchestrator_base_url.is_empty()
            || config.device_id.is_empty()
        {
            return Ok(());
        }

        send_debug_stage_beacon_inner(
            &config.orchestrator_base_url,
            &config.device_id,
            &config.orchestrator_token,
            stage,
        )
    }
}

#[cfg(target_os = "espidf")]
fn send_debug_stage_beacon_inner(
    base_url: &str,
    device_id: &str,
    token: &str,
    stage: &str,
) -> Result<(), String> {
    if option_env!("PHOTOFRAME_DEBUG_STAGE_BEACON").is_none()
        || base_url.is_empty()
        || device_id.is_empty()
    {
        return Ok(());
    }

    let url = format!(
        "{}/api/v1/device/debug-stage?device_id={}&stage={}",
        trim_trailing_slash(base_url),
        url_encode_component(device_id),
        url_encode_component(stage),
    );
    match http_get_bytes(&url, Some((&PHOTOFRAME_TOKEN_HEADER, token))) {
        Ok(_) => Ok(()),
        Err(err) => {
            println!(
                "photoframe-rs/debug-stage: failed device_id={} stage={} err={}",
                device_id, stage, err
            );
            Err(err)
        }
    }
}

#[cfg(target_os = "espidf")]
fn send_fetch_debug_stage(plan: &ImageFetchPlan, stage: &str) {
    let _ = send_debug_stage_beacon_inner(
        &plan.debug_stage_base_url,
        &plan.device_id,
        &plan.orchestrator_token,
        stage,
    );
}

#[cfg(not(target_os = "espidf"))]
fn send_fetch_debug_stage(_plan: &ImageFetchPlan, _stage: &str) {}

pub struct EspIdfDisplay;
impl Display for EspIdfDisplay {
    fn render(
        &mut self,
        _artifact: &ImageArtifact,
        _config: &DeviceRuntimeConfig,
        _force_refresh: bool,
    ) -> Result<(), FailureKind> {
        Err(FailureKind::GeneralFailure)
    }
}

pub struct EspIdfFirmwareUpdater;

impl FirmwareUpdater for EspIdfFirmwareUpdater {
    fn install_update(
        &mut self,
        config: &DeviceRuntimeConfig,
        directive: &FirmwareUpdateDirective,
    ) -> Result<bool, String> {
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = (config, directive);
            Err("EspIdfFirmwareUpdater 尚未接入 OTA".into())
        }

        #[cfg(target_os = "espidf")]
        {
            if directive.version.trim().is_empty() || directive.app_bin_url.trim().is_empty() {
                return Ok(false);
            }
            if directive.version == config.firmware_version() {
                return Ok(false);
            }
            let token = orchestrator_token_for_url(&directive.app_bin_url, config);
            install_firmware_inner(&directive.app_bin_url, token.as_deref(), directive, config)
                .map(|_| true)
        }
    }

    fn confirm_running_firmware(&mut self, config: &DeviceRuntimeConfig) -> Result<(), String> {
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = config;
            Ok(())
        }

        #[cfg(target_os = "espidf")]
        {
            confirm_running_firmware_inner(config)
        }
    }

    fn current_status(&mut self, config: &DeviceRuntimeConfig) -> FirmwareRuntimeStatus {
        #[cfg(not(target_os = "espidf"))]
        {
            FirmwareRuntimeStatus {
                ota_target_version: config.ota_target_version.clone(),
                ota_last_error: config.ota_last_error.clone(),
                ota_last_attempt_epoch: config.ota_last_attempt_epoch,
                ..FirmwareRuntimeStatus::default()
            }
        }

        #[cfg(target_os = "espidf")]
        unsafe {
            let mut status = FirmwareRuntimeStatus {
                ota_target_version: config.ota_target_version.clone(),
                ota_last_error: config.ota_last_error.clone(),
                ota_last_attempt_epoch: config.ota_last_attempt_epoch,
                ..FirmwareRuntimeStatus::default()
            };
            let running = normalize_running_partition(sys::esp_ota_get_running_partition());
            if running.is_null() {
                return status;
            }
            status.running_partition = partition_label(running);
            let mut state: sys::esp_ota_img_states_t = Default::default();
            let err = sys::esp_ota_get_state_partition(running, &mut state as *mut _);
            status.ota_state = if err == sys::ESP_ERR_NOT_SUPPORTED {
                if status.running_partition == "factory" {
                    "factory".into()
                } else {
                    "baseline".into()
                }
            } else if err == sys::ESP_ERR_NOT_FOUND {
                "unknown".into()
            } else if check_esp(err, "esp_ota_get_state_partition").is_err() {
                "error".into()
            } else {
                #[allow(non_upper_case_globals)]
                match state {
                    sys::esp_ota_img_states_t_ESP_OTA_IMG_NEW => "new",
                    sys::esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY => "pending_verify",
                    sys::esp_ota_img_states_t_ESP_OTA_IMG_VALID => "valid",
                    sys::esp_ota_img_states_t_ESP_OTA_IMG_INVALID => "invalid",
                    sys::esp_ota_img_states_t_ESP_OTA_IMG_ABORTED => "aborted",
                    sys::esp_ota_img_states_t_ESP_OTA_IMG_UNDEFINED => "undefined",
                    _ => "unknown",
                }
                .into()
            };
            status
        }
    }
}

#[cfg(target_os = "espidf")]
const PHOTOFRAME_TOKEN_HEADER: &str = "X-PhotoFrame-Token";
#[cfg(target_os = "espidf")]
const PHOTO_TOKEN_HEADER: &str = "X-Photo-Token";
#[cfg(target_os = "espidf")]
const MAX_HTTP_REDIRECTS: usize = 5;
#[cfg(target_os = "espidf")]
const HTTP_READ_RETRY_LIMIT: usize = 8;

#[cfg(test)]
fn resolve_redirect_url(current_url: &str, location: &str) -> Option<String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Some(location.to_string());
    }

    let (origin, rest) = photoframe_app::split_url_origin_and_rest(current_url)?;
    if location.starts_with("//") {
        let scheme_end = origin.find("://")?;
        let scheme = &origin[..scheme_end];
        return Some(format!("{scheme}:{location}"));
    }
    if location.starts_with('/') {
        return Some(format!("{origin}{location}"));
    }

    let path_end = rest.find('?').unwrap_or(rest.len());
    let path = &rest[..path_end];
    let base_dir = path.rfind('/').map(|index| &path[..=index]).unwrap_or("/");
    Some(format!("{origin}{base_dir}{location}"))
}

#[cfg(target_os = "espidf")]
fn is_redirect_status(status: i32) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

#[cfg(target_os = "espidf")]
pub fn current_reset_reason_label() -> &'static str {
    match unsafe { sys::esp_reset_reason() } {
        sys::esp_reset_reason_t_ESP_RST_UNKNOWN => "unknown",
        sys::esp_reset_reason_t_ESP_RST_POWERON => "poweron",
        sys::esp_reset_reason_t_ESP_RST_EXT => "ext",
        sys::esp_reset_reason_t_ESP_RST_SW => "sw",
        sys::esp_reset_reason_t_ESP_RST_PANIC => "panic",
        sys::esp_reset_reason_t_ESP_RST_INT_WDT => "int_wdt",
        sys::esp_reset_reason_t_ESP_RST_TASK_WDT => "task_wdt",
        sys::esp_reset_reason_t_ESP_RST_WDT => "wdt",
        sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP => "deepsleep",
        sys::esp_reset_reason_t_ESP_RST_BROWNOUT => "brownout",
        sys::esp_reset_reason_t_ESP_RST_SDIO => "sdio",
        sys::esp_reset_reason_t_ESP_RST_USB => "usb",
        sys::esp_reset_reason_t_ESP_RST_JTAG => "jtag",
        sys::esp_reset_reason_t_ESP_RST_EFUSE => "efuse",
        sys::esp_reset_reason_t_ESP_RST_PWR_GLITCH => "pwr_glitch",
        sys::esp_reset_reason_t_ESP_RST_CPU_LOCKUP => "cpu_lockup",
        _ => "other",
    }
}

#[cfg(target_os = "espidf")]
fn image_format_label(format: &ImageFormat) -> &'static str {
    match format {
        ImageFormat::Bmp => "bmp",
        ImageFormat::Jpeg => "jpeg",
    }
}

#[cfg(target_os = "espidf")]
fn payload_to_patch(payload: DeviceConfigPayload) -> RemoteConfigPatch {
    RemoteConfigPatch {
        orchestrator_enabled: payload.orchestrator_enabled,
        orchestrator_base_url: payload.orchestrator_base_url,
        orchestrator_token: payload.orchestrator_token,
        image_url_template: payload.image_url_template,
        photo_token: payload.photo_token,
        wifi_profiles: payload.wifi_profiles,
        interval_minutes: payload.interval_minutes,
        retry_base_minutes: payload.retry_base_minutes,
        retry_max_minutes: payload.retry_max_minutes,
        max_failure_before_long_sleep: payload.max_failure_before_long_sleep,
        display_rotation: payload.display_rotation,
        color_process_mode: payload.color_process_mode,
        dither_mode: payload.dither_mode,
        six_color_tolerance: payload.six_color_tolerance,
        timezone: payload.timezone,
    }
}

#[cfg(target_os = "espidf")]
fn orchestrator_token_for_url(url: &str, config: &DeviceRuntimeConfig) -> Option<String> {
    if config.orchestrator_token.is_empty() {
        return None;
    }
    let expected_origin = photoframe_app::split_url_origin_and_rest(&config.orchestrator_base_url)
        .map(|(origin, _)| origin)?;
    let candidate_origin =
        photoframe_app::split_url_origin_and_rest(url).map(|(origin, _)| origin)?;
    if candidate_origin == expected_origin {
        return Some(config.orchestrator_token.clone());
    }
    None
}

#[cfg(target_os = "espidf")]
unsafe fn partition_label(partition: *const sys::esp_partition_t) -> String {
    if partition.is_null() {
        return String::new();
    }
    unsafe {
        CStr::from_ptr((*partition).label.as_ptr() as *const _)
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(target_os = "espidf")]
unsafe fn find_app_partition_by_subtype(
    subtype: sys::esp_partition_subtype_t,
) -> *const sys::esp_partition_t {
    unsafe {
        sys::esp_partition_find_first(
            sys::esp_partition_type_t_ESP_PARTITION_TYPE_APP,
            subtype,
            ptr::null(),
        )
    }
}

#[cfg(target_os = "espidf")]
unsafe fn find_ota_partition(slot: u32) -> *const sys::esp_partition_t {
    unsafe {
        find_app_partition_by_subtype(
            sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_APP_OTA_MIN + slot,
        )
    }
}

#[cfg(target_os = "espidf")]
unsafe fn normalize_running_partition(
    running: *const sys::esp_partition_t,
) -> *const sys::esp_partition_t {
    if running.is_null() || unsafe { partition_label(running) } != "factory" {
        return running;
    }

    let factory = unsafe {
        find_app_partition_by_subtype(
            sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_APP_FACTORY,
        )
    };
    if !factory.is_null() {
        return running;
    }

    let running_address = unsafe { (*running).address };
    let ota0 = unsafe { find_ota_partition(0) };
    if !ota0.is_null() && unsafe { (*ota0).address == running_address } {
        return ota0;
    }
    let ota1 = unsafe { find_ota_partition(1) };
    if !ota1.is_null() && unsafe { (*ota1).address == running_address } {
        return ota1;
    }
    running
}

#[cfg(target_os = "espidf")]
unsafe fn select_inactive_update_partition() -> *const sys::esp_partition_t {
    let running = unsafe { normalize_running_partition(sys::esp_ota_get_running_partition()) };
    let running_label = unsafe { partition_label(running) };
    if running_label == "ota_0" {
        let target = unsafe { find_ota_partition(1) };
        if !target.is_null() {
            return target;
        }
    } else if running_label == "ota_1" {
        let target = unsafe { find_ota_partition(0) };
        if !target.is_null() {
            return target;
        }
    }
    unsafe { sys::esp_ota_get_next_update_partition(ptr::null()) }
}

#[cfg(target_os = "espidf")]
fn confirm_running_firmware_inner(config: &DeviceRuntimeConfig) -> Result<(), String> {
    unsafe {
        let running = normalize_running_partition(sys::esp_ota_get_running_partition());
        if running.is_null() {
            return Ok(());
        }
        let mut state: sys::esp_ota_img_states_t = Default::default();
        let err = sys::esp_ota_get_state_partition(running, &mut state as *mut _);
        if err == sys::ESP_ERR_NOT_SUPPORTED || err == sys::ESP_ERR_NOT_FOUND {
            return Ok(());
        }
        check_esp(err, "esp_ota_get_state_partition")?;
        #[allow(non_upper_case_globals)]
        if state == sys::esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY
            || state == sys::esp_ota_img_states_t_ESP_OTA_IMG_NEW
        {
            check_esp(
                sys::esp_ota_mark_app_valid_cancel_rollback(),
                "esp_ota_mark_app_valid_cancel_rollback",
            )?;
            emit_diag_log(
                "INFO",
                "photoframe-rs/ota: marked running slot valid".to_string(),
            );
            let _ = send_debug_stage_beacon(config, "ota_mark_valid");
        }
        Ok(())
    }
}

#[cfg(target_os = "espidf")]
fn install_firmware_inner(
    url_text: &str,
    orchestrator_token: Option<&str>,
    directive: &FirmwareUpdateDirective,
    config: &DeviceRuntimeConfig,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let url = CString::new(url_text).map_err(|err| err.to_string())?;
    let _ = send_debug_stage_beacon(config, "ota_start");
    unsafe {
        let mut client_config: sys::esp_http_client_config_t = std::mem::zeroed();
        client_config.url = url.as_ptr();
        client_config.timeout_ms = 60_000;
        client_config.disable_auto_redirect = true;
        if is_https_url(url_text) {
            client_config.crt_bundle_attach = Some(sys::esp_crt_bundle_attach);
        }

        let client = sys::esp_http_client_init(&client_config);
        if client.is_null() {
            return Err("esp_http_client_init failed".into());
        }
        if let Some(token) = orchestrator_token
            && !token.is_empty()
            && let Err(err) = set_header(client, PHOTOFRAME_TOKEN_HEADER, token)
        {
            sys::esp_http_client_cleanup(client);
            return Err(err);
        }

        let mut redirect_count = 0usize;
        loop {
            if let Err(err) =
                check_esp(sys::esp_http_client_open(client, 0), "esp_http_client_open")
            {
                sys::esp_http_client_cleanup(client);
                return Err(err);
            }
            let _ = sys::esp_http_client_fetch_headers(client);
            let status_code = sys::esp_http_client_get_status_code(client);
            if is_redirect_status(status_code) {
                if redirect_count >= MAX_HTTP_REDIRECTS {
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(format!("too many redirects: {status_code}"));
                }
                if let Err(err) = check_esp(
                    sys::esp_http_client_set_redirection(client),
                    "esp_http_client_set_redirection",
                ) {
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(err);
                }
                let mut flushed = 0;
                let _ = sys::esp_http_client_flush_response(client, &mut flushed);
                sys::esp_http_client_close(client);
                redirect_count += 1;
                continue;
            }
            if status_code != 200 {
                let _ = send_debug_stage_beacon(config, "ota_fail_http");
                let body = read_body_stream(client).unwrap_or_default();
                sys::esp_http_client_close(client);
                sys::esp_http_client_cleanup(client);
                let preview = String::from_utf8_lossy(&body)
                    .chars()
                    .take(120)
                    .collect::<String>();
                return Err(format!(
                    "firmware download status={status_code} url={url_text} body={preview}"
                ));
            }
            let _ = send_debug_stage_beacon(config, "ota_http_ok");

            let content_len = sys::esp_http_client_get_content_length(client);
            if directive.size_bytes > 0
                && content_len > 0
                && content_len as u64 != directive.size_bytes
            {
                let _ = send_debug_stage_beacon(config, "ota_fail_size_header");
                sys::esp_http_client_close(client);
                sys::esp_http_client_cleanup(client);
                return Err(format!(
                    "firmware size mismatch before download: expected={} actual_header={}",
                    directive.size_bytes, content_len
                ));
            }

            let partition = select_inactive_update_partition();
            if partition.is_null() {
                let _ = send_debug_stage_beacon(config, "ota_fail_no_slot");
                sys::esp_http_client_close(client);
                sys::esp_http_client_cleanup(client);
                return Err("esp_ota_get_next_update_partition returned null".into());
            }
            let partition_stage = format!("ota_target_{}", partition_label(partition));
            let _ = send_debug_stage_beacon(config, &partition_stage);
            let mut handle: sys::esp_ota_handle_t = Default::default();
            if let Err(err) = check_esp(
                sys::esp_ota_begin(partition, sys::OTA_SIZE_UNKNOWN as usize, &mut handle),
                "esp_ota_begin",
            ) {
                let _ = send_debug_stage_beacon(config, "ota_fail_begin");
                sys::esp_http_client_close(client);
                sys::esp_http_client_cleanup(client);
                return Err(err);
            }
            let _ = send_debug_stage_beacon(config, "ota_begin_ok");

            let mut hasher = Sha256::new();
            let mut total = 0usize;
            let mut chunk = vec![0u8; 4096];
            let mut transient_reads = 0usize;
            let mut stage_25 = false;
            let mut stage_50 = false;
            let mut stage_75 = false;
            let mut stage_100 = false;
            loop {
                let read = sys::esp_http_client_read(
                    client,
                    chunk.as_mut_ptr() as *mut c_char,
                    chunk.len() as i32,
                );
                if read == -(sys::ESP_ERR_HTTP_EAGAIN as i32) {
                    transient_reads = transient_reads.saturating_add(1);
                    if transient_reads > HTTP_READ_RETRY_LIMIT {
                        let _ = send_debug_stage_beacon(config, "ota_fail_read_timeout");
                        let _ = sys::esp_ota_abort(handle);
                        sys::esp_http_client_close(client);
                        sys::esp_http_client_cleanup(client);
                        return Err(format!(
                            "esp_http_client_read timed out repeatedly: retries={} bytes={}",
                            transient_reads, total
                        ));
                    }
                    continue;
                }
                if read < 0 {
                    let _ = send_debug_stage_beacon(config, "ota_fail_read");
                    let _ = sys::esp_ota_abort(handle);
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(format!(
                        "esp_http_client_read failed: code={} bytes={}",
                        read, total
                    ));
                }
                if read == 0 {
                    if !sys::esp_http_client_is_complete_data_received(client) {
                        transient_reads = transient_reads.saturating_add(1);
                        if transient_reads > HTTP_READ_RETRY_LIMIT {
                            let _ = send_debug_stage_beacon(config, "ota_fail_incomplete");
                            let _ = sys::esp_ota_abort(handle);
                            sys::esp_http_client_close(client);
                            sys::esp_http_client_cleanup(client);
                            return Err(format!(
                                "firmware download incomplete after repeated empty reads: retries={} bytes={}",
                                transient_reads, total
                            ));
                        }
                        continue;
                    }
                    break;
                }
                transient_reads = 0;
                let size = read as usize;
                let slice = &chunk[..size];
                hasher.update(slice);
                total = total.saturating_add(size);
                if directive.size_bytes > 0 && total > directive.size_bytes as usize {
                    let _ = send_debug_stage_beacon(config, "ota_fail_size_exceed");
                    let _ = sys::esp_ota_abort(handle);
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(format!(
                        "firmware size exceeded: expected={} actual_so_far={}",
                        directive.size_bytes, total
                    ));
                }
                if let Err(err) = check_esp(
                    sys::esp_ota_write(handle, slice.as_ptr() as *const _, size as _),
                    "esp_ota_write",
                ) {
                    let _ = send_debug_stage_beacon(config, "ota_fail_write");
                    let _ = sys::esp_ota_abort(handle);
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(err);
                }
                if directive.size_bytes > 0 {
                    let progress = total.saturating_mul(100) / directive.size_bytes as usize;
                    if !stage_25 && progress >= 25 {
                        stage_25 = true;
                        let _ = send_debug_stage_beacon(config, "ota_download_25");
                    }
                    if !stage_50 && progress >= 50 {
                        stage_50 = true;
                        let _ = send_debug_stage_beacon(config, "ota_download_50");
                    }
                    if !stage_75 && progress >= 75 {
                        stage_75 = true;
                        let _ = send_debug_stage_beacon(config, "ota_download_75");
                    }
                    if !stage_100 && progress >= 100 {
                        stage_100 = true;
                        let _ = send_debug_stage_beacon(config, "ota_download_100");
                    }
                }
            }
            sys::esp_http_client_close(client);
            sys::esp_http_client_cleanup(client);

            if directive.size_bytes > 0 && total != directive.size_bytes as usize {
                let _ = send_debug_stage_beacon(config, "ota_fail_size_final");
                let _ = sys::esp_ota_abort(handle);
                return Err(format!(
                    "firmware size mismatch after download: expected={} actual={}",
                    directive.size_bytes, total
                ));
            }
            let actual_sha256 = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            if !directive.sha256.eq_ignore_ascii_case(&actual_sha256) {
                let _ = send_debug_stage_beacon(config, "ota_fail_sha");
                let _ = sys::esp_ota_abort(handle);
                return Err(format!(
                    "firmware sha256 mismatch: expected={} actual={}",
                    directive.sha256, actual_sha256
                ));
            }
            let _ = send_debug_stage_beacon(config, "ota_sha_ok");
            check_esp(sys::esp_ota_end(handle), "esp_ota_end")?;
            let _ = send_debug_stage_beacon(config, "ota_end_ok");
            check_esp(
                sys::esp_ota_set_boot_partition(partition),
                "esp_ota_set_boot_partition",
            )?;
            let _ = send_debug_stage_beacon(config, "ota_boot_set_ok");
            emit_diag_log(
                "INFO",
                format!(
                    "photoframe-rs/ota: prepared update version={} bytes={} sha256={}",
                    directive.version, total, actual_sha256
                ),
            );
            let _ = send_debug_stage_beacon(config, "ota_ready_reboot");
            return Ok(());
        }
    }
}

#[cfg(target_os = "espidf")]
fn fetch_image_inner(plan: &ImageFetchPlan) -> Result<ImageFetchOutcome, String> {
    let url = CString::new(plan.url.clone()).map_err(|err| err.to_string())?;
    let fetch_start = Instant::now();
    unsafe {
        let mut config: sys::esp_http_client_config_t = std::mem::zeroed();
        config.url = url.as_ptr();
        config.timeout_ms = 20_000;
        config.disable_auto_redirect = true;
        if is_https_url(&plan.url) {
            config.crt_bundle_attach = Some(sys::esp_crt_bundle_attach);
        }

        let client = sys::esp_http_client_init(&config);
        if client.is_null() {
            return Err("esp_http_client_init failed".into());
        }

        if !plan.photo_token.is_empty() {
            if let Err(err) = set_header(client, PHOTO_TOKEN_HEADER, &plan.photo_token) {
                sys::esp_http_client_cleanup(client);
                return Err(err);
            }
        }
        if !plan.orchestrator_token.is_empty() {
            if let Err(err) = set_header(client, PHOTOFRAME_TOKEN_HEADER, &plan.orchestrator_token)
            {
                sys::esp_http_client_cleanup(client);
                return Err(err);
            }
        }
        if let Some(value) = &plan.previous_etag
            && !value.is_empty()
            && let Err(err) = set_header(client, "If-None-Match", value)
        {
            sys::esp_http_client_cleanup(client);
            return Err(err);
        }
        if let Some(value) = &plan.previous_last_modified
            && !value.is_empty()
            && let Err(err) = set_header(client, "If-Modified-Since", value)
        {
            sys::esp_http_client_cleanup(client);
            return Err(err);
        }

        let mut redirect_count = 0usize;
        loop {
            send_fetch_debug_stage(plan, "fetch_http_open");
            let open_start = Instant::now();
            if let Err(err) =
                check_esp(sys::esp_http_client_open(client, 0), "esp_http_client_open")
            {
                sys::esp_http_client_cleanup(client);
                return Err(err);
            }
            let _ = sys::esp_http_client_fetch_headers(client);
            let status_code = sys::esp_http_client_get_status_code(client);
            let headers_ms = open_start.elapsed().as_millis();
            send_fetch_debug_stage(plan, "fetch_headers_ok");

            if is_redirect_status(status_code) {
                if redirect_count >= MAX_HTTP_REDIRECTS {
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(format!("too many redirects: {status_code}"));
                }
                if let Err(err) = check_esp(
                    sys::esp_http_client_set_redirection(client),
                    "esp_http_client_set_redirection",
                ) {
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(err);
                }
                let mut flushed = 0;
                let _ = sys::esp_http_client_flush_response(client, &mut flushed);
                sys::esp_http_client_close(client);
                redirect_count += 1;
                continue;
            }

            let content_len = sys::esp_http_client_get_content_length(client);
            let content_type = get_header_value(client, "Content-Type")?;
            let etag = get_header_value(client, "ETag")?;
            let last_modified = get_header_value(client, "Last-Modified")?;
            if status_code == 304 {
                emit_diag_log(
                    "INFO",
                    format!(
                        "photoframe-rs/timing: fetch status=304 total={}ms headers={}ms body=0ms bytes=0 changed=false format=unchanged url={}",
                        fetch_start.elapsed().as_millis(),
                        headers_ms,
                        plan.url
                    ),
                );
                sys::esp_http_client_close(client);
                sys::esp_http_client_cleanup(client);
                return Ok(ImageFetchOutcome {
                    ok: true,
                    status_code,
                    error: String::new(),
                    image_changed: false,
                    sha256: plan.previous_sha256.clone(),
                    etag,
                    last_modified,
                    artifact: None,
                });
            }

            if status_code != 200 {
                sys::esp_http_client_close(client);
                sys::esp_http_client_cleanup(client);
                let extra = if status_code == 401 || status_code == 403 {
                    ", check X-Photo-Token"
                } else {
                    ""
                };
                return Err(format!(
                    "unexpected status: {status_code}{extra} url={}",
                    plan.url
                ));
            }

            if content_len <= 0 || content_len > 4 * 1024 * 1024 {
                sys::esp_http_client_close(client);
                sys::esp_http_client_cleanup(client);
                return Err(format!("invalid content length: {content_len}"));
            }

            let body_start = Instant::now();
            let data = match read_body_exact(client, content_len as usize, &plan.url) {
                Ok(data) => data,
                Err(err) => {
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(err);
                }
            };
            let body_ms = body_start.elapsed().as_millis();
            send_fetch_debug_stage(plan, "fetch_body_ok");
            sys::esp_http_client_close(client);
            sys::esp_http_client_cleanup(client);

            let sha256 = sha256_hex(&data);
            let image_changed = sha256 != plan.previous_sha256;
            let format = detect_format(content_type.as_deref(), &data);
            println!(
                "photoframe-rs/timing: fetch status=200 total={}ms headers={}ms body={}ms bytes={} changed={} format={} url={}",
                fetch_start.elapsed().as_millis(),
                headers_ms,
                body_ms,
                data.len(),
                image_changed,
                image_format_label(&format),
                plan.url
            );
            let artifact = Some(ImageArtifact {
                format,
                width: 0,
                height: 0,
                bytes: data,
            });

            return Ok(ImageFetchOutcome {
                ok: true,
                status_code,
                error: String::new(),
                image_changed,
                sha256,
                etag,
                last_modified,
                artifact,
            });
        }
    }
}

#[cfg(target_os = "espidf")]
fn http_get_json<T: serde::de::DeserializeOwned>(
    url: &str,
    token_header: Option<(&str, &str)>,
) -> Result<T, String> {
    let response = http_get_bytes(url, token_header)?;
    serde_json::from_slice(&response).map_err(|err| err.to_string())
}

#[cfg(target_os = "espidf")]
fn http_get_bytes(url: &str, token_header: Option<(&str, &str)>) -> Result<Vec<u8>, String> {
    let url = CString::new(url).map_err(|err| err.to_string())?;
    unsafe {
        let mut config: sys::esp_http_client_config_t = std::mem::zeroed();
        config.url = url.as_ptr();
        config.timeout_ms = 20_000;
        config.disable_auto_redirect = true;
        if is_https_url(url.to_str().unwrap_or_default()) {
            config.crt_bundle_attach = Some(sys::esp_crt_bundle_attach);
        }
        let client = sys::esp_http_client_init(&config);
        if client.is_null() {
            return Err("esp_http_client_init failed".into());
        }
        if let Some((header, token)) = token_header
            && !token.is_empty()
            && let Err(err) = set_header(client, header, token)
        {
            sys::esp_http_client_cleanup(client);
            return Err(err);
        }

        let mut redirect_count = 0usize;
        loop {
            if let Err(err) =
                check_esp(sys::esp_http_client_open(client, 0), "esp_http_client_open")
            {
                sys::esp_http_client_cleanup(client);
                return Err(err);
            }
            let _ = sys::esp_http_client_fetch_headers(client);
            let status = sys::esp_http_client_get_status_code(client);

            if is_redirect_status(status) {
                if redirect_count >= MAX_HTTP_REDIRECTS {
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(format!("too many redirects: {status}"));
                }
                if let Err(err) = check_esp(
                    sys::esp_http_client_set_redirection(client),
                    "esp_http_client_set_redirection",
                ) {
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(err);
                }
                let mut flushed = 0;
                let _ = sys::esp_http_client_flush_response(client, &mut flushed);
                sys::esp_http_client_close(client);
                redirect_count += 1;
                continue;
            }

            let body = match read_body_stream(client) {
                Ok(body) => body,
                Err(err) => {
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(err);
                }
            };
            let server_header = get_header_value(client, "Server").ok().flatten();
            let via_header = get_header_value(client, "Via").ok().flatten();
            let content_type = get_header_value(client, "Content-Type").ok().flatten();
            let www_authenticate = get_header_value(client, "WWW-Authenticate").ok().flatten();
            sys::esp_http_client_close(client);
            sys::esp_http_client_cleanup(client);
            if status != 200 {
                let body_preview = String::from_utf8_lossy(&body);
                let body_preview = body_preview.trim();
                let body_preview = if body_preview.is_empty() {
                    String::new()
                } else {
                    body_preview.chars().take(160).collect::<String>()
                };
                let mut details = format!(
                    "unexpected status: {status} url={}",
                    url.to_str().unwrap_or_default()
                );
                if let Some(value) = server_header.as_deref() {
                    details.push_str(&format!(" server={value}"));
                }
                if let Some(value) = via_header.as_deref() {
                    details.push_str(&format!(" via={value}"));
                }
                if let Some(value) = content_type.as_deref() {
                    details.push_str(&format!(" content_type={value}"));
                }
                if let Some(value) = www_authenticate.as_deref() {
                    details.push_str(&format!(" www_authenticate={value}"));
                }
                if !body_preview.is_empty() {
                    details.push_str(&format!(" body={body_preview}"));
                }
                return Err(details);
            }
            return Ok(body);
        }
    }
}

#[cfg(target_os = "espidf")]
fn http_post_json_status(
    url: &str,
    token_header: Option<(&str, &str)>,
    body: &[u8],
) -> Result<i32, String> {
    let url = CString::new(url).map_err(|err| err.to_string())?;
    unsafe {
        let mut config: sys::esp_http_client_config_t = std::mem::zeroed();
        config.url = url.as_ptr();
        config.timeout_ms = 20_000;
        config.method = sys::esp_http_client_method_t_HTTP_METHOD_POST;
        config.disable_auto_redirect = false;
        if is_https_url(url.to_str().unwrap_or_default()) {
            config.crt_bundle_attach = Some(sys::esp_crt_bundle_attach);
        }
        let client = sys::esp_http_client_init(&config);
        if client.is_null() {
            return Err("esp_http_client_init failed".into());
        }
        if let Some((header, token)) = token_header
            && !token.is_empty()
        {
            if let Err(err) = set_header(client, header, token) {
                sys::esp_http_client_cleanup(client);
                return Err(format!("set_header {header} failed: {err}"));
            }
        }
        if let Err(err) = set_header(client, "Content-Type", "application/json") {
            sys::esp_http_client_cleanup(client);
            return Err(format!("set_header Content-Type failed: {err}"));
        }
        if let Err(err) = check_esp(
            sys::esp_http_client_set_post_field(
                client,
                body.as_ptr() as *const c_char,
                body.len() as i32,
            ),
            "esp_http_client_set_post_field",
        ) {
            sys::esp_http_client_cleanup(client);
            return Err(err);
        }
        if let Err(err) = check_esp(
            sys::esp_http_client_perform(client),
            "esp_http_client_perform",
        ) {
            sys::esp_http_client_cleanup(client);
            return Err(err);
        }
        let status = sys::esp_http_client_get_status_code(client);
        sys::esp_http_client_cleanup(client);
        Ok(status)
    }
}

#[cfg(target_os = "espidf")]
unsafe fn set_header(
    client: sys::esp_http_client_handle_t,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let key = CString::new(key).map_err(|err| err.to_string())?;
    let value = CString::new(value).map_err(|err| err.to_string())?;
    check_esp(
        unsafe { sys::esp_http_client_set_header(client, key.as_ptr(), value.as_ptr()) },
        "esp_http_client_set_header",
    )
}

#[cfg(target_os = "espidf")]
unsafe fn get_header_value(
    client: sys::esp_http_client_handle_t,
    key: &str,
) -> Result<Option<String>, String> {
    let key = CString::new(key).map_err(|err| err.to_string())?;
    let mut ptr_value: *mut c_char = ptr::null_mut();
    let err = unsafe { sys::esp_http_client_get_header(client, key.as_ptr(), &mut ptr_value) };
    if err != 0 || ptr_value.is_null() {
        return Ok(None);
    }
    Ok(Some(
        unsafe { CStr::from_ptr(ptr_value) }
            .to_string_lossy()
            .into_owned(),
    ))
}

#[cfg(target_os = "espidf")]
unsafe fn read_body_stream(client: sys::esp_http_client_handle_t) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 1024];
    let mut transient_reads = 0usize;
    loop {
        let read = unsafe {
            sys::esp_http_client_read(
                client,
                chunk.as_mut_ptr() as *mut c_char,
                chunk.len() as i32,
            )
        };
        if read == -(sys::ESP_ERR_HTTP_EAGAIN as i32) {
            transient_reads = transient_reads.saturating_add(1);
            if transient_reads > HTTP_READ_RETRY_LIMIT {
                return Err(format!(
                    "esp_http_client_read timed out repeatedly while draining body: retries={} bytes={}",
                    transient_reads,
                    out.len()
                ));
            }
            continue;
        }
        if read < 0 {
            return Err(format!(
                "esp_http_client_read failed while draining body: code={} bytes={}",
                read,
                out.len()
            ));
        }
        if read == 0 {
            if unsafe { !sys::esp_http_client_is_complete_data_received(client) } {
                transient_reads = transient_reads.saturating_add(1);
                if transient_reads > HTTP_READ_RETRY_LIMIT {
                    return Err(format!(
                        "body read incomplete after repeated empty reads: retries={} bytes={}",
                        transient_reads,
                        out.len()
                    ));
                }
                continue;
            }
            break;
        }
        transient_reads = 0;
        out.extend_from_slice(&chunk[..read as usize]);
    }
    Ok(out)
}

#[cfg(target_os = "espidf")]
unsafe fn read_body_exact(
    client: sys::esp_http_client_handle_t,
    content_len: usize,
    url: &str,
) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; content_len];
    let mut offset = 0usize;
    let mut transient_reads = 0usize;
    const READ_CHUNK: usize = 1024;
    while offset < content_len {
        let remaining = content_len - offset;
        let request_len = remaining.min(READ_CHUNK);
        let read = unsafe {
            sys::esp_http_client_read(
                client,
                out[offset..].as_mut_ptr() as *mut c_char,
                request_len as i32,
            )
        };
        if read == -(sys::ESP_ERR_HTTP_EAGAIN as i32) {
            transient_reads = transient_reads.saturating_add(1);
            if transient_reads > HTTP_READ_RETRY_LIMIT {
                return Err(format!(
                    "esp_http_client_read timed out repeatedly: retries={} bytes={}/{} url={}",
                    transient_reads, offset, content_len, url
                ));
            }
            continue;
        }
        if read < 0 {
            return Err(format!(
                "esp_http_client_read failed: code={} bytes={}/{} url={}",
                read, offset, content_len, url
            ));
        }
        if read == 0 {
            if unsafe { !sys::esp_http_client_is_complete_data_received(client) } {
                transient_reads = transient_reads.saturating_add(1);
                if transient_reads > HTTP_READ_RETRY_LIMIT {
                    return Err(format!(
                        "body read incomplete after repeated empty reads: retries={} bytes={}/{} url={}",
                        transient_reads, offset, content_len, url
                    ));
                }
                continue;
            }
            break;
        }
        transient_reads = 0;
        offset += read as usize;
    }
    if offset != content_len {
        return Err(format!("incomplete body: {offset}/{content_len} url={url}"));
    }
    Ok(out)
}

#[cfg(target_os = "espidf")]
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(target_os = "espidf")]
fn detect_format(content_type: Option<&str>, data: &[u8]) -> ImageFormat {
    if let Some(content_type) = content_type {
        let ct = content_type.to_ascii_lowercase();
        if ct.contains("jpeg") || ct.contains("jpg") {
            return ImageFormat::Jpeg;
        }
        if ct.contains("bmp") || ct.contains("bitmap") {
            return ImageFormat::Bmp;
        }
    }
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        ImageFormat::Jpeg
    } else {
        ImageFormat::Bmp
    }
}

#[cfg(target_os = "espidf")]
fn trim_trailing_slash(input: &str) -> String {
    input.trim_end_matches('/').to_string()
}

#[cfg(target_os = "espidf")]
fn is_https_url(url: &str) -> bool {
    url.starts_with("https://")
}

#[cfg(target_os = "espidf")]
fn url_encode_component(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
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

fn clamp_i32(value: i32, min_value: i32, max_value: i32) -> i32 {
    value.clamp(min_value, max_value)
}

#[cfg(test)]
mod tests {
    use super::resolve_redirect_url;

    #[test]
    fn resolve_redirect_url_supports_absolute_and_root_relative() {
        assert_eq!(
            resolve_redirect_url(
                "https://picsum.photos/480/800?date=2026-03-09",
                "https://fastly.picsum.photos/id/58/480/800.jpg"
            ),
            Some("https://fastly.picsum.photos/id/58/480/800.jpg".to_string())
        );
        assert_eq!(
            resolve_redirect_url(
                "https://picsum.photos/480/800?date=2026-03-09",
                "/id/58/480/800.jpg"
            ),
            Some("https://picsum.photos/id/58/480/800.jpg".to_string())
        );
    }

    #[test]
    fn resolve_redirect_url_supports_scheme_relative_and_relative_path() {
        assert_eq!(
            resolve_redirect_url(
                "https://picsum.photos/480/800?date=2026-03-09",
                "//fastly.picsum.photos/id/58/480/800.jpg"
            ),
            Some("https://fastly.picsum.photos/id/58/480/800.jpg".to_string())
        );
        assert_eq!(
            resolve_redirect_url("https://picsum.photos/images/list", "next?page=2"),
            Some("https://picsum.photos/images/next?page=2".to_string())
        );
    }
}
