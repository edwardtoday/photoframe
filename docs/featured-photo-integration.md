# Featured Photo 固件集成需求（初版）

## 背景

- 设备：Waveshare `ESP32-S3-PhotoPainter`（480x800 彩色墨水屏）
- 上游固件：`upstream/ESP32-S3-PhotoPainter/`
- 图片服务：`immich-featured-today`，部署在 `http://192.168.58.113:8000`

## 业务目标

1. 首次启动或按键触发时进入配网流程，保存 Wi-Fi 凭据。
2. 正常运行时，每小时拉取一次当日图片并刷新屏幕。
3. 刷新完成后进入深度睡眠以降低功耗。

## 图片接口约定（当前）

- URL 模板：
  - `http://192.168.58.113:8000/image/480x800?date=YYYY-MM-DD`
- 示例（2026-02-07）：
  - `http://192.168.58.113:8000/image/480x800?date=2026-02-07`
- 本地实测（2026-02-07）：
  - HTTP `200 OK`
  - `Content-Type: image/bmp`
  - `Content-Length: 1152054`

> 注：当前接口需要 `date` 参数；如果固件希望“自动今天”，可在服务端新增默认日期逻辑，或固件先通过 NTP 获取日期后拼接参数。

## 固件侧建议实现点（面向后续开发）

- 网络：
  - 使用 ESP-IDF Wi-Fi STA + NVS 持久化凭据。
  - 配网可选 SoftAP + captive portal，或先做串口配置版本。
- 拉图：
  - 使用 HTTP Client 分块下载 BMP 到 PSRAM/SD，避免一次性占满 RAM。
  - 校验 BMP 头（分辨率 480x800、位深 24bit）后再显示。
- 刷新：
  - 复用上游墨水屏驱动路径，补充“从 HTTP 缓冲区渲染”的入口。
- 省电：
  - 用 RTC 定时唤醒（3600s）+ 深度睡眠。
  - 失败重试采用指数退避，避免频繁唤醒耗电。

