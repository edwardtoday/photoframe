#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use photoframe_app::{DeviceRuntimeConfig, ImageArtifact, ImageFormat, PowerSample};
#[cfg(target_os = "espidf")]
use photoframe_domain::FailureKind;
#[cfg(target_os = "espidf")]
use std::{
    sync::{OnceLock, mpsc},
    thread,
    time::Instant,
};

#[cfg(target_os = "espidf")]
const RENDER_WORKER_STACK_SIZE: usize = 64 * 1024;

#[cfg(target_os = "espidf")]
struct RenderRequest {
    artifact: ImageArtifact,
    config: DeviceRuntimeConfig,
    result_tx: mpsc::SyncSender<Result<(), FailureKind>>,
}

#[cfg(target_os = "espidf")]
struct RenderWorker {
    tx: mpsc::Sender<RenderRequest>,
}

#[cfg(target_os = "espidf")]
static RENDER_WORKER: OnceLock<RenderWorker> = OnceLock::new();

#[cfg(target_os = "espidf")]
fn render_worker() -> &'static RenderWorker {
    RENDER_WORKER.get_or_init(|| RenderWorker::spawn().expect("render worker spawn failed"))
}

#[cfg(target_os = "espidf")]
impl RenderWorker {
    fn spawn() -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<RenderRequest>();
        thread::Builder::new()
            .name("pf-render".into())
            .stack_size(RENDER_WORKER_STACK_SIZE)
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    let mut latest = first;
                    let mut responders = vec![latest.result_tx.clone()];
                    let mut merged = 0usize;
                    while let Ok(next) = rx.try_recv() {
                        responders.push(next.result_tx.clone());
                        latest = next;
                        merged += 1;
                    }
                    if merged > 0 {
                        println!(
                            "photoframe-rs/render: coalesced {} pending requests for device_id={}",
                            merged, latest.config.device_id
                        );
                    }
                    let result = render_image_direct(&latest.artifact, &latest.config);
                    for responder in responders {
                        let _ = responder.send(result);
                    }
                }
            })
            .map_err(|err| err.to_string())?;
        Ok(Self { tx })
    }

    fn render(
        &self,
        artifact: ImageArtifact,
        config: DeviceRuntimeConfig,
    ) -> Result<(), FailureKind> {
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        self.tx
            .send(RenderRequest {
                artifact,
                config,
                result_tx,
            })
            .map_err(|err| {
                println!("photoframe-rs/render: enqueue failed: {err}");
                FailureKind::GeneralFailure
            })?;
        result_rx.recv().map_err(|err| {
            println!("photoframe-rs/render: await result failed: {err}");
            FailureKind::GeneralFailure
        })?
    }
}

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
    println!(
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
    let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "after_power_ready");

    let options = crate::render_core::RenderOptions {
        panel_rotation: config.display_rotation as u8,
        color_process_mode: config.color_process_mode as u8,
        dithering_mode: config.dither_mode as u8,
        six_color_tolerance: config.six_color_tolerance as u8,
    };
    let packed = match artifact.format {
        ImageFormat::Bmp => {
            let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "before_bmp_pack");
            let pack_start = Instant::now();
            let packed = crate::render_core::render_bmp24_to_packed(&artifact.bytes, options)
                .map_err(|err| {
                    println!("photoframe-rs/render: bmp render failed: {err}");
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
            let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "after_bmp_pack");
            packed
        }
        ImageFormat::Jpeg => {
            let _ =
                photoframe_platform_espidf::send_debug_stage_beacon(config, "before_jpeg_decode");
            let decode_start = Instant::now();
            let decoded = crate::jpeg::decode_rgb888(&artifact.bytes).map_err(|err| {
                println!("photoframe-rs/render: jpeg decode failed: {err}");
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
            println!(
                "photoframe-rs/render: jpeg decoded width={} height={} rgb_len={}",
                decoded.width, decoded.height, decoded.rgb_len
            );
            let _ =
                photoframe_platform_espidf::send_debug_stage_beacon(config, "after_jpeg_decode");
            let expected_rgb_len = (decoded.width as usize)
                .checked_mul(decoded.height as usize)
                .and_then(|pixels| pixels.checked_mul(3))
                .ok_or_else(|| {
                    println!(
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
                println!(
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
                println!("photoframe-rs/render: rgb->packed failed: {err}");
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
    let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "before_panel_flush");
    let flush_start = Instant::now();
    if let Err(err) = crate::panel::flush_packed_image(&packed.bytes) {
        flush_ms = flush_start.elapsed().as_millis();
        let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "panel_flush_failed");
        println!("photoframe-rs/render: panel flush failed: {err}");
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
        render_worker().render(artifact.clone(), config.clone())
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
