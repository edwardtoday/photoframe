#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use std::{ffi::c_void, ptr};

#[cfg(target_os = "espidf")]
#[repr(C)]
#[derive(Clone, Copy)]
struct JpegResolution {
    width: u16,
    height: u16,
}

#[cfg(target_os = "espidf")]
#[repr(C)]
struct JpegDecConfig {
    output_type: i32,
    scale: JpegResolution,
    clipper: JpegResolution,
    rotate: i32,
    block_enable: bool,
}

#[cfg(target_os = "espidf")]
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct JpegDecHeaderInfo {
    width: u16,
    height: u16,
}

#[cfg(target_os = "espidf")]
#[repr(C)]
struct JpegDecIo {
    inbuf: *mut u8,
    inbuf_len: i32,
    inbuf_remain: i32,
    outbuf: *mut u8,
    out_size: i32,
}

#[cfg(target_os = "espidf")]
const JPEG_PIXEL_FORMAT_RGB888: i32 = 1;
#[cfg(target_os = "espidf")]
const JPEG_ROTATE_0D: i32 = 0;

#[cfg(target_os = "espidf")]
unsafe extern "C" {
    fn jpeg_dec_open(config: *mut JpegDecConfig, jpeg_dec: *mut *mut c_void) -> i32;
    fn jpeg_dec_parse_header(
        jpeg_dec: *mut c_void,
        io: *mut JpegDecIo,
        out_info: *mut JpegDecHeaderInfo,
    ) -> i32;
    fn jpeg_dec_get_outbuf_len(jpeg_dec: *mut c_void, outbuf_len: *mut i32) -> i32;
    fn jpeg_dec_process(jpeg_dec: *mut c_void, io: *mut JpegDecIo) -> i32;
    fn jpeg_dec_close(jpeg_dec: *mut c_void) -> i32;
    fn jpeg_calloc_align(size: usize, aligned: i32) -> *mut c_void;
    fn jpeg_free_align(data: *mut c_void);
}

#[cfg(target_os = "espidf")]
pub struct DecodedJpeg {
    pub rgb: *mut u8,
    pub rgb_len: usize,
    pub width: i32,
    pub height: i32,
}

#[cfg(target_os = "espidf")]
impl Drop for DecodedJpeg {
    fn drop(&mut self) {
        if !self.rgb.is_null() {
            unsafe { jpeg_free_align(self.rgb as *mut c_void) };
            self.rgb = ptr::null_mut();
        }
    }
}

#[cfg(target_os = "espidf")]
pub fn decode_rgb888(jpeg: &[u8]) -> Result<DecodedJpeg, String> {
    if jpeg.len() < 16 {
        return Err("invalid jpeg buffer".into());
    }

    let mut config = JpegDecConfig {
        output_type: JPEG_PIXEL_FORMAT_RGB888,
        scale: JpegResolution {
            width: 0,
            height: 0,
        },
        clipper: JpegResolution {
            width: 0,
            height: 0,
        },
        rotate: JPEG_ROTATE_0D,
        block_enable: false,
    };

    let mut dec: *mut c_void = ptr::null_mut();
    let ret = unsafe { jpeg_dec_open(&mut config, &mut dec) };
    if ret != 0 {
        return Err(format!("jpeg_dec_open failed: {ret}"));
    }

    let result = (|| {
        let mut io = JpegDecIo {
            inbuf: jpeg.as_ptr() as *mut u8,
            inbuf_len: jpeg.len() as i32,
            inbuf_remain: 0,
            outbuf: ptr::null_mut(),
            out_size: 0,
        };
        let mut info = JpegDecHeaderInfo::default();
        let ret = unsafe { jpeg_dec_parse_header(dec, &mut io, &mut info) };
        if ret != 0 {
            return Err(format!("jpeg_dec_parse_header failed: {ret}"));
        }

        let mut outbuf_len = 0;
        let ret = unsafe { jpeg_dec_get_outbuf_len(dec, &mut outbuf_len) };
        if ret != 0 || outbuf_len <= 0 {
            return Err(format!("jpeg_dec_get_outbuf_len failed: {ret}"));
        }

        let outbuf = unsafe { jpeg_calloc_align(outbuf_len as usize, 16) } as *mut u8;
        if outbuf.is_null() {
            return Err("jpeg output alloc failed".into());
        }
        io.outbuf = outbuf;

        let ret = unsafe { jpeg_dec_process(dec, &mut io) };
        if ret != 0 {
            unsafe { jpeg_free_align(outbuf as *mut c_void) };
            return Err(format!("jpeg_dec_process failed: {ret}"));
        }

        Ok(DecodedJpeg {
            rgb: outbuf,
            rgb_len: outbuf_len as usize,
            width: info.width as i32,
            height: info.height as i32,
        })
    })();

    unsafe {
        let _ = jpeg_dec_close(dec);
    }
    result
}

#[cfg(not(target_os = "espidf"))]
pub fn decode_rgb888(_jpeg: &[u8]) -> Result<(), String> {
    Err("jpeg decode only works on espidf target".into())
}
