#include "portal_server.h"

#include <algorithm>
#include <array>
#include <cerrno>
#include <cstring>
#include <string>

#include "cJSON.h"
#include "esp_log.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/sockets.h"

namespace {
constexpr const char* kTag = "portal_server";
constexpr uint16_t kDnsPort = 53;
constexpr const char* kPortalHtml = R"HTML(
<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>PhotoFrame 配网</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; margin: 1rem; }
    input, button, select { width: 100%; margin: .4rem 0; padding: .6rem; font-size: 1rem; }
    .card { border: 1px solid #ddd; border-radius: .5rem; padding: 1rem; margin-bottom: 1rem; }
    .muted { color: #666; font-size: .9rem; }
  </style>
</head>
<body>
  <h2>PhotoFrame 配置门户</h2>
  <p class="muted">保存后设备会自动重启并尝试联网。</p>

  <div class="card">
    <h3>Wi-Fi</h3>
    <button onclick="scanWifi()">扫描 Wi-Fi</button>
    <select id="ssidSelect" onchange="fillSsid()"><option value="">手动输入 SSID</option></select>
    <input id="ssid" placeholder="SSID" />
    <input id="password" type="password" placeholder="Password（留空则保持不变）" />
    <p class="muted">仅在需要修改 Wi-Fi 密码时填写；留空不会覆盖当前密码。</p>
  </div>

  <div class="card">
    <h3>拉图配置</h3>
    <input id="urlTemplate" placeholder="URL 模板，例如 http://host/image/480x800?date=%DATE%" />
    <select id="orchEnabled">
      <option value="1">编排服务：启用（推荐）</option>
      <option value="0">编排服务：关闭（仅按 URL 模板拉图）</option>
    </select>
    <input id="orchBaseUrl" placeholder="编排服务地址，例如 http://192.168.58.113:18081" />
    <input id="deviceId" placeholder="设备 ID（留空则自动生成）" />
    <input id="orchToken" placeholder="编排服务 Token（可选）" />
    <input id="photoToken" placeholder="图片拉取 Token（可选，HTTP Header: X-Photo-Token）" />
    <input id="interval" type="number" min="1" placeholder="刷新间隔（分钟）" />
    <input id="retryBase" type="number" min="1" placeholder="失败重试基数（分钟）" />
    <input id="retryMax" type="number" min="1" placeholder="失败重试上限（分钟）" />
    <input id="maxFail" type="number" min="1" placeholder="连续失败阈值" />
    <select id="rotation">
      <option value="0">旋转 0（推荐）</option>
      <option value="2">旋转 180</option>
    </select>
    <select id="colorMode">
      <option value="0">色彩模式：自动判断（推荐）</option>
      <option value="1">色彩模式：总是转换为 6 色</option>
      <option value="2">色彩模式：认为输入已是 6 色</option>
    </select>
    <select id="ditherMode">
      <option value="1">转换抖动：有序抖动（推荐）</option>
      <option value="0">转换抖动：关闭</option>
    </select>
    <input id="colorTol" type="number" min="0" max="64" placeholder="6 色判断容差（0-64）" />
    <input id="timezone" placeholder="时区，例如 Asia/Shanghai 或 UTC" />
  </div>

  <button onclick="saveAll()">保存配置并重启</button>
  <pre id="out"></pre>

  <script>
    const out = (msg) => document.getElementById('out').textContent = msg;
    let loadedConfig = null;

    async function api(path, opt = {}) {
      const r = await fetch(path, {headers: {'Content-Type': 'application/json'}, ...opt});
      const t = await r.text();
      let j = null;
      try { j = JSON.parse(t); } catch { }
      if (!r.ok) throw new Error((j && j.error) || t || ('HTTP ' + r.status));
      return j;
    }

    async function loadConfig() {
      const cfg = await api('/api/config');
      loadedConfig = cfg;
      document.getElementById('ssid').value = cfg.wifi_ssid ?? '';
      document.getElementById('urlTemplate').value = cfg.image_url_template ?? '';
      document.getElementById('orchEnabled').value = String(cfg.orchestrator_enabled ?? 1);
      document.getElementById('orchBaseUrl').value = cfg.orchestrator_base_url ?? '';
      document.getElementById('deviceId').value = cfg.device_id ?? '';
      document.getElementById('orchToken').value = cfg.orchestrator_token ?? '';
      document.getElementById('photoToken').value = cfg.photo_token ?? '';
      document.getElementById('interval').value = cfg.interval_minutes ?? 60;
      document.getElementById('retryBase').value = cfg.retry_base_minutes ?? 5;
      document.getElementById('retryMax').value = cfg.retry_max_minutes ?? 240;
      document.getElementById('maxFail').value = cfg.max_failure_before_long_sleep ?? 24;
      document.getElementById('rotation').value = String(cfg.display_rotation ?? 0);
      document.getElementById('colorMode').value = String(cfg.color_process_mode ?? 0);
      document.getElementById('ditherMode').value = String(cfg.dither_mode ?? 1);
      document.getElementById('colorTol').value = cfg.six_color_tolerance ?? 0;
      document.getElementById('timezone').value = cfg.timezone ?? 'UTC';
      out(JSON.stringify(cfg, null, 2));
    }

    async function scanWifi() {
      try {
        const data = await api('/api/wifi/scan');
        const sel = document.getElementById('ssidSelect');
        sel.innerHTML = '<option value="">手动输入 SSID</option>';
        (data.networks || []).forEach(n => {
          const op = document.createElement('option');
          op.value = n.ssid;
          op.textContent = `${n.ssid} (RSSI ${n.rssi})`;
          sel.appendChild(op);
        });
        out('扫描完成，共 ' + (data.networks || []).length + ' 个网络');
      } catch (e) {
        out('扫描失败: ' + e.message);
      }
    }

    function fillSsid() {
      const v = document.getElementById('ssidSelect').value;
      if (v) document.getElementById('ssid').value = v;
    }

    async function saveAll() {
      const payload = {
        image_url_template: document.getElementById('urlTemplate').value,
        orchestrator_enabled: Number(document.getElementById('orchEnabled').value),
        orchestrator_base_url: document.getElementById('orchBaseUrl').value,
        device_id: document.getElementById('deviceId').value,
        orchestrator_token: document.getElementById('orchToken').value,
        photo_token: document.getElementById('photoToken').value,
        interval_minutes: Number(document.getElementById('interval').value),
        retry_base_minutes: Number(document.getElementById('retryBase').value),
        retry_max_minutes: Number(document.getElementById('retryMax').value),
        max_failure_before_long_sleep: Number(document.getElementById('maxFail').value),
        display_rotation: Number(document.getElementById('rotation').value),
        color_process_mode: Number(document.getElementById('colorMode').value),
        dither_mode: Number(document.getElementById('ditherMode').value),
        six_color_tolerance: Number(document.getElementById('colorTol').value),
        timezone: document.getElementById('timezone').value,
      };

      const ssid = document.getElementById('ssid').value.trim();
      const oldSsid = (loadedConfig?.wifi_ssid ?? '').trim();
      if (ssid !== '') {
        payload.wifi_ssid = ssid;
      } else if (oldSsid === '') {
        payload.wifi_ssid = '';
      }

      const password = document.getElementById('password').value;
      if (password !== '') {
        payload.wifi_password = password;
      }

      try {
        const ret = await api('/api/config', {method: 'POST', body: JSON.stringify(payload)});
        out(JSON.stringify(ret, null, 2));
      } catch (e) {
        out('保存失败: ' + e.message);
      }
    }

    loadConfig();
  </script>
</body>
</html>
)HTML";

std::string ReadBody(httpd_req_t* req) {
  std::string body;
  body.resize(req->content_len);
  int offset = 0;
  while (offset < req->content_len) {
    const int n = httpd_req_recv(req, body.data() + offset, req->content_len - offset);
    if (n <= 0) {
      return {};
    }
    offset += n;
  }
  return body;
}

void SendJson(httpd_req_t* req, const char* json) {
  httpd_resp_set_type(req, "application/json");
  httpd_resp_set_hdr(req, "Cache-Control", "no-store");
  httpd_resp_send(req, json, HTTPD_RESP_USE_STRLEN);
}
}  // namespace

bool PortalServer::Start(AppConfig* config, RuntimeStatus* status, ConfigStore* store, bool enable_dns) {
  if (server_ != nullptr) {
    return true;
  }
  config_ = config;
  status_ = status;
  store_ = store;
  should_reboot_.store(false);

  httpd_config_t cfg = HTTPD_DEFAULT_CONFIG();
  cfg.server_port = 80;
  cfg.max_uri_handlers = 16;
  cfg.lru_purge_enable = true;

  if (httpd_start(&server_, &cfg) != ESP_OK) {
    ESP_LOGE(kTag, "httpd_start failed");
    server_ = nullptr;
    return false;
  }

  httpd_uri_t root = {
      .uri = "/", .method = HTTP_GET, .handler = &PortalServer::HandleRoot, .user_ctx = this};
  httpd_uri_t get_cfg = {.uri = "/api/config",
                         .method = HTTP_GET,
                         .handler = &PortalServer::HandleGetConfig,
                         .user_ctx = this};
  httpd_uri_t post_cfg = {.uri = "/api/config",
                          .method = HTTP_POST,
                          .handler = &PortalServer::HandlePostConfig,
                          .user_ctx = this};
  httpd_uri_t scan = {.uri = "/api/wifi/scan",
                      .method = HTTP_GET,
                      .handler = &PortalServer::HandleScanWifi,
                      .user_ctx = this};

  httpd_register_uri_handler(server_, &root);
  httpd_register_uri_handler(server_, &get_cfg);
  httpd_register_uri_handler(server_, &post_cfg);
  httpd_register_uri_handler(server_, &scan);

  if (enable_dns) {
    if (!StartDnsServer()) {
      ESP_LOGW(kTag, "dns server start failed, captive portal may be limited");
    }
  }

  ESP_LOGI(kTag, "portal server started");
  return true;
}

void PortalServer::Stop() {
  StopDnsServer();
  if (server_ != nullptr) {
    httpd_stop(server_);
    server_ = nullptr;
  }
}

esp_err_t PortalServer::HandleRoot(httpd_req_t* req) {
  httpd_resp_set_type(req, "text/html; charset=utf-8");
  httpd_resp_send(req, kPortalHtml, HTTPD_RESP_USE_STRLEN);
  return ESP_OK;
}

esp_err_t PortalServer::HandleGetConfig(httpd_req_t* req) {
  auto* self = static_cast<PortalServer*>(req->user_ctx);
  return self->SendConfigJson(req);
}

esp_err_t PortalServer::SendConfigJson(httpd_req_t* req) {
  cJSON* root = cJSON_CreateObject();
  cJSON_AddStringToObject(root, "wifi_ssid", config_->wifi_ssid.c_str());
  cJSON_AddNumberToObject(root, "wifi_profile_count", config_->wifi_profile_count);
  cJSON_AddNumberToObject(root, "last_connected_wifi_index", config_->last_connected_wifi_index);
  cJSON* wifi_profiles = cJSON_CreateArray();
  for (int i = 0; i < config_->wifi_profile_count && i < AppConfig::kMaxWifiProfiles; ++i) {
    cJSON* item = cJSON_CreateObject();
    cJSON_AddStringToObject(item, "ssid", config_->wifi_profiles[i].ssid.c_str());
    cJSON_AddNumberToObject(item, "password_len",
                            static_cast<double>(config_->wifi_profiles[i].password.size()));
    cJSON_AddItemToArray(wifi_profiles, item);
  }
  cJSON_AddItemToObject(root, "wifi_profiles", wifi_profiles);
  cJSON_AddStringToObject(root, "image_url_template", config_->image_url_template.c_str());
  cJSON_AddNumberToObject(root, "orchestrator_enabled", config_->orchestrator_enabled);
  cJSON_AddStringToObject(root, "orchestrator_base_url", config_->orchestrator_base_url.c_str());
  cJSON_AddStringToObject(root, "device_id", config_->device_id.c_str());
  cJSON_AddStringToObject(root, "orchestrator_token", config_->orchestrator_token.c_str());
  cJSON_AddStringToObject(root, "photo_token", config_->photo_token.c_str());
  cJSON_AddStringToObject(root, "timezone", config_->timezone.c_str());
  cJSON_AddNumberToObject(root, "interval_minutes", config_->interval_minutes);
  cJSON_AddNumberToObject(root, "retry_base_minutes", config_->retry_base_minutes);
  cJSON_AddNumberToObject(root, "retry_max_minutes", config_->retry_max_minutes);
  cJSON_AddNumberToObject(root, "max_failure_before_long_sleep",
                          config_->max_failure_before_long_sleep);
  cJSON_AddNumberToObject(root, "display_rotation", config_->display_rotation);
  cJSON_AddNumberToObject(root, "color_process_mode", config_->color_process_mode);
  cJSON_AddNumberToObject(root, "dither_mode", config_->dither_mode);
  cJSON_AddNumberToObject(root, "six_color_tolerance", config_->six_color_tolerance);
  cJSON_AddBoolToObject(root, "wifi_connected", status_->wifi_connected);
  cJSON_AddBoolToObject(root, "force_refresh", status_->force_refresh);
  cJSON_AddNumberToObject(root, "last_http_status", status_->last_http_status);
  cJSON_AddBoolToObject(root, "image_changed", status_->image_changed);
  cJSON_AddStringToObject(root, "image_source", status_->image_source.c_str());
  cJSON_AddNumberToObject(root, "next_wakeup_epoch", static_cast<double>(status_->next_wakeup_epoch));
  cJSON_AddNumberToObject(root, "battery_mv", status_->battery_mv);
  cJSON_AddNumberToObject(root, "battery_percent", status_->battery_percent);
  cJSON_AddNumberToObject(root, "charging", status_->charging);
  cJSON_AddNumberToObject(root, "vbus_good", status_->vbus_good);
  cJSON_AddStringToObject(root, "last_error", status_->last_error.c_str());

  char* str = cJSON_PrintUnformatted(root);
  SendJson(req, str != nullptr ? str : "{}");
  if (str != nullptr) {
    cJSON_free(str);
  }
  cJSON_Delete(root);
  return ESP_OK;
}

esp_err_t PortalServer::HandlePostConfig(httpd_req_t* req) {
  auto* self = static_cast<PortalServer*>(req->user_ctx);
  const std::string body = ReadBody(req);
  if (body.empty()) {
    httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "empty body");
    return ESP_FAIL;
  }

  cJSON* root = cJSON_Parse(body.c_str());
  if (root == nullptr) {
    httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "invalid json");
    return ESP_FAIL;
  }

  bool wifi_changed = false;
  bool display_cfg_changed = false;

  const cJSON* ssid = cJSON_GetObjectItemCaseSensitive(root, "wifi_ssid");
  const cJSON* password = cJSON_GetObjectItemCaseSensitive(root, "wifi_password");
  const bool ssid_provided = cJSON_IsString(ssid) && ssid->valuestring != nullptr;
  const bool password_provided = cJSON_IsString(password) && password->valuestring != nullptr;
  if (ssid_provided) {
    const std::string next_ssid = ssid->valuestring;
    if (next_ssid.empty() && !self->config_->wifi_ssid.empty()) {
      ESP_LOGW(kTag, "ignore empty wifi_ssid update to keep existing credentials");
    } else if (self->config_->wifi_ssid != next_ssid) {
      self->config_->wifi_ssid = next_ssid;
      wifi_changed = true;
    }
  }
  if (password_provided) {
    if (password->valuestring[0] == '\0') {
      ESP_LOGI(kTag, "wifi password left blank in portal request, keep existing password");
    } else {
      const std::string next_password = password->valuestring;
      if (self->config_->wifi_password != next_password) {
        self->config_->wifi_password = next_password;
        wifi_changed = true;
      }
    }
  }

  ESP_LOGI(kTag, "apply config request: ssid_provided=%d pwd_provided=%d pwd_len=%u wifi_changed=%d",
           ssid_provided ? 1 : 0, password_provided ? 1 : 0,
           static_cast<unsigned>(self->config_->wifi_password.size()), wifi_changed ? 1 : 0);

  const cJSON* url = cJSON_GetObjectItemCaseSensitive(root, "image_url_template");
  if (cJSON_IsString(url) && url->valuestring != nullptr) {
    self->config_->image_url_template = url->valuestring;
  }

  const cJSON* orch_enabled = cJSON_GetObjectItemCaseSensitive(root, "orchestrator_enabled");
  if (cJSON_IsNumber(orch_enabled)) {
    self->config_->orchestrator_enabled = orch_enabled->valueint ? 1 : 0;
  }

  const cJSON* orch_url = cJSON_GetObjectItemCaseSensitive(root, "orchestrator_base_url");
  if (cJSON_IsString(orch_url) && orch_url->valuestring != nullptr) {
    self->config_->orchestrator_base_url = orch_url->valuestring;
  }

  const cJSON* device_id = cJSON_GetObjectItemCaseSensitive(root, "device_id");
  if (cJSON_IsString(device_id) && device_id->valuestring != nullptr) {
    self->config_->device_id = device_id->valuestring;
  }

  const cJSON* orch_token = cJSON_GetObjectItemCaseSensitive(root, "orchestrator_token");
  if (cJSON_IsString(orch_token) && orch_token->valuestring != nullptr) {
    self->config_->orchestrator_token = orch_token->valuestring;
  }

  const cJSON* photo_token = cJSON_GetObjectItemCaseSensitive(root, "photo_token");
  if (cJSON_IsString(photo_token) && photo_token->valuestring != nullptr) {
    self->config_->photo_token = photo_token->valuestring;
  }

  const cJSON* tz = cJSON_GetObjectItemCaseSensitive(root, "timezone");
  if (cJSON_IsString(tz) && tz->valuestring != nullptr) {
    self->config_->timezone = tz->valuestring;
  }

  const cJSON* interval = cJSON_GetObjectItemCaseSensitive(root, "interval_minutes");
  if (cJSON_IsNumber(interval)) {
    self->config_->interval_minutes = std::max(1, interval->valueint);
  }

  const cJSON* retry_base = cJSON_GetObjectItemCaseSensitive(root, "retry_base_minutes");
  if (cJSON_IsNumber(retry_base)) {
    self->config_->retry_base_minutes = std::max(1, retry_base->valueint);
  }

  const cJSON* retry_max = cJSON_GetObjectItemCaseSensitive(root, "retry_max_minutes");
  if (cJSON_IsNumber(retry_max)) {
    self->config_->retry_max_minutes = std::max(self->config_->retry_base_minutes, retry_max->valueint);
  }

  const cJSON* max_fail = cJSON_GetObjectItemCaseSensitive(root, "max_failure_before_long_sleep");
  if (cJSON_IsNumber(max_fail)) {
    self->config_->max_failure_before_long_sleep = std::max(1, max_fail->valueint);
  }

  const cJSON* rotation = cJSON_GetObjectItemCaseSensitive(root, "display_rotation");
  if (cJSON_IsNumber(rotation)) {
    const int next_rotation = (rotation->valueint == 0) ? 0 : 2;
    if (self->config_->display_rotation != next_rotation) {
      self->config_->display_rotation = next_rotation;
      display_cfg_changed = true;
    }
  }

  const cJSON* color_mode = cJSON_GetObjectItemCaseSensitive(root, "color_process_mode");
  if (cJSON_IsNumber(color_mode)) {
    const int next_mode = std::clamp<int>(
        color_mode->valueint, static_cast<int>(AppConfig::kColorProcessAuto),
        static_cast<int>(AppConfig::kColorProcessAssumeSixColor));
    if (self->config_->color_process_mode != next_mode) {
      self->config_->color_process_mode = next_mode;
      display_cfg_changed = true;
    }
  }

  const cJSON* dither_mode = cJSON_GetObjectItemCaseSensitive(root, "dither_mode");
  if (cJSON_IsNumber(dither_mode)) {
    const int next_dither = std::clamp<int>(dither_mode->valueint,
                                            static_cast<int>(AppConfig::kDitherNone),
                                            static_cast<int>(AppConfig::kDitherOrdered));
    if (self->config_->dither_mode != next_dither) {
      self->config_->dither_mode = next_dither;
      display_cfg_changed = true;
    }
  }

  const cJSON* color_tol = cJSON_GetObjectItemCaseSensitive(root, "six_color_tolerance");
  if (cJSON_IsNumber(color_tol)) {
    const int next_tol = std::clamp(color_tol->valueint, 0, 64);
    if (self->config_->six_color_tolerance != next_tol) {
      self->config_->six_color_tolerance = next_tol;
      display_cfg_changed = true;
    }
  }

  if (display_cfg_changed) {
    // 显示参数变化时清空 hash，确保下一轮即使图片 URL/内容不变也会刷新面板。
    self->config_->last_image_sha256.clear();
  }

  const bool ok = self->store_->Save(*self->config_);
  cJSON_Delete(root);

  cJSON* ret = cJSON_CreateObject();
  cJSON_AddBoolToObject(ret, "ok", ok);
  cJSON_AddBoolToObject(ret, "reboot_required", wifi_changed);
  if (!ok) {
    cJSON_AddStringToObject(ret, "error", "save failed");
  }

  char* str = cJSON_PrintUnformatted(ret);
  SendJson(req, str != nullptr ? str : "{}");
  if (str != nullptr) {
    cJSON_free(str);
  }
  cJSON_Delete(ret);

  if (ok) {
    self->should_reboot_.store(true);
  }
  return ok ? ESP_OK : ESP_FAIL;
}

esp_err_t PortalServer::HandleScanWifi(httpd_req_t* req) {
  wifi_scan_config_t scan_cfg = {};
  scan_cfg.show_hidden = false;
  esp_err_t err = esp_wifi_scan_start(&scan_cfg, true);
  if (err != ESP_OK) {
    httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "scan failed");
    return err;
  }

  uint16_t count = 20;
  wifi_ap_record_t records[20] = {};
  err = esp_wifi_scan_get_ap_records(&count, records);
  if (err != ESP_OK) {
    httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "scan read failed");
    return err;
  }

  cJSON* root = cJSON_CreateObject();
  cJSON* networks = cJSON_AddArrayToObject(root, "networks");
  for (uint16_t i = 0; i < count; ++i) {
    cJSON* ap = cJSON_CreateObject();
    cJSON_AddStringToObject(ap, "ssid", reinterpret_cast<const char*>(records[i].ssid));
    cJSON_AddNumberToObject(ap, "rssi", records[i].rssi);
    cJSON_AddNumberToObject(ap, "authmode", records[i].authmode);
    cJSON_AddItemToArray(networks, ap);
  }

  char* str = cJSON_PrintUnformatted(root);
  SendJson(req, str != nullptr ? str : "{}");
  if (str != nullptr) {
    cJSON_free(str);
  }
  cJSON_Delete(root);
  return ESP_OK;
}

bool PortalServer::StartDnsServer() {
  dns_running_.store(true);
  BaseType_t ok = xTaskCreate(&PortalServer::DnsTask, "dns_portal", 4096, this, 5, nullptr);
  return ok == pdPASS;
}

void PortalServer::StopDnsServer() {
  dns_running_.store(false);
  if (dns_sock_ >= 0) {
    shutdown(dns_sock_, SHUT_RDWR);
    close(dns_sock_);
    dns_sock_ = -1;
  }
}

void PortalServer::DnsTask(void* arg) {
  auto* self = static_cast<PortalServer*>(arg);

  const int sock = socket(AF_INET, SOCK_DGRAM, 0);
  if (sock < 0) {
    ESP_LOGE(kTag, "dns socket failed: %d", errno);
    vTaskDelete(nullptr);
    return;
  }
  self->dns_sock_ = sock;

  sockaddr_in addr = {};
  addr.sin_family = AF_INET;
  addr.sin_port = htons(kDnsPort);
  addr.sin_addr.s_addr = htonl(INADDR_ANY);

  if (bind(sock, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) < 0) {
    ESP_LOGE(kTag, "dns bind failed: %d", errno);
    close(sock);
    self->dns_sock_ = -1;
    vTaskDelete(nullptr);
    return;
  }

  std::array<uint8_t, 512> req = {};
  std::array<uint8_t, 512> resp = {};

  while (self->dns_running_.load()) {
    sockaddr_in from = {};
    socklen_t from_len = sizeof(from);
    const int n = recvfrom(sock, req.data(), req.size(), 0, reinterpret_cast<sockaddr*>(&from),
                           &from_len);
    if (n <= 12) {
      continue;
    }

    // 查找 query 末尾。
    int q_end = 12;
    while (q_end < n && req[q_end] != 0) {
      q_end += req[q_end] + 1;
    }
    if (q_end + 5 >= n) {
      continue;
    }
    const int question_len = q_end + 5 - 12;

    memset(resp.data(), 0, resp.size());
    resp[0] = req[0];
    resp[1] = req[1];
    resp[2] = 0x81;
    resp[3] = 0x80;
    resp[4] = 0x00;
    resp[5] = 0x01;
    resp[6] = 0x00;
    resp[7] = 0x01;

    memcpy(resp.data() + 12, req.data() + 12, static_cast<size_t>(question_len));
    int o = 12 + question_len;

    resp[o++] = 0xC0;
    resp[o++] = 0x0C;
    resp[o++] = 0x00;
    resp[o++] = 0x01;
    resp[o++] = 0x00;
    resp[o++] = 0x01;
    resp[o++] = 0x00;
    resp[o++] = 0x00;
    resp[o++] = 0x00;
    resp[o++] = 0x3C;
    resp[o++] = 0x00;
    resp[o++] = 0x04;
    resp[o++] = 192;
    resp[o++] = 168;
    resp[o++] = 73;
    resp[o++] = 1;

    sendto(sock, resp.data(), o, 0, reinterpret_cast<sockaddr*>(&from), from_len);
  }

  close(sock);
  self->dns_sock_ = -1;
  vTaskDelete(nullptr);
}
