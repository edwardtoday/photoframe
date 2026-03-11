#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use photoframe_app::{DeviceRuntimeConfig, ImageArtifact, ImageFormat, PowerSample};
#[cfg(target_os = "espidf")]
use photoframe_domain::FailureKind;

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
        let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "before_power_ready");
        if !crate::power::ensure_ready_for_render() {
            let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "power_ready_fail");
            return Err(FailureKind::PmicSoftFailure);
        }
        let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "after_power_ready");

        let options = crate::render_core::RenderOptions {
            panel_rotation: config.display_rotation as u8,
            color_process_mode: config.color_process_mode as u8,
            dithering_mode: config.dither_mode as u8,
            six_color_tolerance: config.six_color_tolerance as u8,
        };
        let packed = match artifact.format {
            ImageFormat::Bmp => {
                let _ =
                    photoframe_platform_espidf::send_debug_stage_beacon(config, "before_bmp_pack");
                let packed =
                    crate::render_core::render_bmp24_to_packed(&artifact.bytes, options).map_err(
                        |err| {
                            println!("photoframe-rs/render: bmp render failed: {err}");
                            FailureKind::GeneralFailure
                        },
                    )?;
                let _ =
                    photoframe_platform_espidf::send_debug_stage_beacon(config, "after_bmp_pack");
                packed
            }
            ImageFormat::Jpeg => {
                let _ = photoframe_platform_espidf::send_debug_stage_beacon(
                    config,
                    "before_jpeg_decode",
                );
                let decoded = crate::jpeg::decode_rgb888(&artifact.bytes).map_err(|err| {
                        println!("photoframe-rs/render: jpeg decode failed: {err}");
                        FailureKind::GeneralFailure
                    })?;
                println!(
                    "photoframe-rs/render: jpeg decoded width={} height={} rgb_len={}",
                    decoded.width, decoded.height, decoded.rgb_len
                );
                let _ = photoframe_platform_espidf::send_debug_stage_beacon(
                    config,
                    "after_jpeg_decode",
                );
                let expected_rgb_len = (decoded.width as usize)
                    .checked_mul(decoded.height as usize)
                    .and_then(|pixels| pixels.checked_mul(3))
                    .ok_or_else(|| {
                        println!(
                            "photoframe-rs/render: jpeg size overflow width={} height={}",
                            decoded.width, decoded.height
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
                    return Err(FailureKind::GeneralFailure);
                }
                let rgb = unsafe { std::slice::from_raw_parts(decoded.rgb, expected_rgb_len) };
                let _ =
                    photoframe_platform_espidf::send_debug_stage_beacon(config, "before_rgb_pack");
                crate::render_core::render_rgb888_to_packed(
                    rgb,
                    decoded.width as usize,
                    decoded.height as usize,
                    options,
                )
                .map_err(|err| {
                    println!("photoframe-rs/render: rgb->packed failed: {err}");
                    FailureKind::GeneralFailure
                })
                .inspect(|_| {
                    let _ = photoframe_platform_espidf::send_debug_stage_beacon(
                        config,
                        "after_rgb_pack",
                    );
                })?
            }
        };
        let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "before_panel_flush");
        crate::panel::flush_packed_image(&packed.bytes).map_err(|err| {
            println!("photoframe-rs/render: panel flush failed: {err}");
            FailureKind::GeneralFailure
        })?;
        let _ = photoframe_platform_espidf::send_debug_stage_beacon(config, "after_panel_flush");
        Ok(())
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
