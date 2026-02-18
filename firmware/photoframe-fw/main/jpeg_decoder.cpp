#include "jpeg_decoder.h"

#include <cstdio>
#include <cstring>

extern "C" {
#include "esp_jpeg_common.h"
#include "esp_jpeg_dec.h"
}

#include "esp_log.h"
#include "esp_timer.h"

namespace {
constexpr const char* kTag = "jpeg_decoder";

const char* JpegErrorName(jpeg_error_t err) {
  switch (err) {
    case JPEG_ERR_OK:
      return "OK";
    case JPEG_ERR_FAIL:
      return "FAIL";
    case JPEG_ERR_NO_MEM:
      return "NO_MEM";
    case JPEG_ERR_NO_MORE_DATA:
      return "NO_MORE_DATA";
    case JPEG_ERR_INVALID_PARAM:
      return "INVALID_PARAM";
    case JPEG_ERR_BAD_DATA:
      return "BAD_DATA";
    case JPEG_ERR_UNSUPPORT_FMT:
      return "UNSUPPORT_FMT";
    case JPEG_ERR_UNSUPPORT_STD:
      return "UNSUPPORT_STD";
    default:
      return "UNKNOWN";
  }
}

std::string FormatJpegError(const char* what, jpeg_error_t err) {
  char buf[128] = {};
  snprintf(buf, sizeof(buf), "%s: %s(%d)", what, JpegErrorName(err), static_cast<int>(err));
  return std::string(buf);
}
}  // namespace

bool JpegDecoder::DecodeRgb888(const uint8_t* jpeg,
                               size_t len,
                               JpegDecodedImage* out,
                               std::string* error) {
  if (out == nullptr) {
    return false;
  }
  *out = JpegDecodedImage{};

  if (jpeg == nullptr || len < 16) {
    if (error != nullptr) {
      *error = "invalid jpeg buffer";
    }
    return false;
  }

  const int64_t start_us = esp_timer_get_time();

  jpeg_dec_config_t config = DEFAULT_JPEG_DEC_CONFIG();
  // 统一输出 RGB888，后续复用现有墨水屏 6 色量化链路。
  config.output_type = JPEG_PIXEL_FORMAT_RGB888;
  config.rotate = JPEG_ROTATE_0D;
  config.block_enable = false;

  jpeg_dec_handle_t dec = nullptr;
  jpeg_error_t ret = jpeg_dec_open(&config, &dec);
  if (ret != JPEG_ERR_OK) {
    if (error != nullptr) {
      *error = FormatJpegError("jpeg_dec_open failed", ret);
    }
    return false;
  }

  jpeg_dec_io_t io = {};
  io.inbuf = const_cast<uint8_t*>(jpeg);
  io.inbuf_len = static_cast<int>(len);

  jpeg_dec_header_info_t info = {};
  ret = jpeg_dec_parse_header(dec, &io, &info);
  if (ret != JPEG_ERR_OK) {
    if (error != nullptr) {
      *error = FormatJpegError("jpeg_dec_parse_header failed", ret);
    }
    jpeg_dec_close(dec);
    return false;
  }

  int outbuf_len = 0;
  ret = jpeg_dec_get_outbuf_len(dec, &outbuf_len);
  if (ret != JPEG_ERR_OK || outbuf_len <= 0) {
    if (error != nullptr) {
      *error = FormatJpegError("jpeg_dec_get_outbuf_len failed", ret);
    }
    jpeg_dec_close(dec);
    return false;
  }

  // ESP32-S3 上 JPEG 解码输出要求 16 字节对齐，避免图像左右错列。
  uint8_t* outbuf = static_cast<uint8_t*>(jpeg_calloc_align(static_cast<size_t>(outbuf_len), 16));
  if (outbuf == nullptr) {
    if (error != nullptr) {
      *error = "jpeg output alloc failed";
    }
    jpeg_dec_close(dec);
    return false;
  }
  io.outbuf = outbuf;

  ret = jpeg_dec_process(dec, &io);
  if (ret != JPEG_ERR_OK) {
    if (error != nullptr) {
      *error = FormatJpegError("jpeg_dec_process failed", ret);
    }
    jpeg_free_align(outbuf);
    jpeg_dec_close(dec);
    return false;
  }

  // Decoder deinitialize
  jpeg_dec_close(dec);

  out->rgb = outbuf;
  out->rgb_len = static_cast<size_t>(outbuf_len);
  out->width = static_cast<int>(info.width);
  out->height = static_cast<int>(info.height);

  const int64_t cost_us = esp_timer_get_time() - start_us;
  ESP_LOGI(kTag, "jpeg decoded: %dx%d rgb_len=%u cost=%lldms",
           out->width, out->height, static_cast<unsigned>(out->rgb_len),
           static_cast<long long>(cost_us / 1000));
  return true;
}

void JpegDecoder::FreeDecodedImage(JpegDecodedImage* img) {
  if (img == nullptr) {
    return;
  }
  if (img->rgb != nullptr) {
    jpeg_free_align(img->rgb);
    img->rgb = nullptr;
  }
  img->rgb_len = 0;
  img->width = 0;
  img->height = 0;
}
