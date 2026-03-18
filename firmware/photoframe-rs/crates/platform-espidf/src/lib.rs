#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

use photoframe_app::{
    Clock, DeviceRuntimeConfig, Display, ImageArtifact, ImageFetchOutcome, ImageFetchPlan,
    ImageFetcher, OrchestratorApi, Storage,
};
use photoframe_contracts::{DeviceCheckinRequest, DeviceNextResponse};
use photoframe_domain::FailureKind;

#[cfg(target_os = "espidf")]
use photoframe_app::{ImageFormat, WifiCredential};
#[cfg(target_os = "espidf")]
use photoframe_contracts::{DeviceConfigPayload, DeviceConfigResponse, RemoteConfigPatch};
#[cfg(target_os = "espidf")]
use photoframe_domain::{device_id_from_mac_suffix, token_hex_from_bytes};

#[cfg(target_os = "espidf")]
use esp_idf_sys as sys;

#[cfg(target_os = "espidf")]
use std::{
    ffi::{CStr, CString},
    os::raw::c_char,
    ptr,
    time::Instant,
};

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
            config.last_image_etag = self.get_string("img_etag")?.unwrap_or_default();
            config.last_image_last_modified = self.get_string("img_lm")?.unwrap_or_default();
            config.preferred_image_origin = self.get_string("img_origin")?.unwrap_or_default();
            config.last_success_epoch = self.get_i64("last_ok")?.unwrap_or(0);
            config.last_time_sync_epoch = self.get_i64("time_sync")?.unwrap_or(0);
            config.failure_count = self.get_i32("fail_cnt")?.unwrap_or(0).max(0) as u32;
            config.remote_config_version = self.get_i32("cfg_ver")?.unwrap_or(0).max(0);

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
            self.set_string("img_etag", &config.last_image_etag)?;
            self.set_string("img_lm", &config.last_image_last_modified)?;
            self.set_string("img_origin", &config.preferred_image_origin)?;
            self.set_i64("last_ok", config.last_success_epoch)?;
            self.set_i64("time_sync", config.last_time_sync_epoch)?;
            self.set_i32("fail_cnt", config.failure_count as i32)?;
            self.set_i32("cfg_ver", config.remote_config_version)?;
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
                "{}/api/v1/device/config?device_id={}&now_epoch={}&current_version={}",
                trim_trailing_slash(&config.orchestrator_base_url),
                url_encode_component(&config.device_id),
                now_epoch,
                config.remote_config_version.max(0),
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
                "{}/api/v1/device/next?device_id={}&now_epoch={}&default_poll_seconds={}&failure_count={}&accept_formats=jpeg,bmp",
                trim_trailing_slash(&config.orchestrator_base_url),
                url_encode_component(&config.device_id),
                now_epoch,
                default_poll_seconds,
                config.failure_count,
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
                            println!(
                                "photoframe-rs/checkin: ok device_id={} base={}/{} attempt={}/3 status={} url={}",
                                payload.device_id,
                                base_index + 1,
                                base_urls.len(),
                                attempt + 1,
                                status,
                                url
                            );
                            return Ok(());
                        }
                        Ok(status) => {
                            last_error = format!("non-2xx status={status}");
                            println!(
                                "photoframe-rs/checkin: non-2xx device_id={} base={}/{} attempt={}/3 status={} url={}",
                                payload.device_id,
                                base_index + 1,
                                base_urls.len(),
                                attempt + 1,
                                status,
                                url
                            );
                        }
                        Err(err) => {
                            last_error = err.clone();
                            println!(
                                "photoframe-rs/checkin: error device_id={} base={}/{} attempt={}/3 err={} url={}",
                                payload.device_id,
                                base_index + 1,
                                base_urls.len(),
                                attempt + 1,
                                err,
                                url
                            );
                        }
                    }
                }
                println!(
                    "photoframe-rs/checkin: base failed device_id={} base={}/{} last_error={} url={}",
                    payload.device_id,
                    base_index + 1,
                    base_urls.len(),
                    last_error,
                    url
                );
            }
            Err(last_error)
        }
    }

    fn report_debug_stage(
        &mut self,
        config: &DeviceRuntimeConfig,
        stage: &str,
    ) -> Result<(), String> {
        send_debug_stage_beacon(config, stage)
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

        let url = format!(
            "{}/api/v1/device/debug-stage?device_id={}&stage={}",
            trim_trailing_slash(&config.orchestrator_base_url),
            url_encode_component(&config.device_id),
            url_encode_component(stage),
        );
        match http_get_bytes(
            &url,
            Some((&PHOTOFRAME_TOKEN_HEADER, &config.orchestrator_token)),
        ) {
            Ok(_) => Ok(()),
            Err(err) => {
                println!(
                    "photoframe-rs/debug-stage: failed device_id={} stage={} err={}",
                    config.device_id, stage, err
                );
                Err(err)
            }
        }
    }
}

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

#[cfg(target_os = "espidf")]
const PHOTOFRAME_TOKEN_HEADER: &str = "X-PhotoFrame-Token";
#[cfg(target_os = "espidf")]
const PHOTO_TOKEN_HEADER: &str = "X-Photo-Token";
#[cfg(target_os = "espidf")]
const MAX_HTTP_REDIRECTS: usize = 5;

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
                println!(
                    "photoframe-rs/timing: fetch status=304 total={}ms headers={}ms body=0ms bytes=0 changed=false format=unchanged url={}",
                    fetch_start.elapsed().as_millis(),
                    headers_ms,
                    plan.url
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
            let data = match read_body_exact(client, content_len as usize) {
                Ok(data) => data,
                Err(err) => {
                    sys::esp_http_client_close(client);
                    sys::esp_http_client_cleanup(client);
                    return Err(err);
                }
            };
            let body_ms = body_start.elapsed().as_millis();
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
    loop {
        let read = unsafe {
            sys::esp_http_client_read(
                client,
                chunk.as_mut_ptr() as *mut c_char,
                chunk.len() as i32,
            )
        };
        if read < 0 {
            return Err("esp_http_client_read failed".into());
        }
        if read == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..read as usize]);
    }
    Ok(out)
}

#[cfg(target_os = "espidf")]
unsafe fn read_body_exact(
    client: sys::esp_http_client_handle_t,
    content_len: usize,
) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; content_len];
    let mut offset = 0usize;
    while offset < content_len {
        let read = unsafe {
            sys::esp_http_client_read(
                client,
                out[offset..].as_mut_ptr() as *mut c_char,
                (content_len - offset) as i32,
            )
        };
        if read <= 0 {
            break;
        }
        offset += read as usize;
    }
    if offset != content_len {
        return Err(format!("incomplete body: {offset}/{content_len}"));
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
