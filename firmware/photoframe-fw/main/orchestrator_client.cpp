#include "orchestrator_client.h"

#include <algorithm>
#include <cctype>
#include <cstdio>
#include <cstring>

#include "cJSON.h"
#include "esp_http_client.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_system.h"

namespace {
constexpr const char* kTag = "orchestrator";
constexpr int kHttpTimeoutMs = 12000;

std::string TrimTrailingSlash(const std::string& input) {
  std::string out = input;
  while (!out.empty() && out.back() == '/') {
    out.pop_back();
  }
  return out;
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

bool ReadResponseBody(esp_http_client_handle_t client, std::string* body) {
  if (body == nullptr) {
    return false;
  }

  body->clear();
  const int content_len = esp_http_client_get_content_length(client);
  if (content_len > 0 && content_len < 64 * 1024) {
    body->reserve(static_cast<size_t>(content_len));
  }

  char chunk[512] = {};
  while (true) {
    const int n = esp_http_client_read(client, chunk, sizeof(chunk));
    if (n < 0) {
      return false;
    }
    if (n == 0) {
      break;
    }
    body->append(chunk, static_cast<size_t>(n));
  }
  return true;
}

void SetCommonHeaders(esp_http_client_handle_t client, const AppConfig& cfg) {
  esp_http_client_set_header(client, "Accept", "application/json");
  if (!cfg.orchestrator_token.empty()) {
    esp_http_client_set_header(client, "X-PhotoFrame-Token", cfg.orchestrator_token.c_str());
  }
}
}  // namespace

std::string OrchestratorClient::EnsureDeviceId(AppConfig* cfg) {
  if (cfg == nullptr) {
    return "";
  }
  if (!cfg->device_id.empty()) {
    return cfg->device_id;
  }

  uint8_t mac[6] = {0};
  if (esp_read_mac(mac, ESP_MAC_WIFI_STA) == ESP_OK) {
    char buf[32] = {0};
    snprintf(buf, sizeof(buf), "pf-%02x%02x%02x%02x", mac[2], mac[3], mac[4], mac[5]);
    cfg->device_id = buf;
  } else {
    cfg->device_id = "pf-unknown";
  }
  return cfg->device_id;
}

FrameDirective OrchestratorClient::FetchDirective(const AppConfig& cfg, time_t now_epoch) {
  FrameDirective directive;
  if (cfg.orchestrator_enabled == 0) {
    directive.error = "orchestrator disabled";
    return directive;
  }
  if (cfg.orchestrator_base_url.empty()) {
    directive.error = "orchestrator base url is empty";
    return directive;
  }
  if (cfg.device_id.empty()) {
    directive.error = "device id is empty";
    return directive;
  }

  const int default_poll_seconds = std::max(60, cfg.interval_minutes * 60);
  std::string url = TrimTrailingSlash(cfg.orchestrator_base_url) +
                    "/api/v1/device/next?device_id=" + UrlEncode(cfg.device_id) +
                    "&now_epoch=" + std::to_string(static_cast<long long>(now_epoch)) +
                    "&default_poll_seconds=" + std::to_string(default_poll_seconds) +
                    "&failure_count=" + std::to_string(std::max(0, cfg.failure_count));

  esp_http_client_config_t http_cfg = {};
  http_cfg.url = url.c_str();
  http_cfg.timeout_ms = kHttpTimeoutMs;
  http_cfg.disable_auto_redirect = false;

  esp_http_client_handle_t client = esp_http_client_init(&http_cfg);
  if (client == nullptr) {
    directive.error = "esp_http_client_init failed";
    return directive;
  }
  SetCommonHeaders(client, cfg);

  esp_err_t err = esp_http_client_open(client, 0);
  if (err != ESP_OK) {
    directive.error = std::string("open failed: ") + esp_err_to_name(err);
    esp_http_client_cleanup(client);
    return directive;
  }

  (void)esp_http_client_fetch_headers(client);
  directive.status_code = esp_http_client_get_status_code(client);

  std::string body;
  const bool body_ok = ReadResponseBody(client, &body);
  esp_http_client_close(client);
  esp_http_client_cleanup(client);

  if (!body_ok) {
    directive.error = "read response failed";
    return directive;
  }
  if (directive.status_code != 200) {
    directive.error = "unexpected status: " + std::to_string(directive.status_code);
    return directive;
  }

  cJSON* root = cJSON_Parse(body.c_str());
  if (root == nullptr) {
    directive.error = "invalid json";
    return directive;
  }

  const cJSON* image_url = cJSON_GetObjectItemCaseSensitive(root, "image_url");
  if (cJSON_IsString(image_url) && image_url->valuestring != nullptr) {
    directive.image_url = image_url->valuestring;
  }

  const cJSON* source = cJSON_GetObjectItemCaseSensitive(root, "source");
  if (cJSON_IsString(source) && source->valuestring != nullptr) {
    directive.source = source->valuestring;
  }

  const cJSON* poll = cJSON_GetObjectItemCaseSensitive(root, "poll_after_seconds");
  if (cJSON_IsNumber(poll)) {
    directive.poll_after_seconds = std::clamp(poll->valueint, 60, 86400);
  }

  const cJSON* until = cJSON_GetObjectItemCaseSensitive(root, "valid_until_epoch");
  if (cJSON_IsNumber(until)) {
    directive.valid_until_epoch = static_cast<int64_t>(until->valuedouble);
  }

  const cJSON* error = cJSON_GetObjectItemCaseSensitive(root, "error");
  if (directive.image_url.empty()) {
    if (cJSON_IsString(error) && error->valuestring != nullptr) {
      directive.error = error->valuestring;
    } else {
      directive.error = "missing image_url";
    }
    cJSON_Delete(root);
    return directive;
  }

  directive.ok = true;
  cJSON_Delete(root);
  return directive;
}

bool OrchestratorClient::ReportCheckin(const AppConfig& cfg, const DeviceCheckinPayload& payload) {
  if (cfg.orchestrator_enabled == 0 || cfg.orchestrator_base_url.empty() || cfg.device_id.empty()) {
    return false;
  }

  cJSON* root = cJSON_CreateObject();
  cJSON_AddStringToObject(root, "device_id", cfg.device_id.c_str());
  cJSON_AddNumberToObject(root, "checkin_epoch", static_cast<double>(payload.now_epoch));
  cJSON_AddNumberToObject(root, "next_wakeup_epoch", static_cast<double>(payload.next_wakeup_epoch));
  cJSON_AddNumberToObject(root, "sleep_seconds", static_cast<double>(payload.sleep_seconds));
  cJSON_AddNumberToObject(root, "poll_interval_seconds", payload.poll_interval_seconds);
  cJSON_AddNumberToObject(root, "failure_count", std::max(0, payload.failure_count));
  cJSON_AddNumberToObject(root, "last_http_status", payload.last_http_status);
  cJSON_AddBoolToObject(root, "fetch_ok", payload.fetch_ok);
  cJSON_AddBoolToObject(root, "image_changed", payload.image_changed);
  cJSON_AddStringToObject(root, "image_source", payload.image_source.c_str());
  cJSON_AddStringToObject(root, "last_error", payload.last_error.c_str());

  char* json = cJSON_PrintUnformatted(root);
  cJSON_Delete(root);
  if (json == nullptr) {
    return false;
  }

  const std::string url = TrimTrailingSlash(cfg.orchestrator_base_url) + "/api/v1/device/checkin";

  esp_http_client_config_t http_cfg = {};
  http_cfg.url = url.c_str();
  http_cfg.timeout_ms = kHttpTimeoutMs;
  http_cfg.method = HTTP_METHOD_POST;
  http_cfg.disable_auto_redirect = false;

  esp_http_client_handle_t client = esp_http_client_init(&http_cfg);
  if (client == nullptr) {
    cJSON_free(json);
    return false;
  }

  SetCommonHeaders(client, cfg);
  esp_http_client_set_header(client, "Content-Type", "application/json");
  esp_http_client_set_post_field(client, json, static_cast<int>(strlen(json)));

  const esp_err_t err = esp_http_client_perform(client);
  const int status_code = esp_http_client_get_status_code(client);
  esp_http_client_cleanup(client);
  cJSON_free(json);

  if (err != ESP_OK) {
    ESP_LOGW(kTag, "check-in failed: %s", esp_err_to_name(err));
    return false;
  }
  if (status_code < 200 || status_code >= 300) {
    ESP_LOGW(kTag, "check-in non-2xx status=%d", status_code);
    return false;
  }
  return true;
}
