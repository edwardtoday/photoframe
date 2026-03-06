#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <string>
#include <vector>

#include "config_store.h"
#include "image_client.h"
#include "jpeg_decoder.h"
#include "orchestrator_client.h"
#include "photopainter_epd.h"
#include "portal_server.h"
#include "power_manager.h"

#include "esp_event.h"
#include "esp_attr.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_sleep.h"
#include "esp_sntp.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "driver/gpio.h"
#include "driver/rtc_io.h"
#include "driver/usb_serial_jtag.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/task.h"
#include "lwip/ip4_addr.h"
#include "esp_idf_version.h"

namespace {
constexpr const char* kTag = "photoframe_main";

constexpr gpio_num_t kKeyButton = GPIO_NUM_4;   // KEY: 手动同步（不再默认打开配置窗口）
constexpr gpio_num_t kBootButton = GPIO_NUM_0;  // BOOT: 手动强制刷新
constexpr int kStaConnectTimeoutSec = 25;
constexpr int kStaConnectRetry = 5;
constexpr const char* kApSsid = "PhotoFrame-Setup";
constexpr const char* kApPassword = "12345678";
constexpr uint8_t kApIpA = 192;
constexpr uint8_t kApIpB = 168;
constexpr uint8_t kApIpC = 73;
constexpr uint8_t kApIpD = 1;
constexpr int kKeyWakePortalWindowSec = 120;
constexpr int kPortalLoopStepMs = 200;
constexpr int kEpdRefreshMaxRetries = 3;
constexpr int kEpdRefreshRetryDelayMs = 500;
constexpr int kPmicInitMaxRetries = 3;
constexpr int kPmicInitRetryDelayMs = 150;
constexpr uint64_t kSpuriousExt1TimerOnlyMaxSec = 600;

EventGroupHandle_t g_wifi_events = nullptr;
constexpr int kWifiConnectedBit = BIT0;
constexpr int kWifiFailBit = BIT1;
int g_wifi_retry = 0;
int g_wifi_retry_limit = kStaConnectRetry;
int g_last_disconnect_reason = 0;
bool g_wifi_ready = false;
esp_netif_t* g_sta_netif = nullptr;
esp_netif_t* g_ap_netif = nullptr;

// 深睡会导致程序从头启动，但 RTC slow memory 可保留少量数据；
// 用它缓存上一次成功读到的电源信息，避免某轮 I2C/PMIC 抽风时“整轮不上报”。
RTC_DATA_ATTR int g_cached_battery_mv = -1;
RTC_DATA_ATTR int g_cached_battery_percent = -1;
RTC_DATA_ATTR int g_cached_charging = -1;
RTC_DATA_ATTR int g_cached_vbus_good = -1;
RTC_DATA_ATTR int64_t g_cached_power_epoch = 0;

void RefreshPowerStatus(RuntimeStatus* status);

const char* WifiReasonToString(wifi_err_reason_t reason) {
  switch (reason) {
    case WIFI_REASON_UNSPECIFIED:
      return "UNSPECIFIED";
    case WIFI_REASON_AUTH_EXPIRE:
      return "AUTH_EXPIRE";
    case WIFI_REASON_AUTH_LEAVE:
      return "AUTH_LEAVE";
    case WIFI_REASON_ASSOC_TOOMANY:
      return "ASSOC_TOOMANY";
    case WIFI_REASON_ASSOC_LEAVE:
      return "ASSOC_LEAVE";
    case WIFI_REASON_ASSOC_NOT_AUTHED:
      return "ASSOC_NOT_AUTHED";
    case WIFI_REASON_IE_INVALID:
      return "IE_INVALID";
    case WIFI_REASON_4WAY_HANDSHAKE_TIMEOUT:
      return "4WAY_HANDSHAKE_TIMEOUT";
    case WIFI_REASON_GROUP_KEY_UPDATE_TIMEOUT:
      return "GROUP_KEY_UPDATE_TIMEOUT";
    case WIFI_REASON_IE_IN_4WAY_DIFFERS:
      return "IE_IN_4WAY_DIFFERS";
    case WIFI_REASON_BEACON_TIMEOUT:
      return "BEACON_TIMEOUT";
    case WIFI_REASON_NO_AP_FOUND:
      return "NO_AP_FOUND";
    case WIFI_REASON_AUTH_FAIL:
      return "AUTH_FAIL";
    case WIFI_REASON_ASSOC_FAIL:
      return "ASSOC_FAIL";
    case WIFI_REASON_HANDSHAKE_TIMEOUT:
      return "HANDSHAKE_TIMEOUT";
    case WIFI_REASON_CONNECTION_FAIL:
      return "CONNECTION_FAIL";
    case WIFI_REASON_NO_AP_FOUND_W_COMPATIBLE_SECURITY:
      return "NO_AP_FOUND_W_COMPATIBLE_SECURITY";
    case WIFI_REASON_NO_AP_FOUND_IN_AUTHMODE_THRESHOLD:
      return "NO_AP_FOUND_IN_AUTHMODE_THRESHOLD";
    case WIFI_REASON_NO_AP_FOUND_IN_RSSI_THRESHOLD:
      return "NO_AP_FOUND_IN_RSSI_THRESHOLD";
    default:
      return "UNKNOWN";
  }
}

const char* WifiReasonHint(wifi_err_reason_t reason) {
  // 给出可执行诊断提示，便于现场快速定位是密码、信号还是路由器兼容性问题。
  switch (reason) {
    case WIFI_REASON_AUTH_FAIL:
    case WIFI_REASON_HANDSHAKE_TIMEOUT:
    case WIFI_REASON_4WAY_HANDSHAKE_TIMEOUT:
      return "check password and WPA mode";
    case WIFI_REASON_NO_AP_FOUND:
    case WIFI_REASON_NO_AP_FOUND_IN_AUTHMODE_THRESHOLD:
    case WIFI_REASON_NO_AP_FOUND_IN_RSSI_THRESHOLD:
      return "check SSID spelling and 2.4GHz coverage";
    case WIFI_REASON_NO_AP_FOUND_W_COMPATIBLE_SECURITY:
      return "router security incompatible, try WPA2-PSK";
    case WIFI_REASON_ASSOC_FAIL:
    case WIFI_REASON_CONNECTION_FAIL:
      return "router may reject STA, disable smart-connect/WPA3-only";
    case WIFI_REASON_BEACON_TIMEOUT:
      return "signal unstable, try closer AP/channel 1/6/11";
    default:
      return "check router settings then retry";
  }
}

void WifiEventHandler(void* arg, esp_event_base_t event_base, int32_t event_id, void* event_data) {
  if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_START) {
    esp_wifi_connect();
    return;
  }

  if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_DISCONNECTED) {
    wifi_err_reason_t reason = WIFI_REASON_UNSPECIFIED;
    if (event_data != nullptr) {
      auto* disconn = static_cast<wifi_event_sta_disconnected_t*>(event_data);
      reason = static_cast<wifi_err_reason_t>(disconn->reason);
    }
    g_last_disconnect_reason = static_cast<int>(reason);
    ESP_LOGW(kTag, "wifi disconnected, reason=%d(%s), hint=%s", g_last_disconnect_reason,
             WifiReasonToString(reason), WifiReasonHint(reason));

    if (g_wifi_retry < g_wifi_retry_limit) {
      ++g_wifi_retry;
      esp_wifi_connect();
      ESP_LOGW(kTag, "wifi reconnect retry %d/%d", g_wifi_retry, g_wifi_retry_limit);
    } else {
      ESP_LOGW(kTag, "wifi reconnect exhausted for this cycle");
      xEventGroupSetBits(g_wifi_events, kWifiFailBit);
    }
    return;
  }

  if (event_base == IP_EVENT && event_id == IP_EVENT_STA_GOT_IP) {
    auto* got_ip = static_cast<ip_event_got_ip_t*>(event_data);
    ESP_LOGI(kTag, "got ip: " IPSTR, IP2STR(&got_ip->ip_info.ip));
    g_wifi_retry = 0;
    g_last_disconnect_reason = 0;
    xEventGroupSetBits(g_wifi_events, kWifiConnectedBit);
  }
}

bool EnsureWifiStack() {
  if (g_wifi_ready) {
    return true;
  }

  ESP_ERROR_CHECK(esp_netif_init());
  ESP_ERROR_CHECK(esp_event_loop_create_default());
  g_sta_netif = esp_netif_create_default_wifi_sta();
  g_ap_netif = esp_netif_create_default_wifi_ap();
  if (g_sta_netif == nullptr || g_ap_netif == nullptr) {
    ESP_LOGE(kTag, "failed to create default netif");
    return false;
  }

  wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
  ESP_ERROR_CHECK(esp_wifi_init(&cfg));
  ESP_ERROR_CHECK(esp_wifi_set_storage(WIFI_STORAGE_RAM));

  g_wifi_events = xEventGroupCreate();
  if (g_wifi_events == nullptr) {
    ESP_LOGE(kTag, "failed to create wifi event group");
    return false;
  }

  ESP_ERROR_CHECK(esp_event_handler_instance_register(
      WIFI_EVENT, ESP_EVENT_ANY_ID, &WifiEventHandler, nullptr, nullptr));
  ESP_ERROR_CHECK(esp_event_handler_instance_register(
      IP_EVENT, IP_EVENT_STA_GOT_IP, &WifiEventHandler, nullptr, nullptr));

  g_wifi_ready = true;
  return true;
}

void ConfigureStaHostname(const AppConfig& config) {
  if (g_sta_netif == nullptr) {
    return;
  }
  if (config.device_id.empty()) {
    return;
  }
  const esp_err_t err = esp_netif_set_hostname(g_sta_netif, config.device_id.c_str());
  if (err != ESP_OK) {
    ESP_LOGW(kTag, "set sta hostname failed: %s", esp_err_to_name(err));
  } else {
    ESP_LOGI(kTag, "sta hostname=%s", config.device_id.c_str());
  }
}

bool ConfigureApNetwork() {
  if (g_ap_netif == nullptr) {
    return false;
  }

  // 默认 AP 可能尚未启动 DHCP，允许 already stopped 状态继续设置固定网段。
  esp_err_t err = esp_netif_dhcps_stop(g_ap_netif);
  if (err != ESP_OK && err != ESP_ERR_ESP_NETIF_DHCP_ALREADY_STOPPED) {
    ESP_LOGE(kTag, "stop dhcps failed: %s", esp_err_to_name(err));
    return false;
  }

  esp_netif_ip_info_t ip_info = {};
  IP4_ADDR(&ip_info.ip, kApIpA, kApIpB, kApIpC, kApIpD);
  IP4_ADDR(&ip_info.gw, kApIpA, kApIpB, kApIpC, kApIpD);
  IP4_ADDR(&ip_info.netmask, 255, 255, 255, 0);

  err = esp_netif_set_ip_info(g_ap_netif, &ip_info);
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "set ap ip failed: %s", esp_err_to_name(err));
    return false;
  }

  err = esp_netif_dhcps_start(g_ap_netif);
  if (err != ESP_OK && err != ESP_ERR_ESP_NETIF_DHCP_ALREADY_STARTED) {
    ESP_LOGE(kTag, "start dhcps failed: %s", esp_err_to_name(err));
    return false;
  }
  return true;
}

bool StartConfigApMode() {
  if (!ConfigureApNetwork()) {
    ESP_LOGE(kTag, "ap network config failed");
    return false;
  }

  wifi_config_t ap_cfg = {};
  strncpy(reinterpret_cast<char*>(ap_cfg.ap.ssid), kApSsid, sizeof(ap_cfg.ap.ssid));
  strncpy(reinterpret_cast<char*>(ap_cfg.ap.password), kApPassword, sizeof(ap_cfg.ap.password));
  ap_cfg.ap.ssid_len = strlen(kApSsid);
  ap_cfg.ap.channel = 1;
  ap_cfg.ap.max_connection = 4;
  ap_cfg.ap.authmode = WIFI_AUTH_WPA2_PSK;
  ap_cfg.ap.pmf_cfg.required = false;

  if (strlen(kApPassword) < 8) {
    ap_cfg.ap.authmode = WIFI_AUTH_OPEN;
  }

  ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_APSTA));
  ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_AP, &ap_cfg));
  ESP_ERROR_CHECK(esp_wifi_start());

  ESP_LOGI(kTag, "config AP started, ssid=%s ip=%u.%u.%u.%u", kApSsid, kApIpA, kApIpB, kApIpC,
           kApIpD);
  return true;
}

bool ConnectToStaOnce(const std::string& ssid, const std::string& password, RuntimeStatus* status) {
  if (ssid.empty()) {
    status->last_error = "wifi ssid is empty";
    return false;
  }

  wifi_config_t sta_cfg = {};
  strncpy(reinterpret_cast<char*>(sta_cfg.sta.ssid), ssid.c_str(), sizeof(sta_cfg.sta.ssid));
  strncpy(reinterpret_cast<char*>(sta_cfg.sta.password), password.c_str(),
          sizeof(sta_cfg.sta.password));
  sta_cfg.sta.scan_method = WIFI_ALL_CHANNEL_SCAN;
  sta_cfg.sta.sort_method = WIFI_CONNECT_AP_BY_SIGNAL;
  sta_cfg.sta.threshold.authmode = WIFI_AUTH_OPEN;
  sta_cfg.sta.pmf_cfg.capable = true;
  sta_cfg.sta.pmf_cfg.required = false;

  ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
  ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &sta_cfg));

  ESP_LOGI(kTag, "wifi connect start: ssid=%s password_len=%u", ssid.c_str(),
           static_cast<unsigned>(password.size()));
  if (password.empty()) {
    ESP_LOGW(kTag, "wifi password is empty, secured AP may reject with reason=210");
  }

  g_wifi_retry = 0;
  g_wifi_retry_limit = kStaConnectRetry;
  g_last_disconnect_reason = 0;
  xEventGroupClearBits(g_wifi_events, kWifiConnectedBit | kWifiFailBit);

  ESP_ERROR_CHECK(esp_wifi_start());
  ESP_ERROR_CHECK(esp_wifi_connect());

  const EventBits_t bits = xEventGroupWaitBits(g_wifi_events, kWifiConnectedBit | kWifiFailBit,
                                                pdTRUE, pdFALSE,
                                                pdMS_TO_TICKS(kStaConnectTimeoutSec * 1000));
  if (bits & kWifiConnectedBit) {
    status->wifi_connected = true;
    return true;
  }

  status->wifi_connected = false;
  status->last_error = "wifi connect timeout/fail, ssid=" + ssid;
  if (g_last_disconnect_reason > 0) {
    const wifi_err_reason_t reason = static_cast<wifi_err_reason_t>(g_last_disconnect_reason);
    status->last_error += ", reason=" + std::to_string(g_last_disconnect_reason) + "(" +
                         WifiReasonToString(reason) + ")";
    ESP_LOGW(kTag, "wifi connect failed, ssid=%s last reason=%d(%s), hint=%s", ssid.c_str(),
             g_last_disconnect_reason, WifiReasonToString(reason), WifiReasonHint(reason));
  }
  return false;
}

void EnsurePrimaryWifiInProfiles(AppConfig* cfg) {
  if (cfg == nullptr || cfg->wifi_ssid.empty()) {
    return;
  }

  for (int i = 0; i < cfg->wifi_profile_count; ++i) {
    if (cfg->wifi_profiles[i].ssid == cfg->wifi_ssid) {
      if (!cfg->wifi_password.empty()) {
        cfg->wifi_profiles[i].password = cfg->wifi_password;
      }
      return;
    }
  }

  if (cfg->wifi_profile_count < AppConfig::kMaxWifiProfiles) {
    cfg->wifi_profiles[cfg->wifi_profile_count].ssid = cfg->wifi_ssid;
    cfg->wifi_profiles[cfg->wifi_profile_count].password = cfg->wifi_password;
    cfg->wifi_profile_count += 1;
    return;
  }

  // 超出容量时淘汰最旧配置，保证最近配置过的 Wi-Fi 可被保留。
  for (int i = 1; i < AppConfig::kMaxWifiProfiles; ++i) {
    cfg->wifi_profiles[i - 1] = cfg->wifi_profiles[i];
  }
  cfg->wifi_profiles[AppConfig::kMaxWifiProfiles - 1].ssid = cfg->wifi_ssid;
  cfg->wifi_profiles[AppConfig::kMaxWifiProfiles - 1].password = cfg->wifi_password;
  if (cfg->last_connected_wifi_index > 0) {
    cfg->last_connected_wifi_index -= 1;
  }
}

void PersistConnectedProfile(AppConfig* cfg, int profile_index, ConfigStore* store) {
  if (cfg == nullptr || profile_index < 0 || profile_index >= cfg->wifi_profile_count) {
    return;
  }

  cfg->wifi_ssid = cfg->wifi_profiles[profile_index].ssid;
  cfg->wifi_password = cfg->wifi_profiles[profile_index].password;
  cfg->last_connected_wifi_index = profile_index;

  if (store != nullptr) {
    (void)store->Save(*cfg);
  }
}

bool ConnectToSta(AppConfig* cfg, RuntimeStatus* status, ConfigStore* store) {
  if (cfg == nullptr || status == nullptr) {
    return false;
  }

  EnsurePrimaryWifiInProfiles(cfg);
  if (cfg->wifi_profile_count <= 0) {
    status->last_error = "no wifi profile configured";
    return false;
  }

  int candidate_indexes[AppConfig::kMaxWifiProfiles] = {};
  int candidate_count = 0;
  auto push_candidate = [&](int idx) {
    if (idx < 0 || idx >= cfg->wifi_profile_count) {
      return;
    }
    if (cfg->wifi_profiles[idx].ssid.empty()) {
      return;
    }
    for (int i = 0; i < candidate_count; ++i) {
      if (candidate_indexes[i] == idx) {
        return;
      }
    }
    candidate_indexes[candidate_count++] = idx;
  };

  push_candidate(cfg->last_connected_wifi_index);
  for (int i = 0; i < cfg->wifi_profile_count; ++i) {
    push_candidate(i);
  }

  for (int i = 0; i < candidate_count; ++i) {
    const int profile_index = candidate_indexes[i];
    const auto& profile = cfg->wifi_profiles[profile_index];
    ESP_LOGI(kTag, "wifi profile try %d/%d idx=%d ssid=%s", i + 1, candidate_count, profile_index,
             profile.ssid.c_str());
    if (ConnectToStaOnce(profile.ssid, profile.password, status)) {
      ESP_LOGI(kTag, "wifi connected with profile idx=%d ssid=%s", profile_index,
               profile.ssid.c_str());
      PersistConnectedProfile(cfg, profile_index, store);
      return true;
    }
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_wifi_stop());
  }

  status->wifi_connected = false;
  if (status->last_error.empty()) {
    status->last_error = "wifi connect failed for all profiles";
  }
  return false;
}

bool SplitUrlOriginAndRest(const std::string& url, std::string* origin, std::string* rest) {
  const size_t scheme_pos = url.find("://");
  if (scheme_pos == std::string::npos) {
    return false;
  }
  const size_t host_start = scheme_pos + 3;
  const size_t path_pos = url.find('/', host_start);
  if (path_pos == std::string::npos) {
    if (origin != nullptr) {
      *origin = url;
    }
    if (rest != nullptr) {
      *rest = "/";
    }
    return true;
  }
  if (origin != nullptr) {
    *origin = url.substr(0, path_pos);
  }
  if (rest != nullptr) {
    *rest = url.substr(path_pos);
  }
  return true;
}

std::string NormalizeOrigin(const std::string& origin) {
  std::string out = origin;
  while (out.size() > 1 && !out.empty() && out.back() == '/') {
    out.pop_back();
  }
  return out;
}

bool BuildUrlWithOrigin(const std::string& url, const std::string& origin, std::string* out_url) {
  if (out_url == nullptr || origin.empty()) {
    return false;
  }

  std::string url_origin;
  std::string url_rest;
  if (!SplitUrlOriginAndRest(url, &url_origin, &url_rest)) {
    return false;
  }

  const std::string normalized = NormalizeOrigin(origin);
  std::string origin_part;
  std::string origin_rest;
  if (!SplitUrlOriginAndRest(normalized, &origin_part, &origin_rest) || origin_rest != "/") {
    return false;
  }

  *out_url = origin_part + url_rest;
  return true;
}

bool ShiftDateParamDays(const std::string& url, int delta_days, std::string* shifted_url) {
  if (shifted_url == nullptr || delta_days == 0) {
    return false;
  }

  const std::string marker = "date=";
  const size_t marker_pos = url.find(marker);
  if (marker_pos == std::string::npos) {
    return false;
  }

  const size_t date_pos = marker_pos + marker.size();
  if (date_pos + 10 > url.size()) {
    return false;
  }

  const std::string date_text = url.substr(date_pos, 10);
  int year = 0;
  int month = 0;
  int day = 0;
  if (sscanf(date_text.c_str(), "%4d-%2d-%2d", &year, &month, &day) != 3) {
    return false;
  }

  std::tm tm_date = {};
  tm_date.tm_year = year - 1900;
  tm_date.tm_mon = month - 1;
  tm_date.tm_mday = day + delta_days;
  tm_date.tm_hour = 12;
  const time_t normalized = mktime(&tm_date);
  if (normalized == static_cast<time_t>(-1)) {
    return false;
  }

  std::tm shifted_tm = {};
  localtime_r(&normalized, &shifted_tm);
  char shifted_date[16] = {};
  if (strftime(shifted_date, sizeof(shifted_date), "%Y-%m-%d", &shifted_tm) != 10) {
    return false;
  }

  *shifted_url = url;
  shifted_url->replace(date_pos, 10, shifted_date);
  return true;
}

void AddUniqueUrl(const std::string& url, std::vector<std::string>* urls) {
  if (urls == nullptr || url.empty()) {
    return;
  }
  for (const auto& item : *urls) {
    if (item == url) {
      return;
    }
  }
  urls->push_back(url);
}

std::vector<std::string> BuildFetchUrlCandidates(const std::string& primary_url,
                                                 const std::string& preferred_origin) {
  std::vector<std::string> base_urls;
  if (!preferred_origin.empty()) {
    std::string preferred_url;
    if (BuildUrlWithOrigin(primary_url, preferred_origin, &preferred_url)) {
      AddUniqueUrl(preferred_url, &base_urls);
    }
  }
  AddUniqueUrl(primary_url, &base_urls);

  std::vector<std::string> candidates;
  for (const auto& url : base_urls) {
    AddUniqueUrl(url, &candidates);
  }
  for (const auto& url : base_urls) {
    std::string fallback_url;
    if (ShiftDateParamDays(url, -1, &fallback_url)) {
      AddUniqueUrl(fallback_url, &candidates);
    }
  }
  return candidates;
}

bool ExtractUrlOrigin(const std::string& url, std::string* origin) {
  if (origin == nullptr || url.empty()) {
    return false;
  }
  return SplitUrlOriginAndRest(url, origin, nullptr);
}

std::vector<std::string> BuildCheckinBaseUrlCandidates(const AppConfig& cfg,
                                                       const std::string& fetch_url_used,
                                                       const std::string& fallback_url) {
  std::vector<std::string> candidates;
  AddUniqueUrl(NormalizeOrigin(cfg.orchestrator_base_url), &candidates);

  std::string origin;
  if (ExtractUrlOrigin(fetch_url_used, &origin)) {
    AddUniqueUrl(origin, &candidates);
  }
  if (ExtractUrlOrigin(cfg.preferred_image_origin, &origin)) {
    AddUniqueUrl(origin, &candidates);
  }
  if (ExtractUrlOrigin(fallback_url, &origin)) {
    AddUniqueUrl(origin, &candidates);
  }
  if (ExtractUrlOrigin(cfg.image_url_template, &origin)) {
    AddUniqueUrl(origin, &candidates);
  }
  return candidates;
}

bool ReportCheckinWithFallback(const AppConfig& cfg, const DeviceCheckinPayload& payload,
                               const std::string& fetch_url_used,
                               const std::string& fallback_url) {
  if (cfg.orchestrator_enabled == 0 || cfg.orchestrator_base_url.empty() || cfg.device_id.empty()) {
    return false;
  }

  const std::vector<std::string> candidates =
      BuildCheckinBaseUrlCandidates(cfg, fetch_url_used, fallback_url);
  for (size_t i = 0; i < candidates.size(); ++i) {
    AppConfig attempt_cfg = cfg;
    attempt_cfg.orchestrator_base_url = candidates[i];
    if (OrchestratorClient::ReportCheckin(attempt_cfg, payload)) {
      if (i > 0) {
        ESP_LOGW(kTag, "checkin switched base url to %s", candidates[i].c_str());
      }
      return true;
    }
    ESP_LOGW(kTag, "checkin failed via base url candidate %u/%u: %s",
             static_cast<unsigned>(i + 1), static_cast<unsigned>(candidates.size()),
             candidates[i].c_str());
  }
  return false;
}

bool IsKeyButtonPressed() {
  return gpio_get_level(kKeyButton) == 0;
}

bool IsBootButtonPressed() {
  return gpio_get_level(kBootButton) == 0;
}

bool IsUsbSerialConnected() {
#if CONFIG_USJ_ENABLE_USB_SERIAL_JTAG
  return usb_serial_jtag_is_connected();
#else
  return false;
#endif
}

enum class LongPressAction {
  kNone,
  // 非破坏性：仅打开 STA 配置窗口（不清 Wi-Fi）。
  kOpenStaPortalWindow,
  // 破坏性：清 Wi-Fi 并进入 AP 配网（逃生口）。
  kClearWifiAndEnterPortal,
};

LongPressAction DetectLongPressAction() {
  if (!IsKeyButtonPressed() && !IsBootButtonPressed()) {
    return LongPressAction::kNone;
  }

  ESP_LOGI(kTag, "button pressed at boot, waiting for long-press...");

  constexpr int64_t kLongPressMs = 3000;
  const int64_t deadline_us = esp_timer_get_time() + kLongPressMs * 1000LL;
  while (esp_timer_get_time() < deadline_us) {
    // 短按会很快松开：尽早返回，避免人为手动同步被额外阻塞 3 秒。
    if (!IsKeyButtonPressed() && !IsBootButtonPressed()) {
      return LongPressAction::kNone;
    }
    vTaskDelay(pdMS_TO_TICKS(20));
  }

  // 长按阈值到达后再决定动作：优先 BOOT（作为“清 Wi-Fi 逃生口”）。
  if (IsBootButtonPressed()) {
    return LongPressAction::kClearWifiAndEnterPortal;
  }
  if (IsKeyButtonPressed()) {
    return LongPressAction::kOpenStaPortalWindow;
  }
  return LongPressAction::kNone;
}

uint64_t SecondsToMicroseconds(uint64_t seconds) {
  // 避免 32-bit 乘法导致溢出，进而出现“睡眠时长异常/频繁唤醒”的隐蔽耗电问题。
  return seconds * 1000000ULL;
}

void HoldInsteadOfDeepSleepWhileUsbConnected(RuntimeStatus* status, uint64_t planned_sleep_seconds,
                                             const char* sleep_kind) {
  if (status == nullptr) {
    return;
  }

  // 刷新一次 vbus_good：用户常用“USB 供电”作为调试环境（即便未打开串口监控，也希望设备不深睡）。
  RefreshPowerStatus(status);

  const bool usb_serial_connected = IsUsbSerialConnected();
  const bool usb_power_present = (status->vbus_good == 1);
  if (!usb_serial_connected && !usb_power_present) {
    return;
  }

  ESP_LOGW(kTag,
           "usb present (serial=%d vbus=%d), skip %s deep sleep (planned %llus); keep awake for observation",
           usb_serial_connected ? 1 : 0, usb_power_present ? 1 : 0,
           sleep_kind == nullptr ? "unknown" : sleep_kind,
           static_cast<unsigned long long>(planned_sleep_seconds));

  int64_t last_log_us = 0;
  while (true) {
    // USB 供电或串口连接任意一个成立，就保持唤醒。
    // vbus_good 只会在 RefreshPowerStatus() 后更新，因此这里允许最多 10 秒的滞后。
    const bool still_serial = IsUsbSerialConnected();
    const bool still_vbus = (status->vbus_good == 1);
    if (!still_serial && !still_vbus) {
      break;
    }

    const int64_t now_us = esp_timer_get_time();
    constexpr int64_t kLogEveryUs = 10LL * 1000000LL;
    if (last_log_us == 0 || now_us - last_log_us >= kLogEveryUs) {
      RefreshPowerStatus(status);
      ESP_LOGI(kTag, "usb hold: batt=%d%%/%dmV charging=%d vbus=%d next_sleep=%llus",
               status->battery_percent, status->battery_mv, status->charging, status->vbus_good,
               static_cast<unsigned long long>(planned_sleep_seconds));
      last_log_us = now_us;
    }
    vTaskDelay(pdMS_TO_TICKS(100));
  }

  ESP_LOGW(kTag, "usb no longer present, resume deep sleep");
}

void EnterDeepSleep(uint64_t seconds) {
  ESP_LOGI(kTag, "enter deep sleep for %llu seconds", static_cast<unsigned long long>(seconds));
  PowerManager::PrepareForDeepSleep();
  ESP_ERROR_CHECK(esp_sleep_disable_wakeup_source(ESP_SLEEP_WAKEUP_ALL));
  ESP_ERROR_CHECK(esp_sleep_enable_timer_wakeup(SecondsToMicroseconds(seconds)));
  const uint64_t wakeup_pins =
      (1ULL << static_cast<int>(kKeyButton)) | (1ULL << static_cast<int>(kBootButton));

  // EXT1 需要 RTC 外设域保持供电，否则 RTC 上拉也可能失效。
  ESP_ERROR_CHECK_WITHOUT_ABORT(
      esp_sleep_pd_config(ESP_PD_DOMAIN_RTC_PERIPH, ESP_PD_OPTION_ON));

#if ESP_IDF_VERSION >= ESP_IDF_VERSION_VAL(5, 0, 0)
  ESP_ERROR_CHECK(esp_sleep_enable_ext1_wakeup_io(wakeup_pins, ESP_EXT1_WAKEUP_ANY_LOW));
#else
  ESP_ERROR_CHECK(esp_sleep_enable_ext1_wakeup(wakeup_pins, ESP_EXT1_WAKEUP_ANY_LOW));
#endif

  // 深睡阶段 GPIO 的数字域配置会丢失，EXT1 唤醒依赖 RTC 域。
  // 若不显式配置 RTC 上拉，按键脚可能浮空导致 ANY_LOW 误唤醒，形成“每几分钟醒一次”的耗电灾难。
  ESP_ERROR_CHECK_WITHOUT_ABORT(rtc_gpio_pulldown_dis(kKeyButton));
  ESP_ERROR_CHECK_WITHOUT_ABORT(rtc_gpio_pullup_en(kKeyButton));
  ESP_ERROR_CHECK_WITHOUT_ABORT(rtc_gpio_pulldown_dis(kBootButton));
  ESP_ERROR_CHECK_WITHOUT_ABORT(rtc_gpio_pullup_en(kBootButton));

  vTaskDelay(pdMS_TO_TICKS(150));
  esp_deep_sleep_start();
}

void EnterDeepSleepTimerOnly(uint64_t seconds) {
  ESP_LOGW(kTag, "enter timer-only deep sleep for %llu seconds",
           static_cast<unsigned long long>(seconds));
  PowerManager::PrepareForDeepSleep();
  ESP_ERROR_CHECK(esp_sleep_disable_wakeup_source(ESP_SLEEP_WAKEUP_ALL));
  ESP_ERROR_CHECK(esp_sleep_enable_timer_wakeup(SecondsToMicroseconds(seconds)));
  vTaskDelay(pdMS_TO_TICKS(150));
  esp_deep_sleep_start();
}

void SleepNow(RuntimeStatus* status, uint64_t seconds, bool timer_only) {
  HoldInsteadOfDeepSleepWhileUsbConnected(status, seconds, timer_only ? "timer-only" : "normal");
  if (timer_only) {
    EnterDeepSleepTimerOnly(seconds);
  } else {
    EnterDeepSleep(seconds);
  }
}

enum class WakeSource {
  TIMER,
  KEY,
  BOOT,
  SPURIOUS_EXT1,
  OTHER,
};

WakeSource GetWakeSource() {
  const esp_sleep_wakeup_cause_t cause = esp_sleep_get_wakeup_cause();
  if (cause == ESP_SLEEP_WAKEUP_TIMER) {
    ESP_LOGI(kTag, "wakeup cause=TIMER");
    return WakeSource::TIMER;
  }
  if (cause == ESP_SLEEP_WAKEUP_EXT1) {
    const uint64_t pins = esp_sleep_get_ext1_wakeup_status();
    const bool boot_pin = (pins & (1ULL << static_cast<int>(kBootButton))) != 0;
    const bool key_pin = (pins & (1ULL << static_cast<int>(kKeyButton))) != 0;

    // 省电与易用性的折中：
    // - 误唤醒（浮空/抖动）通常是“瞬态低电平”，唤醒后按钮已回到松开状态。
    // - 人为短按可能在启动过程中释放；若只读一次电平，容易被误判成 SPURIOUS，导致“按了不生效”。
    // 这里做一个很短的采样窗口：在几十毫秒内只要捕获到一次按下，就视为人为唤醒。
    bool boot_seen_low = false;
    bool key_seen_low = false;
    int boot_level = gpio_get_level(kBootButton);
    int key_level = gpio_get_level(kKeyButton);
    if (boot_pin && boot_level == 0) {
      boot_seen_low = true;
    }
    if (key_pin && key_level == 0) {
      key_seen_low = true;
    }
    if (!boot_seen_low && !key_seen_low && (boot_pin || key_pin)) {
      for (int i = 0; i < 8; ++i) {
        vTaskDelay(pdMS_TO_TICKS(10));
        boot_level = gpio_get_level(kBootButton);
        key_level = gpio_get_level(kKeyButton);
        if (boot_pin && boot_level == 0) {
          boot_seen_low = true;
        }
        if (key_pin && key_level == 0) {
          key_seen_low = true;
        }
        if (boot_seen_low || key_seen_low) {
          break;
        }
      }
    }

    ESP_LOGI(kTag,
             "wakeup cause=EXT1 pins=0x%llx key=%d boot=%d seen_low(key=%d boot=%d)",
             static_cast<unsigned long long>(pins), key_level, boot_level, key_seen_low ? 1 : 0,
             boot_seen_low ? 1 : 0);

    if (boot_pin && boot_seen_low) {
      return WakeSource::BOOT;
    }
    if (key_pin && key_seen_low) {
      return WakeSource::KEY;
    }
    ESP_LOGW(kTag, "ext1 wake but buttons not observed pressed, treat as SPURIOUS_EXT1");
    return WakeSource::SPURIOUS_EXT1;
  }
  ESP_LOGI(kTag, "wakeup cause=OTHER(%d)", static_cast<int>(cause));
  return WakeSource::OTHER;
}

void ApplyTimezone(const std::string& timezone) {
  if (!timezone.empty()) {
    setenv("TZ", timezone.c_str(), 1);
    tzset();
  }
}

bool ShouldSyncTime(const AppConfig& config, int64_t now_epoch) {
  constexpr int64_t kMinValidEpoch = 1735689600;  // 2025-01-01 UTC
  // RTC 时间明显不可信时强制校时。
  if (now_epoch < kMinValidEpoch) {
    return true;
  }
  // 首次或历史记录异常时也触发一次校时，避免长期漂移。
  if (config.last_time_sync_epoch < kMinValidEpoch) {
    return true;
  }

  // 正常情况下每天校一次即可，避免每轮都跑 SNTP 增加唤醒时长与耗电。
  constexpr int64_t kSyncIntervalSec = 24 * 3600;
  const int64_t age = now_epoch - config.last_time_sync_epoch;
  return age < 0 || age >= kSyncIntervalSec;
}

bool SyncTime(const std::string& timezone) {
  ApplyTimezone(timezone);

  esp_sntp_setoperatingmode(SNTP_OPMODE_POLL);
  esp_sntp_setservername(0, "pool.ntp.org");
  esp_sntp_setservername(1, "time.cloudflare.com");
  esp_sntp_init();

  for (int i = 0; i < 20; ++i) {
    time_t now = time(nullptr);
    if (now > 1735689600) {  // 2025-01-01 UTC
      std::tm tm_local = {};
      std::tm tm_utc = {};
      localtime_r(&now, &tm_local);
      gmtime_r(&now, &tm_utc);

      char local_buf[64] = {};
      char utc_buf[64] = {};
      strftime(local_buf, sizeof(local_buf), "%Y-%m-%d %H:%M:%S %Z", &tm_local);
      strftime(utc_buf, sizeof(utc_buf), "%Y-%m-%d %H:%M:%S UTC", &tm_utc);

      ESP_LOGI(kTag, "time synced, epoch=%lld local=%s utc=%s", static_cast<long long>(now),
               local_buf, utc_buf);
      return true;
    }
    vTaskDelay(pdMS_TO_TICKS(500));
  }

  ESP_LOGW(kTag, "time sync timeout, continue with current rtc time");
  return false;
}

uint64_t CalcBackoffSeconds(AppConfig* cfg) {
  cfg->failure_count = std::max(1, cfg->failure_count);
  const int exp = std::min(cfg->failure_count - 1, 10);
  const int factor = 1 << exp;
  int minutes = cfg->retry_base_minutes * factor;
  minutes = std::min(minutes, cfg->retry_max_minutes);

  if (cfg->failure_count >= cfg->max_failure_before_long_sleep) {
    minutes = std::max(minutes, cfg->retry_max_minutes);
  }

  return static_cast<uint64_t>(std::max(1, minutes)) * 60ULL;
}

int EstimateBatteryPercentFromMv(int battery_mv) {
  // 单节锂电常见静置电压曲线（简化），用于 PMIC 百分比异常时的兜底展示。
  struct CurvePoint {
    int mv;
    int percent;
  };
  static constexpr CurvePoint kCurve[] = {
      {4200, 100}, {4160, 95}, {4120, 88}, {4080, 80}, {4040, 72}, {4000, 64},
      {3960, 56},  {3920, 48}, {3880, 40}, {3840, 32}, {3800, 24}, {3760, 16},
      {3720, 10},  {3680, 6},  {3600, 3},  {3500, 1},  {3300, 0},
  };

  if (battery_mv <= kCurve[sizeof(kCurve) / sizeof(kCurve[0]) - 1].mv) {
    return 0;
  }
  if (battery_mv >= kCurve[0].mv) {
    return 100;
  }

  for (size_t i = 0; i + 1 < sizeof(kCurve) / sizeof(kCurve[0]); ++i) {
    const CurvePoint high = kCurve[i];
    const CurvePoint low = kCurve[i + 1];
    if (battery_mv <= high.mv && battery_mv >= low.mv) {
      const int span_mv = std::max(1, high.mv - low.mv);
      const int offset_mv = battery_mv - low.mv;
      const int span_percent = high.percent - low.percent;
      const int percent = low.percent + (offset_mv * span_percent) / span_mv;
      return std::clamp(percent, 0, 100);
    }
  }

  return -1;
}

void ConfigureButtonGpio() {
  gpio_config_t cfg = {};
  cfg.pin_bit_mask =
      (1ULL << static_cast<int>(kKeyButton)) | (1ULL << static_cast<int>(kBootButton));
  cfg.mode = GPIO_MODE_INPUT;
  cfg.pull_up_en = GPIO_PULLUP_ENABLE;
  cfg.pull_down_en = GPIO_PULLDOWN_DISABLE;
  cfg.intr_type = GPIO_INTR_DISABLE;
  ESP_ERROR_CHECK(gpio_config(&cfg));
}

std::string StaIpString() {
  if (g_sta_netif == nullptr) {
    return "";
  }

  esp_netif_ip_info_t ip_info = {};
  if (esp_netif_get_ip_info(g_sta_netif, &ip_info) != ESP_OK) {
    return "";
  }

  char ip_buf[16] = {};
  snprintf(ip_buf, sizeof(ip_buf), IPSTR, IP2STR(&ip_info.ip));
  return ip_buf;
}

void RefreshPowerStatus(RuntimeStatus* status) {
  if (status == nullptr) {
    return;
  }

  bool pmic_ok = false;
  // 经验：刚上电时 PMIC/I2C 可能短暂不稳定，做少量重试避免整轮电量缺失。
  for (int attempt = 1; attempt <= 3; ++attempt) {
    if (PowerManager::Init()) {
      pmic_ok = true;
      break;
    }
    if (attempt < 3) {
      vTaskDelay(pdMS_TO_TICKS(60));
    }
  }
  if (!pmic_ok) {
    ESP_LOGW(kTag, "pmic init failed, skip battery status");
    if ((g_cached_battery_mv > 0) || (g_cached_battery_percent >= 0) ||
        (g_cached_charging == 0 || g_cached_charging == 1) ||
        (g_cached_vbus_good == 0 || g_cached_vbus_good == 1)) {
      status->battery_mv = g_cached_battery_mv;
      status->battery_percent = g_cached_battery_percent;
      status->charging = g_cached_charging;
      status->vbus_good = g_cached_vbus_good;
      ESP_LOGW(kTag, "use cached power: batt=%d%%/%dmV charging=%d vbus=%d cached_epoch=%lld",
               status->battery_percent, status->battery_mv, status->charging, status->vbus_good,
               static_cast<long long>(g_cached_power_epoch));
    }
    return;
  }

  PowerStatus power = {};
  bool ok = false;
  // PMIC 在刚上电/刚启用 ADC 通道的瞬间可能短暂读不到电量/电压；这里做多次采样兜底。
  // 注意：ReadStatus() 内部已做 I2C 级别重试，这里是“跨调用”的二次重试。
  for (int attempt = 1; attempt <= 3; ++attempt) {
    PowerStatus sample = {};
    if (!PowerManager::ReadStatus(&sample)) {
      if (attempt < 3) {
        vTaskDelay(pdMS_TO_TICKS(60));
      }
      continue;
    }
    power = sample;
    ok = true;
    if (power.battery_mv > 0 || power.battery_percent >= 0) {
      break;
    }
    if (attempt < 3) {
      vTaskDelay(pdMS_TO_TICKS(60));
    }
  }
  if (!ok) {
    ESP_LOGW(kTag, "pmic read failed, skip battery status");
    if ((g_cached_battery_mv > 0) || (g_cached_battery_percent >= 0) ||
        (g_cached_charging == 0 || g_cached_charging == 1) ||
        (g_cached_vbus_good == 0 || g_cached_vbus_good == 1)) {
      status->battery_mv = g_cached_battery_mv;
      status->battery_percent = g_cached_battery_percent;
      status->charging = g_cached_charging;
      status->vbus_good = g_cached_vbus_good;
      ESP_LOGW(kTag, "use cached power: batt=%d%%/%dmV charging=%d vbus=%d cached_epoch=%lld",
               status->battery_percent, status->battery_mv, status->charging, status->vbus_good,
               static_cast<long long>(g_cached_power_epoch));
    }
    return;
  }

  status->battery_mv = power.battery_mv;
  status->battery_percent = power.battery_percent;
  status->charging = power.charging ? 1 : 0;
  status->vbus_good = power.vbus_good ? 1 : 0;

  // AXP 百分比寄存器在部分板子上会长期卡 100（尤其是电池供电且非充电状态）。
  // 遇到“百分比异常偏满”时，用电压估算做展示兜底，避免后台长期误判电量健康。
  const int estimated_percent = EstimateBatteryPercentFromMv(status->battery_mv);
  const bool on_battery = (status->vbus_good == 0 && status->charging == 0);
  const bool suspect_percent_stuck_full = (status->battery_percent >= 100 && status->battery_mv > 0 &&
                                           status->battery_mv <= 4185);
  const bool missing_percent = (status->battery_percent < 0);
  if (on_battery && estimated_percent >= 0 && (suspect_percent_stuck_full || missing_percent)) {
    ESP_LOGW(kTag, "battery percent corrected by mv: raw=%d est=%d mv=%d",
             status->battery_percent, estimated_percent, status->battery_mv);
    status->battery_percent = estimated_percent;
  }

  if (status->battery_mv > 0) {
    g_cached_battery_mv = status->battery_mv;
  }
  if (status->battery_percent >= 0) {
    g_cached_battery_percent = status->battery_percent;
  }
  if (status->charging == 0 || status->charging == 1) {
    g_cached_charging = status->charging;
  }
  if (status->vbus_good == 0 || status->vbus_good == 1) {
    g_cached_vbus_good = status->vbus_good;
  }
  g_cached_power_epoch = static_cast<int64_t>(time(nullptr));

  ESP_LOGI(kTag,
           "power: vbus=%d charging=%d batt=%dmV percent=%d state=%s",
           status->vbus_good, status->charging, status->battery_mv, status->battery_percent,
           PowerManager::ChargerStateName(power.charger_state));
}

bool EnsurePmicReadyForRender() {
  // 渲染前先确保 PMIC 可用，避免面板无电时在 BUSY 等待里空耗几十秒。
  for (int attempt = 1; attempt <= kPmicInitMaxRetries; ++attempt) {
    if (PowerManager::Init()) {
      return true;
    }
    if (attempt < kPmicInitMaxRetries) {
      ESP_LOGW(kTag, "pmic init failed before render, retry %d/%d", attempt, kPmicInitMaxRetries);
      vTaskDelay(pdMS_TO_TICKS(kPmicInitRetryDelayMs));
    }
  }
  ESP_LOGE(kTag, "pmic unavailable before render, skip epd refresh");
  return false;
}

void RunPortalWindowOnSta(AppConfig* config, RuntimeStatus* status, ConfigStore* store) {
  PortalServer portal;
  if (!portal.Start(config, status, store, false)) {
    ESP_LOGW(kTag, "start sta portal failed, skip window");
    return;
  }

  const std::string ip = StaIpString();
  if (ip.empty()) {
    ESP_LOGI(kTag, "key wake portal opened for %d seconds", kKeyWakePortalWindowSec);
  } else {
    ESP_LOGI(kTag, "key wake portal opened for %d seconds: http://%s/", kKeyWakePortalWindowSec,
             ip.c_str());
  }

  const int64_t deadline_us = esp_timer_get_time() +
                              static_cast<int64_t>(kKeyWakePortalWindowSec) * 1000000LL;
  while (esp_timer_get_time() < deadline_us) {
    if (portal.ShouldReboot()) {
      ESP_LOGI(kTag, "portal config saved, rebooting now");
      portal.Stop();
      vTaskDelay(pdMS_TO_TICKS(300));
      esp_restart();
      return;
    }
    vTaskDelay(pdMS_TO_TICKS(kPortalLoopStepMs));
  }

  portal.Stop();
  ESP_LOGI(kTag, "key wake portal window expired");
}
}  // namespace

extern "C" void app_main(void) {
  ConfigStore store;
  AppConfig config;
  RuntimeStatus status;

  if (!store.Init() || !store.Load(&config)) {
    ESP_LOGE(kTag, "config store init/load failed");
    vTaskDelay(pdMS_TO_TICKS(2000));
    esp_restart();
    return;
  }

  bool identity_updated = false;
  if (config.device_id.empty()) {
    // 首次启动自动生成设备标识，便于 NAS 编排服务识别设备状态。
    const std::string generated_device_id = OrchestratorClient::EnsureDeviceId(&config);
    ESP_LOGI(kTag, "generated device_id=%s", generated_device_id.c_str());
    identity_updated = true;
  }
  if (config.orchestrator_token.empty()) {
    // 设备侧首次自动生成 token，后台审批后即可建立设备级鉴权。
    const std::string generated_token = OrchestratorClient::EnsureDeviceToken(&config);
    ESP_LOGI(kTag, "generated device token len=%u", static_cast<unsigned>(generated_token.size()));
    identity_updated = true;
  }
  if (identity_updated) {
    store.Save(config);
  }

  ConfigureButtonGpio();
  const WakeSource wake_source = GetWakeSource();
  const LongPressAction long_press_action = DetectLongPressAction();
  bool enter_ap_portal = false;
  bool open_sta_portal_window = false;
  bool skip_network_cycle = false;

  if (long_press_action == LongPressAction::kClearWifiAndEnterPortal) {
    ESP_LOGW(kTag, "long-press BOOT detected, clear wifi and enter portal");
    store.ClearWifi();
    config.wifi_ssid.clear();
    config.wifi_password.clear();
    enter_ap_portal = true;
  } else if (long_press_action == LongPressAction::kOpenStaPortalWindow) {
    open_sta_portal_window = true;
    ESP_LOGI(kTag, "long-press KEY detected, open sta portal window");
  }

  RefreshPowerStatus(&status);

  if (long_press_action == LongPressAction::kNone) {
    switch (wake_source) {
      case WakeSource::BOOT:
        status.force_refresh = true;
        ESP_LOGI(kTag, "wake source=BOOT, force refresh enabled");
        break;
      case WakeSource::KEY:
        status.force_refresh = false;
        // KEY=手动同步：不再打开 120 秒窗口，避免误触与固定成本放大耗电。
        ESP_LOGI(kTag, "wake source=KEY, manual sync triggered");
        break;
      case WakeSource::TIMER:
        status.force_refresh = false;
        ESP_LOGI(kTag, "wake source=TIMER");
        break;
      case WakeSource::SPURIOUS_EXT1:
        status.force_refresh = false;
        skip_network_cycle = true;
        ESP_LOGW(kTag, "wake source=SPURIOUS_EXT1, skip network cycle to avoid drain");
        break;
      default:
        status.force_refresh = false;
        ESP_LOGI(kTag, "wake source=OTHER");
        break;
    }
  } else {
    status.force_refresh = false;
  }

  if (skip_network_cycle) {
    const uint64_t normal_sleep_seconds =
        static_cast<uint64_t>(std::max(1, config.interval_minutes)) * 60ULL;
    const uint64_t sleep_seconds =
        std::max<uint64_t>(60ULL, std::min<uint64_t>(normal_sleep_seconds, kSpuriousExt1TimerOnlyMaxSec));
    SleepNow(&status, sleep_seconds, true);
    return;
  }

  if (!EnsureWifiStack()) {
    ESP_LOGE(kTag, "wifi stack init failed");
    vTaskDelay(pdMS_TO_TICKS(2000));
    esp_restart();
    return;
  }

  ConfigureStaHostname(config);

  if (config.wifi_ssid.empty() || enter_ap_portal) {
    if (!StartConfigApMode()) {
      ESP_LOGE(kTag, "start config ap failed");
      vTaskDelay(pdMS_TO_TICKS(2000));
      esp_restart();
      return;
    }
    PortalServer portal;
    if (!portal.Start(&config, &status, &store, true)) {
      ESP_LOGE(kTag, "portal start failed");
      vTaskDelay(pdMS_TO_TICKS(2000));
      esp_restart();
      return;
    }

    ESP_LOGI(kTag, "enter portal mode: connect Wi-Fi to %s, then open http://%u.%u.%u.%u/", kApSsid,
             kApIpA, kApIpB, kApIpC, kApIpD);
    while (true) {
      if (portal.ShouldReboot()) {
        ESP_LOGI(kTag, "config saved, rebooting...");
        vTaskDelay(pdMS_TO_TICKS(500));
        esp_restart();
      }
      vTaskDelay(pdMS_TO_TICKS(500));
    }
  }

  if (!ConnectToSta(&config, &status, &store)) {
    ESP_LOGW(kTag, "wifi connect failed, fallback sleep");
    config.failure_count += 1;
    store.Save(config);
    // 连接失败走指数退避，减少离线状态下的无效唤醒耗电。
    SleepNow(&status, CalcBackoffSeconds(&config), false);
    return;
  }

  if (open_sta_portal_window) {
    // KEY 唤醒时提供 120 秒本地配置窗口，便于直接通过设备局域网 IP 调整参数。
    RunPortalWindowOnSta(&config, &status, &store);
  }

  ApplyTimezone(config.timezone);
  const int64_t time_before_sync = static_cast<int64_t>(time(nullptr));
  if (ShouldSyncTime(config, time_before_sync)) {
    const bool time_ok = SyncTime(config.timezone);
    if (time_ok) {
      config.last_time_sync_epoch = static_cast<int64_t>(time(nullptr));
      (void)store.Save(config);
    }
  }
  time_t now = time(nullptr);

  if (config.orchestrator_enabled != 0 && !config.orchestrator_base_url.empty()) {
    const DeviceConfigSyncResult sync_result =
        OrchestratorClient::SyncDeviceConfig(&config, &store, static_cast<int64_t>(now));
    if (!sync_result.ok) {
      ESP_LOGW(kTag, "orchestrator config sync failed: base=%s err=%s",
               config.orchestrator_base_url.c_str(), sync_result.error.c_str());
    } else if (sync_result.updated) {
      ESP_LOGI(kTag, "orchestrator config updated to version=%d, reboot to apply",
               sync_result.config_version);
      ESP_ERROR_CHECK_WITHOUT_ABORT(esp_wifi_stop());
      vTaskDelay(pdMS_TO_TICKS(300));
      esp_restart();
      return;
    }
  }

  now = time(nullptr);
  std::string url = ImageClient::BuildDatedUrl(config.image_url_template, now, config.device_id);
  const std::string fallback_url = url;
  uint64_t success_sleep_seconds = static_cast<uint64_t>(std::max(1, config.interval_minutes)) * 60ULL;
  status.image_source = "daily";
  bool used_orchestrator_directive = false;

  if (config.orchestrator_enabled != 0 && !config.orchestrator_base_url.empty()) {
    const FrameDirective directive = OrchestratorClient::FetchDirective(config, now);
    if (directive.ok) {
      url = directive.image_url;
      status.image_source = directive.source;
      if (directive.poll_after_seconds > 0) {
        success_sleep_seconds = static_cast<uint64_t>(directive.poll_after_seconds);
      }
      used_orchestrator_directive = true;
      ESP_LOGI(kTag, "orchestrator source=%s poll_after=%llus", status.image_source.c_str(),
               static_cast<unsigned long long>(success_sleep_seconds));
    } else {
      ESP_LOGW(kTag, "orchestrator unavailable, base=%s, fallback daily url: %s",
               config.orchestrator_base_url.c_str(), directive.error.c_str());
    }
  }

  // BOOT 强刷需要拿到正文并重新渲染，不能命中 304，因此这里绕过条件 GET。
  const std::string previous_etag = status.force_refresh ? "" : config.last_image_etag;
  const std::string previous_last_modified =
      status.force_refresh ? "" : config.last_image_last_modified;

  const std::vector<std::string> fetch_urls =
      BuildFetchUrlCandidates(url, config.preferred_image_origin);
  ImageFetchResult fetch;
  std::string fetch_url_used;
  for (size_t i = 0; i < fetch_urls.size(); ++i) {
    ESP_LOGI(kTag, "fetch url candidate %u/%u: %s", static_cast<unsigned>(i + 1),
             static_cast<unsigned>(fetch_urls.size()), fetch_urls[i].c_str());
    fetch = ImageClient::FetchImage(fetch_urls[i], config.last_image_sha256, config.photo_token,
                                    previous_etag, previous_last_modified);
    if (fetch.ok) {
      fetch_url_used = fetch_urls[i];
      break;
    }
    ESP_LOGW(kTag, "fetch candidate failed: http=%d err=%s", fetch.status_code, fetch.error.c_str());
  }
  if (!fetch.ok && used_orchestrator_directive && status.image_source == "daily" &&
      !fallback_url.empty() && fallback_url != url) {
    // 自动内外网切换：当编排服务给出的 URL 不可达（常见于离家后仍返回内网 URL），回退到配置模板。
    ESP_LOGW(kTag, "fetch directive url failed, fallback to template url: %s", fallback_url.c_str());
    fetch = ImageClient::FetchImage(fallback_url, config.last_image_sha256, config.photo_token,
                                    previous_etag, previous_last_modified);
    if (fetch.ok) {
      fetch_url_used = fallback_url;
    }
  }

  status.last_http_status = fetch.status_code;
  status.image_changed = fetch.image_changed;
  if (fetch.ok) {
    ESP_LOGI(kTag,
             "fetch ok: changed=%d force_refresh=%d prev_sha=%s new_sha=%s url=%s",
             fetch.image_changed ? 1 : 0, status.force_refresh ? 1 : 0,
             config.last_image_sha256.empty() ? "-" : config.last_image_sha256.c_str(),
             fetch.sha256.c_str(), fetch_url_used.empty() ? "-" : fetch_url_used.c_str());
  }

  const bool should_refresh_epd = status.force_refresh || fetch.image_changed;
  bool render_ok = true;
  bool render_blocked_by_pmic = false;

  JpegDecodedImage jpeg_img;
  if (fetch.ok && fetch.data != nullptr && should_refresh_epd &&
      fetch.format == ImageFetchResult::ImageFormat::kJpeg) {
    std::string decode_err;
    if (!JpegDecoder::DecodeRgb888(fetch.data, fetch.data_len, &jpeg_img, &decode_err)) {
      render_ok = false;
      status.last_error = "jpeg decode failed: " + decode_err;
      ESP_LOGE(kTag, "%s", status.last_error.c_str());
    }
  }

  if (fetch.ok && fetch.data != nullptr && should_refresh_epd && render_ok) {
    if (!EnsurePmicReadyForRender()) {
      render_ok = false;
      render_blocked_by_pmic = true;
      status.last_error = "pmic init failed";
      ESP_LOGE(kTag, "%s", status.last_error.c_str());
    } else {
      int retry_count = 0;
      while (retry_count < kEpdRefreshMaxRetries) {
        if (retry_count > 0) {
          ESP_LOGW(kTag, "epd refresh retry %d/%d", retry_count, kEpdRefreshMaxRetries);
          vTaskDelay(pdMS_TO_TICKS(kEpdRefreshRetryDelayMs));
        }

        bool retryable_failure = true;
        PhotoPainterEpd epd;
        PhotoPainterEpd::RenderOptions render_opts;
        render_opts.panel_rotation = static_cast<uint8_t>(config.display_rotation);
        render_opts.color_process_mode = static_cast<uint8_t>(config.color_process_mode);
        render_opts.dithering_mode = static_cast<uint8_t>(config.dither_mode);
        render_opts.six_color_tolerance = static_cast<uint8_t>(config.six_color_tolerance);

        ESP_LOGI(kTag,
                 "start e-paper refresh: force=%d changed=%d bytes=%u retry=%d",
                 status.force_refresh ? 1 : 0, fetch.image_changed ? 1 : 0,
                 static_cast<unsigned>(fetch.data_len), retry_count);

        if (!epd.Init()) {
          // 面板初始化失败通常是供电/硬件路径问题，重试会重复 45s BUSY 超时，直接快速失败省电。
          render_ok = false;
          retryable_failure = false;
          status.last_error = "epd init failed";
          ESP_LOGE(kTag, "%s", status.last_error.c_str());
        } else if (fetch.format == ImageFetchResult::ImageFormat::kBmp) {
          if (!epd.DrawBmp24(fetch.data, fetch.data_len, render_opts)) {
            render_ok = false;
            status.last_error = "bmp decode/render failed";
            ESP_LOGE(kTag, "%s", status.last_error.c_str());
          } else {
            render_ok = true;
            ESP_LOGI(kTag, "e-paper refresh done");
            break;
          }
        } else if (fetch.format == ImageFetchResult::ImageFormat::kJpeg) {
          if (jpeg_img.rgb == nullptr) {
            render_ok = false;
            retryable_failure = false;
            status.last_error = "jpeg decode missing buffer";
            ESP_LOGE(kTag, "%s", status.last_error.c_str());
          } else if (!epd.DrawRgb24(jpeg_img.rgb, jpeg_img.width, jpeg_img.height, render_opts)) {
            render_ok = false;
            status.last_error = "jpeg render failed";
            ESP_LOGE(kTag, "%s", status.last_error.c_str());
          } else {
            render_ok = true;
            ESP_LOGI(kTag, "e-paper refresh done");
            break;
          }
        } else {
          render_ok = false;
          retryable_failure = false;
          status.last_error = "unsupported image format";
          ESP_LOGE(kTag, "%s", status.last_error.c_str());
        }

        if (!retryable_failure) {
          break;
        }
        retry_count += 1;
      }
    }
  } else if (fetch.ok && fetch.data != nullptr) {
    ESP_LOGI(kTag, "image hash unchanged, skip e-paper refresh");
  }

  if (jpeg_img.rgb != nullptr) {
    JpegDecoder::FreeDecodedImage(&jpeg_img);
  }

  const bool cycle_ok = fetch.ok && render_ok;

  if (cycle_ok) {
    config.failure_count = 0;
    if (fetch.image_changed) {
      config.last_image_sha256 = fetch.sha256;
    }
    if (!fetch.etag.empty()) {
      config.last_image_etag = fetch.etag;
    }
    if (!fetch.last_modified.empty()) {
      config.last_image_last_modified = fetch.last_modified;
    }
    std::string used_origin;
    if (!fetch_url_used.empty() && SplitUrlOriginAndRest(fetch_url_used, &used_origin, nullptr)) {
      config.preferred_image_origin = used_origin;
    }
    config.last_success_epoch = static_cast<int64_t>(time(nullptr));
    store.Save(config);

    const int64_t now_epoch = static_cast<int64_t>(time(nullptr));
    status.next_wakeup_epoch = now_epoch + static_cast<int64_t>(success_sleep_seconds);

    std::tm now_local_tm = {};
    char now_local_buf[64] = {};
    time_t now_time = static_cast<time_t>(now_epoch);
    localtime_r(&now_time, &now_local_tm);
    strftime(now_local_buf, sizeof(now_local_buf), "%Y-%m-%d %H:%M:%S %Z", &now_local_tm);

    if (config.orchestrator_enabled != 0) {
      RefreshPowerStatus(&status);
      DeviceCheckinPayload payload;
      payload.fetch_ok = true;
      payload.image_changed = fetch.image_changed;
      payload.last_http_status = fetch.status_code;
      payload.failure_count = config.failure_count;
      payload.poll_interval_seconds = std::max(1, config.interval_minutes) * 60;
      payload.sleep_seconds = success_sleep_seconds;
      payload.now_epoch = now_epoch;
      payload.next_wakeup_epoch = status.next_wakeup_epoch;
      payload.battery_mv = status.battery_mv;
      payload.battery_percent = status.battery_percent;
      payload.charging = status.charging;
      payload.vbus_good = status.vbus_good;
      payload.image_source = status.image_source;
      payload.last_error = status.last_error;
      payload.sta_ip = StaIpString();
      const bool checkin_ok = ReportCheckinWithFallback(config, payload, fetch_url_used, fallback_url);
      ESP_LOGI(kTag, "orchestrator checkin (ok cycle): url=%s result=%s",
               config.orchestrator_base_url.c_str(), checkin_ok ? "ok" : "fail");
    }

    ESP_LOGI(kTag,
             "cycle ok: local=%s epoch=%lld source=%s http=%d changed=%d sleep=%llus batt=%d%%/%dmV charging=%d",
             now_local_buf, static_cast<long long>(now_epoch), status.image_source.c_str(),
             status.last_http_status, status.image_changed ? 1 : 0,
             static_cast<unsigned long long>(success_sleep_seconds), status.battery_percent,
             status.battery_mv, status.charging);

    ImageClient::FreeResultBuffer(&fetch);
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_wifi_stop());
    // 正常路径按服务下发或本地默认间隔休眠。
    SleepNow(&status, success_sleep_seconds, false);
    return;
  }

  if (!fetch.ok) {
    status.last_error = fetch.error;
    ESP_LOGW(kTag, "fetch failed: %s", fetch.error.c_str());
  } else if (status.last_error.empty()) {
    status.last_error = "render failed";
    ESP_LOGW(kTag, "render failed without detail, treat as fetch failure");
  }

  const bool soft_pmic_failure = fetch.ok && should_refresh_epd && render_blocked_by_pmic;
  uint64_t backoff_sleep_seconds = 0;
  if (soft_pmic_failure) {
    // PMIC 通信偶发失败时不走指数退避，避免出现“几小时没上报”。
    config.failure_count = 0;
    backoff_sleep_seconds = static_cast<uint64_t>(std::max(1, config.interval_minutes)) * 60ULL;
    ESP_LOGW(kTag, "soft failure(pmic): keep regular sleep=%llus",
             static_cast<unsigned long long>(backoff_sleep_seconds));
  } else {
    config.failure_count += 1;
    backoff_sleep_seconds = CalcBackoffSeconds(&config);
  }
  store.Save(config);

  const int64_t now_epoch = static_cast<int64_t>(time(nullptr));
  status.next_wakeup_epoch = now_epoch + static_cast<int64_t>(backoff_sleep_seconds);

  if (config.orchestrator_enabled != 0) {
    RefreshPowerStatus(&status);
    DeviceCheckinPayload payload;
    payload.fetch_ok = false;
    payload.image_changed = fetch.image_changed;
    payload.last_http_status = fetch.status_code;
    payload.failure_count = config.failure_count;
    payload.poll_interval_seconds = std::max(1, config.interval_minutes) * 60;
    payload.sleep_seconds = backoff_sleep_seconds;
    payload.now_epoch = now_epoch;
    payload.next_wakeup_epoch = status.next_wakeup_epoch;
    payload.battery_mv = status.battery_mv;
    payload.battery_percent = status.battery_percent;
    payload.charging = status.charging;
    payload.vbus_good = status.vbus_good;
    payload.image_source = status.image_source;
    payload.last_error = status.last_error;
    payload.sta_ip = StaIpString();
    const bool checkin_ok = ReportCheckinWithFallback(config, payload, fetch_url_used, fallback_url);
    ESP_LOGI(kTag, "orchestrator checkin (fail cycle): url=%s result=%s",
             config.orchestrator_base_url.c_str(), checkin_ok ? "ok" : "fail");
  }

  ESP_LOGW(kTag,
           "cycle fail: http=%d err=%s backoff=%llus batt=%d%%/%dmV charging=%d",
           status.last_http_status, status.last_error.c_str(),
           static_cast<unsigned long long>(backoff_sleep_seconds), status.battery_percent,
           status.battery_mv, status.charging);

  ImageClient::FreeResultBuffer(&fetch);
  ESP_ERROR_CHECK_WITHOUT_ABORT(esp_wifi_stop());
  SleepNow(&status, backoff_sleep_seconds, false);
}
