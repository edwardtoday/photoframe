use photoframe_app::{DeviceRuntimeConfig, LocalConfigPatch, WifiCredential};

#[test]
fn portal_ignores_empty_ssid_when_existing_credentials_present() {
    let mut config = DeviceRuntimeConfig {
        primary_wifi_ssid: "HomeWiFi".into(),
        primary_wifi_password: "secret".into(),
        wifi_profiles: vec![WifiCredential {
            ssid: "HomeWiFi".into(),
            password: "secret".into(),
        }],
        ..DeviceRuntimeConfig::default()
    };

    let outcome = config.apply_local_config_patch(&LocalConfigPatch {
        wifi_ssid: Some(String::new()),
        wifi_password: Some(String::new()),
        ..LocalConfigPatch::default()
    });

    assert!(!outcome.wifi_changed);
    assert_eq!(config.wifi_profiles[0].ssid, "HomeWiFi");
    assert_eq!(config.wifi_profiles[0].password, "secret");
}

#[test]
fn portal_blank_password_keeps_existing_password() {
    let mut config = DeviceRuntimeConfig {
        primary_wifi_ssid: "HomeWiFi".into(),
        primary_wifi_password: "secret".into(),
        wifi_profiles: vec![WifiCredential {
            ssid: "HomeWiFi".into(),
            password: "secret".into(),
        }],
        ..DeviceRuntimeConfig::default()
    };

    let outcome = config.apply_local_config_patch(&LocalConfigPatch {
        wifi_ssid: Some("HomeWiFi".into()),
        wifi_password: Some(String::new()),
        ..LocalConfigPatch::default()
    });

    assert!(!outcome.wifi_changed);
    assert_eq!(config.wifi_profiles[0].password, "secret");
}

#[test]
fn portal_display_changes_clear_cached_hash() {
    let mut config = DeviceRuntimeConfig {
        last_image_sha256: "old-hash".into(),
        ..DeviceRuntimeConfig::default()
    };

    let outcome = config.apply_local_config_patch(&LocalConfigPatch {
        display_rotation: Some(2),
        ..LocalConfigPatch::default()
    });

    assert!(outcome.display_config_changed);
    assert!(config.last_image_sha256.is_empty());
}
