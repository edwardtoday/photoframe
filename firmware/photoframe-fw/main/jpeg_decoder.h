#pragma once

#include <cstddef>
#include <cstdint>
#include <string>

// JPEG 解码输出：RGB888（每像素 3 字节，R/G/B），由解码器分配内存。
struct JpegDecodedImage {
  uint8_t* rgb = nullptr;
  size_t rgb_len = 0;
  int width = 0;
  int height = 0;
};

class JpegDecoder {
 public:
  // 解码 JPEG -> RGB888。成功时 out->rgb 需要调用 FreeDecodedImage 释放。
  static bool DecodeRgb888(const uint8_t* jpeg,
                           size_t len,
                           JpegDecodedImage* out,
                           std::string* error);
  static void FreeDecodedImage(JpegDecodedImage* img);
};

