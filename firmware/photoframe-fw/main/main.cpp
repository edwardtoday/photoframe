#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <string>

#include "config_store.h"
#include "image_client.h"
#include "orchestrator_client.h"
#include "photopainter_epd.h"
#include "portal_server.h"
#include "power_manager.h"

#include "esp_event.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_sleep.h"
#include "esp_sntp.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/task.h"
#include "lwip/ip4_addr.h"

namespace {
constexpr const char* kTag = "photoframe_main";

constexpr gpio_num_t kKeyButton = GPIO_NUM_4;   // KEY: 唤醒后打开 120 秒配置窗口
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

EventGroupHandle_t g_wifi_events = nullptr;
constexpr int kWifiConnectedBit = BIT0;
constexpr int kWifiFailBit = BIT1;
int g_wifi_retry = 0;
int g_wifi_retry_limit = kStaConnectRetry;
int g_last_disconnect_reason = 0;
bool g_wifi_ready = false;
esp_netif_t* g_sta_netif = nullptr;
esp_netif_t* g_ap_netif = nullptr;

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

bool IsKeyButtonPressed() {
  return gpio_get_level(kKeyButton) == 0;
}

bool IsBootButtonPressed() {
  return gpio_get_level(kBootButton) == 0;
}

bool ShouldEnterPortalByLongPress() {
  if (!IsKeyButtonPressed() && !IsBootButtonPressed()) {
    return false;
  }
  ESP_LOGI(kTag, "button pressed at boot, waiting for long-press...");
  vTaskDelay(pdMS_TO_TICKS(3000));
  return IsKeyButtonPressed() || IsBootButtonPressed();
}

void EnterDeepSleep(uint64_t seconds) {
  ESP_LOGI(kTag, "enter deep sleep for %llu seconds", static_cast<unsigned long long>(seconds));
  ESP_ERROR_CHECK(esp_sleep_enable_timer_wakeup(seconds * 1000000ULL));
  const uint64_t wakeup_pins =
      (1ULL << static_cast<int>(kKeyButton)) | (1ULL << static_cast<int>(kBootButton));
  ESP_ERROR_CHECK(esp_sleep_enable_ext1_wakeup(wakeup_pins, ESP_EXT1_WAKEUP_ANY_LOW));
  vTaskDelay(pdMS_TO_TICKS(150));
  esp_deep_sleep_start();
}

enum class WakeSource {
  TIMER,
  KEY,
  BOOT,
  OTHER,
};

WakeSource GetWakeSource() {
  const esp_sleep_wakeup_cause_t cause = esp_sleep_get_wakeup_cause();
  if (cause == ESP_SLEEP_WAKEUP_TIMER) {
    return WakeSource::TIMER;
  }
  if (cause == ESP_SLEEP_WAKEUP_EXT1) {
    const uint64_t pins = esp_sleep_get_ext1_wakeup_status();
    if (pins & (1ULL << static_cast<int>(kBootButton))) {
      return WakeSource::BOOT;
    }
    if (pins & (1ULL << static_cast<int>(kKeyButton))) {
      return WakeSource::KEY;
    }
  }
  return WakeSource::OTHER;
}

bool SyncTime(const std::string& timezone) {
  if (!timezone.empty()) {
    setenv("TZ", timezone.c_str(), 1);
    tzset();
  }

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

  if (!PowerManager::Init()) {
    ESP_LOGW(kTag, "pmic init failed, skip battery status");
    return;
  }

  PowerStatus power = {};
  if (!PowerManager::ReadStatus(&power)) {
    ESP_LOGW(kTag, "pmic read failed, skip battery status");
    return;
  }

  status->battery_mv = power.battery_mv;
  status->battery_percent = power.battery_percent;
  status->charging = power.charging ? 1 : 0;
  status->vbus_good = power.vbus_good ? 1 : 0;

  ESP_LOGI(kTag,
           "power: vbus=%d charging=%d batt=%dmV percent=%d state=%s",
           status->vbus_good, status->charging, status->battery_mv, status->battery_percent,
           PowerManager::ChargerStateName(power.charger_state));
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
  RefreshPowerStatus(&status);

  const bool long_press_portal = ShouldEnterPortalByLongPress();
  if (long_press_portal) {
    ESP_LOGW(kTag, "long-press detected, clear wifi and enter portal");
    store.ClearWifi();
    config.wifi_ssid.clear();
    config.wifi_password.clear();
  }

  const WakeSource wake_source = GetWakeSource();
  bool open_sta_portal_window = false;
  if (!long_press_portal) {
    switch (wake_source) {
      case WakeSource::BOOT:
        status.force_refresh = true;
        ESP_LOGI(kTag, "wake source=BOOT, force refresh enabled");
        break;
      case WakeSource::KEY:
        status.force_refresh = false;
        open_sta_portal_window = true;
        ESP_LOGI(kTag, "wake source=KEY, open portal window for %d seconds", kKeyWakePortalWindowSec);
        break;
      case WakeSource::TIMER:
        status.force_refresh = false;
        ESP_LOGI(kTag, "wake source=TIMER");
        break;
      default:
        status.force_refresh = false;
        ESP_LOGI(kTag, "wake source=OTHER");
        break;
    }
  }

  if (!EnsureWifiStack()) {
    ESP_LOGE(kTag, "wifi stack init failed");
    vTaskDelay(pdMS_TO_TICKS(2000));
    esp_restart();
    return;
  }

  if (config.wifi_ssid.empty() || long_press_portal) {
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
    EnterDeepSleep(CalcBackoffSeconds(&config));
    return;
  }

  if (open_sta_portal_window) {
    // KEY 唤醒时提供 120 秒本地配置窗口，便于直接通过设备局域网 IP 调整参数。
    RunPortalWindowOnSta(&config, &status, &store);
  }

  SyncTime(config.timezone);
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
  uint64_t success_sleep_seconds = static_cast<uint64_t>(std::max(1, config.interval_minutes)) * 60ULL;
  status.image_source = "daily";

  if (config.orchestrator_enabled != 0 && !config.orchestrator_base_url.empty()) {
    const FrameDirective directive = OrchestratorClient::FetchDirective(config, now);
    if (directive.ok) {
      url = directive.image_url;
      status.image_source = directive.source;
      if (directive.poll_after_seconds > 0) {
        success_sleep_seconds = static_cast<uint64_t>(directive.poll_after_seconds);
      }
      ESP_LOGI(kTag, "orchestrator source=%s poll_after=%llus", status.image_source.c_str(),
               static_cast<unsigned long long>(success_sleep_seconds));
    } else {
      ESP_LOGW(kTag, "orchestrator unavailable, base=%s, fallback daily url: %s",
               config.orchestrator_base_url.c_str(), directive.error.c_str());
    }
  }

  ESP_LOGI(kTag, "fetch url: %s", url.c_str());

  ImageFetchResult fetch = ImageClient::FetchBmp(url, config.last_image_sha256, config.photo_token);
  status.last_http_status = fetch.status_code;
  status.image_changed = fetch.image_changed;
  if (fetch.ok) {
    ESP_LOGI(kTag,
             "fetch ok: changed=%d force_refresh=%d prev_sha=%s new_sha=%s",
             fetch.image_changed ? 1 : 0, status.force_refresh ? 1 : 0,
             config.last_image_sha256.empty() ? "-" : config.last_image_sha256.c_str(),
             fetch.sha256.c_str());
  }

  const bool should_refresh_epd = status.force_refresh || fetch.image_changed;
  bool render_ok = true;

  if (fetch.ok && fetch.data != nullptr && should_refresh_epd) {
    int retry_count = 0;
    while (retry_count < kEpdRefreshMaxRetries) {
      if (retry_count > 0) {
        ESP_LOGW(kTag, "epd refresh retry %d/%d", retry_count, kEpdRefreshMaxRetries);
        vTaskDelay(pdMS_TO_TICKS(kEpdRefreshRetryDelayMs));
      }

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
        render_ok = false;
        status.last_error = "epd init failed";
        ESP_LOGE(kTag, "%s", status.last_error.c_str());
      } else if (!epd.DrawBmp24(fetch.data, fetch.data_len, render_opts)) {
        render_ok = false;
        status.last_error = "bmp decode/render failed";
        ESP_LOGE(kTag, "%s", status.last_error.c_str());
      } else {
        render_ok = true;
        ESP_LOGI(kTag, "e-paper refresh done");
        break;
      }
      retry_count += 1;
    }
  } else if (fetch.ok && fetch.data != nullptr) {
    ESP_LOGI(kTag, "image hash unchanged, skip e-paper refresh");
  }

  const bool cycle_ok = fetch.ok && render_ok;

  if (cycle_ok) {
    config.failure_count = 0;
    if (fetch.image_changed) {
      config.last_image_sha256 = fetch.sha256;
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
      const bool checkin_ok = OrchestratorClient::ReportCheckin(config, payload);
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
    EnterDeepSleep(success_sleep_seconds);
    return;
  }

  if (!fetch.ok) {
    status.last_error = fetch.error;
    ESP_LOGW(kTag, "fetch failed: %s", fetch.error.c_str());
  } else if (status.last_error.empty()) {
    status.last_error = "render failed";
    ESP_LOGW(kTag, "render failed without detail, treat as fetch failure");
  }

  config.failure_count += 1;
  store.Save(config);

  const uint64_t backoff_sleep_seconds = CalcBackoffSeconds(&config);
  const int64_t now_epoch = static_cast<int64_t>(time(nullptr));
  status.next_wakeup_epoch = now_epoch + static_cast<int64_t>(backoff_sleep_seconds);

  if (config.orchestrator_enabled != 0) {
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
    const bool checkin_ok = OrchestratorClient::ReportCheckin(config, payload);
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
  EnterDeepSleep(backoff_sleep_seconds);
}
