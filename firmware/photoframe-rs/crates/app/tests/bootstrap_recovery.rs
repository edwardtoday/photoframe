use photoframe_app::DeviceRuntimeConfig;
use photoframe_contracts::{DeviceConfigPayload, RemoteWifiProfile};

fn recovery_payload() -> DeviceConfigPayload {
    DeviceConfigPayload {
        orchestrator_enabled: Some(1),
        orchestrator_base_url: Some("https://901.qingpei.me:40009".into()),
        orchestrator_token: Some("orch-token".into()),
        image_url_template: Some(
            "https://901.qingpei.me:40009/public/daily.jpg?device_id=%DEVICE_ID%".into(),
        ),
        photo_token: Some("photo-token".into()),
        wifi_profiles: Some(vec![
            RemoteWifiProfile {
                ssid: "OpenWrt".into(),
                password: Some("sansiAX3".into()),
            },
            RemoteWifiProfile {
                ssid: "Qing-IoT".into(),
                password: Some("jiajuzhuanyong".into()),
            },
        ]),
        interval_minutes: Some(60),
        retry_base_minutes: Some(5),
        retry_max_minutes: Some(240),
        max_failure_before_long_sleep: Some(24),
        display_rotation: Some(2),
        color_process_mode: Some(0),
        dither_mode: Some(1),
        six_color_tolerance: Some(6),
        timezone: Some("CST-8".into()),
    }
}

#[test]
fn bootstrap_recovery_applies_on_factory_like_config() {
    let mut config = DeviceRuntimeConfig {
        device_id: "pf-d9369c80".into(),
        ..DeviceRuntimeConfig::default()
    };

    assert!(config.should_apply_bootstrap_recovery());

    let outcome = config.apply_bootstrap_payload(&recovery_payload());

    assert!(outcome.display_config_changed);
    assert_eq!(config.orchestrator_base_url, "https://901.qingpei.me:40009");
    assert_eq!(config.orchestrator_token, "orch-token");
    assert_eq!(config.photo_token, "photo-token");
    assert_eq!(
        config.image_url_template,
        "https://901.qingpei.me:40009/public/daily.jpg?device_id=%DEVICE_ID%"
    );
    assert_eq!(config.timezone, "CST-8");
    assert_eq!(config.display_rotation, 2);
    assert_eq!(config.wifi_profiles.len(), 2);
}

#[test]
fn bootstrap_recovery_skips_device_with_remote_config() {
    let config = DeviceRuntimeConfig {
        orchestrator_base_url: "https://901.qingpei.me:40009".into(),
        orchestrator_token: "orch-token".into(),
        photo_token: "photo-token".into(),
        image_url_template: "https://901.qingpei.me:40009/public/daily.jpg?device_id=%DEVICE_ID%"
            .into(),
        remote_config_version: 3,
        ..DeviceRuntimeConfig::default()
    };

    assert!(!config.should_apply_bootstrap_recovery());
}
