#include "image_client.h"

#include <algorithm>
#include <cctype>
#include <cstdio>
#include <cstring>

#include "esp_crt_bundle.h"
#include "esp_heap_caps.h"
#include "esp_http_client.h"
#include "esp_log.h"
#include "mbedtls/sha256.h"

namespace {
constexpr const char* kTag = "image_client";
constexpr const char* kPhotoTokenHeader = "X-Photo-Token";

bool IsHttpsUrl(const std::string& url) {
  return url.rfind("https://", 0) == 0;
}

std::string Sha256Hex(const uint8_t* data, size_t len) {
  uint8_t digest[32] = {0};
  mbedtls_sha256_context ctx;
  mbedtls_sha256_init(&ctx);
  mbedtls_sha256_starts(&ctx, 0);
  mbedtls_sha256_update(&ctx, data, len);
  mbedtls_sha256_finish(&ctx, digest);
  mbedtls_sha256_free(&ctx);

  char out[65] = {0};
  for (size_t i = 0; i < sizeof(digest); ++i) {
    snprintf(out + i * 2, 3, "%02x", digest[i]);
  }
  return std::string(out);
}


std::string UrlEncode(const std::string& input) {
  std::string out;
  out.reserve(input.size() * 2);
  for (unsigned char c : input) {
    if (std::isalnum(c) || c == '-' || c == '_' || c == '.' || c == '~') {
      out.push_back(static_cast<char>(c));
      continue;
    }
    char buf[4] = {0};
    snprintf(buf, sizeof(buf), "%%%02X", static_cast<unsigned>(c));
    out.append(buf);
  }
  return out;
}
}  // namespace

std::string ImageClient::BuildDatedUrl(const std::string& tpl, time_t now,
                                       const std::string& device_id) {
  std::tm tm_local = {};
  localtime_r(&now, &tm_local);

  char date_buf[16] = {0};
  strftime(date_buf, sizeof(date_buf), "%Y-%m-%d", &tm_local);
  std::string url = tpl;

  size_t pos = 0;
  while ((pos = url.find("%DATE%", pos)) != std::string::npos) {
    url.replace(pos, 6, date_buf);
    pos += 10;
  }

  const std::string safe_device_id = device_id.empty() ? "unknown" : device_id;
  pos = 0;
  while ((pos = url.find("%DEVICE_ID%", pos)) != std::string::npos) {
    url.replace(pos, strlen("%DEVICE_ID%"), safe_device_id);
    pos += safe_device_id.size();
  }

  if (!device_id.empty() && url.find("device_id=") == std::string::npos) {
    const size_t fragment_pos = url.find('#');
    std::string base = url;
    std::string fragment;
    if (fragment_pos != std::string::npos) {
      base = url.substr(0, fragment_pos);
      fragment = url.substr(fragment_pos);
    }

    const char connector = (base.find('?') == std::string::npos) ? '?' : '&';
    base.push_back(connector);
    base.append("device_id=");
    base.append(UrlEncode(device_id));
    url = base + fragment;
  }

  // 仅替换模板占位符，不再自动补 date= 参数，避免未校时阶段出现 1970-01-01。
  return url;
}

ImageFetchResult ImageClient::FetchBmp(const std::string& url,
                                       const std::string& previous_sha256,
                                       const std::string& photo_token) {
  ImageFetchResult result;
  esp_http_client_config_t cfg = {};
  cfg.url = url.c_str();
  cfg.timeout_ms = 20000;
  cfg.disable_auto_redirect = false;
  if (IsHttpsUrl(url)) {
    // 启用系统证书包校验 HTTPS 证书，确保公网拉图链路默认安全。
    cfg.crt_bundle_attach = esp_crt_bundle_attach;
  }

  esp_http_client_handle_t client = esp_http_client_init(&cfg);
  if (client == nullptr) {
    result.error = "esp_http_client_init failed";
    return result;
  }

  if (!photo_token.empty()) {
    esp_http_client_set_header(client, kPhotoTokenHeader, photo_token.c_str());
  }

  esp_err_t err = esp_http_client_open(client, 0);
  if (err != ESP_OK) {
    result.error = std::string("http open failed: ") + esp_err_to_name(err);
    esp_http_client_cleanup(client);
    return result;
  }

  const int header_len = esp_http_client_fetch_headers(client);
  (void)header_len;
  result.status_code = esp_http_client_get_status_code(client);
  const int content_len = esp_http_client_get_content_length(client);

  if (result.status_code != 200) {
    result.error = "unexpected status: " + std::to_string(result.status_code);
    if (result.status_code == 401 || result.status_code == 403) {
      result.error += ", check X-Photo-Token";
    }
    esp_http_client_close(client);
    esp_http_client_cleanup(client);
    return result;
  }

  char* ctype = nullptr;
  if (esp_http_client_get_header(client, "Content-Type", &ctype) == ESP_OK && ctype != nullptr) {
    if (std::string(ctype).find("image/bmp") == std::string::npos) {
      result.error = std::string("unexpected Content-Type: ") + ctype;
      esp_http_client_close(client);
      esp_http_client_cleanup(client);
      return result;
    }
  }

  if (content_len <= 0 || content_len > (4 * 1024 * 1024)) {
    result.error = "invalid content length: " + std::to_string(content_len);
    esp_http_client_close(client);
    esp_http_client_cleanup(client);
    return result;
  }

  uint8_t* buf = static_cast<uint8_t*>(
      heap_caps_malloc(static_cast<size_t>(content_len), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT));
  if (buf == nullptr) {
    result.error = "failed to allocate bmp buffer";
    esp_http_client_close(client);
    esp_http_client_cleanup(client);
    return result;
  }

  int offset = 0;
  while (offset < content_len) {
    const int n = esp_http_client_read(client, reinterpret_cast<char*>(buf + offset),
                                       content_len - offset);
    if (n <= 0) {
      break;
    }
    offset += n;
  }

  esp_http_client_close(client);
  esp_http_client_cleanup(client);

  if (offset != content_len) {
    result.error = "incomplete body: " + std::to_string(offset) + "/" + std::to_string(content_len);
    free(buf);
    return result;
  }

  result.sha256 = Sha256Hex(buf, static_cast<size_t>(content_len));
  if (result.sha256.empty()) {
    result.error = "sha256 failed";
    free(buf);
    return result;
  }
  result.image_changed = result.sha256 != previous_sha256;
  result.ok = true;
  result.data = buf;
  result.data_len = static_cast<size_t>(content_len);

  ESP_LOGI(kTag, "downloaded bmp %d bytes sha256=%s", content_len, result.sha256.c_str());
  return result;
}

void ImageClient::FreeResultBuffer(ImageFetchResult* result) {
  if (result != nullptr && result->data != nullptr) {
    free(result->data);
    result->data = nullptr;
    result->data_len = 0;
  }
}
