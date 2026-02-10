#pragma once

#include <atomic>

#include "config_store.h"

#include "esp_http_server.h"

class PortalServer {
 public:
  bool Start(AppConfig* config, RuntimeStatus* status, ConfigStore* store, bool enable_dns = true);
  void Stop();

  bool ShouldReboot() const { return should_reboot_.load(); }

 private:
  static esp_err_t HandleRoot(httpd_req_t* req);
  static esp_err_t HandleGetConfig(httpd_req_t* req);
  static esp_err_t HandlePostConfig(httpd_req_t* req);
  static esp_err_t HandleScanWifi(httpd_req_t* req);

  esp_err_t SendConfigJson(httpd_req_t* req);

  static void DnsTask(void* arg);
  bool StartDnsServer();
  void StopDnsServer();

  httpd_handle_t server_ = nullptr;
  AppConfig* config_ = nullptr;
  RuntimeStatus* status_ = nullptr;
  ConfigStore* store_ = nullptr;

  std::atomic<bool> should_reboot_{false};
  std::atomic<bool> dns_running_{false};
  int dns_sock_ = -1;
};
