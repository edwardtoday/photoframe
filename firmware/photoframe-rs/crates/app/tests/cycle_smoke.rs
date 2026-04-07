use photoframe_app::{
    BootContext, Clock, CycleExit, CycleRunner, DeviceRuntimeConfig, Display, ImageArtifact,
    FirmwareRuntimeStatus, FirmwareUpdater, ImageFetchOutcome, ImageFetchPlan, ImageFetcher, ImageFormat,
    LogUploadProvider, OrchestratorApi, PowerSample, Storage, WifiCredential,
};
use photoframe_contracts::{
    DeviceCheckinRequest, DeviceLogUploadRequest, DeviceLogUploadRequestBody,
    DeviceNextResponse, FirmwareUpdateDirective,
};
use photoframe_domain::{FailureKind, LongPressAction, WakeSource};

const NEXT_BEIJING_SYNC_SLEEP_SECONDS: u64 = 43_600;

#[derive(Default)]
struct FakeClock;
impl Clock for FakeClock {
    fn now_epoch(&self) -> i64 {
        1_760_000_000
    }

    fn today_date_string(&self) -> String {
        "2026-03-07".into()
    }
}

struct FakeStorage {
    config: DeviceRuntimeConfig,
    save_count: usize,
}
impl Storage for FakeStorage {
    fn load_config(&mut self) -> Result<DeviceRuntimeConfig, String> {
        Ok(self.config.clone())
    }

    fn save_config(&mut self, config: &DeviceRuntimeConfig) -> Result<(), String> {
        self.config = config.clone();
        self.save_count += 1;
        Ok(())
    }
}

#[derive(Default)]
struct FakeOrchestrator {
    sync_result: Option<DeviceRuntimeConfig>,
    directive: Option<DeviceNextResponse>,
    sync_calls: usize,
    directive_calls: usize,
    last_preferred_poll_seconds: Option<u64>,
    checkin_calls: usize,
    last_checkin_payload: Option<DeviceCheckinRequest>,
    checkin_error: Option<String>,
    config_applied_calls: Vec<(i32, bool, String)>,
    debug_stages: Vec<String>,
    upload_log_calls: usize,
    last_log_upload_request: Option<DeviceLogUploadRequest>,
    last_log_upload_payload: Option<DeviceLogUploadRequestBody>,
    upload_log_error: Option<String>,
}
impl OrchestratorApi for FakeOrchestrator {
    fn sync_config(
        &mut self,
        _config: &DeviceRuntimeConfig,
        _now_epoch: i64,
    ) -> Result<Option<DeviceRuntimeConfig>, String> {
        self.sync_calls += 1;
        Ok(self.sync_result.clone())
    }

    fn fetch_directive(
        &mut self,
        _config: &DeviceRuntimeConfig,
        _now_epoch: i64,
        preferred_poll_seconds: u64,
    ) -> Result<Option<DeviceNextResponse>, String> {
        self.directive_calls += 1;
        self.last_preferred_poll_seconds = Some(preferred_poll_seconds);
        Ok(self.directive.clone())
    }

    fn report_checkin(
        &mut self,
        _base_urls: &[String],
        payload: &DeviceCheckinRequest,
    ) -> Result<(), String> {
        self.checkin_calls += 1;
        self.last_checkin_payload = Some(payload.clone());
        if let Some(error) = &self.checkin_error {
            return Err(error.clone());
        }
        Ok(())
    }

    fn report_config_applied(
        &mut self,
        _config: &DeviceRuntimeConfig,
        config_version: i32,
        applied: bool,
        error: &str,
        _applied_epoch: i64,
    ) -> Result<(), String> {
        self.config_applied_calls
            .push((config_version, applied, error.to_string()));
        Ok(())
    }

    fn report_debug_stage(
        &mut self,
        _config: &DeviceRuntimeConfig,
        stage: &str,
    ) -> Result<(), String> {
        self.debug_stages.push(stage.to_string());
        Ok(())
    }

    fn upload_logs(
        &mut self,
        _config: &DeviceRuntimeConfig,
        request: &DeviceLogUploadRequest,
        payload: &DeviceLogUploadRequestBody,
    ) -> Result<(), String> {
        self.upload_log_calls += 1;
        self.last_log_upload_request = Some(request.clone());
        self.last_log_upload_payload = Some(payload.clone());
        if let Some(error) = &self.upload_log_error {
            return Err(error.clone());
        }
        Ok(())
    }
}

#[derive(Default)]
struct FakeImageFetcher {
    fetch_calls: Vec<ImageFetchPlan>,
    queued_results: Vec<ImageFetchOutcome>,
}
impl ImageFetcher for FakeImageFetcher {
    fn fetch(&mut self, plan: &ImageFetchPlan) -> ImageFetchOutcome {
        self.fetch_calls.push(plan.clone());
        if self.queued_results.is_empty() {
            panic!("missing fetch result");
        }
        self.queued_results.remove(0)
    }
}

#[derive(Default)]
struct FakeDisplay {
    render_calls: usize,
}
fn seeded_config() -> DeviceRuntimeConfig {
    DeviceRuntimeConfig {
        wifi_profiles: vec![WifiCredential {
            ssid: "HomeWiFi".into(),
            password: "secret".into(),
        }],
        device_id: "pf-a1b2c3d4".into(),
        ..DeviceRuntimeConfig::default()
    }
}

impl Display for FakeDisplay {
    fn render(
        &mut self,
        _artifact: &ImageArtifact,
        _config: &DeviceRuntimeConfig,
        _force_refresh: bool,
    ) -> Result<(), FailureKind> {
        self.render_calls += 1;
        Ok(())
    }
}

#[derive(Default)]
struct FakeLogUploadProvider {
    payload: Option<DeviceLogUploadRequestBody>,
    calls: usize,
}

impl LogUploadProvider for FakeLogUploadProvider {
    fn collect_logs(
        &mut self,
        _config: &DeviceRuntimeConfig,
        _request: &DeviceLogUploadRequest,
        _uploaded_epoch: i64,
    ) -> Option<DeviceLogUploadRequestBody> {
        self.calls += 1;
        self.payload.clone()
    }
}

#[derive(Default)]
struct FakeFirmwareUpdater {
    should_apply: bool,
    install_calls: usize,
    confirm_calls: usize,
    last_directive: Option<FirmwareUpdateDirective>,
    status: FirmwareRuntimeStatus,
}

impl FirmwareUpdater for FakeFirmwareUpdater {
    fn install_update(
        &mut self,
        _config: &DeviceRuntimeConfig,
        directive: &FirmwareUpdateDirective,
    ) -> Result<bool, String> {
        self.install_calls += 1;
        self.last_directive = Some(directive.clone());
        Ok(self.should_apply)
    }

    fn confirm_running_firmware(&mut self, _config: &DeviceRuntimeConfig) -> Result<(), String> {
        self.confirm_calls += 1;
        Ok(())
    }

    fn current_status(&mut self, _config: &DeviceRuntimeConfig) -> FirmwareRuntimeStatus {
        self.status.clone()
    }
}

#[test]
fn spurious_ext1_skips_network_cycle() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator::default(),
        FakeImageFetcher::default(),
        FakeDisplay::default(),
    );

    let report = runner
        .run(BootContext {
            wake_source: WakeSource::SpuriousExt1,
            long_press_action: LongPressAction::None,
            sta_ip: None,
            power_sample: PowerSample::default(),
        })
        .unwrap();

    assert_eq!(
        report.exit,
        CycleExit::Sleep {
            seconds: NEXT_BEIJING_SYNC_SLEEP_SECONDS,
            timer_only: true,
        }
    );
    assert_eq!(runner.orchestrator().sync_calls, 0);
    assert!(runner.image_fetcher().fetch_calls.is_empty());
}

#[test]
fn updated_config_requests_reboot_before_fetch() {
    let mut next_config = DeviceRuntimeConfig::default();
    next_config.remote_config_version = 2;

    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator {
            sync_result: Some(next_config),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher::default(),
        FakeDisplay::default(),
    );

    let report = runner
        .run(BootContext {
            wake_source: WakeSource::Timer,
            long_press_action: LongPressAction::None,
            sta_ip: None,
            power_sample: PowerSample::default(),
        })
        .unwrap();

    assert_eq!(report.exit, CycleExit::RebootForConfig);
    assert!(runner.image_fetcher().fetch_calls.is_empty());
    assert_eq!(
        runner.orchestrator().config_applied_calls,
        vec![(2, true, String::new())]
    );
}

#[test]
fn daily_directive_failure_falls_back_to_template_url() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url: "http://192.168.1.10:18081/public/daily.bmp".into(),
                source: Some("daily".into()),
                poll_after_seconds: Some(1800),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: None,
                firmware_update: None,
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![
                ImageFetchOutcome::failed(404, "not found"),
                ImageFetchOutcome::failed(404, "not found"),
                ImageFetchOutcome {
                    ok: true,
                    status_code: 200,
                    error: String::new(),
                    image_changed: false,
                    sha256: "same".into(),
                    etag: None,
                    last_modified: None,
                    artifact: None,
                },
            ],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
    );

    let report = runner.run(BootContext {
        wake_source: WakeSource::Timer,
        long_press_action: LongPressAction::None,
        sta_ip: None,
        power_sample: PowerSample::default(),
    });

    assert!(report.is_ok());
    assert_eq!(runner.image_fetcher().fetch_calls.len(), 3);
    assert!(
        runner.image_fetcher().fetch_calls[2]
            .url
            .contains("date=2026-03-07")
    );
}

#[test]
fn not_modified_cycle_skips_render_but_still_succeeds() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: DeviceRuntimeConfig {
                last_image_sha256: "same".into(),
                last_image_etag: "etag-1".into(),
                ..seeded_config()
            },
            save_count: 0,
        },
        FakeOrchestrator::default(),
        FakeImageFetcher {
            queued_results: vec![ImageFetchOutcome {
                ok: true,
                status_code: 304,
                error: String::new(),
                image_changed: false,
                sha256: "same".into(),
                etag: Some("etag-1".into()),
                last_modified: None,
                artifact: None,
            }],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
    );

    let report = runner
        .run(BootContext {
            wake_source: WakeSource::Timer,
            long_press_action: LongPressAction::None,
            sta_ip: None,
            power_sample: PowerSample::default(),
        })
        .unwrap();

    assert_eq!(runner.display().render_calls, 0);
    let payload = runner.orchestrator().last_checkin_payload.as_ref().unwrap();
    assert!(!payload.display_applied);
    assert!(payload.displayed_image_url.is_empty());
    assert!(payload.displayed_image_sha256.is_empty());
    assert_eq!(
        report.exit,
        CycleExit::Sleep {
            seconds: NEXT_BEIJING_SYNC_SLEEP_SECONDS,
            timer_only: false
        }
    );
}

#[test]
fn failed_cycle_sleeps_until_next_beijing_sync_window() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator::default(),
        FakeImageFetcher {
            queued_results: vec![
                ImageFetchOutcome::failed(502, "fetch failed"),
                ImageFetchOutcome::failed(502, "fetch failed"),
                ImageFetchOutcome::failed(502, "fetch failed"),
            ],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
    );

    let report = runner
        .run(BootContext {
            wake_source: WakeSource::Timer,
            long_press_action: LongPressAction::None,
            sta_ip: None,
            power_sample: PowerSample::default(),
        })
        .unwrap();

    assert_eq!(
        report.exit,
        CycleExit::Sleep {
            seconds: NEXT_BEIJING_SYNC_SLEEP_SECONDS,
            timer_only: false,
        }
    );
}

#[test]
fn successful_cycle_uses_directive_and_reports_checkin() {
    let mut runner = CycleRunner::new_with_services(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url: "https://cdn.example.com/override.jpg".into(),
                source: Some("override".into()),
                poll_after_seconds: Some(900),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: None,
                firmware_update: None,
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![ImageFetchOutcome {
                ok: true,
                status_code: 200,
                error: String::new(),
                image_changed: true,
                sha256: "new-sha".into(),
                etag: Some("etag-1".into()),
                last_modified: Some("Mon, 01 Jan 2026 00:00:00 GMT".into()),
                artifact: Some(ImageArtifact {
                    format: ImageFormat::Jpeg,
                    width: 800,
                    height: 480,
                    bytes: vec![1, 2, 3],
                }),
            }],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
        FakeFirmwareUpdater {
            status: FirmwareRuntimeStatus {
                running_partition: "ota_0".into(),
                ota_state: "valid".into(),
                ota_target_version: "0.1.0+abcdef12".into(),
                ota_last_error: String::new(),
                ota_last_attempt_epoch: 1_760_000_000,
            },
            ..FakeFirmwareUpdater::default()
        },
        FakeLogUploadProvider::default(),
    );

    let report = runner
        .run(BootContext {
            wake_source: WakeSource::Timer,
            long_press_action: LongPressAction::OpenStaPortalWindow,
            sta_ip: Some("192.168.1.50".into()),
            power_sample: PowerSample {
                battery_mv: 4050,
                battery_percent: 71,
                charging: 0,
                vbus_good: 0,
            },
        })
        .unwrap();

    assert_eq!(
        report.exit,
        CycleExit::Sleep {
            seconds: 900,
            timer_only: false,
        }
    );
    assert_eq!(report.image_source, "override");
    assert_eq!(
        report.fetch_url_used.as_deref(),
        Some("https://cdn.example.com/override.jpg")
    );
    assert!(report.portal_window_opened);
    assert_eq!(runner.orchestrator().checkin_calls, 1);
    assert_eq!(runner.display().render_calls, 1);
    assert_eq!(
        runner.orchestrator().last_checkin_payload.as_ref().map(|payload| (
            payload.sleep_seconds,
            payload.poll_interval_seconds,
            payload.image_source.as_str(),
            payload.display_applied,
            payload.displayed_image_url.as_str(),
            payload.displayed_image_sha256.as_str(),
            payload.running_partition.as_str(),
            payload.ota_state.as_str(),
        )),
        Some((
            900,
            3600,
            "override",
            true,
            "https://cdn.example.com/override.jpg",
            "new-sha",
            "ota_0",
            "valid",
        ))
    );
}

#[test]
fn successful_daily_cycle_requests_beijing_sleep_window_from_orchestrator() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url: "https://901.qingpei.me:40009/public/daily.jpg?device_id=pf-a1b2c3d4"
                    .into(),
                source: Some("daily".into()),
                poll_after_seconds: Some(3600),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: None,
                firmware_update: None,
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![ImageFetchOutcome {
                ok: true,
                status_code: 200,
                error: String::new(),
                image_changed: false,
                artifact: None,
                sha256: String::new(),
                etag: Some("\"etag\"".into()),
                last_modified: None,
            }],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
    );

    let report = runner
        .run(BootContext {
            wake_source: WakeSource::Timer,
            long_press_action: LongPressAction::None,
            sta_ip: None,
            power_sample: PowerSample::default(),
        })
        .unwrap();

    assert_eq!(
        runner.orchestrator().last_preferred_poll_seconds,
        Some(NEXT_BEIJING_SYNC_SLEEP_SECONDS)
    );
    assert_eq!(
        report.exit,
        CycleExit::Sleep {
            seconds: 3600,
            timer_only: false,
        }
    );
}

#[test]
fn successful_cycle_keeps_sleep_plan_when_checkin_fails() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url: "https://cdn.example.com/override.jpg".into(),
                source: Some("override".into()),
                poll_after_seconds: Some(900),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: None,
                firmware_update: None,
            }),
            checkin_error: Some("post failed".into()),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![ImageFetchOutcome {
                ok: true,
                status_code: 200,
                error: String::new(),
                image_changed: true,
                sha256: "new-sha".into(),
                etag: None,
                last_modified: None,
                artifact: Some(ImageArtifact {
                    format: ImageFormat::Jpeg,
                    width: 800,
                    height: 480,
                    bytes: vec![1, 2, 3],
                }),
            }],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
    );

    let report = runner
        .run(BootContext {
            wake_source: WakeSource::Timer,
            long_press_action: LongPressAction::None,
            sta_ip: Some("192.168.1.50".into()),
            power_sample: PowerSample {
                battery_mv: 4050,
                battery_percent: 71,
                charging: 0,
                vbus_good: 0,
            },
        })
        .unwrap();

    assert_eq!(
        report.exit,
        CycleExit::Sleep {
            seconds: 900,
            timer_only: false,
        }
    );
    assert!(!report.checkin_reported);
    assert_eq!(runner.orchestrator().checkin_calls, 1);
    assert_eq!(runner.display().render_calls, 1);
}

#[test]
fn successful_cycle_reports_debug_stages_in_order() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url: "https://901.qingpei.me:40009/public/daily.jpg?device_id=pf-a1b2c3d4"
                    .into(),
                source: Some("daily".into()),
                poll_after_seconds: Some(900),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: None,
                firmware_update: None,
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![ImageFetchOutcome {
                ok: true,
                status_code: 200,
                error: String::new(),
                image_changed: true,
                sha256: "new-sha".into(),
                etag: None,
                last_modified: None,
                artifact: Some(ImageArtifact {
                    format: ImageFormat::Jpeg,
                    width: 800,
                    height: 480,
                    bytes: vec![1, 2, 3],
                }),
            }],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
    );

    let _ = runner
        .run(BootContext {
            wake_source: WakeSource::Timer,
            long_press_action: LongPressAction::None,
            sta_ip: Some("192.168.1.50".into()),
            power_sample: PowerSample {
                battery_mv: 4050,
                battery_percent: 71,
                charging: 0,
                vbus_good: 0,
            },
        })
        .unwrap();

    assert_eq!(
        runner.orchestrator().debug_stages,
        vec![
            "before_fetch".to_string(),
            "after_fetch_ok".to_string(),
            "after_render_ok".to_string(),
            "after_save_ok".to_string(),
            "before_checkin_ok".to_string(),
        ]
    );
}

#[test]
fn same_origin_image_fetch_uses_orchestrator_token_header() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: DeviceRuntimeConfig {
                wifi_profiles: vec![WifiCredential {
                    ssid: "HomeWiFi".into(),
                    password: "secret".into(),
                }],
                device_id: "pf-a1b2c3d4".into(),
                orchestrator_base_url: "http://192.168.1.10:18081".into(),
                orchestrator_token: "orch-token".into(),
                ..DeviceRuntimeConfig::default()
            },
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url: "http://192.168.1.10:18081/api/v1/preview/current.bmp".into(),
                source: Some("daily".into()),
                poll_after_seconds: Some(1800),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: None,
                firmware_update: None,
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![ImageFetchOutcome {
                ok: true,
                status_code: 200,
                error: String::new(),
                image_changed: false,
                sha256: "same".into(),
                etag: None,
                last_modified: None,
                artifact: None,
            }],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
    );

    let report = runner.run(BootContext {
        wake_source: WakeSource::Timer,
        long_press_action: LongPressAction::None,
        sta_ip: None,
        power_sample: PowerSample::default(),
    });

    assert!(report.is_ok());
    assert_eq!(runner.image_fetcher().fetch_calls.len(), 1);
    assert_eq!(
        runner.image_fetcher().fetch_calls[0].orchestrator_token,
        "orch-token"
    );
}

#[test]
fn cross_origin_image_fetch_does_not_use_orchestrator_token_header() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: DeviceRuntimeConfig {
                wifi_profiles: vec![WifiCredential {
                    ssid: "HomeWiFi".into(),
                    password: "secret".into(),
                }],
                device_id: "pf-a1b2c3d4".into(),
                orchestrator_base_url: "http://192.168.1.10:18081".into(),
                orchestrator_token: "orch-token".into(),
                ..DeviceRuntimeConfig::default()
            },
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url: "https://cdn.example.com/override.jpg".into(),
                source: Some("override".into()),
                poll_after_seconds: Some(1800),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: None,
                firmware_update: None,
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![ImageFetchOutcome {
                ok: true,
                status_code: 200,
                error: String::new(),
                image_changed: false,
                sha256: "same".into(),
                etag: None,
                last_modified: None,
                artifact: None,
            }],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
    );

    let report = runner.run(BootContext {
        wake_source: WakeSource::Timer,
        long_press_action: LongPressAction::None,
        sta_ip: None,
        power_sample: PowerSample::default(),
    });

    assert!(report.is_ok());
    assert_eq!(runner.image_fetcher().fetch_calls.len(), 1);
    assert_eq!(runner.image_fetcher().fetch_calls[0].orchestrator_token, "");
}

#[test]
fn directive_url_prefers_orchestrator_origin_before_public_base_origin() {
    let mut runner = CycleRunner::new(
        FakeClock,
        FakeStorage {
            config: DeviceRuntimeConfig {
                wifi_profiles: vec![WifiCredential {
                    ssid: "HomeWiFi".into(),
                    password: "secret".into(),
                }],
                device_id: "pf-a1b2c3d4".into(),
                orchestrator_base_url: "http://192.168.233.11:8081".into(),
                orchestrator_token: "orch-token".into(),
                ..DeviceRuntimeConfig::default()
            },
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url:
                    "http://192.168.58.113:8081/api/v1/preview/current.bmp?device_id=pf-a1b2c3d4"
                        .into(),
                source: Some("daily".into()),
                poll_after_seconds: Some(1800),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: None,
                firmware_update: None,
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![ImageFetchOutcome {
                ok: true,
                status_code: 200,
                error: String::new(),
                image_changed: false,
                sha256: "same".into(),
                etag: None,
                last_modified: None,
                artifact: None,
            }],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
    );

    let report = runner.run(BootContext {
        wake_source: WakeSource::Timer,
        long_press_action: LongPressAction::None,
        sta_ip: None,
        power_sample: PowerSample::default(),
    });

    assert!(report.is_ok());
    assert_eq!(runner.image_fetcher().fetch_calls.len(), 1);
    assert!(
        runner.image_fetcher().fetch_calls[0]
            .url
            .starts_with("http://192.168.233.11:8081/api/v1/preview/current.bmp")
    );
}

#[test]
fn successful_cycle_uploads_logs_when_requested() {
    let request = DeviceLogUploadRequest {
        request_id: 77,
        max_lines: 32,
        max_bytes: 2048,
        reason: Some("collect boot logs".into()),
        created_epoch: 1_760_000_000,
        expires_epoch: Some(1_760_003_600),
    };
    let payload = DeviceLogUploadRequestBody {
        device_id: "pf-a1b2c3d4".into(),
        request_id: 77,
        uploaded_epoch: 1_760_000_000,
        line_count: 2,
        truncated: false,
        uploaded_bytes: Some(12),
        buffer_total_lines: Some(4),
        buffer_total_bytes: Some(24),
        buffer_boot_id: Some(8),
        lines: vec!["line-1".into(), "line-2".into()],
    };

    let mut runner = CycleRunner::new_with_log_upload_provider(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url: "https://901.qingpei.me:40009/public/daily.jpg?device_id=pf-a1b2c3d4"
                    .into(),
                source: Some("daily".into()),
                poll_after_seconds: Some(900),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: Some(request.clone()),
                firmware_update: None,
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![ImageFetchOutcome {
                ok: true,
                status_code: 200,
                error: String::new(),
                image_changed: false,
                sha256: "same".into(),
                etag: None,
                last_modified: None,
                artifact: None,
            }],
            ..FakeImageFetcher::default()
        },
        FakeDisplay::default(),
        FakeLogUploadProvider {
            payload: Some(payload.clone()),
            calls: 0,
        },
    );

    let report = runner
        .run(BootContext {
            wake_source: WakeSource::Timer,
            long_press_action: LongPressAction::None,
            sta_ip: None,
            power_sample: PowerSample::default(),
        })
        .unwrap();

    assert!(report.logs_uploaded);
    assert_eq!(runner.orchestrator().upload_log_calls, 1);
    assert_eq!(runner.orchestrator().last_log_upload_request.as_ref(), Some(&request));
    assert_eq!(runner.orchestrator().last_log_upload_payload.as_ref(), Some(&payload));
}

#[test]
fn firmware_update_applied_reboots_before_image_fetch() {
    let mut runner = CycleRunner::new_with_services(
        FakeClock,
        FakeStorage {
            config: seeded_config(),
            save_count: 0,
        },
        FakeOrchestrator {
            directive: Some(DeviceNextResponse {
                image_url: "https://901.qingpei.me:40009/public/daily.jpg?device_id=pf-a1b2c3d4"
                    .into(),
                source: Some("daily".into()),
                poll_after_seconds: Some(900),
                valid_until_epoch: None,
                server_epoch: None,
                device_epoch: None,
                device_clock_ok: None,
                effective_epoch: None,
                log_upload_request: None,
                firmware_update: Some(FirmwareUpdateDirective {
                    rollout_id: 12,
                    version: "9.9.9-test".into(),
                    app_bin_url: "https://example.com/fw/app.bin".into(),
                    sha256: "deadbeef".into(),
                    size_bytes: 1024,
                    min_battery_percent: Some(20),
                    requires_vbus: false,
                    created_epoch: 1_760_000_000,
                }),
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher::default(),
        FakeDisplay::default(),
        FakeFirmwareUpdater {
            should_apply: true,
            ..FakeFirmwareUpdater::default()
        },
        FakeLogUploadProvider::default(),
    );

    let report = runner
        .run(BootContext {
            wake_source: WakeSource::Timer,
            long_press_action: LongPressAction::None,
            sta_ip: None,
            power_sample: PowerSample {
                battery_mv: 4100,
                battery_percent: 80,
                charging: 0,
                vbus_good: 0,
            },
        })
        .unwrap();

    assert_eq!(report.exit, CycleExit::RebootForFirmwareUpdate);
    assert!(runner.image_fetcher().fetch_calls.is_empty());
}
