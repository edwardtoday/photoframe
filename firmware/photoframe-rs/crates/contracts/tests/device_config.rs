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
      "note": "公网 token 切换",
      "log_upload_request": {
        "request_id": 34,
        "max_lines": 80,
        "max_bytes": 4096,
        "reason": "collect wake trace",
        "created_epoch": 1760000000,
        "expires_epoch": null
      }
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
    let request = response
        .log_upload_request
        .expect("missing log upload request");
    assert_eq!(request.request_id, 34);
    assert_eq!(request.max_lines, 80);
    assert_eq!(request.max_bytes, 4096);
    assert_eq!(request.reason.as_deref(), Some("collect wake trace"));
    assert_eq!(request.expires_epoch, None);
}
