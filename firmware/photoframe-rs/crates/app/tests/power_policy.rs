use photoframe_app::{PowerCache, PowerSample, normalize_power_sample};

#[test]
fn stuck_full_percent_is_corrected_by_voltage_on_battery() {
    let result = normalize_power_sample(
        PowerSample {
            battery_mv: 4040,
            battery_percent: 100,
            charging: 0,
            vbus_good: 0,
        },
        None,
    );

    assert_eq!(result.sample.battery_percent, 72);
    assert_eq!(result.cache.battery_percent, 72);
}

#[test]
fn missing_sample_uses_cached_values() {
    let result = normalize_power_sample(
        PowerSample::default(),
        Some(PowerCache {
            battery_mv: 3990,
            battery_percent: 60,
            charging: 0,
            vbus_good: 0,
            cached_epoch: 123,
        }),
    );

    assert_eq!(result.sample.battery_mv, 3990);
    assert_eq!(result.sample.battery_percent, 60);
}
