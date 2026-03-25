#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use photoframe_app::{DeviceRuntimeConfig, ImageArtifact, ImageFormat, PowerSample};
#[cfg(target_os = "espidf")]
use photoframe_domain::FailureKind;
#[cfg(target_os = "espidf")]
use std::time::Instant;

#[cfg(target_os = "espidf")]
use crate::wifi::EspWifiManager;

#[cfg(target_os = "espidf")]
const TRACE_NONE: u32 = 0;
#[cfg(target_os = "espidf")]
const TRACE_BEFORE_POWER_READY: u32 = 1;
#[cfg(target_os = "espidf")]
const TRACE_AFTER_POWER_READY: u32 = 2;
#[cfg(target_os = "espidf")]
const TRACE_BEFORE_BMP_PACK: u32 = 3;
#[cfg(target_os = "espidf")]
const TRACE_AFTER_BMP_PACK: u32 = 4;
#[cfg(target_os = "espidf")]
const TRACE_BEFORE_PANEL_FLUSH: u32 = 5;
#[cfg(target_os = "espidf")]
const TRACE_AFTER_PANEL_FLUSH: u32 = 6;
#[cfg(target_os = "espidf")]
const TRACE_PANEL_INIT_ENTER: u32 = 20;
#[cfg(target_os = "espidf")]
const TRACE_PANEL_INIT_DONE: u32 = 21;
#[cfg(target_os = "espidf")]
const TRACE_PANEL_TURN_ON_04: u32 = 22;
#[cfg(target_os = "espidf")]
const TRACE_PANEL_TURN_ON_12: u32 = 23;
#[cfg(target_os = "espidf")]
const TRACE_PANEL_TURN_ON_02: u32 = 24;

#[cfg(target_os = "espidf")]
#[unsafe(link_section = ".rtc.data")]
static mut LAST_RENDER_TRACE: u32 = TRACE_NONE;

#[cfg(target_os = "espidf")]
fn image_format_name(format: &ImageFormat) -> &'static str {
    match format {
        ImageFormat::Bmp => "bmp",
        ImageFormat::Jpeg => "jpeg",
    }
}

#[cfg(target_os = "espidf")]
fn log_render_timing(
    status: &str,
    stage: &str,
    format: &ImageFormat,
    input_bytes: usize,
    output_bytes: usize,
    power_ms: u128,
    decode_ms: u128,
    pack_ms: u128,
    flush_ms: u128,
    total_ms: u128,
) {
    let rendered = format!(
        "photoframe-rs/timing: render status={} stage={} format={} input_bytes={} output_bytes={} power={}ms decode={}ms pack={}ms flush={}ms total={}ms",
        status,
        stage,
        image_format_name(format),
        input_bytes,
        output_bytes,
        power_ms,
        decode_ms,
        pack_ms,
        flush_ms,
        total_ms
    );
    println!("{}", rendered);
    crate::diag::append("INFO", &rendered);
}

#[cfg(target_os = "espidf")]
pub(crate) fn record_render_trace(stage: u32) {
    unsafe {
        LAST_RENDER_TRACE = stage;
    }
}

#[cfg(target_os = "espidf")]
pub(crate) fn take_render_trace() -> Option<&'static str> {
    let stage = unsafe { LAST_RENDER_TRACE };
    unsafe {
        LAST_RENDER_TRACE = TRACE_NONE;
    }
    match stage {
        TRACE_NONE => None,
        TRACE_BEFORE_POWER_READY => Some("before_power_ready"),
        TRACE_AFTER_POWER_READY => Some("after_power_ready"),
        TRACE_BEFORE_BMP_PACK => Some("before_bmp_pack"),
        TRACE_AFTER_BMP_PACK => Some("after_bmp_pack"),
        TRACE_BEFORE_PANEL_FLUSH => Some("before_panel_flush"),
        TRACE_AFTER_PANEL_FLUSH => Some("after_panel_flush"),
        TRACE_PANEL_INIT_ENTER => Some("panel_init_enter"),
        TRACE_PANEL_INIT_DONE => Some("panel_init_done"),
        TRACE_PANEL_TURN_ON_04 => Some("panel_turn_on_04"),
        TRACE_PANEL_TURN_ON_12 => Some("panel_turn_on_12"),
        TRACE_PANEL_TURN_ON_02 => Some("panel_turn_on_02"),
        _ => Some("unknown"),
    }
}

#[cfg(target_os = "espidf")]
fn render_image_direct(
    artifact: &ImageArtifact,
    config: &DeviceRuntimeConfig,
) -> Result<(), FailureKind> {
    let render_start = Instant::now();
    let mut decode_ms = 0u128;
    let mut pack_ms = 0u128;
    let mut flush_ms = 0u128;

    record_render_trace(TRACE_BEFORE_POWER_READY);
    let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "before_power_ready");
    let power_start = Instant::now();
    if !crate::power::ensure_ready_for_render() {
        let power_ms = power_start.elapsed().as_millis();
        let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "power_ready_fail");
        log_render_timing(
            "err",
            "power_ready",
            &artifact.format,
            artifact.bytes.len(),
            0,
            power_ms,
            decode_ms,
            pack_ms,
            flush_ms,
            render_start.elapsed().as_millis(),
        );
        return Err(FailureKind::PmicSoftFailure);
    }
    let power_ms = power_start.elapsed().as_millis();
    record_render_trace(TRACE_AFTER_POWER_READY);
    let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "after_power_ready");

    let options = crate::render_core::RenderOptions {
        panel_rotation: config.display_rotation as u8,
        color_process_mode: config.color_process_mode as u8,
        dithering_mode: config.dither_mode as u8,
        six_color_tolerance: config.six_color_tolerance as u8,
    };
    let packed = match artifact.format {
        ImageFormat::Bmp => {
            record_render_trace(TRACE_BEFORE_BMP_PACK);
            let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "before_bmp_pack");
            let pack_start = Instant::now();
            let packed = crate::render_core::render_bmp24_to_packed(&artifact.bytes, options)
                .map_err(|err| {
                    crate::device_log!("ERROR", "photoframe-rs/render: bmp render failed: {err}");
                    log_render_timing(
                        "err",
                        "bmp_pack",
                        &artifact.format,
                        artifact.bytes.len(),
                        0,
                        power_ms,
                        decode_ms,
                        pack_start.elapsed().as_millis(),
                        flush_ms,
                        render_start.elapsed().as_millis(),
                    );
                    FailureKind::GeneralFailure
                })?;
            pack_ms = pack_start.elapsed().as_millis();
            record_render_trace(TRACE_AFTER_BMP_PACK);
            let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "after_bmp_pack");
            packed
        }
        ImageFormat::Jpeg => {
            let _ =
                photoframe_platform_espidf::send_debug_stage_beacon(config, "before_jpeg_decode");
            let decode_start = Instant::now();
            let decoded = crate::jpeg::decode_rgb888(&artifact.bytes).map_err(|err| {
                crate::device_log!("ERROR", "photoframe-rs/render: jpeg decode failed: {err}");
                log_render_timing(
                    "err",
                    "jpeg_decode",
                    &artifact.format,
                    artifact.bytes.len(),
                    0,
                    power_ms,
                    decode_start.elapsed().as_millis(),
                    pack_ms,
                    flush_ms,
                    render_start.elapsed().as_millis(),
                );
                FailureKind::GeneralFailure
            })?;
            decode_ms = decode_start.elapsed().as_millis();
            crate::device_log!(
                "INFO",
                "photoframe-rs/render: jpeg decoded width={} height={} rgb_len={}",
                decoded.width, decoded.height, decoded.rgb_len
            );
            let _ =
                photoframe_platform_espidf::send_debug_stage_beacon(config, "after_jpeg_decode");
            let expected_rgb_len = (decoded.width as usize)
                .checked_mul(decoded.height as usize)
                .and_then(|pixels| pixels.checked_mul(3))
                .ok_or_else(|| {
                    crate::device_log!(
                        "ERROR",
                        "photoframe-rs/render: jpeg size overflow width={} height={}",
                        decoded.width, decoded.height
                    );
                    log_render_timing(
                        "err",
                        "jpeg_size_overflow",
                        &artifact.format,
                        artifact.bytes.len(),
                        0,
                        power_ms,
                        decode_ms,
                        pack_ms,
                        flush_ms,
                        render_start.elapsed().as_millis(),
                    );
                    FailureKind::GeneralFailure
                })?;
            if decoded.rgb_len < expected_rgb_len {
                crate::device_log!(
                    "ERROR",
                    "photoframe-rs/render: jpeg rgb_len too short actual={} expected={}",
                    decoded.rgb_len, expected_rgb_len
                );
                let _ = photoframe_platform_espidf::send_debug_stage_beacon(
                    config,
                    "jpeg_rgb_len_short",
                );
                log_render_timing(
                    "err",
                    "jpeg_rgb_len_short",
                    &artifact.format,
                    artifact.bytes.len(),
                    0,
                    power_ms,
                    decode_ms,
                    pack_ms,
                    flush_ms,
                    render_start.elapsed().as_millis(),
                );
                return Err(FailureKind::GeneralFailure);
            }
            let rgb = unsafe { std::slice::from_raw_parts(decoded.rgb, expected_rgb_len) };
            let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "before_rgb_pack");
            let pack_start = Instant::now();
            let packed = crate::render_core::render_rgb888_to_packed(
                rgb,
                decoded.width as usize,
                decoded.height as usize,
                options,
            )
            .map_err(|err| {
                crate::device_log!("ERROR", "photoframe-rs/render: rgb->packed failed: {err}");
                log_render_timing(
                    "err",
                    "rgb_pack",
                    &artifact.format,
                    artifact.bytes.len(),
                    0,
                    power_ms,
                    decode_ms,
                    pack_start.elapsed().as_millis(),
                    flush_ms,
                    render_start.elapsed().as_millis(),
                );
                FailureKind::GeneralFailure
            })?;
            pack_ms = pack_start.elapsed().as_millis();
            let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "after_rgb_pack");
            packed
        }
    };
    record_render_trace(TRACE_BEFORE_PANEL_FLUSH);
    let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "before_panel_flush");
    let hostname = if config.device_id.is_empty() {
        "photoframe-rs"
    } else {
        config.device_id.as_str()
    };
    crate::device_log!("INFO", "photoframe-rs/render: pause wifi before panel flush");
    EspWifiManager::pause_for_render();
    let flush_start = Instant::now();
    let flush_result = crate::panel::flush_packed_image(&packed.bytes);
    flush_ms = flush_start.elapsed().as_millis();
    crate::device_log!("INFO", "photoframe-rs/render: resume wifi after panel flush");
    if let Err(err) = EspWifiManager::reconnect_after_render(hostname, config) {
        crate::device_log!("WARN", "photoframe-rs/render: wifi resume failed after flush: {err}");
        log_render_timing(
            "err",
            "wifi_resume",
            &artifact.format,
            artifact.bytes.len(),
            packed.bytes.len(),
            power_ms,
            decode_ms,
            pack_ms,
            flush_ms,
            render_start.elapsed().as_millis(),
        );
        return Err(FailureKind::GeneralFailure);
    }
    if let Err(err) = flush_result {
        let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "panel_flush_failed");
        crate::device_log!("ERROR", "photoframe-rs/render: panel flush failed: {err}");
        log_render_timing(
            "err",
            "panel_flush",
            &artifact.format,
            artifact.bytes.len(),
            packed.bytes.len(),
            power_ms,
            decode_ms,
            pack_ms,
            flush_ms,
            render_start.elapsed().as_millis(),
        );
        return Err(FailureKind::GeneralFailure);
    }
    flush_ms = flush_start.elapsed().as_millis();
    record_render_trace(TRACE_AFTER_PANEL_FLUSH);
    let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "after_panel_flush");
    log_render_timing(
        "ok",
        "done",
        &artifact.format,
        artifact.bytes.len(),
        packed.bytes.len(),
        power_ms,
        decode_ms,
        pack_ms,
        flush_ms,
        render_start.elapsed().as_millis(),
    );
    Ok(())
}

pub struct EspRuntimeBridge;

impl EspRuntimeBridge {
    #[cfg(target_os = "espidf")]
    pub fn read_power_sample() -> Option<PowerSample> {
        crate::power::read_power_sample()
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn read_power_sample() -> Option<()> {
        None
    }

    #[cfg(target_os = "espidf")]
    pub fn render_image(
        artifact: &ImageArtifact,
        config: &DeviceRuntimeConfig,
    ) -> Result<(), FailureKind> {
        render_image_direct(artifact, config)
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn render_image(_artifact: &(), _config: &()) -> Result<(), ()> {
        Err(())
    }

    #[cfg(target_os = "espidf")]
    pub fn prepare_for_sleep() {
        crate::power::prepare_for_sleep()
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn prepare_for_sleep() {}
}
