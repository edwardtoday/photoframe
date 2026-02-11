#include "power_manager.h"

#include <algorithm>

#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "esp_log.h"

namespace {
constexpr const char* kTag = "power_manager";

constexpr i2c_port_num_t kI2cPort = I2C_NUM_0;
constexpr int kI2cSclPin = 48;
constexpr int kI2cSdaPin = 47;
constexpr int kI2cFreqHz = 300000;
constexpr int kI2cTimeoutMs = 200;

constexpr uint8_t kAxp2101Addr = 0x34;
constexpr uint8_t kRegChipId = 0x03;
constexpr uint8_t kRegStatus1 = 0x00;
constexpr uint8_t kRegStatus2 = 0x01;
constexpr uint8_t kRegAdcChannelCtrl = 0x30;
constexpr uint8_t kRegAdcBattH = 0x34;
constexpr uint8_t kRegAdcBattL = 0x35;
constexpr uint8_t kRegBatteryPercent = 0xA4;
constexpr uint8_t kRegBattDetCtrl = 0x68;
constexpr uint8_t kRegLdoOnOffCtrl0 = 0x90;
constexpr uint8_t kRegLdoVol2Ctrl = 0x94;  // ALDO3
constexpr uint8_t kRegLdoVol3Ctrl = 0x95;  // ALDO4

constexpr uint8_t kExpectedChipId = 0x4A;
constexpr int kAldoTargetMv = 3300;
constexpr int kAldoStepMv = 100;
constexpr int kAldoMinMv = 500;
constexpr uint8_t kAldoCode3300 =
    static_cast<uint8_t>((kAldoTargetMv - kAldoMinMv) / kAldoStepMv);

i2c_master_bus_handle_t g_bus = nullptr;
i2c_master_dev_handle_t g_dev = nullptr;
bool g_ready = false;

bool ReadReg(uint8_t reg, uint8_t* value) {
  if (g_dev == nullptr || value == nullptr) {
    return false;
  }

  for (int i = 0; i < 3; ++i) {
    if (i2c_master_transmit_receive(g_dev, &reg, 1, value, 1, kI2cTimeoutMs) == ESP_OK) {
      return true;
    }
  }
  return false;
}

bool WriteReg(uint8_t reg, uint8_t value) {
  if (g_dev == nullptr) {
    return false;
  }

  uint8_t payload[2] = {reg, value};
  for (int i = 0; i < 3; ++i) {
    if (i2c_master_transmit(g_dev, payload, sizeof(payload), kI2cTimeoutMs) == ESP_OK) {
      return true;
    }
  }
  return false;
}

bool UpdateRegBits(uint8_t reg, uint8_t mask, uint8_t value) {
  uint8_t cur = 0;
  if (!ReadReg(reg, &cur)) {
    return false;
  }
  const uint8_t next = static_cast<uint8_t>((cur & ~mask) | (value & mask));
  if (next == cur) {
    return true;
  }
  return WriteReg(reg, next);
}

bool EnableRegBits(uint8_t reg, uint8_t bits) {
  uint8_t cur = 0;
  if (!ReadReg(reg, &cur)) {
    return false;
  }
  const uint8_t next = static_cast<uint8_t>(cur | bits);
  if (next == cur) {
    return true;
  }
  return WriteReg(reg, next);
}

bool ConfigureAldo3300(uint8_t reg) {
  // 仅修改电压低 5 位，保留寄存器其余控制位。
  return UpdateRegBits(reg, 0x1F, kAldoCode3300);
}
}  // namespace

bool PowerManager::Init() {
  if (g_ready) {
    return true;
  }

  if (g_bus == nullptr) {
    i2c_master_bus_config_t bus_cfg = {};
    bus_cfg.i2c_port = kI2cPort;
    bus_cfg.scl_io_num = static_cast<gpio_num_t>(kI2cSclPin);
    bus_cfg.sda_io_num = static_cast<gpio_num_t>(kI2cSdaPin);
    bus_cfg.clk_source = I2C_CLK_SRC_DEFAULT;
    bus_cfg.glitch_ignore_cnt = 7;
    bus_cfg.flags.enable_internal_pullup = true;

    if (i2c_new_master_bus(&bus_cfg, &g_bus) != ESP_OK) {
      ESP_LOGE(kTag, "i2c bus init failed");
      return false;
    }
  }

  if (g_dev == nullptr) {
    i2c_device_config_t dev_cfg = {};
    dev_cfg.dev_addr_length = I2C_ADDR_BIT_LEN_7;
    dev_cfg.device_address = kAxp2101Addr;
    dev_cfg.scl_speed_hz = kI2cFreqHz;

    if (i2c_master_bus_add_device(g_bus, &dev_cfg, &g_dev) != ESP_OK) {
      ESP_LOGE(kTag, "pmic device add failed");
      return false;
    }
  }

  uint8_t chip_id = 0;
  if (!ReadReg(kRegChipId, &chip_id)) {
    ESP_LOGE(kTag, "read chip id failed");
    return false;
  }
  if (chip_id != kExpectedChipId) {
    ESP_LOGW(kTag, "unexpected pmic chip id=0x%02x (expect 0x%02x)", chip_id, kExpectedChipId);
  }

  bool ok = true;
  ok = ConfigureAldo3300(kRegLdoVol2Ctrl) && ok;
  ok = ConfigureAldo3300(kRegLdoVol3Ctrl) && ok;
  ok = EnableRegBits(kRegLdoOnOffCtrl0, static_cast<uint8_t>((1U << 2) | (1U << 3))) && ok;
  ok = EnableRegBits(kRegAdcChannelCtrl, 0x01) && ok;  // 电池电压测量
  ok = EnableRegBits(kRegBattDetCtrl, 0x01) && ok;     // 电池检测

  if (!ok) {
    ESP_LOGE(kTag, "pmic register init failed");
    return false;
  }

  g_ready = true;
  ESP_LOGI(kTag, "pmic init done, ALDO3/ALDO4=3300mV");
  return true;
}

bool PowerManager::ReadStatus(PowerStatus* status) {
  if (status == nullptr) {
    return false;
  }

  status->pmic_ready = g_ready;
  if (!g_ready) {
    return false;
  }

  uint8_t status1 = 0;
  uint8_t status2 = 0;
  if (!ReadReg(kRegStatus1, &status1) || !ReadReg(kRegStatus2, &status2)) {
    return false;
  }

  status->vbus_good = (status1 & (1U << 5)) != 0;
  status->battery_present = (status1 & (1U << 3)) != 0;

  const int charge_mode = (status2 >> 5) & 0x03;
  status->charging = (charge_mode == 0x01);
  status->charger_state = status2 & 0x07;

  if (status->battery_present) {
    uint8_t h = 0;
    uint8_t l = 0;
    if (ReadReg(kRegAdcBattH, &h) && ReadReg(kRegAdcBattL, &l)) {
      status->battery_mv = static_cast<int>(((h & 0x1F) << 8) | l);
    }

    uint8_t percent = 0;
    if (ReadReg(kRegBatteryPercent, &percent)) {
      if (percent <= 100) {
        status->battery_percent = percent;
      }
    }
  }

  return true;
}

const char* PowerManager::ChargerStateName(int state) {
  switch (state) {
    case 0:
      return "tri-charge";
    case 1:
      return "pre-charge";
    case 2:
      return "cc";
    case 3:
      return "cv";
    case 4:
      return "done";
    case 5:
      return "stop";
    default:
      return "unknown";
  }
}
