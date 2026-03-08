use photoframe_contracts::DeviceConfigResponse;

#[test]
fn parses_device_config_response_with_nested_config() {
    let json = r#"
    {
      "device_id": "pf-a1b2c3d4",
      "server_epoch": 1760000000,
      "device_epoch": 0,
      "device_clock_ok": false,
      "effective_epoch": 1760000000,
      "config_version": 18,
      "config": {
        "interval_minutes": 60,
        "image_url_template": "https://example.com/daily.bmp",
        "timezone": "Asia/Shanghai"
      },
      "note": "公网 token 切换"
    }
    "#;

    let response: DeviceConfigResponse = serde_json::from_str(json).unwrap();

    assert_eq!(response.device_id, "pf-a1b2c3d4");
    assert_eq!(response.config_version, 18);
    assert_eq!(response.config.interval_minutes, Some(60));
    assert_eq!(
        response.config.image_url_template.as_deref(),
        Some("https://example.com/daily.bmp")
    );
    assert_eq!(response.config.timezone.as_deref(), Some("Asia/Shanghai"));
    assert_eq!(response.device_clock_ok, Some(false));
}
