use photoframe_app::{DeviceRuntimeConfig, WifiCredential};
use photoframe_contracts::{RemoteConfigPatch, RemoteWifiProfile};

#[test]
fn remote_patch_clamps_display_values_and_marks_refresh() {
    let mut config = DeviceRuntimeConfig {
        last_image_sha256: "abc".into(),
        ..DeviceRuntimeConfig::default()
    };

    let outcome = config.apply_remote_config_patch(&RemoteConfigPatch {
        display_rotation: Some(99),
        color_process_mode: Some(99),
        dither_mode: Some(99),
        six_color_tolerance: Some(99),
        ..RemoteConfigPatch::default()
    });

    assert!(outcome.display_config_changed);
    assert_eq!(config.display_rotation, 2);
    assert_eq!(config.color_process_mode, 2);
    assert_eq!(config.dither_mode, 1);
    assert_eq!(config.six_color_tolerance, 64);
    assert!(config.last_image_sha256.is_empty());
}

#[test]
fn remote_patch_replaces_wifi_profiles_and_preserves_existing_password_when_missing() {
    let mut config = DeviceRuntimeConfig {
        primary_wifi_ssid: "Home".into(),
        primary_wifi_password: "secret".into(),
        wifi_profiles: vec![
            WifiCredential {
                ssid: "Home".into(),
                password: "secret".into(),
            },
            WifiCredential {
                ssid: "Office".into(),
                password: "office-secret".into(),
            },
        ],
        last_connected_wifi_index: Some(1),
        ..DeviceRuntimeConfig::default()
    };

    config.apply_remote_config_patch(&RemoteConfigPatch {
        wifi_profiles: Some(vec![
            RemoteWifiProfile {
                ssid: "Home".into(),
                password: None,
            },
            RemoteWifiProfile {
                ssid: "Guest".into(),
                password: Some("one".into()),
            },
        ]),
        ..RemoteConfigPatch::default()
    });

    assert_eq!(config.wifi_profiles.len(), 2);
    assert_eq!(config.wifi_profiles[0].ssid, "Home");
    assert_eq!(config.wifi_profiles[0].password, "secret");
    assert_eq!(config.wifi_profiles[1].ssid, "Guest");
    assert_eq!(config.wifi_profiles[1].password, "one");
    assert_eq!(config.last_connected_wifi_index, None);
}

#[test]
fn remote_patch_empty_wifi_profiles_clears_wifi_credentials() {
    let mut config = DeviceRuntimeConfig {
        primary_wifi_ssid: "Home".into(),
        primary_wifi_password: "secret".into(),
        wifi_profiles: vec![WifiCredential {
            ssid: "Home".into(),
            password: "secret".into(),
        }],
        last_connected_wifi_index: Some(0),
        ..DeviceRuntimeConfig::default()
    };

    config.apply_remote_config_patch(&RemoteConfigPatch {
        wifi_profiles: Some(vec![]),
        ..RemoteConfigPatch::default()
    });

    assert!(config.wifi_profiles.is_empty());
    assert!(config.primary_wifi_ssid.is_empty());
    assert!(config.primary_wifi_password.is_empty());
    assert_eq!(config.last_connected_wifi_index, None);
}

#[test]
fn reported_config_includes_non_empty_firmware_version() {
    let config = DeviceRuntimeConfig::default();

    let reported = config.to_reported_config();

    assert!(!reported.firmware_version.trim().is_empty());
}
