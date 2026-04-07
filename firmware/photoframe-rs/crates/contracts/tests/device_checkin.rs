use photoframe_contracts::{DeviceCheckinRequest, ReportedConfig, ReportedWifiProfile};

#[test]
fn serializes_device_checkin_with_reported_config() {
    let payload = DeviceCheckinRequest {
        device_id: "pf-a1b2c3d4".into(),
        checkin_epoch: 1_760_000_000,
        next_wakeup_epoch: 1_760_003_600,
        sleep_seconds: 3_600,
        poll_interval_seconds: 3_600,
        failure_count: 0,
        last_http_status: 200,
        fetch_ok: true,
        image_changed: true,
        display_applied: true,
        image_source: "daily".into(),
        displayed_image_url:
            "http://192.168.58.113:18081/api/v1/assets/daily-2025-10-09-sierra-colorful.jpg"
                .into(),
        displayed_image_sha256: "abc123".into(),
        last_error: String::new(),
        sta_ip: Some("192.168.1.50".into()),
        battery_mv: 4050,
        battery_percent: 71,
        charging: 0,
        vbus_good: 0,
        running_partition: "ota_0".into(),
        ota_state: "valid".into(),
        ota_target_version: "0.1.0+abcdef12".into(),
        ota_last_error: String::new(),
        ota_last_attempt_epoch: 1_760_000_000,
        reported_config: ReportedConfig {
            firmware_version: "0.1.0+abcdef12".into(),
            orchestrator_enabled: 1,
            orchestrator_base_url: "http://192.168.1.10:18081".into(),
            orchestrator_token: "devtoken".into(),
            image_url_template: "https://example.com/daily.bmp".into(),
            photo_token: "phototoken".into(),
            interval_minutes: 60,
            retry_base_minutes: 5,
            retry_max_minutes: 240,
            max_failure_before_long_sleep: 24,
            display_rotation: 0,
            color_process_mode: 0,
            dither_mode: 1,
            six_color_tolerance: 0,
            timezone: "Asia/Shanghai".into(),
            wifi_profiles: vec![ReportedWifiProfile {
                ssid: "HomeWiFi".into(),
                password_set: true,
            }],
        },
    };

    let json = serde_json::to_value(payload).unwrap();

    assert_eq!(json["device_id"], "pf-a1b2c3d4");
    assert_eq!(
        json["reported_config"]["firmware_version"],
        "0.1.0+abcdef12"
    );
    assert_eq!(json["reported_config"]["interval_minutes"], 60);
    assert_eq!(
        json["reported_config"]["wifi_profiles"][0]["ssid"],
        "HomeWiFi"
    );
    assert_eq!(
        json["reported_config"]["wifi_profiles"][0]["password_set"],
        true
    );
    assert_eq!(json["running_partition"], "ota_0");
    assert_eq!(json["ota_state"], "valid");
    assert_eq!(json["ota_target_version"], "0.1.0+abcdef12");
    assert_eq!(json["ota_last_attempt_epoch"], 1_760_000_000);
    assert_eq!(json["display_applied"], true);
    assert_eq!(
        json["displayed_image_url"],
        "http://192.168.58.113:18081/api/v1/assets/daily-2025-10-09-sierra-colorful.jpg"
    );
    assert_eq!(json["displayed_image_sha256"], "abc123");
}
