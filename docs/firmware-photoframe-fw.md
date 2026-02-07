# photoframe-fw（自有固件）

本目录是你的私有固件实现，不改 `upstream/`。

- 工程目录：`firmware/photoframe-fw/`
- 目标芯片：`ESP32-S3`
- 面板：Waveshare 7.3" 彩色墨水屏（引脚沿用上游）

## 已实现能力（阶段 C + D）

1. **配网（Captive Portal）**
   - 长按按键（GPIO4）上电 3 秒进入配网模式。
   - 设备启动 AP：`PhotoFrame-Setup` / `12345678`
   - 浏览器访问 `http://192.168.4.1/`
   - 支持扫描附近 Wi-Fi、填写 SSID/密码并保存。

2. **定时拉图 + 刷新 + 深睡**
   - 正常模式下连接家里 Wi-Fi。
   - 根据配置 URL 拉取 BMP（默认模板：`.../image/480x800?date=%DATE%`）。
   - 成功后刷新墨水屏，进入深度睡眠。
   - 默认 60 分钟唤醒一次。

3. **失败重试策略（指数退避）**
   - 失败后按 `retry_base_minutes * 2^(failure_count-1)` 退避。
   - 退避上限 `retry_max_minutes`。
   - 连续失败计数持久化到 NVS。

4. **按键强制刷新**
   - 深睡时按键可触发外部唤醒（EXT1）。
   - 唤醒后执行强制拉图（即使图片 hash 未变化也允许刷新）。

5. **断网恢复机制**
   - STA 模式自动重连 + 有限重试。
   - 连接失败进入退避深睡，下一轮自动恢复。

6. **配置查询/修改接口**
   - `GET /api/config`：查询当前配置与运行状态。
   - `POST /api/config`：更新 Wi-Fi、URL、轮询间隔、重试参数、时区、旋转参数。
   - `GET /api/wifi/scan`：扫描 AP 列表。

## 配置项（NVS 持久化）

- `wifi_ssid` / `wifi_password`
- `image_url_template`（支持 `%DATE%` 占位）
- `interval_minutes`（默认 60）
- `retry_base_minutes` / `retry_max_minutes`
- `max_failure_before_long_sleep`
- `display_rotation`（当前支持 `0` 或 `2`）
- `timezone`（默认 `UTC`）
- `last_image_sha256`（用于避免重复刷新）

## 接口示例（Portal 模式）

默认在无 Wi-Fi 凭据、或上电长按按键 3 秒后进入 Portal 模式。

```bash
# 查询当前配置与运行状态
curl -s http://192.168.4.1/api/config

# 扫描附近 Wi-Fi
curl -s http://192.168.4.1/api/wifi/scan

# 更新轮询与重试参数
curl -s -X POST http://192.168.4.1/api/config \
  -H "Content-Type: application/json" \
  --data-binary @- <<JSON
{
  "wifi_ssid": "YourWiFi",
  "wifi_password": "YourPassword",
  "image_url_template": "http://192.168.58.113:8000/image/480x800?date=%DATE%",
  "interval_minutes": 60,
  "retry_base_minutes": 5,
  "retry_max_minutes": 240,
  "max_failure_before_long_sleep": 24,
  "display_rotation": 2,
  "timezone": "Asia/Shanghai"
}
JSON
```

## 失败重试行为

- 失败后休眠时长：`retry_base_minutes * 2^(failure_count-1)`
- 上限：`retry_max_minutes`
- `failure_count` 达到 `max_failure_before_long_sleep` 后，保持最长退避间隔
- 任意一次成功拉图后，`failure_count` 归零

## 编译

```bash
scripts/build-photoframe-fw.sh
```

## 烧录

```bash
scripts/flash-host.py \
  --project-dir firmware/photoframe-fw \
  --port /dev/cu.usbmodemXXXX
```

## 串口监控

```bash
scripts/monitor-host.sh /dev/cu.usbmodemXXXX 115200
```
