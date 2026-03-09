use photoframe_app::{DeviceRuntimeConfig, WifiCredential};

#[test]
fn ensure_primary_wifi_adds_primary_profile_when_missing() {
    let mut config = DeviceRuntimeConfig {
        primary_wifi_ssid: "HomeWiFi".into(),
        primary_wifi_password: "secret".into(),
        ..DeviceRuntimeConfig::default()
    };

    config.ensure_primary_wifi_in_profiles();

    assert_eq!(config.wifi_profiles.len(), 1);
    assert_eq!(config.wifi_profiles[0].ssid, "HomeWiFi");
    assert_eq!(config.wifi_profiles[0].password, "secret");
}

#[test]
fn ensure_primary_wifi_rotates_oldest_when_full() {
    let mut profiles = Vec::new();
    for idx in 0..DeviceRuntimeConfig::MAX_WIFI_PROFILES {
        let ssid = format!("AP-{idx}");
        profiles.push(WifiCredential {
            ssid: ssid.clone(),
            password: ssid,
        });
    }

    let mut config = DeviceRuntimeConfig {
        primary_wifi_ssid: "Newest".into(),
        primary_wifi_password: "new-pass".into(),
        wifi_profiles: profiles,
        last_connected_wifi_index: Some(2),
        ..DeviceRuntimeConfig::default()
    };

    config.ensure_primary_wifi_in_profiles();

    assert_eq!(config.wifi_profiles.len(), DeviceRuntimeConfig::MAX_WIFI_PROFILES);
    assert_eq!(config.wifi_profiles.first().map(|item| item.ssid.as_str()), Some("AP-1"));
    assert_eq!(
        config.wifi_profiles.last().map(|item| item.ssid.as_str()),
        Some("Newest")
    );
    assert_eq!(config.last_connected_wifi_index, Some(1));
}

#[test]
fn wifi_connection_order_prefers_last_successful_profile() {
    let config = DeviceRuntimeConfig {
        wifi_profiles: vec![
            WifiCredential {
                ssid: "A".into(),
                password: "a".into(),
            },
            WifiCredential {
                ssid: "B".into(),
                password: "b".into(),
            },
            WifiCredential {
                ssid: "C".into(),
                password: "c".into(),
            },
        ],
        last_connected_wifi_index: Some(1),
        ..DeviceRuntimeConfig::default()
    };

    assert_eq!(config.wifi_connection_order(), vec![1, 0, 2]);
}
