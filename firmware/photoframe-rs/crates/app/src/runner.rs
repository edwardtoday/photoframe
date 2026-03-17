use photoframe_contracts::{DeviceCheckinRequest, DeviceNextResponse};
use photoframe_domain::{
    CycleAction, FailureKind, LongPressAction, WakeSource, apply_cycle_outcome,
    decide_cycle_action, sleep_seconds_until_next_beijing_sync,
};

use crate::{
    DeviceRuntimeConfig, ImageArtifact, ImageFetchOutcome, ImageFetchPlan,
    build_checkin_base_url_candidates, build_dated_url, build_fetch_url_candidates,
    model::PowerSample, split_url_origin_and_rest,
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

    fn report_debug_stage(
        &mut self,
        _config: &DeviceRuntimeConfig,
        _stage: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

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
}

pub struct CycleRunner<C, S, O, I, D> {
    clock: C,
    storage: S,
    orchestrator: O,
    image_fetcher: I,
    display: D,
}

impl<C, S, O, I, D> CycleRunner<C, S, O, I, D> {
    pub fn new(clock: C, storage: S, orchestrator: O, image_fetcher: I, display: D) -> Self {
        Self {
            clock,
            storage,
            orchestrator,
            image_fetcher,
            display,
        }
    }

    pub fn orchestrator(&self) -> &O {
        &self.orchestrator
    }

    pub fn image_fetcher(&self) -> &I {
        &self.image_fetcher
    }

    pub fn display(&self) -> &D {
        &self.display
    }
}

impl<C, S, O, I, D> CycleRunner<C, S, O, I, D>
where
    C: Clock,
    S: Storage,
    O: OrchestratorApi,
    I: ImageFetcher,
    D: Display,
{
    /// 单轮编排只处理“策略与状态转换”，不依赖硬件细节，便于宿主机验证。
    pub fn run(&mut self, boot: BootContext) -> Result<CycleReport, String> {
        let mut config = self.storage.load_config()?;
        let action = decide_cycle_action(boot.wake_source);
        let portal_window_opened =
            matches!(boot.long_press_action, LongPressAction::OpenStaPortalWindow);
        let now_epoch = self.clock.now_epoch();

        if matches!(
            boot.long_press_action,
            LongPressAction::ClearWifiAndEnterPortal
        ) {
            config.clear_wifi_credentials();
            self.storage.save_config(&config)?;
            return Ok(CycleReport {
                exit: CycleExit::EnterApPortal,
                action,
                image_source: "portal".into(),
                fetch_url_used: None,
                checkin_reported: false,
                portal_window_opened: false,
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
            });
        }

        if config.orchestrator_enabled
            && !config.orchestrator_base_url.is_empty()
            && let Some(next_config) = self.orchestrator.sync_config(&config, now_epoch)?
        {
            self.storage.save_config(&next_config)?;
            return Ok(CycleReport {
                exit: CycleExit::RebootForConfig,
                action,
                image_source: "config-sync".into(),
                fetch_url_used: None,
                checkin_reported: false,
                portal_window_opened,
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

        if config.orchestrator_enabled && !config.orchestrator_base_url.is_empty() {
            if let Some(directive) =
                self.orchestrator
                    .fetch_directive(&config, now_epoch, success_sleep_seconds)?
            {
                url = directive.image_url;
                image_source = directive.source.unwrap_or_else(|| "daily".into());
                if let Some(seconds) = directive.poll_after_seconds {
                    success_sleep_seconds = success_sleep_seconds.min(u64::from(seconds.max(60)));
                }
                used_orchestrator_directive = true;
            }
        }

        let force_refresh = matches!(action, CycleAction::ForceRefresh);
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

        for candidate in fetch_urls {
            let orchestrator_token =
                orchestrator_token_for_url(orchestrator_origin.as_deref(), &candidate, &config);
            let result = self.image_fetcher.fetch(&ImageFetchPlan {
                url: candidate.clone(),
                previous_sha256: config.last_image_sha256.clone(),
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
                url: fallback_url.clone(),
                previous_sha256: config.last_image_sha256.clone(),
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
                if let Err(kind) = self.display.render(artifact, &config, force_refresh) {
                    render_failure = Some(kind);
                    last_error = match kind {
                        FailureKind::PmicSoftFailure => "pmic soft failure".into(),
                        _ => "render failed".into(),
                    };
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
            if fetch.image_changed {
                config.last_image_sha256 = fetch.sha256.clone();
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
                    &image_source,
                    "",
                    boot.sta_ip.clone(),
                    boot.power_sample,
                    next_wakeup_epoch,
                    success_sleep_seconds,
                    fetch_url_used.as_deref().unwrap_or_default(),
                    &fallback_url,
                )
                .unwrap_or(false);

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
            });
        }

        let failure_kind = render_failure.unwrap_or(FailureKind::GeneralFailure);
        let decision =
            apply_cycle_outcome(&config.retry_policy(), config.failure_count, failure_kind);
        let failure_sleep_seconds =
            sleep_seconds_until_next_beijing_sync(now_epoch).unwrap_or(decision.sleep_seconds);
        let _ = self.orchestrator.report_debug_stage(
            &config,
            if fetch.ok {
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
                &image_source,
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
        })
    }

    fn report_checkin(
        &mut self,
        config: &DeviceRuntimeConfig,
        last_http_status: i32,
        fetch_ok: bool,
        image_changed: bool,
        image_source: &str,
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
        let payload = DeviceCheckinRequest {
            device_id: config.device_id.clone(),
            checkin_epoch: self.clock.now_epoch(),
            next_wakeup_epoch,
            sleep_seconds,
            poll_interval_seconds: sleep_seconds.min(u64::from(u32::MAX)) as u32,
            failure_count: config.failure_count,
            last_http_status,
            fetch_ok,
            image_changed,
            image_source: image_source.to_string(),
            last_error: last_error.to_string(),
            sta_ip,
            battery_mv: power_sample.battery_mv,
            battery_percent: power_sample.battery_percent,
            charging: power_sample.charging,
            vbus_good: power_sample.vbus_good,
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
