#pragma once

#include <cstdint>
#include <string>

#include "nvs.h"

struct AppConfig {
  enum ColorProcessMode {
    kColorProcessAuto = 0,
    kColorProcessForceConvert = 1,
    kColorProcessAssumeSixColor = 2,
  };

  enum DitherMode {
    kDitherNone = 0,
    kDitherOrdered = 1,
  };

  std::string wifi_ssid;
  std::string wifi_password;
  std::string image_url_template = "http://192.168.58.113:8000/image/480x800?date=%DATE%";
  int orchestrator_enabled = 1;
  std::string orchestrator_base_url = "http://192.168.58.113:8081";
  std::string device_id = "";
  std::string orchestrator_token;
  std::string timezone = "UTC";
  int interval_minutes = 60;
  int retry_base_minutes = 5;
  int retry_max_minutes = 240;
  int max_failure_before_long_sleep = 24;
  int display_rotation = 2;
  int color_process_mode = kColorProcessAuto;
  int dither_mode = kDitherOrdered;
  int six_color_tolerance = 0;

  std::string last_image_sha256;
  int64_t last_success_epoch = 0;
  int failure_count = 0;
};

struct RuntimeStatus {
  bool wifi_connected = false;
  bool force_refresh = false;
  int last_http_status = 0;
  bool image_changed = false;
  std::string image_source = "daily";
  int64_t next_wakeup_epoch = 0;
  std::string last_error;
};

class ConfigStore {
 public:
  bool Init();
  bool Load(AppConfig* cfg);
  bool Save(const AppConfig& cfg);
  bool SaveWifi(const std::string& ssid, const std::string& password);
  bool ClearWifi();

 private:
  bool SetString(const char* key, const std::string& value);
  std::string GetString(const char* key, const std::string& fallback);
  bool SetI32(const char* key, int32_t value);
  int32_t GetI32(const char* key, int32_t fallback);
  bool SetI64(const char* key, int64_t value);
  int64_t GetI64(const char* key, int64_t fallback);

  nvs_handle_t nvs_ = 0;
};
