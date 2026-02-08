#include <algorithm>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <string>

#include "config_store.h"
#include "image_client.h"
#include "photopainter_epd.h"
#include "portal_server.h"

#include "esp_event.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_sleep.h"
#include "esp_sntp.h"
#include "esp_system.h"
#include "esp_wifi.h"
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/task.h"

namespace {
constexpr const char* kTag = "photoframe_main";

constexpr gpio_num_t kKeyButton = GPIO_NUM_4;
constexpr int kStaConnectTimeoutSec = 25;
constexpr int kStaConnectRetry = 5;
constexpr const char* kApSsid = "PhotoFrame-Setup";
constexpr const char* kApPassword = "12345678";

EventGroupHandle_t g_wifi_events = nullptr;
constexpr int kWifiConnectedBit = BIT0;
constexpr int kWifiFailBit = BIT1;
int g_wifi_retry = 0;
int g_wifi_retry_limit = kStaConnectRetry;
bool g_wifi_ready = false;

void WifiEventHandler(void* arg, esp_event_base_t event_base, int32_t event_id, void* event_data) {
  if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_START) {
    esp_wifi_connect();
    return;
  }

  if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_DISCONNECTED) {
    if (g_wifi_retry < g_wifi_retry_limit) {
      ++g_wifi_retry;
      esp_wifi_connect();
      ESP_LOGW(kTag, "wifi disconnected, retry %d/%d", g_wifi_retry, g_wifi_retry_limit);
    } else {
      xEventGroupSetBits(g_wifi_events, kWifiFailBit);
    }
    return;
  }

  if (event_base == IP_EVENT && event_id == IP_EVENT_STA_GOT_IP) {
    auto* got_ip = static_cast<ip_event_got_ip_t*>(event_data);
    ESP_LOGI(kTag, "got ip: " IPSTR, IP2STR(&got_ip->ip_info.ip));
    g_wifi_retry = 0;
    xEventGroupSetBits(g_wifi_events, kWifiConnectedBit);
  }
}

bool EnsureWifiStack() {
  if (g_wifi_ready) {
    return true;
  }

  ESP_ERROR_CHECK(esp_netif_init());
  ESP_ERROR_CHECK(esp_event_loop_create_default());
  esp_netif_create_default_wifi_sta();
  esp_netif_create_default_wifi_ap();

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

bool StartConfigApMode() {
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

  ESP_LOGI(kTag, "config AP started, ssid=%s", kApSsid);
  return true;
}

bool ConnectToSta(const AppConfig& cfg, RuntimeStatus* status) {
  if (cfg.wifi_ssid.empty()) {
    status->last_error = "wifi ssid is empty";
    return false;
  }

  wifi_config_t sta_cfg = {};
  strncpy(reinterpret_cast<char*>(sta_cfg.sta.ssid), cfg.wifi_ssid.c_str(),
          sizeof(sta_cfg.sta.ssid));
  strncpy(reinterpret_cast<char*>(sta_cfg.sta.password), cfg.wifi_password.c_str(),
          sizeof(sta_cfg.sta.password));
  sta_cfg.sta.scan_method = WIFI_ALL_CHANNEL_SCAN;
  sta_cfg.sta.sort_method = WIFI_CONNECT_AP_BY_SIGNAL;
  sta_cfg.sta.threshold.authmode = WIFI_AUTH_OPEN;
  sta_cfg.sta.pmf_cfg.capable = true;
  sta_cfg.sta.pmf_cfg.required = false;

  ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
  ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &sta_cfg));

  g_wifi_retry = 0;
  g_wifi_retry_limit = kStaConnectRetry;
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
  status->last_error = "wifi connect timeout/fail";
  return false;
}

bool IsButtonPressed() {
  return gpio_get_level(kKeyButton) == 0;
}

bool ShouldEnterPortalByLongPress() {
  if (!IsButtonPressed()) {
    return false;
  }
  ESP_LOGI(kTag, "button pressed at boot, waiting for long-press...");
  vTaskDelay(pdMS_TO_TICKS(3000));
  return IsButtonPressed();
}

void EnterDeepSleep(uint64_t seconds) {
  ESP_LOGI(kTag, "enter deep sleep for %llu seconds", static_cast<unsigned long long>(seconds));
  ESP_ERROR_CHECK(esp_sleep_enable_timer_wakeup(seconds * 1000000ULL));
  ESP_ERROR_CHECK(
      esp_sleep_enable_ext1_wakeup(1ULL << static_cast<int>(kKeyButton), ESP_EXT1_WAKEUP_ANY_LOW));
  vTaskDelay(pdMS_TO_TICKS(150));
  esp_deep_sleep_start();
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
      ESP_LOGI(kTag, "time synced, epoch=%lld", static_cast<long long>(now));
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
  cfg.pin_bit_mask = 1ULL << static_cast<int>(kKeyButton);
  cfg.mode = GPIO_MODE_INPUT;
  cfg.pull_up_en = GPIO_PULLUP_ENABLE;
  cfg.pull_down_en = GPIO_PULLDOWN_DISABLE;
  cfg.intr_type = GPIO_INTR_DISABLE;
  ESP_ERROR_CHECK(gpio_config(&cfg));
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

  ConfigureButtonGpio();

  const bool long_press_portal = ShouldEnterPortalByLongPress();
  if (long_press_portal) {
    ESP_LOGW(kTag, "long-press detected, clear wifi and enter portal");
    store.ClearWifi();
    config.wifi_ssid.clear();
    config.wifi_password.clear();
  }

  const esp_sleep_wakeup_cause_t wake_cause = esp_sleep_get_wakeup_cause();
  // 由按键唤醒时强制刷新，避免 hash 未变化导致用户误以为按键无效。
  status.force_refresh = (wake_cause == ESP_SLEEP_WAKEUP_EXT1) && !long_press_portal;

  if (!EnsureWifiStack()) {
    ESP_LOGE(kTag, "wifi stack init failed");
    vTaskDelay(pdMS_TO_TICKS(2000));
    esp_restart();
    return;
  }

  if (config.wifi_ssid.empty() || long_press_portal) {
    StartConfigApMode();
    PortalServer portal;
    if (!portal.Start(&config, &status, &store)) {
      ESP_LOGE(kTag, "portal start failed");
      vTaskDelay(pdMS_TO_TICKS(2000));
      esp_restart();
      return;
    }

    ESP_LOGI(kTag, "enter portal mode: connect Wi-Fi to %s, then open http://192.168.4.1/", kApSsid);
    while (true) {
      if (portal.ShouldReboot()) {
        ESP_LOGI(kTag, "config saved, rebooting...");
        vTaskDelay(pdMS_TO_TICKS(500));
        esp_restart();
      }
      vTaskDelay(pdMS_TO_TICKS(500));
    }
  }

  if (!ConnectToSta(config, &status)) {
    ESP_LOGW(kTag, "wifi connect failed, fallback sleep");
    config.failure_count += 1;
    store.Save(config);
    // 连接失败走指数退避，减少离线状态下的无效唤醒耗电。
    EnterDeepSleep(CalcBackoffSeconds(&config));
    return;
  }

  SyncTime(config.timezone);
  const time_t now = time(nullptr);
  const std::string url = ImageClient::BuildDatedUrl(config.image_url_template, now);
  ESP_LOGI(kTag, "fetch url: %s", url.c_str());

  ImageFetchResult fetch = ImageClient::FetchBmp(url, config.last_image_sha256);
  status.last_http_status = fetch.status_code;
  status.image_changed = fetch.image_changed;

  PhotoPainterEpd epd;
  PhotoPainterEpd::RenderOptions render_opts;
  render_opts.panel_rotation = static_cast<uint8_t>(config.display_rotation);
  render_opts.color_process_mode = static_cast<uint8_t>(config.color_process_mode);
  render_opts.dithering_mode = static_cast<uint8_t>(config.dither_mode);
  render_opts.six_color_tolerance = static_cast<uint8_t>(config.six_color_tolerance);
  if (!epd.Init()) {
    status.last_error = "epd init failed";
  } else if (fetch.ok && fetch.data != nullptr) {
    if (status.force_refresh || fetch.image_changed) {
      if (!epd.DrawBmp24(fetch.data, fetch.data_len, render_opts)) {
        status.last_error = "bmp decode/render failed";
      }
    } else {
      ESP_LOGI(kTag, "image hash unchanged, skip e-paper refresh");
    }
  }

  if (fetch.ok) {
    config.failure_count = 0;
    if (fetch.image_changed) {
      config.last_image_sha256 = fetch.sha256;
    }
    config.last_success_epoch = static_cast<int64_t>(time(nullptr));
    store.Save(config);

    ImageClient::FreeResultBuffer(&fetch);
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_wifi_stop());
    // 正常路径按固定轮询间隔休眠。
    EnterDeepSleep(static_cast<uint64_t>(std::max(1, config.interval_minutes)) * 60ULL);
    return;
  }

  status.last_error = fetch.error;
  ESP_LOGW(kTag, "fetch failed: %s", fetch.error.c_str());
  config.failure_count += 1;
  store.Save(config);

  ImageClient::FreeResultBuffer(&fetch);
  ESP_ERROR_CHECK_WITHOUT_ABORT(esp_wifi_stop());
  EnterDeepSleep(CalcBackoffSeconds(&config));
}
