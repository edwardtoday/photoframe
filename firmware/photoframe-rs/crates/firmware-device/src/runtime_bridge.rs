#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
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
        if !crate::power::ensure_ready_for_render() {
            return Err(FailureKind::PmicSoftFailure);
        }

        let options = crate::render_core::RenderOptions {
            panel_rotation: config.display_rotation as u8,
            color_process_mode: config.color_process_mode as u8,
            dithering_mode: config.dither_mode as u8,
            six_color_tolerance: config.six_color_tolerance as u8,
        };
        let packed = match artifact.format {
            ImageFormat::Bmp => crate::render_core::render_bmp24_to_packed(&artifact.bytes, options)
                .map_err(|err| {
                    println!("photoframe-rs/render: bmp render failed: {err}");
                    FailureKind::GeneralFailure
                })?,
            ImageFormat::Jpeg => {
                let decoded = crate::jpeg::decode_rgb888(&artifact.bytes)
                    .map_err(|err| {
                        println!("photoframe-rs/render: jpeg decode failed: {err}");
                        FailureKind::GeneralFailure
                    })?;
                let rgb = unsafe { std::slice::from_raw_parts(decoded.rgb, decoded.rgb_len) };
                crate::render_core::render_rgb888_to_packed(
                    rgb,
                    decoded.width as usize,
                    decoded.height as usize,
                    options,
                )
                .map_err(|err| {
                    println!("photoframe-rs/render: rgb->packed failed: {err}");
                    FailureKind::GeneralFailure
                })?
            }
        };
        crate::panel::flush_packed_image(&packed.bytes).map_err(|err| {
            println!("photoframe-rs/render: panel flush failed: {err}");
            FailureKind::GeneralFailure
        })
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
