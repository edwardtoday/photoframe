# photoframe-fw（自有固件）

本目录是你的私有固件实现，不改 `upstream/`。

- 工程目录：`firmware/photoframe-fw/`
- 目标芯片：`ESP32-S3`
- 面板：Waveshare 7.3" 彩色墨水屏（引脚沿用上游）

## 已实现能力（阶段 C + D + 编排接入）

1. **配网（Captive Portal）**
   - 长按按键（GPIO4）上电 3 秒进入配网模式。
   - 设备启动 AP：`PhotoFrame-Setup` / `12345678`
   - AP 网段固定为 `192.168.73.1/24`（避免与常见 `192.168.4.1` 冲突）。
   - 浏览器访问 `http://192.168.73.1/`
   - 支持扫描附近 Wi-Fi、填写 SSID/密码并保存。

2. **定时拉图 + 刷新 + 深睡**
   - 正常模式下连接家里 Wi-Fi。
   - 支持两种拉图来源：
     - 编排服务（推荐）：`/api/v1/device/next` 下发当前应显示图片 URL
     - 传统模板：`image_url_template`（支持 `%DATE%` 与 `%DEVICE_ID%` 占位）
   - 自动判断图片是否已是 6 色：已是则直通显示，否则设备端转换。
   - 串口日志会输出处理耗时（`detect=xxms total=xxms`），便于评估设备端转换成本。
   - 成功后刷新墨水屏，进入深度睡眠。

3. **失败重试策略（指数退避）**
   - 失败后按 `retry_base_minutes * 2^(failure_count-1)` 退避。
   - 退避上限 `retry_max_minutes`。
   - 连续失败计数持久化到 NVS。

4. **按键唤醒增强（强制刷新 + 本地配置窗口）**
   - 深睡时按键可触发外部唤醒（EXT1）。
   - 按键唤醒后：
     - 强制拉图（即使图片 hash 未变化也允许刷新）
     - 在已连接家里 Wi-Fi 的情况下，开放 120 秒本地配置页（设备局域网 IP）

5. **断网恢复机制**
   - STA 模式自动重连 + 有限重试。
   - 连接失败进入退避深睡，下一轮自动恢复。

6. **设备心跳上报（编排模式）**
   - 每轮完成后上报 `checkin`（含 `next_wakeup_epoch`、失败计数、最近错误）。
   - 后端可据此在 Web 端提示“插播预计生效时间”。

7. **远端配置下发与应用回报**
   - 每次唤醒后，设备会向编排服务查询配置版本：`GET /api/v1/device/config`
   - 有新版本时写入 NVS，并上报应用结果：`POST /api/v1/device/config/applied`
   - 应用新配置后自动重启，保证后续拉图流程按新参数执行。

8. **本地配置接口（Portal）**
   - `GET /api/config`：查询当前配置与运行状态。
   - `POST /api/config`：更新 Wi-Fi、编排服务地址、轮询间隔、重试参数、时区、旋转参数。
   - `GET /api/wifi/scan`：扫描 AP 列表。

## 配置项（NVS 持久化）

- `wifi_ssid` / `wifi_password`
- `orchestrator_enabled`（`1=启用编排` `0=关闭编排`）
- `orchestrator_base_url`（默认 `http://192.168.58.113:18081`）
- `device_id`（首次自动生成，可手工覆盖）
- `orchestrator_token`（可选）
- `photo_token`（可选，拉图时自动携带请求头 `X-Photo-Token`）
- `image_url_template`（编排关闭时使用，支持 `%DATE%`、`%DEVICE_ID%`）
- `interval_minutes`（默认 60）
- `retry_base_minutes` / `retry_max_minutes`
- `max_failure_before_long_sleep`
- `display_rotation`（当前支持 `0` 或 `2`）
- `color_process_mode`（`0=自动判断` `1=总是转换` `2=认为输入已是6色`）
- `dither_mode`（`0=关闭` `1=有序抖动`）
- `six_color_tolerance`（0-64，判断“是否已是6色”的容差）
- `timezone`（默认 `UTC`）
- `last_image_sha256`（用于避免重复刷新）
- `remote_config_version`（最近已应用的远端配置版本）

## 接口示例（Portal 模式）

默认在无 Wi-Fi 凭据、或上电长按按键 3 秒后进入 AP Portal。

```bash
# 查询当前配置与运行状态
curl -s http://192.168.73.1/api/config

# 扫描附近 Wi-Fi
curl -s http://192.168.73.1/api/wifi/scan

# 更新配置
curl -s -X POST http://192.168.73.1/api/config \
  -H "Content-Type: application/json" \
  --data-binary @- <<JSON
{
  "wifi_ssid": "YourWiFi",
  "wifi_password": "YourPassword",
  "orchestrator_enabled": 1,
  "orchestrator_base_url": "http://192.168.58.113:18081",
  "device_id": "pf-livingroom",
  "orchestrator_token": "",
  "photo_token": "",
  "image_url_template": "https://901.qingpei.me:40009/daily.bmp?device_id=%DEVICE_ID%",
  "interval_minutes": 60,
  "retry_base_minutes": 5,
  "retry_max_minutes": 240,
  "max_failure_before_long_sleep": 24,
  "display_rotation": 2,
  "color_process_mode": 0,
  "dither_mode": 1,
  "six_color_tolerance": 0,
  "timezone": "Asia/Shanghai"
}
JSON
```

如果设备是“按键唤醒”进入的 120 秒窗口，请访问设备当前 STA IP（串口日志会打印 URL，例如 `http://192.168.58.120/`）。

## 失败重试行为

- 失败后休眠时长：`retry_base_minutes * 2^(failure_count-1)`
- 上限：`retry_max_minutes`
- `failure_count` 达到 `max_failure_before_long_sleep` 后，保持最长退避间隔
- 任意一次成功拉图后，`failure_count` 归零

## Wi-Fi 诊断日志

当出现 `wifi disconnected` 时，串口会输出：

- `reason=<code>(<name>)`：ESP-IDF 断开原因码与名称
- `hint=<text>`：可执行排查建议（如检查密码、2.4GHz 覆盖或 WPA 模式）

连接超时失败时，还会在 `wifi connect failed` 中带上最后一次断开原因，便于定位“密码错误 / 安全模式不兼容 / 信号问题”。

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
