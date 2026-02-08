#include "photopainter_epd.h"

#include <algorithm>
#include <array>
#include <cassert>
#include <cmath>
#include <cstdint>
#include <cstring>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "driver/gpio.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_timer.h"

namespace {
constexpr const char* kTag = "photopainter_epd";

struct PaletteColor {
  uint8_t code;
  uint8_t r;
  uint8_t g;
  uint8_t b;
};

constexpr std::array<PaletteColor, 6> kPalette = {
    PaletteColor{PhotoPainterEpd::kBlack, 0, 0, 0},
    PaletteColor{PhotoPainterEpd::kWhite, 255, 255, 255},
    PaletteColor{PhotoPainterEpd::kYellow, 255, 255, 0},
    PaletteColor{PhotoPainterEpd::kRed, 255, 0, 0},
    PaletteColor{PhotoPainterEpd::kBlue, 0, 0, 255},
    PaletteColor{PhotoPainterEpd::kGreen, 0, 255, 0},
};

struct PaletteMatch {
  bool matched;
  uint8_t code;
};

uint8_t ClampByte(int value) {
  return static_cast<uint8_t>(std::max(0, std::min(255, value)));
}

PaletteMatch MatchPaletteColor(uint8_t r, uint8_t g, uint8_t b, uint8_t tolerance) {
  for (const auto& p : kPalette) {
    const int dr = std::abs(static_cast<int>(r) - static_cast<int>(p.r));
    const int dg = std::abs(static_cast<int>(g) - static_cast<int>(p.g));
    const int db = std::abs(static_cast<int>(b) - static_cast<int>(p.b));
    if (dr <= tolerance && dg <= tolerance && db <= tolerance) {
      return {true, p.code};
    }
  }
  return {false, PhotoPainterEpd::kWhite};
}

void ApplyOrderedDither(int x, int y, uint8_t* r, uint8_t* g, uint8_t* b) {
  static constexpr int8_t kBayer4x4[4][4] = {
      {0, 8, 2, 10},
      {12, 4, 14, 6},
      {3, 11, 1, 9},
      {15, 7, 13, 5},
  };
  constexpr int kDitherStrength = 5;
  const int8_t threshold = static_cast<int8_t>(kBayer4x4[y & 0x3][x & 0x3] - 8);
  const int delta = static_cast<int>(threshold) * kDitherStrength;
  *r = ClampByte(static_cast<int>(*r) + delta);
  *g = ClampByte(static_cast<int>(*g) + delta);
  *b = ClampByte(static_cast<int>(*b) + delta);
}

#pragma pack(push, 1)
struct BmpFileHeader {
  uint16_t type;
  uint32_t size;
  uint16_t reserved1;
  uint16_t reserved2;
  uint32_t offset;
};

struct BmpInfoHeader {
  uint32_t size;
  int32_t width;
  int32_t height;
  uint16_t planes;
  uint16_t bit_count;
  uint32_t compression;
  uint32_t image_size;
  int32_t xppm;
  int32_t yppm;
  uint32_t clr_used;
  uint32_t clr_important;
};
#pragma pack(pop)
}  // namespace

PhotoPainterEpd::PhotoPainterEpd() = default;

PhotoPainterEpd::~PhotoPainterEpd() {
  if (spi_handle_ != nullptr) {
    spi_bus_remove_device(spi_handle_);
    spi_handle_ = nullptr;
    spi_bus_free(SPI3_HOST);
  }
  if (display_buf_ != nullptr) {
    free(display_buf_);
    display_buf_ = nullptr;
  }
  if (tx_buf_ != nullptr) {
    free(tx_buf_);
    tx_buf_ = nullptr;
  }
}

bool PhotoPainterEpd::EnsureBuffers() {
  if (display_buf_ == nullptr) {
    display_buf_ = static_cast<uint8_t*>(
        heap_caps_malloc(display_len_, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT));
  }
  if (tx_buf_ == nullptr) {
    tx_buf_ = static_cast<uint8_t*>(
        heap_caps_malloc(display_len_, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT));
  }
  if (display_buf_ == nullptr || tx_buf_ == nullptr) {
    ESP_LOGE(kTag, "failed to allocate display buffers in PSRAM");
    return false;
  }
  return true;
}

bool PhotoPainterEpd::InitBus() {
  spi_bus_config_t bus_cfg = {};
  bus_cfg.miso_io_num = -1;
  bus_cfg.mosi_io_num = pin_mosi_;
  bus_cfg.sclk_io_num = pin_clk_;
  bus_cfg.quadwp_io_num = -1;
  bus_cfg.quadhd_io_num = -1;
  bus_cfg.max_transfer_sz = display_len_;

  esp_err_t err = spi_bus_initialize(SPI3_HOST, &bus_cfg, SPI_DMA_CH_AUTO);
  if (err != ESP_OK && err != ESP_ERR_INVALID_STATE) {
    ESP_LOGE(kTag, "spi_bus_initialize failed: %s", esp_err_to_name(err));
    return false;
  }

  spi_device_interface_config_t dev_cfg = {};
  dev_cfg.spics_io_num = -1;
  dev_cfg.clock_speed_hz = 40 * 1000 * 1000;
  dev_cfg.mode = 0;
  dev_cfg.queue_size = 7;
  dev_cfg.flags = SPI_DEVICE_HALFDUPLEX;

  err = spi_bus_add_device(SPI3_HOST, &dev_cfg, &spi_handle_);
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "spi_bus_add_device failed: %s", esp_err_to_name(err));
    return false;
  }

  gpio_config_t out_cfg = {};
  out_cfg.intr_type = GPIO_INTR_DISABLE;
  out_cfg.mode = GPIO_MODE_OUTPUT;
  out_cfg.pin_bit_mask = (1ULL << pin_rst_) | (1ULL << pin_dc_) | (1ULL << pin_cs_);
  out_cfg.pull_up_en = GPIO_PULLUP_ENABLE;
  out_cfg.pull_down_en = GPIO_PULLDOWN_DISABLE;
  err = gpio_config(&out_cfg);
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "gpio_config(out) failed: %s", esp_err_to_name(err));
    return false;
  }

  gpio_config_t in_cfg = {};
  in_cfg.intr_type = GPIO_INTR_DISABLE;
  in_cfg.mode = GPIO_MODE_INPUT;
  in_cfg.pin_bit_mask = (1ULL << pin_busy_);
  in_cfg.pull_up_en = GPIO_PULLUP_ENABLE;
  in_cfg.pull_down_en = GPIO_PULLDOWN_DISABLE;
  err = gpio_config(&in_cfg);
  if (err != ESP_OK) {
    ESP_LOGE(kTag, "gpio_config(in) failed: %s", esp_err_to_name(err));
    return false;
  }

  gpio_set_level(static_cast<gpio_num_t>(pin_rst_), 1);
  gpio_set_level(static_cast<gpio_num_t>(pin_cs_), 1);
  gpio_set_level(static_cast<gpio_num_t>(pin_dc_), 1);
  return true;
}

bool PhotoPainterEpd::Init() {
  if (initialized_) {
    return true;
  }
  if (!EnsureBuffers() || !InitBus()) {
    return false;
  }
  ApplyPanelInitSequence();
  initialized_ = true;
  ESP_LOGI(kTag, "epd init done");
  return true;
}

void PhotoPainterEpd::Reset() {
  gpio_set_level(static_cast<gpio_num_t>(pin_rst_), 1);
  vTaskDelay(pdMS_TO_TICKS(50));
  gpio_set_level(static_cast<gpio_num_t>(pin_rst_), 0);
  vTaskDelay(pdMS_TO_TICKS(20));
  gpio_set_level(static_cast<gpio_num_t>(pin_rst_), 1);
  vTaskDelay(pdMS_TO_TICKS(50));
}

void PhotoPainterEpd::WaitBusy() {
  while (gpio_get_level(static_cast<gpio_num_t>(pin_busy_)) == 0) {
    vTaskDelay(pdMS_TO_TICKS(10));
  }
}

void PhotoPainterEpd::WriteByte(uint8_t value) {
  spi_transaction_t t = {};
  t.length = 8;
  t.tx_buffer = &value;
  ESP_ERROR_CHECK(spi_device_polling_transmit(spi_handle_, &t));
}

void PhotoPainterEpd::WriteCommand(uint8_t cmd) {
  gpio_set_level(static_cast<gpio_num_t>(pin_dc_), 0);
  gpio_set_level(static_cast<gpio_num_t>(pin_cs_), 0);
  WriteByte(cmd);
  gpio_set_level(static_cast<gpio_num_t>(pin_cs_), 1);
}

void PhotoPainterEpd::WriteData(uint8_t data) {
  gpio_set_level(static_cast<gpio_num_t>(pin_dc_), 1);
  gpio_set_level(static_cast<gpio_num_t>(pin_cs_), 0);
  WriteByte(data);
  gpio_set_level(static_cast<gpio_num_t>(pin_cs_), 1);
}

void PhotoPainterEpd::WriteBuffer(const uint8_t* data, size_t len) {
  gpio_set_level(static_cast<gpio_num_t>(pin_dc_), 1);
  gpio_set_level(static_cast<gpio_num_t>(pin_cs_), 0);

  spi_transaction_t t = {};
  constexpr size_t kChunk = 5000;
  size_t offset = 0;
  while (offset < len) {
    const size_t chunk = std::min(kChunk, len - offset);
    t.length = static_cast<uint32_t>(chunk * 8);
    t.tx_buffer = data + offset;
    ESP_ERROR_CHECK(spi_device_polling_transmit(spi_handle_, &t));
    offset += chunk;
  }

  gpio_set_level(static_cast<gpio_num_t>(pin_cs_), 1);
}

void PhotoPainterEpd::TurnOnDisplay() {
  WriteCommand(0x04);
  WaitBusy();

  WriteCommand(0x06);
  WriteData(0x6F);
  WriteData(0x1F);
  WriteData(0x17);
  WriteData(0x49);

  WriteCommand(0x12);
  WriteData(0x00);
  WaitBusy();

  WriteCommand(0x02);
  WriteData(0x00);
  WaitBusy();
}

void PhotoPainterEpd::ApplyPanelInitSequence() {
  Reset();
  WaitBusy();
  vTaskDelay(pdMS_TO_TICKS(50));

  WriteCommand(0xAA);
  WriteData(0x49);
  WriteData(0x55);
  WriteData(0x20);
  WriteData(0x08);
  WriteData(0x09);
  WriteData(0x18);

  WriteCommand(0x01);
  WriteData(0x3F);

  WriteCommand(0x00);
  WriteData(0x5F);
  WriteData(0x69);

  WriteCommand(0x03);
  WriteData(0x00);
  WriteData(0x54);
  WriteData(0x00);
  WriteData(0x44);

  WriteCommand(0x05);
  WriteData(0x40);
  WriteData(0x1F);
  WriteData(0x1F);
  WriteData(0x2C);

  WriteCommand(0x06);
  WriteData(0x6F);
  WriteData(0x1F);
  WriteData(0x17);
  WriteData(0x49);

  WriteCommand(0x08);
  WriteData(0x6F);
  WriteData(0x1F);
  WriteData(0x1F);
  WriteData(0x22);

  WriteCommand(0x30);
  WriteData(0x03);

  WriteCommand(0x50);
  WriteData(0x3F);

  WriteCommand(0x60);
  WriteData(0x02);
  WriteData(0x00);

  WriteCommand(0x61);
  WriteData(0x03);
  WriteData(0x20);
  WriteData(0x01);
  WriteData(0xE0);

  WriteCommand(0x84);
  WriteData(0x01);

  WriteCommand(0xE3);
  WriteData(0x2F);

  WriteCommand(0x04);
  WaitBusy();

  ClearDisplayBuffer(kWhite);
  FlushDisplay();
}

void PhotoPainterEpd::SetPackedPixel(uint8_t* buf, int width, int x, int y, uint8_t px) {
  const int index = (y * width + x) >> 1;
  if ((x & 1) == 0) {
    buf[index] = static_cast<uint8_t>((buf[index] & 0x0F) | ((px & 0x0F) << 4));
  } else {
    buf[index] = static_cast<uint8_t>((buf[index] & 0xF0) | (px & 0x0F));
  }
}

uint8_t PhotoPainterEpd::GetPackedPixel(const uint8_t* buf, int width, int x, int y) {
  const int index = (y * width + x) >> 1;
  const uint8_t value = buf[index];
  return ((x & 1) == 0) ? static_cast<uint8_t>((value >> 4) & 0x0F)
                        : static_cast<uint8_t>(value & 0x0F);
}

void PhotoPainterEpd::RotateBuffer(uint8_t rotation) {
  rotation = rotation % 4;
  if (rotation == 0) {
    memcpy(tx_buf_, display_buf_, display_len_);
    return;
  }

  if (rotation == 2) {
    for (int y = 0; y < kPanelHeight; ++y) {
      for (int x = 0; x < kPanelWidth; ++x) {
        const uint8_t px = GetPackedPixel(display_buf_, kPanelWidth, x, y);
        const int nx = kPanelWidth - 1 - x;
        const int ny = kPanelHeight - 1 - y;
        SetPackedPixel(tx_buf_, kPanelWidth, nx, ny, px);
      }
    }
    return;
  }

  // 7.3 寸面板的内存布局是 800x480，90/270 度旋转会导致无效地址映射。
  ESP_LOGW(kTag, "unsupported panel_rotation=%u, fallback to 180", rotation);
  for (int y = 0; y < kPanelHeight; ++y) {
    for (int x = 0; x < kPanelWidth; ++x) {
      const uint8_t px = GetPackedPixel(display_buf_, kPanelWidth, x, y);
      const int nx = kPanelWidth - 1 - x;
      const int ny = kPanelHeight - 1 - y;
      SetPackedPixel(tx_buf_, kPanelWidth, nx, ny, px);
    }
  }
}

uint8_t PhotoPainterEpd::QuantizeColor(uint8_t r, uint8_t g, uint8_t b) const {
  uint8_t best_code = kWhite;
  int best_dist = INT32_MAX;
  for (const auto& p : kPalette) {
    const int dr = static_cast<int>(r) - static_cast<int>(p.r);
    const int dg = static_cast<int>(g) - static_cast<int>(p.g);
    const int db = static_cast<int>(b) - static_cast<int>(p.b);
    const int dist = dr * dr + dg * dg + db * db;
    if (dist < best_dist) {
      best_dist = dist;
      best_code = p.code;
    }
  }
  return best_code;
}

void PhotoPainterEpd::ClearDisplayBuffer(EpdColor color) {
  const uint8_t packed = static_cast<uint8_t>((static_cast<uint8_t>(color) << 4) |
                                              static_cast<uint8_t>(color));
  memset(display_buf_, packed, display_len_);
}

void PhotoPainterEpd::FlushDisplay() {
  WriteCommand(0x10);
  WriteBuffer(tx_buf_, static_cast<size_t>(display_len_));
  TurnOnDisplay();
}

void PhotoPainterEpd::Clear(EpdColor color) {
  if (!initialized_) {
    return;
  }
  ClearDisplayBuffer(color);
  RotateBuffer(0);
  FlushDisplay();
}

bool PhotoPainterEpd::DrawBmp24(const uint8_t* bmp, size_t len, const RenderOptions& options) {
  if (!initialized_ || bmp == nullptr || len < sizeof(BmpFileHeader) + sizeof(BmpInfoHeader)) {
    return false;
  }

  const auto* file = reinterpret_cast<const BmpFileHeader*>(bmp);
  if (file->type != 0x4D42) {
    ESP_LOGE(kTag, "invalid bmp magic: 0x%04x", file->type);
    return false;
  }

  const auto* info = reinterpret_cast<const BmpInfoHeader*>(bmp + sizeof(BmpFileHeader));
  if (info->size < sizeof(BmpInfoHeader) || info->planes != 1 || info->bit_count != 24 ||
      info->compression != 0) {
    ESP_LOGE(kTag, "unsupported bmp: info_size=%lu bit_count=%u compression=%lu",
             static_cast<unsigned long>(info->size), info->bit_count,
             static_cast<unsigned long>(info->compression));
    return false;
  }

  const int in_w = info->width;
  const int in_h_abs = (info->height < 0) ? -info->height : info->height;
  const bool bottom_up = info->height > 0;

  if (!((in_w == kPanelWidth && in_h_abs == kPanelHeight) ||
        (in_w == kPanelHeight && in_h_abs == kPanelWidth))) {
    ESP_LOGE(kTag, "unsupported bmp dimension: %dx%d", in_w, in_h_abs);
    return false;
  }

  const size_t row_stride = static_cast<size_t>((in_w * 3 + 3) & ~3);
  const size_t need = static_cast<size_t>(file->offset) + row_stride * static_cast<size_t>(in_h_abs);
  if (need > len) {
    ESP_LOGE(kTag, "bmp size mismatch: need=%u got=%u", static_cast<unsigned>(need),
             static_cast<unsigned>(len));
    return false;
  }

  const uint8_t* pixels = bmp + file->offset;
  ClearDisplayBuffer(kWhite);

  const int64_t render_start_us = esp_timer_get_time();

  const uint8_t color_mode =
      std::min<uint8_t>(options.color_process_mode, kColorProcessAssumeSixColor);
  const uint8_t dithering_mode = std::min<uint8_t>(options.dithering_mode, kDitherOrdered);
  const uint8_t tolerance = std::min<uint8_t>(options.six_color_tolerance, 64);

  auto get_rgb = [&](int sx, int sy, uint8_t& r, uint8_t& g, uint8_t& b) {
    const int row = bottom_up ? (in_h_abs - 1 - sy) : sy;
    const uint8_t* p = pixels + static_cast<size_t>(row) * row_stride + static_cast<size_t>(sx) * 3;
    b = p[0];
    g = p[1];
    r = p[2];
  };

  auto map_source_xy = [&](int x, int y, int* sx, int* sy) {
    if (in_w == kPanelWidth && in_h_abs == kPanelHeight) {
      *sx = x;
      *sy = y;
      return;
    }
    // 输入是 480x800：先旋转成 800x480。
    *sx = y;
    *sy = in_h_abs - 1 - x;
  };

  bool treat_as_six_color = (color_mode == kColorProcessAssumeSixColor);
  int64_t detect_cost_us = 0;
  if (color_mode == kColorProcessAuto) {
    const int64_t detect_start_us = esp_timer_get_time();
    treat_as_six_color = true;
    for (int y = 0; y < kPanelHeight && treat_as_six_color; ++y) {
      for (int x = 0; x < kPanelWidth; ++x) {
        int sx = 0;
        int sy = 0;
        map_source_xy(x, y, &sx, &sy);
        uint8_t r = 255;
        uint8_t g = 255;
        uint8_t b = 255;
        get_rgb(sx, sy, r, g, b);
        if (!MatchPaletteColor(r, g, b, tolerance).matched) {
          treat_as_six_color = false;
          break;
        }
      }
    }
    detect_cost_us = esp_timer_get_time() - detect_start_us;
  }

  const bool use_dither = !treat_as_six_color && (dithering_mode == kDitherOrdered);

  for (int y = 0; y < kPanelHeight; ++y) {
    for (int x = 0; x < kPanelWidth; ++x) {
      int sx = 0;
      int sy = 0;
      map_source_xy(x, y, &sx, &sy);
      uint8_t r = 255;
      uint8_t g = 255;
      uint8_t b = 255;
      get_rgb(sx, sy, r, g, b);

      uint8_t color_code = kWhite;
      if (treat_as_six_color) {
        const auto match = MatchPaletteColor(r, g, b, tolerance);
        color_code = match.matched ? match.code : QuantizeColor(r, g, b);
      } else {
        if (use_dither) {
          ApplyOrderedDither(x, y, &r, &g, &b);
        }
        color_code = QuantizeColor(r, g, b);
      }

      SetPackedPixel(display_buf_, kPanelWidth, x, y, color_code);
    }
  }

  ESP_LOGI(kTag, "bmp color process: mode=%s dither=%s tolerance=%u",
           treat_as_six_color ? "passthrough-6color" : "convert",
           use_dither ? "ordered" : "none", static_cast<unsigned>(tolerance));

  const int64_t render_cost_us = esp_timer_get_time() - render_start_us;
  ESP_LOGI(kTag,
           "bmp process cost: detect=%lldms total=%lldms pixels=%u",
           static_cast<long long>(detect_cost_us / 1000),
           static_cast<long long>(render_cost_us / 1000),
           static_cast<unsigned>(kPanelWidth * kPanelHeight));

  RotateBuffer(options.panel_rotation);
  FlushDisplay();
  return true;
}
