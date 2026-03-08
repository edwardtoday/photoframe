#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeSource {
    Timer,
    Key,
    Boot,
    SpuriousExt1,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleAction {
    ScheduledSync,
    ManualSync,
    ForceRefresh,
    SleepTimerOnly,
}

/// 根据唤醒来源决定本轮的主动作。
/// 这里先固定当前产品语义，后续 app 层再把它编排成完整流程。
pub fn decide_cycle_action(wake_source: WakeSource) -> CycleAction {
    match wake_source {
        WakeSource::Timer | WakeSource::Other => CycleAction::ScheduledSync,
        WakeSource::Key => CycleAction::ManualSync,
        WakeSource::Boot => CycleAction::ForceRefresh,
        WakeSource::SpuriousExt1 => CycleAction::SleepTimerOnly,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongPressAction {
    None,
    OpenStaPortalWindow,
    ClearWifiAndEnterPortal,
}

/// 长按判定遵循当前固件语义：BOOT 优先级高于 KEY，阈值为 3000ms。
pub fn decide_long_press_action(
    boot_pressed: bool,
    key_pressed: bool,
    held_ms: u64,
) -> LongPressAction {
    const LONG_PRESS_MS: u64 = 3_000;

    if held_ms < LONG_PRESS_MS {
        return LongPressAction::None;
    }

    if boot_pressed {
        return LongPressAction::ClearWifiAndEnterPortal;
    }

    if key_pressed {
        return LongPressAction::OpenStaPortalWindow;
    }

    LongPressAction::None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub interval_minutes: u32,
    pub retry_base_minutes: u32,
    pub retry_max_minutes: u32,
    pub max_failure_before_long_sleep: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Success,
    GeneralFailure,
    PmicSoftFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackoffDecision {
    pub next_failure_count: u32,
    pub sleep_seconds: u64,
}

/// 把“本轮结果”映射为下一轮休眠时长与失败计数。
/// 这里严格对齐当前 C++ 固件的退避/软失败语义，避免功耗策略漂移。
pub fn apply_cycle_outcome(
    policy: &RetryPolicy,
    current_failure_count: u32,
    failure_kind: FailureKind,
) -> BackoffDecision {
    match failure_kind {
        FailureKind::Success => BackoffDecision {
            next_failure_count: 0,
            sleep_seconds: regular_sleep_seconds(policy.interval_minutes),
        },
        FailureKind::PmicSoftFailure => BackoffDecision {
            next_failure_count: 0,
            sleep_seconds: regular_sleep_seconds(policy.interval_minutes),
        },
        FailureKind::GeneralFailure => {
            let next_failure_count = current_failure_count.saturating_add(1).max(1);
            let exponent = next_failure_count.saturating_sub(1).min(10);
            let factor = 1u32 << exponent;
            let mut minutes = policy.retry_base_minutes.max(1).saturating_mul(factor);
            minutes = minutes.min(
                policy
                    .retry_max_minutes
                    .max(policy.retry_base_minutes.max(1)),
            );

            if next_failure_count >= policy.max_failure_before_long_sleep.max(1) {
                minutes = minutes.max(policy.retry_max_minutes.max(1));
            }

            BackoffDecision {
                next_failure_count,
                sleep_seconds: u64::from(minutes.max(1)) * 60,
            }
        }
    }
}

fn regular_sleep_seconds(interval_minutes: u32) -> u64 {
    u64::from(interval_minutes.max(1)) * 60
}

/// 与当前固件保持一致：当 RTC 时间明显无效、上次校时无效、或距离上次校时超过一天时触发校时。
pub fn should_sync_time(now_epoch: i64, last_time_sync_epoch: i64) -> bool {
    const MIN_VALID_EPOCH: i64 = 1_735_689_600; // 2025-01-01 UTC
    const SYNC_INTERVAL_SEC: i64 = 24 * 3600;

    if now_epoch < MIN_VALID_EPOCH {
        return true;
    }

    if last_time_sync_epoch < MIN_VALID_EPOCH {
        return true;
    }

    let age = now_epoch - last_time_sync_epoch;
    age < 0 || age >= SYNC_INTERVAL_SEC
}

/// 深睡时间统一在 64-bit 下换算成微秒，避免乘法溢出导致异常唤醒。
pub fn seconds_to_microseconds(seconds: u64) -> u64 {
    seconds * 1_000_000
}

/// 设备 ID 与当前固件保持一致：固定前缀 `pf-` + STA MAC 后四字节的小写十六进制。
pub fn device_id_from_mac_suffix(mac_suffix: [u8; 4]) -> String {
    format!(
        "pf-{:02x}{:02x}{:02x}{:02x}",
        mac_suffix[0], mac_suffix[1], mac_suffix[2], mac_suffix[3]
    )
}

/// 设备 token 采用 16 字节随机数的小写十六进制编码。
pub fn token_hex_from_bytes(bytes: [u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}
