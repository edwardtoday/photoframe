use photoframe_app::{
    BootContext, Clock, CycleExit, CycleRunner, DeviceRuntimeConfig, Display, ImageArtifact,
    ImageFetchOutcome, ImageFetchPlan, ImageFetcher, ImageFormat, OrchestratorApi, PowerSample,
    Storage, WifiCredential,
};
use photoframe_contracts::{DeviceCheckinRequest, DeviceNextResponse};
use photoframe_domain::{FailureKind, LongPressAction, WakeSource};

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
    checkin_calls: usize,
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
    ) -> Result<Option<DeviceNextResponse>, String> {
        self.directive_calls += 1;
        Ok(self.directive.clone())
    }

    fn report_checkin(
        &mut self,
        _base_urls: &[String],
        _payload: &DeviceCheckinRequest,
    ) -> Result<(), String> {
        self.checkin_calls += 1;
        Ok(())
    }
}

#[derive(Default)]
struct FakeImageFetcher {
    fetch_calls: Vec<String>,
    queued_results: Vec<ImageFetchOutcome>,
}
impl ImageFetcher for FakeImageFetcher {
    fn fetch(&mut self, plan: &ImageFetchPlan) -> ImageFetchOutcome {
        self.fetch_calls.push(plan.url.clone());
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
            seconds: 600,
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
            }),
            ..FakeOrchestrator::default()
        },
        FakeImageFetcher {
            queued_results: vec![
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
    assert_eq!(runner.image_fetcher().fetch_calls.len(), 2);
    assert!(runner.image_fetcher().fetch_calls[1].contains("date=2026-03-07"));
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
    assert_eq!(
        report.exit,
        CycleExit::Sleep {
            seconds: 3600,
            timer_only: false
        }
    );
}

#[test]
fn successful_cycle_uses_directive_and_reports_checkin() {
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
}
