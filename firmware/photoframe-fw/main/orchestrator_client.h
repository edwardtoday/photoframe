#pragma once

#include <cstdint>
#include <ctime>
#include <string>

#include "config_store.h"

struct FrameDirective {
  bool ok = false;
  int status_code = 0;
  std::string image_url;
  std::string source = "daily";
  int poll_after_seconds = 0;
  int64_t valid_until_epoch = 0;
  std::string error;
};

struct DeviceConfigSyncResult {
  bool ok = false;
  bool updated = false;
  int config_version = 0;
  std::string error;
};

struct DeviceCheckinPayload {
  bool fetch_ok = false;
  bool image_changed = false;
  int last_http_status = 0;
  int failure_count = 0;
  int poll_interval_seconds = 3600;
  uint64_t sleep_seconds = 3600;
  int64_t now_epoch = 0;
  int64_t next_wakeup_epoch = 0;
  int battery_mv = -1;
  int battery_percent = -1;
  int charging = -1;
  int vbus_good = -1;
  std::string image_source = "daily";
  std::string last_error;
};

class OrchestratorClient {
 public:
  static std::string EnsureDeviceId(AppConfig* cfg);
  static FrameDirective FetchDirective(const AppConfig& cfg, time_t now_epoch);
  static DeviceConfigSyncResult SyncDeviceConfig(AppConfig* cfg, ConfigStore* store,
                                                 int64_t now_epoch);
  static bool ReportConfigApplied(const AppConfig& cfg, int config_version, bool applied,
                                  const std::string& error, int64_t now_epoch);
  static bool ReportCheckin(const AppConfig& cfg, const DeviceCheckinPayload& payload);
};
