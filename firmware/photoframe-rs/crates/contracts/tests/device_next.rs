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
      "effective_epoch": 1760000000
    }
    "#;

    let response: DeviceNextResponse = serde_json::from_str(json).unwrap();

    assert_eq!(response.image_url, "https://example.com/public/daily.jpg");
    assert_eq!(response.source.as_deref(), Some("daily"));
    assert_eq!(response.poll_after_seconds, Some(3600));
    assert_eq!(response.device_clock_ok, Some(false));
    assert_eq!(response.effective_epoch, Some(1_760_000_000));
}
