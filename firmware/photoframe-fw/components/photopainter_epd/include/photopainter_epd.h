#pragma once

#include <cstddef>
#include <cstdint>

#include "driver/spi_master.h"

class PhotoPainterEpd {
 public:
  enum EpdColor : uint8_t {
    kBlack = 0,
    kWhite = 1,
    kYellow = 2,
    kRed = 3,
    kBlue = 5,
    kGreen = 6,
  };

  PhotoPainterEpd();
  ~PhotoPainterEpd();

  enum ColorProcessMode : uint8_t {
    kColorProcessAuto = 0,
    kColorProcessForceConvert = 1,
    kColorProcessAssumeSixColor = 2,
  };

  enum DitheringMode : uint8_t {
    kDitherNone = 0,
    kDitherOrdered = 1,
  };

  struct RenderOptions {
    uint8_t panel_rotation = 2;
    uint8_t color_process_mode = kColorProcessAuto;
    uint8_t dithering_mode = kDitherOrdered;
    uint8_t six_color_tolerance = 0;
  };

  bool Init();
  void Clear(EpdColor color = kWhite);
  bool DrawBmp24(const uint8_t* bmp, size_t len, const RenderOptions& options);
  // 输入为 RGB888（每像素 3 字节，R/G/B），分辨率需为 800x480 或 480x800。
  bool DrawRgb24(const uint8_t* rgb, int width, int height, const RenderOptions& options);

 private:
  static constexpr int kPanelWidth = 800;
  static constexpr int kPanelHeight = 480;

  bool EnsureBuffers();
  bool InitBus();
  void Reset();
  bool WaitBusy(const char* stage, int timeout_ms = 45000);
  void WriteByte(uint8_t value);
  void WriteCommand(uint8_t cmd);
  void WriteData(uint8_t data);
  void WriteBuffer(const uint8_t* data, size_t len);
  bool TurnOnDisplay();
  bool ApplyPanelInitSequence();
  void RotateBuffer(uint8_t rotation);
  uint8_t QuantizeColor(uint8_t r, uint8_t g, uint8_t b) const;
  void SetPackedPixel(uint8_t* buf, int width, int x, int y, uint8_t px);
  uint8_t GetPackedPixel(const uint8_t* buf, int width, int x, int y);

  void ClearDisplayBuffer(EpdColor color);
  bool FlushDisplay();

  bool initialized_ = false;
  int display_len_ = (kPanelWidth * kPanelHeight) / 2;

  int pin_mosi_ = 11;
  int pin_clk_ = 10;
  int pin_dc_ = 8;
  int pin_cs_ = 9;
  int pin_rst_ = 12;
  int pin_busy_ = 13;

  uint8_t* display_buf_ = nullptr;
  uint8_t* tx_buf_ = nullptr;
  spi_device_handle_t spi_handle_ = nullptr;
};
