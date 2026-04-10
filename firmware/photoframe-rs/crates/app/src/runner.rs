use photoframe_contracts::{
    DeviceCheckinRequest, DeviceLogUploadRequest, DeviceLogUploadRequestBody, DeviceNextResponse,
    FirmwareUpdateDirective,
};
use photoframe_domain::{
    CycleAction, FailureKind, LongPressAction, WakeSource, apply_cycle_outcome,
    decide_cycle_action, sleep_seconds_until_next_beijing_sync,
};

use crate::{
    DeviceRuntimeConfig, ImageArtifact, ImageFetchOutcome, ImageFetchPlan,
    build_checkin_base_url_candidates, build_dated_url, build_fetch_url_candidates,
    extract_date_from_url,
    model::{FirmwareRuntimeStatus, PowerSample},
    split_url_origin_and_rest,
};

pub trait Clock {
    fn now_epoch(&self) -> i64;
    fn today_date_string(&self) -> String;
}

pub trait Storage {
    fn load_config(&mut self) -> Result<DeviceRuntimeConfig, String>;
    fn save_config(&mut self, config: &DeviceRuntimeConfig) -> Result<(), String>;
}

pub trait OrchestratorApi {
    fn sync_config(
        &mut self,
        config: &DeviceRuntimeConfig,
        now_epoch: i64,
    ) -> Result<Option<DeviceRuntimeConfig>, String>;

    fn fetch_directive(
        &mut self,
        config: &DeviceRuntimeConfig,
        now_epoch: i64,
        preferred_poll_seconds: u64,
    ) -> Result<Option<DeviceNextResponse>, String>;

    fn report_checkin(
        &mut self,
        base_urls: &[String],
        payload: &DeviceCheckinRequest,
    ) -> Result<(), String>;

    fn report_config_applied(
        &mut self,
        config: &DeviceRuntimeConfig,
        config_version: i32,
        applied: bool,
        error: &str,
        applied_epoch: i64,
    ) -> Result<(), String> {
        let _ = (config, config_version, applied, error, applied_epoch);
        Ok(())
    }

    fn report_debug_stage(
        &mut self,
        _config: &DeviceRuntimeConfig,
        _stage: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    fn upload_logs(
        &mut self,
        _config: &DeviceRuntimeConfig,
        _request: &DeviceLogUploadRequest,
        _payload: &DeviceLogUploadRequestBody,
    ) -> Result<(), String> {
        Ok(())
    }
}

pub trait LogUploadProvider {
    fn collect_logs(
        &mut self,
        _config: &DeviceRuntimeConfig,
        _request: &DeviceLogUploadRequest,
        _uploaded_epoch: i64,
    ) -> Option<DeviceLogUploadRequestBody> {
        None
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NoopLogUploadProvider;

impl LogUploadProvider for NoopLogUploadProvider {}

pub trait FirmwareUpdater {
    fn install_update(
        &mut self,
        _config: &DeviceRuntimeConfig,
        _directive: &FirmwareUpdateDirective,
    ) -> Result<bool, String> {
        Ok(false)
    }

    fn confirm_running_firmware(&mut self, _config: &DeviceRuntimeConfig) -> Result<(), String> {
        Ok(())
    }

    fn current_status(&mut self, config: &DeviceRuntimeConfig) -> FirmwareRuntimeStatus {
        FirmwareRuntimeStatus {
            ota_target_version: config.ota_target_version.clone(),
            ota_last_error: config.ota_last_error.clone(),
            ota_last_attempt_epoch: config.ota_last_attempt_epoch,
            ..FirmwareRuntimeStatus::default()
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NoopFirmwareUpdater;

impl FirmwareUpdater for NoopFirmwareUpdater {}

pub trait ImageFetcher {
    fn fetch(&mut self, plan: &ImageFetchPlan) -> ImageFetchOutcome;
}

pub trait Display {
    fn render(
        &mut self,
        artifact: &ImageArtifact,
        config: &DeviceRuntimeConfig,
        force_refresh: bool,
    ) -> Result<(), FailureKind>;

    fn after_render_success(
        &mut self,
        _artifact: &ImageArtifact,
        _config: &DeviceRuntimeConfig,
        _image_sha256: &str,
        _image_date: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootContext {
    pub wake_source: WakeSource,
    pub long_press_action: LongPressAction,
    pub sta_ip: Option<String>,
    pub power_sample: PowerSample,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CycleExit {
    EnterApPortal,
    RebootForConfig,
    RebootForFirmwareUpdate,
    Sleep { seconds: u64, timer_only: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleReport {
    pub exit: CycleExit,
    pub action: CycleAction,
    pub image_source: String,
    pub fetch_url_used: Option<String>,
    pub checkin_reported: bool,
    pub portal_window_opened: bool,
    pub logs_uploaded: bool,
}

pub struct CycleRunner<C, S, O, I, D, F = NoopFirmwareUpdater, L = NoopLogUploadProvider> {
    clock: C,
    storage: S,
    orchestrator: O,
    image_fetcher: I,
    display: D,
    firmware_updater: F,
    log_upload_provider: L,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirmwareUpdateDecision {
    Attempt,
    SkipEmpty,
    SkipSameVersion,
    SkipRequiresVbus,
    SkipBattery,
}

impl<C, S, O, I, D> CycleRunner<C, S, O, I, D, NoopFirmwareUpdater, NoopLogUploadProvider> {
    pub fn new(clock: C, storage: S, orchestrator: O, image_fetcher: I, display: D) -> Self {
        Self::new_with_services(
            clock,
            storage,
            orchestrator,
            image_fetcher,
            display,
            NoopFirmwareUpdater,
            NoopLogUploadProvider,
        )
    }
}

impl<C, S, O, I, D, F> CycleRunner<C, S, O, I, D, F, NoopLogUploadProvider> {
    pub fn new_with_firmware_updater(
        clock: C,
        storage: S,
        orchestrator: O,
        image_fetcher: I,
        display: D,
        firmware_updater: F,
    ) -> Self {
        Self::new_with_services(
            clock,
            storage,
            orchestrator,
            image_fetcher,
            display,
            firmware_updater,
            NoopLogUploadProvider,
        )
    }
}

impl<C, S, O, I, D, L> CycleRunner<C, S, O, I, D, NoopFirmwareUpdater, L> {
    pub fn new_with_log_upload_provider(
        clock: C,
        storage: S,
        orchestrator: O,
        image_fetcher: I,
        display: D,
        log_upload_provider: L,
    ) -> Self {
        Self::new_with_services(
            clock,
            storage,
            orchestrator,
            image_fetcher,
            display,
            NoopFirmwareUpdater,
            log_upload_provider,
        )
    }
}

impl<C, S, O, I, D, F, L> CycleRunner<C, S, O, I, D, F, L> {
    pub fn new_with_services(
        clock: C,
        storage: S,
        orchestrator: O,
        image_fetcher: I,
        display: D,
        firmware_updater: F,
        log_upload_provider: L,
    ) -> Self {
        Self {
            clock,
            storage,
            orchestrator,
            image_fetcher,
            display,
            firmware_updater,
            log_upload_provider,
        }
    }

    pub fn orchestrator(&self) -> &O {
        &self.orchestrator
    }

    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    pub fn image_fetcher(&self) -> &I {
        &self.image_fetcher
    }

    pub fn display(&self) -> &D {
        &self.display
    }
}

impl<C, S, O, I, D, F, L> CycleRunner<C, S, O, I, D, F, L>
where
    C: Clock,
    S: Storage,
    O: OrchestratorApi,
    I: ImageFetcher,
    D: Display,
    F: FirmwareUpdater,
    L: LogUploadProvider,
{
    /// 单轮编排只处理“策略与状态转换”，不依赖硬件细节，便于宿主机验证。
    pub fn run(&mut self, boot: BootContext) -> Result<CycleReport, String> {
        let mut config = self.storage.load_config()?;
        let action = decide_cycle_action(boot.wake_source);
        let portal_window_opened = false;
        let now_epoch = self.clock.now_epoch();

        if matches!(boot.long_press_action, LongPressAction::EnterApPortal) {
            return Ok(CycleReport {
                exit: CycleExit::EnterApPortal,
                action,
                image_source: "portal".into(),
                fetch_url_used: None,
                checkin_reported: false,
                portal_window_opened: false,
                logs_uploaded: false,
            });
        }

        if matches!(action, CycleAction::SleepTimerOnly) {
            let normal_sleep_seconds = u64::from(config.interval_minutes.max(1)) * 60;
            let sleep_seconds = sleep_seconds_until_next_beijing_sync(now_epoch)
                .unwrap_or(normal_sleep_seconds.clamp(60, 600));
            return Ok(CycleReport {
                exit: CycleExit::Sleep {
                    seconds: sleep_seconds,
                    timer_only: true,
                },
                action,
                image_source: "spurious_ext1".into(),
                fetch_url_used: None,
                checkin_reported: false,
                portal_window_opened: false,
                logs_uploaded: false,
            });
        }

        if !config.has_wifi_credentials() {
            return Ok(CycleReport {
                exit: CycleExit::EnterApPortal,
                action,
                image_source: "portal".into(),
                fetch_url_used: None,
                checkin_reported: false,
                portal_window_opened,
                logs_uploaded: false,
            });
        }

        if config.orchestrator_enabled
            && !config.orchestrator_base_url.is_empty()
            && let Some(next_config) = self.orchestrator.sync_config(&config, now_epoch)?
        {
            if let Err(err) = self.storage.save_config(&next_config) {
                let _ = self.orchestrator.report_config_applied(
                    &config,
                    next_config.remote_config_version,
                    false,
                    &err,
                    now_epoch,
                );
                return Err(format!("save synced config failed: {err}"));
            }
            let _ = self.orchestrator.report_config_applied(
                &config,
                next_config.remote_config_version,
                true,
                "",
                now_epoch,
            );
            return Ok(CycleReport {
                exit: CycleExit::RebootForConfig,
                action,
                image_source: "config-sync".into(),
                fetch_url_used: None,
                checkin_reported: false,
                portal_window_opened,
                logs_uploaded: false,
            });
        }

        let fallback_url = build_dated_url(
            &config.image_url_template,
            &self.clock.today_date_string(),
            &config.device_id,
        );
        let mut url = fallback_url.clone();
        let mut image_source = String::from("daily");
        let mut success_sleep_seconds = sleep_seconds_until_next_beijing_sync(now_epoch)
            .unwrap_or_else(|| u64::from(config.interval_minutes.max(1)) * 60);
        let mut used_orchestrator_directive = false;
        let mut log_upload_request: Option<DeviceLogUploadRequest> = None;
        let mut firmware_update: Option<FirmwareUpdateDirective> = None;

        if config.orchestrator_enabled && !config.orchestrator_base_url.is_empty() {
            if let Some(directive) =
                self.orchestrator
                    .fetch_directive(&config, now_epoch, success_sleep_seconds)?
            {
                log_upload_request = directive.log_upload_request.clone();
                firmware_update = directive.firmware_update.clone();
                url = directive.image_url;
                image_source = directive.source.unwrap_or_else(|| "daily".into());
                if let Some(seconds) = directive.poll_after_seconds {
                    success_sleep_seconds = success_sleep_seconds.min(u64::from(seconds.max(60)));
                }
                used_orchestrator_directive = true;
            }
        }

        if let Some(update) = firmware_update.as_ref() {
            let decision = self.should_attempt_firmware_update(update, boot.power_sample, &config);
            println!(
                "photoframe-rs/ota: directive version={} current={} batt={} charging={} vbus={} requires_vbus={} min_batt={:?} decision={:?}",
                update.version,
                config.firmware_version(),
                boot.power_sample.battery_percent,
                boot.power_sample.charging,
                boot.power_sample.vbus_good,
                update.requires_vbus,
                update.min_battery_percent,
                decision,
            );
            let ota_stage = match decision {
                FirmwareUpdateDecision::Attempt => "ota_decision_attempt",
                FirmwareUpdateDecision::SkipEmpty => "ota_skip_empty",
                FirmwareUpdateDecision::SkipSameVersion => "ota_skip_same_version",
                FirmwareUpdateDecision::SkipRequiresVbus => "ota_skip_requires_vbus",
                FirmwareUpdateDecision::SkipBattery => "ota_skip_battery",
            };
            let _ = self.orchestrator.report_debug_stage(&config, ota_stage);
            if decision == FirmwareUpdateDecision::Attempt {
                config.ota_target_version = update.version.clone();
                config.ota_last_error.clear();
                config.ota_last_attempt_epoch = now_epoch;
                self.storage.save_config(&config)?;

                match self.firmware_updater.install_update(&config, update) {
                    Ok(true) => {
                        self.storage.save_config(&config)?;
                        return Ok(CycleReport {
                            exit: CycleExit::RebootForFirmwareUpdate,
                            action,
                            image_source: "firmware-update".into(),
                            fetch_url_used: None,
                            checkin_reported: false,
                            portal_window_opened,
                            logs_uploaded: false,
                        });
                    }
                    Ok(false) => {}
                    Err(err) => {
                        config.ota_last_error = err;
                        self.storage.save_config(&config)?;
                    }
                }
            }
        }

        let force_refresh =
            matches!(action, CycleAction::ForceRefresh) || config.manual_history_active;
        let previous_sha256 = if force_refresh {
            String::new()
        } else {
            config.last_image_sha256.clone()
        };
        let previous_etag = if force_refresh || config.last_image_etag.is_empty() {
            None
        } else {
            Some(config.last_image_etag.clone())
        };
        let previous_last_modified = if force_refresh || config.last_image_last_modified.is_empty()
        {
            None
        } else {
            Some(config.last_image_last_modified.clone())
        };

        let mut fetch_urls = Vec::new();
        if used_orchestrator_directive
            && image_source == "daily"
            && let Some((origin, _)) = split_url_origin_and_rest(&config.orchestrator_base_url)
        {
            append_unique_urls(
                &mut fetch_urls,
                build_fetch_url_candidates(&url, &origin).into_iter(),
            );
        }
        append_unique_urls(
            &mut fetch_urls,
            build_fetch_url_candidates(&url, &config.preferred_image_origin).into_iter(),
        );
        let mut fetch = ImageFetchOutcome::failed(0, "fetch not started");
        let mut fetch_url_used = None;
        let orchestrator_origin =
            split_url_origin_and_rest(&config.orchestrator_base_url).map(|(origin, _)| origin);

        let _ = self
            .orchestrator
            .report_debug_stage(&config, "before_fetch");
        for candidate in fetch_urls {
            let orchestrator_token =
                orchestrator_token_for_url(orchestrator_origin.as_deref(), &candidate, &config);
            let result = self.image_fetcher.fetch(&ImageFetchPlan {
                device_id: config.device_id.clone(),
                url: candidate.clone(),
                debug_stage_base_url: config.orchestrator_base_url.clone(),
                previous_sha256: previous_sha256.clone(),
                photo_token: config.photo_token.clone(),
                orchestrator_token,
                previous_etag: previous_etag.clone(),
                previous_last_modified: previous_last_modified.clone(),
            });
            if result.ok {
                fetch_url_used = Some(candidate);
                fetch = result;
                break;
            }
            fetch = result;
        }

        if !fetch.ok
            && used_orchestrator_directive
            && image_source == "daily"
            && fallback_url != url
        {
            let orchestrator_token =
                orchestrator_token_for_url(orchestrator_origin.as_deref(), &fallback_url, &config);
            let result = self.image_fetcher.fetch(&ImageFetchPlan {
                device_id: config.device_id.clone(),
                url: fallback_url.clone(),
                debug_stage_base_url: config.orchestrator_base_url.clone(),
                previous_sha256,
                photo_token: config.photo_token.clone(),
                orchestrator_token,
                previous_etag,
                previous_last_modified,
            });
            if result.ok {
                fetch_url_used = Some(fallback_url.clone());
            }
            fetch = result;
        }

        if fetch.ok {
            let _ = self
                .orchestrator
                .report_debug_stage(&config, "after_fetch_ok");
        }

        let should_refresh = force_refresh || fetch.image_changed;
        let mut render_failure = None;
        let mut last_error = String::new();

        if fetch.ok && should_refresh {
            if let Some(artifact) = fetch.artifact.as_ref() {
                let rendered_image_date = fetch_url_used.as_deref().and_then(extract_date_from_url);
                if let Err(kind) = self.display.render(artifact, &config, force_refresh) {
                    render_failure = Some(kind);
                    last_error = match kind {
                        FailureKind::PmicSoftFailure => "pmic soft failure".into(),
                        _ => "render failed".into(),
                    };
                } else if let Err(err) = self.display.after_render_success(
                    artifact,
                    &config,
                    &fetch.sha256,
                    rendered_image_date.as_deref(),
                ) {
                    render_failure = Some(FailureKind::GeneralFailure);
                    last_error = format!("persist rendered photo failed: {err}");
                }
            } else {
                render_failure = Some(FailureKind::GeneralFailure);
                last_error = "missing render artifact".into();
            }
        }

        let cycle_ok = fetch.ok && render_failure.is_none();
        let now_epoch = self.clock.now_epoch();

        if cycle_ok {
            let _ = self
                .orchestrator
                .report_debug_stage(&config, "after_render_ok");
            config.failure_count = 0;
            if should_refresh {
                config.last_image_sha256 = fetch.sha256.clone();
                config.last_image_date = fetch_url_used
                    .as_deref()
                    .and_then(extract_date_from_url)
                    .unwrap_or_default();
                config.displayed_image_sha256 = fetch.sha256.clone();
                config.displayed_image_date = config.last_image_date.clone();
                config.manual_history_active = false;
            }
            if let Some(value) = &fetch.etag {
                config.last_image_etag = value.clone();
            }
            if let Some(value) = &fetch.last_modified {
                config.last_image_last_modified = value.clone();
            }
            if let Some(url) = &fetch_url_used
                && let Some((origin, _)) = crate::split_url_origin_and_rest(url)
            {
                config.preferred_image_origin = origin;
            }
            config.last_success_epoch = now_epoch;
            self.storage.save_config(&config)?;
            let _ = self
                .orchestrator
                .report_debug_stage(&config, "after_save_ok");
            let _ = self.firmware_updater.confirm_running_firmware(&config);
            if config.ota_target_version == config.firmware_version()
                && !config.ota_last_error.is_empty()
            {
                config.ota_last_error.clear();
                self.storage.save_config(&config)?;
            }

            let next_wakeup_epoch = now_epoch + success_sleep_seconds as i64;
            let _ = self
                .orchestrator
                .report_debug_stage(&config, "before_checkin_ok");
            let checkin_reported = self
                .report_checkin(
                    &config,
                    fetch.status_code,
                    true,
                    fetch.image_changed,
                    should_refresh,
                    &image_source,
                    if should_refresh {
                        fetch_url_used.as_deref().unwrap_or_default()
                    } else {
                        ""
                    },
                    if should_refresh { &fetch.sha256 } else { "" },
                    "",
                    boot.sta_ip.clone(),
                    boot.power_sample,
                    next_wakeup_epoch,
                    success_sleep_seconds,
                    fetch_url_used.as_deref().unwrap_or_default(),
                    &fallback_url,
                )
                .unwrap_or(false);
            let logs_uploaded = self
                .upload_logs_if_requested(&config, log_upload_request.as_ref(), now_epoch)
                .unwrap_or(false);
            let _ = self.firmware_updater.confirm_running_firmware(&config);

            return Ok(CycleReport {
                exit: CycleExit::Sleep {
                    seconds: success_sleep_seconds,
                    timer_only: false,
                },
                action,
                image_source,
                fetch_url_used,
                checkin_reported,
                portal_window_opened,
                logs_uploaded,
            });
        }

        let failure_kind = render_failure.unwrap_or(FailureKind::GeneralFailure);
        let decision =
            apply_cycle_outcome(&config.retry_policy(), config.failure_count, failure_kind);
        let failure_sleep_seconds = decision.sleep_seconds;
        let _ = self.orchestrator.report_debug_stage(
            &config,
            if render_failure.is_some() {
                "after_render_fail"
            } else if fetch.ok {
                "after_fetch_fail"
            } else {
                "after_fetch_http_fail"
            },
        );
        config.failure_count = decision.next_failure_count;
        self.storage.save_config(&config)?;
        let _ = self
            .orchestrator
            .report_debug_stage(&config, "after_save_fail");

        let _ = self
            .orchestrator
            .report_debug_stage(&config, "before_checkin_fail");
        let checkin_reported = self
            .report_checkin(
                &config,
                fetch.status_code,
                false,
                fetch.image_changed,
                false,
                &image_source,
                "",
                "",
                if last_error.is_empty() {
                    &fetch.error
                } else {
                    &last_error
                },
                boot.sta_ip,
                boot.power_sample,
                now_epoch + failure_sleep_seconds as i64,
                failure_sleep_seconds,
                fetch_url_used.as_deref().unwrap_or_default(),
                &fallback_url,
            )
            .unwrap_or(false);
        let logs_uploaded = self
            .upload_logs_if_requested(&config, log_upload_request.as_ref(), now_epoch)
            .unwrap_or(false);

        Ok(CycleReport {
            exit: CycleExit::Sleep {
                seconds: failure_sleep_seconds,
                timer_only: false,
            },
            action,
            image_source,
            fetch_url_used,
            checkin_reported,
            portal_window_opened,
            logs_uploaded,
        })
    }

    fn should_attempt_firmware_update(
        &self,
        update: &FirmwareUpdateDirective,
        power_sample: PowerSample,
        config: &DeviceRuntimeConfig,
    ) -> FirmwareUpdateDecision {
        if update.version.trim().is_empty() || update.app_bin_url.trim().is_empty() {
            return FirmwareUpdateDecision::SkipEmpty;
        }
        if update.version == config.firmware_version() {
            return FirmwareUpdateDecision::SkipSameVersion;
        }
        if update.requires_vbus && power_sample.vbus_good != 1 {
            return FirmwareUpdateDecision::SkipRequiresVbus;
        }
        if let Some(min_percent) = update.min_battery_percent
            && power_sample.battery_percent >= 0
            && power_sample.battery_percent < min_percent
        {
            return FirmwareUpdateDecision::SkipBattery;
        }
        FirmwareUpdateDecision::Attempt
    }

    fn upload_logs_if_requested(
        &mut self,
        config: &DeviceRuntimeConfig,
        request: Option<&DeviceLogUploadRequest>,
        uploaded_epoch: i64,
    ) -> Result<bool, String> {
        let Some(request) = request else {
            return Ok(false);
        };
        println!(
            "photoframe-rs/log-upload: request device_id={} request_id={} max_lines={} max_bytes={}",
            config.device_id, request.request_id, request.max_lines, request.max_bytes
        );
        let Some(payload) = self
            .log_upload_provider
            .collect_logs(config, request, uploaded_epoch)
        else {
            println!(
                "photoframe-rs/log-upload: skipped device_id={} request_id={} collector_returned_none",
                config.device_id, request.request_id
            );
            return Ok(false);
        };
        println!(
            "photoframe-rs/log-upload: prepared device_id={} request_id={} line_count={} truncated={}",
            config.device_id, request.request_id, payload.line_count, payload.truncated
        );
        if let Err(err) = self.orchestrator.upload_logs(config, request, &payload) {
            println!(
                "photoframe-rs/log-upload: failed device_id={} request_id={} err={}",
                config.device_id, request.request_id, err
            );
            return Err(err);
        }
        println!(
            "photoframe-rs/log-upload: completed device_id={} request_id={}",
            config.device_id, request.request_id
        );
        Ok(true)
    }

    fn report_checkin(
        &mut self,
        config: &DeviceRuntimeConfig,
        last_http_status: i32,
        fetch_ok: bool,
        image_changed: bool,
        display_applied: bool,
        image_source: &str,
        displayed_image_url: &str,
        displayed_image_sha256: &str,
        last_error: &str,
        sta_ip: Option<String>,
        power_sample: PowerSample,
        next_wakeup_epoch: i64,
        sleep_seconds: u64,
        fetch_url_used: &str,
        fallback_url: &str,
    ) -> Result<bool, String> {
        if !config.orchestrator_enabled
            || config.orchestrator_base_url.is_empty()
            || config.device_id.is_empty()
        {
            return Ok(false);
        }

        let base_urls = build_checkin_base_url_candidates(
            &config.orchestrator_base_url,
            fetch_url_used,
            fallback_url,
            &config.preferred_image_origin,
            &config.image_url_template,
        );
        let firmware_status = self.firmware_updater.current_status(config);
        let payload = DeviceCheckinRequest {
            device_id: config.device_id.clone(),
            checkin_epoch: self.clock.now_epoch(),
            next_wakeup_epoch,
            sleep_seconds,
            poll_interval_seconds: u64::from(config.interval_minutes.max(1))
                .saturating_mul(60)
                .min(u64::from(u32::MAX)) as u32,
            failure_count: config.failure_count,
            last_http_status,
            fetch_ok,
            image_changed,
            display_applied,
            image_source: image_source.to_string(),
            displayed_image_url: displayed_image_url.to_string(),
            displayed_image_sha256: displayed_image_sha256.to_string(),
            last_error: last_error.to_string(),
            sta_ip,
            battery_mv: power_sample.battery_mv,
            battery_percent: power_sample.battery_percent,
            charging: power_sample.charging,
            vbus_good: power_sample.vbus_good,
            running_partition: firmware_status.running_partition,
            ota_state: firmware_status.ota_state,
            ota_target_version: firmware_status.ota_target_version,
            ota_last_error: firmware_status.ota_last_error,
            ota_last_attempt_epoch: firmware_status.ota_last_attempt_epoch,
            reported_config: config.to_reported_config(),
        };
        self.orchestrator.report_checkin(&base_urls, &payload)?;
        Ok(true)
    }
}

fn orchestrator_token_for_url(
    orchestrator_origin: Option<&str>,
    url: &str,
    config: &DeviceRuntimeConfig,
) -> String {
    if config.orchestrator_token.is_empty() {
        return String::new();
    }
    let Some(expected_origin) = orchestrator_origin else {
        return String::new();
    };
    let Some((origin, _)) = split_url_origin_and_rest(url) else {
        return String::new();
    };
    if origin == expected_origin {
        return config.orchestrator_token.clone();
    }
    String::new()
}

fn append_unique_urls(out: &mut Vec<String>, candidates: impl Iterator<Item = String>) {
    for candidate in candidates {
        if !out.iter().any(|item| item == &candidate) {
            out.push(candidate);
        }
    }
}
