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

bool ApplyRemoteConfigObject(const cJSON* config, AppConfig* cfg, std::string* error) {
  if (config == nullptr || cfg == nullptr || !cJSON_IsObject(config)) {
    if (error != nullptr) {
      *error = "config is not object";
    }
    return false;
  }

  const cJSON* orchestrator_enabled = cJSON_GetObjectItemCaseSensitive(config, "orchestrator_enabled");
  if (cJSON_IsNumber(orchestrator_enabled)) {
    cfg->orchestrator_enabled = orchestrator_enabled->valueint ? 1 : 0;
  }

  const cJSON* orchestrator_base_url = cJSON_GetObjectItemCaseSensitive(config, "orchestrator_base_url");
  if (cJSON_IsString(orchestrator_base_url) && orchestrator_base_url->valuestring != nullptr) {
    cfg->orchestrator_base_url = orchestrator_base_url->valuestring;
  }

  const cJSON* orchestrator_token = cJSON_GetObjectItemCaseSensitive(config, "orchestrator_token");
  if (cJSON_IsString(orchestrator_token) && orchestrator_token->valuestring != nullptr) {
    cfg->orchestrator_token = orchestrator_token->valuestring;
  }

  const cJSON* image_url_template = cJSON_GetObjectItemCaseSensitive(config, "image_url_template");
  if (cJSON_IsString(image_url_template) && image_url_template->valuestring != nullptr) {
    cfg->image_url_template = image_url_template->valuestring;
  }

  const cJSON* photo_token = cJSON_GetObjectItemCaseSensitive(config, "photo_token");
  if (cJSON_IsString(photo_token) && photo_token->valuestring != nullptr) {
    cfg->photo_token = photo_token->valuestring;
  }

  const cJSON* timezone = cJSON_GetObjectItemCaseSensitive(config, "timezone");
  if (cJSON_IsString(timezone) && timezone->valuestring != nullptr) {
    cfg->timezone = timezone->valuestring;
  }

  const cJSON* interval = cJSON_GetObjectItemCaseSensitive(config, "interval_minutes");
  if (cJSON_IsNumber(interval)) {
    cfg->interval_minutes = std::max(1, interval->valueint);
  }

  const cJSON* retry_base = cJSON_GetObjectItemCaseSensitive(config, "retry_base_minutes");
  if (cJSON_IsNumber(retry_base)) {
    cfg->retry_base_minutes = std::max(1, retry_base->valueint);
  }

  const cJSON* retry_max = cJSON_GetObjectItemCaseSensitive(config, "retry_max_minutes");
  if (cJSON_IsNumber(retry_max)) {
    cfg->retry_max_minutes = std::max(cfg->retry_base_minutes, retry_max->valueint);
  }

  const cJSON* max_fail = cJSON_GetObjectItemCaseSensitive(config, "max_failure_before_long_sleep");
  if (cJSON_IsNumber(max_fail)) {
    cfg->max_failure_before_long_sleep = std::max(1, max_fail->valueint);
  }

  const cJSON* rotation = cJSON_GetObjectItemCaseSensitive(config, "display_rotation");
  if (cJSON_IsNumber(rotation)) {
    cfg->display_rotation = (rotation->valueint == 0) ? 0 : 2;
  }

  const cJSON* color_mode = cJSON_GetObjectItemCaseSensitive(config, "color_process_mode");
  if (cJSON_IsNumber(color_mode)) {
    cfg->color_process_mode = std::clamp<int>(
        color_mode->valueint, static_cast<int>(AppConfig::kColorProcessAuto),
        static_cast<int>(AppConfig::kColorProcessAssumeSixColor));
  }

  const cJSON* dither_mode = cJSON_GetObjectItemCaseSensitive(config, "dither_mode");
  if (cJSON_IsNumber(dither_mode)) {
    cfg->dither_mode = std::clamp<int>(
        dither_mode->valueint, static_cast<int>(AppConfig::kDitherNone),
        static_cast<int>(AppConfig::kDitherOrdered));
  }

  const cJSON* tolerance = cJSON_GetObjectItemCaseSensitive(config, "six_color_tolerance");
  if (cJSON_IsNumber(tolerance)) {
    cfg->six_color_tolerance = std::clamp<int>(tolerance->valueint, 0, 64);
  }

  return true;
}

void AddReportedConfig(cJSON* root, const AppConfig& cfg) {
  // 设备每次 checkin 上报当前生效配置，供后台页面用灰字提示默认值。
  if (root == nullptr) {
    return;
  }

  cJSON* reported = cJSON_CreateObject();
  if (reported == nullptr) {
    return;
  }

  cJSON_AddNumberToObject(reported, "orchestrator_enabled", cfg.orchestrator_enabled ? 1 : 0);
  cJSON_AddStringToObject(reported, "orchestrator_base_url", cfg.orchestrator_base_url.c_str());
  cJSON_AddStringToObject(reported, "orchestrator_token", cfg.orchestrator_token.c_str());
  cJSON_AddStringToObject(reported, "image_url_template", cfg.image_url_template.c_str());
  cJSON_AddStringToObject(reported, "photo_token", cfg.photo_token.c_str());
  cJSON_AddNumberToObject(reported, "interval_minutes", std::max(1, cfg.interval_minutes));
  cJSON_AddNumberToObject(reported, "retry_base_minutes", std::max(1, cfg.retry_base_minutes));
  cJSON_AddNumberToObject(reported, "retry_max_minutes",
                          std::max(std::max(1, cfg.retry_base_minutes), cfg.retry_max_minutes));
  cJSON_AddNumberToObject(reported, "max_failure_before_long_sleep",
                          std::max(1, cfg.max_failure_before_long_sleep));
  cJSON_AddNumberToObject(reported, "display_rotation", cfg.display_rotation == 0 ? 0 : 2);
  cJSON_AddNumberToObject(reported, "color_process_mode",
                          std::clamp(cfg.color_process_mode,
                                     static_cast<int>(AppConfig::kColorProcessAuto),
                                     static_cast<int>(AppConfig::kColorProcessAssumeSixColor)));
  cJSON_AddNumberToObject(reported, "dither_mode",
                          std::clamp(cfg.dither_mode, static_cast<int>(AppConfig::kDitherNone),
                                     static_cast<int>(AppConfig::kDitherOrdered)));
  cJSON_AddNumberToObject(reported, "six_color_tolerance", std::clamp(cfg.six_color_tolerance, 0, 64));
  cJSON_AddStringToObject(reported, "timezone", cfg.timezone.c_str());

  cJSON_AddItemToObject(root, "reported_config", reported);
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

DeviceConfigSyncResult OrchestratorClient::SyncDeviceConfig(AppConfig* cfg, ConfigStore* store,
                                                            int64_t now_epoch) {
  DeviceConfigSyncResult result;
  if (cfg == nullptr || store == nullptr) {
    result.error = "cfg/store null";
    return result;
  }
  if (cfg->orchestrator_enabled == 0 || cfg->orchestrator_base_url.empty() || cfg->device_id.empty()) {
    result.ok = true;
    result.config_version = cfg->remote_config_version;
    return result;
  }

  std::string url = TrimTrailingSlash(cfg->orchestrator_base_url) +
                    "/api/v1/device/config?device_id=" + UrlEncode(cfg->device_id) +
                    "&now_epoch=" + std::to_string(static_cast<long long>(now_epoch)) +
                    "&current_version=" + std::to_string(std::max(0, cfg->remote_config_version));

  esp_http_client_config_t http_cfg = {};
  http_cfg.url = url.c_str();
  http_cfg.timeout_ms = kHttpTimeoutMs;
  http_cfg.disable_auto_redirect = false;

  esp_http_client_handle_t client = esp_http_client_init(&http_cfg);
  if (client == nullptr) {
    result.error = "esp_http_client_init failed";
    return result;
  }
  SetCommonHeaders(client, *cfg);

  esp_err_t err = esp_http_client_open(client, 0);
  if (err != ESP_OK) {
    result.error = std::string("open failed: ") + esp_err_to_name(err);
    esp_http_client_cleanup(client);
    return result;
  }

  (void)esp_http_client_fetch_headers(client);
  const int status_code = esp_http_client_get_status_code(client);

  std::string body;
  const bool body_ok = ReadResponseBody(client, &body);
  esp_http_client_close(client);
  esp_http_client_cleanup(client);

  if (!body_ok) {
    result.error = "read response failed";
    return result;
  }
  if (status_code != 200) {
    result.error = "unexpected status: " + std::to_string(status_code);
    return result;
  }

  cJSON* root = cJSON_Parse(body.c_str());
  if (root == nullptr) {
    result.error = "invalid json";
    return result;
  }

  int target_version = std::max(0, cfg->remote_config_version);
  const cJSON* ver = cJSON_GetObjectItemCaseSensitive(root, "config_version");
  if (cJSON_IsNumber(ver)) {
    target_version = std::max(0, ver->valueint);
  }
  result.config_version = target_version;

  if (target_version <= cfg->remote_config_version) {
    result.ok = true;
    cJSON_Delete(root);
    return result;
  }

  const cJSON* config_json = cJSON_GetObjectItemCaseSensitive(root, "config");
  const AppConfig previous = *cfg;
  AppConfig next = *cfg;
  std::string apply_error;
  if (!ApplyRemoteConfigObject(config_json, &next, &apply_error)) {
    result.error = apply_error.empty() ? "invalid config object" : apply_error;
    cJSON_Delete(root);
    (void)ReportConfigApplied(*cfg, target_version, false, result.error, now_epoch);
    return result;
  }

  next.remote_config_version = target_version;
  if (!store->Save(next)) {
    result.error = "save config failed";
    cJSON_Delete(root);
    (void)ReportConfigApplied(*cfg, target_version, false, result.error, now_epoch);
    return result;
  }

  *cfg = next;
  result.ok = true;
  result.updated = true;
  cJSON_Delete(root);
  (void)ReportConfigApplied(previous, target_version, true, "", now_epoch);
  return result;
}

bool OrchestratorClient::ReportConfigApplied(const AppConfig& cfg, int config_version, bool applied,
                                             const std::string& error, int64_t now_epoch) {
  if (cfg.orchestrator_enabled == 0 || cfg.orchestrator_base_url.empty() || cfg.device_id.empty()) {
    return false;
  }

  cJSON* root = cJSON_CreateObject();
  cJSON_AddStringToObject(root, "device_id", cfg.device_id.c_str());
  cJSON_AddNumberToObject(root, "config_version", std::max(0, config_version));
  cJSON_AddBoolToObject(root, "applied", applied);
  cJSON_AddStringToObject(root, "error", error.c_str());
  cJSON_AddNumberToObject(root, "applied_epoch", static_cast<double>(now_epoch));

  char* json = cJSON_PrintUnformatted(root);
  cJSON_Delete(root);
  if (json == nullptr) {
    return false;
  }

  const std::string url = TrimTrailingSlash(cfg.orchestrator_base_url) + "/api/v1/device/config/applied";

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
    ESP_LOGW(kTag, "report config applied failed: %s", esp_err_to_name(err));
    return false;
  }
  if (status_code < 200 || status_code >= 300) {
    ESP_LOGW(kTag, "report config applied non-2xx status=%d", status_code);
    return false;
  }
  return true;
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
  AddReportedConfig(root, cfg);

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
