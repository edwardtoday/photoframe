#include "config_store.h"

#include <algorithm>

#include "esp_log.h"
#include "nvs_flash.h"

namespace {
constexpr const char* kTag = "config_store";
constexpr const char* kNvsNs = "photoframe";

int ClampInt(int value, int min_v, int max_v) {
  if (value < min_v) {
    return min_v;
  }
  if (value > max_v) {
    return max_v;
  }
  return value;
}
}  // namespace

bool ConfigStore::Init() {
  esp_err_t err = nvs_flash_init();
  if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
    ESP_ERROR_CHECK(nvs_flash_erase());
    err = nvs_flash_init();
  }
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "nvs_flash_init failed: %s", esp_err_to_name(err));
    return false;
  }

  err = nvs_open(kNvsNs, NVS_READWRITE, &nvs_);
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "nvs_open failed: %s", esp_err_to_name(err));
    return false;
  }
  return true;
}

bool ConfigStore::Load(AppConfig* cfg) {
  if (cfg == nullptr || nvs_ == 0) {
    return false;
  }

  cfg->wifi_ssid = GetString("wifi_ssid", "");
  cfg->wifi_password = GetString("wifi_pwd", "");
  cfg->image_url_template = GetString("url_tpl", cfg->image_url_template);
  cfg->photo_token = GetString("photo_tok", cfg->photo_token);
  cfg->orchestrator_enabled = GetI32("orch_en", cfg->orchestrator_enabled) ? 1 : 0;
  cfg->orchestrator_base_url = GetString("orch_url", cfg->orchestrator_base_url);
  cfg->device_id = GetString("dev_id", cfg->device_id);
  cfg->orchestrator_token = GetString("orch_tok", cfg->orchestrator_token);
  cfg->timezone = GetString("tz", cfg->timezone);
  // NVS 读到非法值时在加载阶段就做下限兜底，避免后续计算出现负值。
  cfg->interval_minutes =
      std::max<int32_t>(1, GetI32("intv_min", static_cast<int32_t>(cfg->interval_minutes)));
  cfg->retry_base_minutes =
      std::max<int32_t>(1, GetI32("retry_base", static_cast<int32_t>(cfg->retry_base_minutes)));
  cfg->retry_max_minutes = std::max<int32_t>(
      static_cast<int32_t>(cfg->retry_base_minutes),
      GetI32("retry_max", static_cast<int32_t>(cfg->retry_max_minutes)));
  cfg->max_failure_before_long_sleep = std::max<int32_t>(
      1, GetI32("max_fail", static_cast<int32_t>(cfg->max_failure_before_long_sleep)));
  cfg->display_rotation = GetI32("rotation", cfg->display_rotation);
  cfg->color_process_mode = ClampInt(
      GetI32("clr_mode", cfg->color_process_mode), AppConfig::kColorProcessAuto,
      AppConfig::kColorProcessAssumeSixColor);
  cfg->dither_mode = ClampInt(GetI32("dither", cfg->dither_mode), AppConfig::kDitherNone,
                              AppConfig::kDitherOrdered);
  cfg->six_color_tolerance =
      ClampInt(GetI32("clr_tol", cfg->six_color_tolerance), 0, 64);
  cfg->last_image_sha256 = GetString("img_sha256", "");
  cfg->last_success_epoch = GetI64("last_ok", 0);
  cfg->failure_count = std::max<int32_t>(0, GetI32("fail_cnt", 0));
  cfg->remote_config_version = std::max<int32_t>(0, GetI32("cfg_ver", 0));

  if (cfg->display_rotation != 0 && cfg->display_rotation != 2) {
    cfg->display_rotation = 2;
  }
  return true;
}

bool ConfigStore::Save(const AppConfig& cfg) {
  if (nvs_ == 0) {
    return false;
  }

  if (!SetString("wifi_ssid", cfg.wifi_ssid) || !SetString("wifi_pwd", cfg.wifi_password) ||
      !SetString("url_tpl", cfg.image_url_template) || !SetString("photo_tok", cfg.photo_token) ||
      !SetI32("orch_en", cfg.orchestrator_enabled) ||
      !SetString("orch_url", cfg.orchestrator_base_url) || !SetString("dev_id", cfg.device_id) ||
      !SetString("orch_tok", cfg.orchestrator_token) || !SetString("tz", cfg.timezone) ||
      !SetI32("intv_min", cfg.interval_minutes) || !SetI32("retry_base", cfg.retry_base_minutes) ||
      !SetI32("retry_max", cfg.retry_max_minutes) ||
      !SetI32("max_fail", cfg.max_failure_before_long_sleep) ||
      !SetI32("rotation", cfg.display_rotation) || !SetI32("clr_mode", cfg.color_process_mode) ||
      !SetI32("dither", cfg.dither_mode) || !SetI32("clr_tol", cfg.six_color_tolerance) ||
      !SetString("img_sha256", cfg.last_image_sha256) ||
      !SetI64("last_ok", cfg.last_success_epoch) || !SetI32("fail_cnt", cfg.failure_count) ||
      !SetI32("cfg_ver", cfg.remote_config_version)) {
    return false;
  }

  esp_err_t err = nvs_commit(nvs_);
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "nvs_commit failed: %s", esp_err_to_name(err));
    return false;
  }
  return true;
}

bool ConfigStore::SaveWifi(const std::string& ssid, const std::string& password) {
  if (nvs_ == 0) {
    return false;
  }
  if (!SetString("wifi_ssid", ssid) || !SetString("wifi_pwd", password)) {
    return false;
  }
  return nvs_commit(nvs_) == ESP_OK;
}

bool ConfigStore::ClearWifi() {
  if (nvs_ == 0) {
    return false;
  }
  ESP_ERROR_CHECK_WITHOUT_ABORT(nvs_erase_key(nvs_, "wifi_ssid"));
  ESP_ERROR_CHECK_WITHOUT_ABORT(nvs_erase_key(nvs_, "wifi_pwd"));
  return nvs_commit(nvs_) == ESP_OK;
}

bool ConfigStore::SetString(const char* key, const std::string& value) {
  esp_err_t err = nvs_set_str(nvs_, key, value.c_str());
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "nvs_set_str(%s) failed: %s", key, esp_err_to_name(err));
    return false;
  }
  return true;
}

std::string ConfigStore::GetString(const char* key, const std::string& fallback) {
  size_t len = 0;
  esp_err_t err = nvs_get_str(nvs_, key, nullptr, &len);
  if (err == ESP_ERR_NVS_NOT_FOUND) {
    return fallback;
  }
  if (err != ESP_OK || len == 0) {
    return fallback;
  }

  std::string value;
  value.resize(len);
  err = nvs_get_str(nvs_, key, value.data(), &len);
  if (err != ESP_OK) {
    return fallback;
  }
  if (!value.empty() && value.back() == '\0') {
    value.pop_back();
  }
  return value;
}

bool ConfigStore::SetI32(const char* key, int32_t value) {
  esp_err_t err = nvs_set_i32(nvs_, key, value);
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "nvs_set_i32(%s) failed: %s", key, esp_err_to_name(err));
    return false;
  }
  return true;
}

int32_t ConfigStore::GetI32(const char* key, int32_t fallback) {
  int32_t value = fallback;
  esp_err_t err = nvs_get_i32(nvs_, key, &value);
  if (err != ESP_OK) {
    return fallback;
  }
  return value;
}

bool ConfigStore::SetI64(const char* key, int64_t value) {
  esp_err_t err = nvs_set_i64(nvs_, key, value);
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "nvs_set_i64(%s) failed: %s", key, esp_err_to_name(err));
    return false;
  }
  return true;
}

int64_t ConfigStore::GetI64(const char* key, int64_t fallback) {
  int64_t value = fallback;
  esp_err_t err = nvs_get_i64(nvs_, key, &value);
  if (err != ESP_OK) {
    return fallback;
  }
  return value;
}
