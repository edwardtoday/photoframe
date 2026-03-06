#include "power_manager.h"

#include <algorithm>

#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

namespace {
constexpr const char* kTag = "power_manager";

constexpr i2c_port_num_t kI2cPort = I2C_NUM_0;
constexpr int kI2cSclPin = 48;
constexpr int kI2cSdaPin = 47;
constexpr gpio_num_t kAxpIrqPin = GPIO_NUM_21;
// 官方 demo 走 100k；部分板子在弱上拉/长走线场景下高频更容易读写失败，导致 PMIC 不可用并显著耗电。
constexpr int kI2cFreqHz = 100000;
constexpr int kI2cTimeoutMs = 1000;

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

bool SampleI2cLineLevels(int* scl, int* sda) {
  if (scl == nullptr || sda == nullptr) {
    return false;
  }
  // 读电平前先确保是 GPIO 功能，避免被外设复用导致读数不可信。
  (void)gpio_reset_pin(static_cast<gpio_num_t>(kI2cSclPin));
  (void)gpio_reset_pin(static_cast<gpio_num_t>(kI2cSdaPin));
  (void)gpio_set_direction(static_cast<gpio_num_t>(kI2cSclPin), GPIO_MODE_INPUT);
  (void)gpio_set_direction(static_cast<gpio_num_t>(kI2cSdaPin), GPIO_MODE_INPUT);
  (void)gpio_pullup_en(static_cast<gpio_num_t>(kI2cSclPin));
  (void)gpio_pullup_en(static_cast<gpio_num_t>(kI2cSdaPin));
  *scl = gpio_get_level(static_cast<gpio_num_t>(kI2cSclPin));
  *sda = gpio_get_level(static_cast<gpio_num_t>(kI2cSdaPin));
  return true;
}

void LogI2cLineLevels(const char* hint) {
  int scl = -1;
  int sda = -1;
  (void)SampleI2cLineLevels(&scl, &sda);
  ESP_LOGI(kTag, "i2c lines(%s): scl=%d sda=%d", hint == nullptr ? "-" : hint, scl, sda);
}

void PulseAxpIrqPin() {
  // 参考官方 demo：上电时拉低再拉高 IRQ/WAKE 脚，帮助 PMIC 从异常状态恢复。
  gpio_config_t gpio_conf = {};
  gpio_conf.intr_type = GPIO_INTR_DISABLE;
  gpio_conf.mode = GPIO_MODE_OUTPUT;
  gpio_conf.pin_bit_mask = (1ULL << static_cast<uint64_t>(kAxpIrqPin));
  gpio_conf.pull_down_en = GPIO_PULLDOWN_DISABLE;
  gpio_conf.pull_up_en = GPIO_PULLUP_ENABLE;
  if (gpio_config(&gpio_conf) != ESP_OK) {
    ESP_LOGW(kTag, "axp irq pin config failed");
    return;
  }
  gpio_set_level(kAxpIrqPin, 0);
  vTaskDelay(pdMS_TO_TICKS(100));
  gpio_set_level(kAxpIrqPin, 1);
  vTaskDelay(pdMS_TO_TICKS(200));
}

bool RecoverI2cBusByBitBang() {
  // I2C 总线恢复：当某个从设备在传输中途掉电/复位，可能一直拉低 SDA，导致后续事务全部失败。
  // 通过手工脉冲 SCL 释放从设备状态机，然后发一个 STOP。
  gpio_config_t cfg = {};
  cfg.intr_type = GPIO_INTR_DISABLE;
  cfg.mode = GPIO_MODE_INPUT_OUTPUT_OD;
  cfg.pin_bit_mask =
      (1ULL << static_cast<uint64_t>(kI2cSclPin)) | (1ULL << static_cast<uint64_t>(kI2cSdaPin));
  cfg.pull_down_en = GPIO_PULLDOWN_DISABLE;
  cfg.pull_up_en = GPIO_PULLUP_ENABLE;

  (void)gpio_reset_pin(static_cast<gpio_num_t>(kI2cSclPin));
  (void)gpio_reset_pin(static_cast<gpio_num_t>(kI2cSdaPin));
  if (gpio_config(&cfg) != ESP_OK) {
    return false;
  }

  // 释放总线（开漏输出写 1=释放）。
  gpio_set_level(static_cast<gpio_num_t>(kI2cSdaPin), 1);
  gpio_set_level(static_cast<gpio_num_t>(kI2cSclPin), 1);
  vTaskDelay(pdMS_TO_TICKS(2));

  int scl = gpio_get_level(static_cast<gpio_num_t>(kI2cSclPin));
  int sda = gpio_get_level(static_cast<gpio_num_t>(kI2cSdaPin));
  ESP_LOGI(kTag, "i2c recover start: scl=%d sda=%d", scl, sda);

  if (scl == 0) {
    // 参考官方上电序列先拉 IRQ，再尝试一次总线恢复。
    ESP_LOGW(kTag, "i2c recover detected scl low, pulse axp irq");
    PulseAxpIrqPin();
    gpio_set_level(static_cast<gpio_num_t>(kI2cSdaPin), 1);
    gpio_set_level(static_cast<gpio_num_t>(kI2cSclPin), 1);
    vTaskDelay(pdMS_TO_TICKS(2));
    scl = gpio_get_level(static_cast<gpio_num_t>(kI2cSclPin));
    sda = gpio_get_level(static_cast<gpio_num_t>(kI2cSdaPin));
    ESP_LOGI(kTag, "i2c recover after axp pulse: scl=%d sda=%d", scl, sda);
    if (scl == 0) {
      // SCL 仍被拉低（硬件层异常），本轮直接失败，避免空耗 10+ 秒重试。
      ESP_LOGW(kTag, "i2c recover abort: scl still stuck low");
      return false;
    }
  }

  // 若 SDA 低，尝试打 9 个时钟把从设备移出“等待 ACK/数据”状态。
  for (int i = 0; i < 9 && sda == 0; ++i) {
    gpio_set_level(static_cast<gpio_num_t>(kI2cSclPin), 0);
    vTaskDelay(pdMS_TO_TICKS(1));
    gpio_set_level(static_cast<gpio_num_t>(kI2cSclPin), 1);
    vTaskDelay(pdMS_TO_TICKS(1));
    sda = gpio_get_level(static_cast<gpio_num_t>(kI2cSdaPin));
  }

  // 发 STOP：SCL 高时 SDA 从低到高。
  gpio_set_level(static_cast<gpio_num_t>(kI2cSdaPin), 0);
  vTaskDelay(pdMS_TO_TICKS(1));
  gpio_set_level(static_cast<gpio_num_t>(kI2cSclPin), 1);
  vTaskDelay(pdMS_TO_TICKS(1));
  gpio_set_level(static_cast<gpio_num_t>(kI2cSdaPin), 1);
  vTaskDelay(pdMS_TO_TICKS(2));

  scl = gpio_get_level(static_cast<gpio_num_t>(kI2cSclPin));
  sda = gpio_get_level(static_cast<gpio_num_t>(kI2cSdaPin));
  ESP_LOGI(kTag, "i2c recover done: scl=%d sda=%d", scl, sda);
  return (scl == 1 && sda == 1);
}

esp_err_t WaitBusIdle() {
  if (g_bus == nullptr) {
    return ESP_OK;
  }
  return i2c_master_bus_wait_all_done(g_bus, pdMS_TO_TICKS(1000));
}

void ResetI2cBus() {
  // 失败恢复：若 I2C/PMIC 卡死（例如上电瞬间 ACK 异常），需要彻底重建 bus/dev 句柄。
  if (g_dev != nullptr) {
    esp_err_t err = i2c_master_bus_rm_device(g_dev);
    if (err != ESP_OK) {
      ESP_LOGW(kTag, "i2c rm device failed: %s", esp_err_to_name(err));
    }
    g_dev = nullptr;
  }
  if (g_bus != nullptr) {
    esp_err_t err = i2c_del_master_bus(g_bus);
    if (err != ESP_OK) {
      ESP_LOGW(kTag, "i2c del bus failed: %s", esp_err_to_name(err));
    }
    g_bus = nullptr;
  }
  g_ready = false;
}

bool ReadReg(uint8_t reg, uint8_t* value) {
  if (g_dev == nullptr || value == nullptr) {
    return false;
  }

  (void)WaitBusIdle();

  esp_err_t last_err = ESP_OK;
  for (int i = 0; i < 3; ++i) {
    last_err = i2c_master_transmit_receive(g_dev, &reg, 1, value, 1, kI2cTimeoutMs);
    if (last_err == ESP_OK) {
      return true;
    }
    // 读失败时尝试 reset bus（弱上拉/瞬态干扰下可恢复），然后再重试。
    if (g_bus != nullptr) {
      (void)i2c_master_bus_reset(g_bus);
    }
    if (i + 1 < 3) {
      vTaskDelay(pdMS_TO_TICKS(20));
    }
  }
  ESP_LOGW(kTag, "i2c read reg 0x%02x failed: %s", reg, esp_err_to_name(last_err));
  return false;
}

bool WriteReg(uint8_t reg, uint8_t value) {
  if (g_dev == nullptr) {
    return false;
  }

  (void)WaitBusIdle();

  uint8_t payload[2] = {reg, value};
  esp_err_t last_err = ESP_OK;
  for (int i = 0; i < 3; ++i) {
    last_err = i2c_master_transmit(g_dev, payload, sizeof(payload), kI2cTimeoutMs);
    if (last_err == ESP_OK) {
      return true;
    }
    if (g_bus != nullptr) {
      (void)i2c_master_bus_reset(g_bus);
    }
    if (i + 1 < 3) {
      vTaskDelay(pdMS_TO_TICKS(20));
    }
  }
  ESP_LOGW(kTag, "i2c write reg 0x%02x failed: %s", reg, esp_err_to_name(last_err));
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

bool DisableRegBits(uint8_t reg, uint8_t bits) {
  uint8_t cur = 0;
  if (!ReadReg(reg, &cur)) {
    return false;
  }
  const uint8_t next = static_cast<uint8_t>(cur & ~bits);
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
    LogI2cLineLevels("before init");
    const bool bus_recovered = RecoverI2cBusByBitBang();
    if (!bus_recovered) {
      int scl = -1;
      int sda = -1;
      if (SampleI2cLineLevels(&scl, &sda) && scl == 0) {
        ESP_LOGW(kTag, "i2c scl still low after recover, skip pmic init this round");
        return false;
      }
    }
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
      ResetI2cBus();
      return false;
    }
  }

  if (g_bus != nullptr) {
    // 新建 bus 后先做一次 reset，尽量清掉上电瞬态带来的“总线占用/时序错位”。
    (void)i2c_master_bus_reset(g_bus);
  }

  uint8_t chip_id = 0;
  bool chip_ok = false;
  // 刚上电时 I2C/PMIC 可能尚未完全稳定，增加少量重试避免偶发“整轮电量缺失”。
  for (int attempt = 1; attempt <= 5; ++attempt) {
    if (ReadReg(kRegChipId, &chip_id)) {
      chip_ok = true;
      break;
    }
    if (attempt < 5) {
      vTaskDelay(pdMS_TO_TICKS(50));
    }
  }
  if (!chip_ok) {
    ESP_LOGE(kTag, "read chip id failed");
    LogI2cLineLevels("chip id failed");
    ResetI2cBus();
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
    ResetI2cBus();
    return false;
  }

  g_ready = true;
  ESP_LOGI(kTag, "pmic init done, ALDO3/ALDO4=3300mV");
  return true;
}

void PowerManager::PrepareForDeepSleep() {
  if (!g_ready) {
    return;
  }

  // 仅关闭采样通道，不再关闭 ALDO3/ALDO4。
  // 实测关闭 ALDO 会导致下次唤醒后 I2C 总线被外设拉低（SCL/SDA=0），
  // 进而 PMIC 不可读、画面不刷新、电量不上报。
  bool ok = true;
  ok = DisableRegBits(kRegAdcChannelCtrl, 0x01) && ok;  // 关闭电池电压测量通道
  ok = DisableRegBits(kRegBattDetCtrl, 0x01) && ok;     // 关闭电池检测

  if (ok) {
    ESP_LOGI(kTag, "pmic prepared for deep sleep (adc/battdet off)");
  } else {
    ESP_LOGW(kTag, "pmic deep sleep prep partially failed");
  }
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
    ESP_LOGW(kTag, "read status regs failed, reset i2c bus");
    LogI2cLineLevels("status regs failed");
    ResetI2cBus();
    return false;
  }

  status->vbus_good = (status1 & (1U << 5)) != 0;
  status->battery_present = (status1 & (1U << 3)) != 0;

  const int charge_mode = (status2 >> 5) & 0x03;
  status->charging = (charge_mode == 0x01);
  status->charger_state = status2 & 0x07;

  // 兼容策略：不要强依赖 battery_present 位。
  // 实测某些板子/时序下 battery_present 可能短暂不稳定，但 0xA4 电量寄存器仍然可读。
  uint8_t percent = 0;
  if (ReadReg(kRegBatteryPercent, &percent) && percent <= 100) {
    status->battery_percent = percent;
  }

  uint8_t h = 0;
  uint8_t l = 0;
  if (ReadReg(kRegAdcBattH, &h) && ReadReg(kRegAdcBattL, &l)) {
    const int mv = static_cast<int>(((h & 0x1F) << 8) | l);
    if (mv > 0) {
      status->battery_mv = mv;
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
