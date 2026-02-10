#pragma once

#include <cstddef>
#include <cstdint>
#include <ctime>
#include <string>

struct ImageFetchResult {
  bool ok = false;
  bool image_changed = false;
  int status_code = 0;
  std::string sha256;
  std::string error;
  uint8_t* data = nullptr;
  size_t data_len = 0;
};

class ImageClient {
 public:
  static std::string BuildDatedUrl(const std::string& tpl, time_t now,
                                   const std::string& device_id = "");
  static ImageFetchResult FetchBmp(const std::string& url, const std::string& previous_sha256,
                                   const std::string& photo_token = "");
  static void FreeResultBuffer(ImageFetchResult* result);
};
