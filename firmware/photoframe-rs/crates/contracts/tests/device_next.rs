use photoframe_contracts::DeviceNextResponse;

#[test]
fn parses_device_next_response_with_clock_fields() {
    let json = r#"
    {
      "image_url": "https://example.com/public/daily.jpg",
      "source": "daily",
      "poll_after_seconds": 3600,
      "valid_until_epoch": 1760003600,
      "server_epoch": 1760000000,
      "device_epoch": 0,
      "device_clock_ok": false,
      "effective_epoch": 1760000000,
      "log_upload_request": {
        "request_id": 12,
        "max_lines": 120,
        "max_bytes": 8192,
        "reason": "collect wifi diagnostics",
        "created_epoch": 1760000000,
        "expires_epoch": 1760003600
      },
      "firmware_update": {
        "rollout_id": 5,
        "version": "0.2.0+abcd1234",
        "app_bin_url": "https://example.com/fw/app.bin",
        "sha256": "deadbeef",
        "size_bytes": 1677721,
        "min_battery_percent": 50,
        "requires_vbus": false,
        "created_epoch": 1760000000
      }
    }
    "#;

    let response: DeviceNextResponse = serde_json::from_str(json).unwrap();

    assert_eq!(response.image_url, "https://example.com/public/daily.jpg");
    assert_eq!(response.source.as_deref(), Some("daily"));
    assert_eq!(response.poll_after_seconds, Some(3600));
    assert_eq!(response.device_clock_ok, Some(false));
    assert_eq!(response.effective_epoch, Some(1_760_000_000));
    let request = response.log_upload_request.expect("missing log upload request");
    assert_eq!(request.request_id, 12);
    assert_eq!(request.max_lines, 120);
    assert_eq!(request.max_bytes, 8192);
    assert_eq!(request.reason.as_deref(), Some("collect wifi diagnostics"));
    assert_eq!(request.created_epoch, 1_760_000_000);
    assert_eq!(request.expires_epoch, Some(1_760_003_600));
    let firmware = response.firmware_update.expect("missing firmware update");
    assert_eq!(firmware.rollout_id, 5);
    assert_eq!(firmware.version, "0.2.0+abcd1234");
    assert_eq!(firmware.app_bin_url, "https://example.com/fw/app.bin");
    assert_eq!(firmware.sha256, "deadbeef");
    assert_eq!(firmware.size_bytes, 1_677_721);
    assert_eq!(firmware.min_battery_percent, Some(50));
    assert!(!firmware.requires_vbus);
}
