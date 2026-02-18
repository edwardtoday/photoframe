#pragma once

#include <cstdint>

struct PowerStatus {
  bool pmic_ready = false;
  bool vbus_good = false;
  bool battery_present = false;
  bool charging = false;
  int battery_mv = -1;
  int battery_percent = -1;
  int charger_state = -1;
};

class PowerManager {
 public:
  static bool Init();
  static bool ReadStatus(PowerStatus* status);
  // 深睡前的省电准备：关闭不需要的外围供电/采样通道（不会影响 ESP 本体供电）。
  static void PrepareForDeepSleep();
  static const char* ChargerStateName(int state);
};
