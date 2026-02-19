#pragma once

#include <cstddef>
#include <cstdint>
#include <ctime>
#include <string>

struct ImageFetchResult {
  bool ok = false;
  bool image_changed = false;
  int status_code = 0;
  std::string content_type;
  std::string etag;
  std::string last_modified;
  enum class ImageFormat : uint8_t {
    kUnknown = 0,
    kBmp = 1,
    kJpeg = 2,
  };
  ImageFormat format = ImageFormat::kUnknown;
  std::string sha256;
  std::string error;
  uint8_t* data = nullptr;
  size_t data_len = 0;
};

class ImageClient {
 public:
  static std::string BuildDatedUrl(const std::string& tpl, time_t now,
                                   const std::string& device_id = "");
  // 下载图片原始字节，并根据 Content-Type/magic 粗略识别格式（BMP/JPEG）。
  // 若服务端支持 ETag/Last-Modified，可传入上次保存的值用于条件 GET（304 不下载正文）。
  static ImageFetchResult FetchImage(const std::string& url, const std::string& previous_sha256,
                                     const std::string& photo_token = "",
                                     const std::string& previous_etag = "",
                                     const std::string& previous_last_modified = "");
  static void FreeResultBuffer(ImageFetchResult* result);
};
