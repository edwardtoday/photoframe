# 编排服务接口定义（省电优先）

本接口用于配合相框固件实现：

- 设备只在唤醒窗口主动拉取任务（pull）
- Web 提交插播后，服务根据设备 `next_wakeup_epoch` 给出预计生效时间
- 设备每次唤醒可同步“待生效配置”，并上报已应用状态

鉴权约定：

- 管理接口使用 `PHOTOFRAME_TOKEN`（请求头 `X-PhotoFrame-Token`）。
- 设备接口（`/api/v1/device/*`）优先使用设备 token 映射（`DEVICE_TOKEN_MAP_JSON` / `DEVICE_TOKEN_MAP`）校验 `X-PhotoFrame-Token`，并绑定 `device_id`。
- 静态资源接口 `GET /api/v1/assets/{asset_name}` 不再匿名开放：
  - 管理端可用 `X-PhotoFrame-Token: <PHOTOFRAME_TOKEN>` 或 query `?token=<PHOTOFRAME_TOKEN>`
  - 设备端可带 `device_id=...`，并继续使用自己的 `X-PhotoFrame-Token`
- 若未配置设备 token 映射：
  - 设备首次携带新 token 请求时会进入“待审批”；
  - 管理端通过 `POST /api/v1/device-tokens/{device_id}/approve` 信任后放行；
  - 兼容旧配置：当设备仍使用 `PHOTOFRAME_TOKEN` 时可继续访问。

## 1) 设备拉取当前任务

`GET /api/v1/device/next`

### Query

- `device_id`：设备唯一标识（必填）
- `now_epoch`：当前时间戳（可选）
- `default_poll_seconds`：设备默认轮询间隔（可选）
- `failure_count`：设备失败计数（可选）
- `accept_formats`：可选，图片格式偏好（逗号分隔，如 `jpeg,bmp`）。当插播资源存在 `.jpg` 派生文件时，服务会优先下发更小的 JPEG URL。

### Header

- `X-PhotoFrame-Token`：设备 token

### Response

```json
{
  "device_id": "pf-a1b2c3d4",
  "server_epoch": 1760000000,
  "source": "override",
  "image_url": "http://192.168.58.113:18081/api/v1/assets/xxxx.bmp?device_id=pf-a1b2c3d4",
  "valid_until_epoch": 1760001800,
  "poll_after_seconds": 900,
  "default_poll_seconds": 3600,
  "active_override_id": 12,
  "log_upload_request": {
    "request_id": 21,
    "max_lines": 800,
    "max_bytes": 65536,
    "reason": "collect wake diagnostics",
    "created_epoch": 1760000000,
    "expires_epoch": 1760003600
  }
}
```

说明：

- `source=override`：有插播生效
- `source=daily`：无插播，回退到每日图
- `poll_after_seconds`：服务建议下次唤醒间隔（用于平衡省电和插播时效）
- 服务会在每次 `device/next` 响应后先写一条 `status=sent` 的图片发布记录；
  设备随后 `checkin` 回报 `display_applied=true` 时，会把同一条记录更新为 `status=displayed`
- `log_upload_request`：可选，一次性日志采集指令。设备若收到该字段，应在本轮主周期结束前 best-effort 上传最近一段诊断日志，不应阻塞正常拉图与休眠。
- `firmware_update`：可选，A/B OTA 升级指令。设备若收到该字段，应优先检查电量 / VBUS 门槛，再把新固件下载到 inactive slot；只有写入和 SHA256 校验成功后才切换 boot partition 并重启。
- `image_url` 可能是：
  - `.../api/v1/assets/<sha>.bmp?device_id=...` 或 `.../api/v1/assets/<sha>.jpg?device_id=...`（插播资源；取决于 `accept_formats` 与派生文件是否存在）
  - `.../api/v1/assets/daily-<date>-<algorithm>-<profile>.bmp?device_id=...` 或对应 `.jpg`（日图；由 orchestrator 从上游 JPG 抓图后按当前 Daily Dither 算法与 palette profile 生成的静态缓存，并按上游 `ETag` / `Last-Modified` 做条件校验）

## 2) 设备上报心跳

`POST /api/v1/device/checkin`

### Header

- `X-PhotoFrame-Token`：设备 token

### Body

```json
{
  "device_id": "pf-a1b2c3d4",
  "checkin_epoch": 1760000000,
  "next_wakeup_epoch": 1760003600,
  "sleep_seconds": 3600,
  "poll_interval_seconds": 3600,
  "failure_count": 0,
  "last_http_status": 200,
  "fetch_ok": true,
  "image_changed": true,
  "display_applied": true,
  "image_source": "override",
  "displayed_image_url": "http://192.168.58.113:18081/api/v1/assets/xxxx.bmp?device_id=pf-a1b2c3d4",
  "displayed_image_sha256": "abc123...",
  "last_error": "",
  "sta_ip": "192.168.58.120",
  "battery_mv": 3987,
  "battery_percent": 84,
  "charging": 1,
  "vbus_good": 1,
  "reported_config": {
    "interval_minutes": 60,
    "image_url_template": "https://901.qingpei.me:40009/daily.bmp",
    "wifi_profiles": [
      { "ssid": "HomeWiFi", "password_set": true },
      { "ssid": "OfficeWiFi", "password_set": true }
    ],
    "color_process_mode": 0
  }
}
```

`reported_config` 为设备当前生效配置快照，用于 Web 表单灰字提示；敏感字段会在服务端返回时脱敏。

- 供电状态字段：`battery_mv`（mV）、`battery_percent`、`charging`（1/0）、`vbus_good`（1/0）
- `display_applied=true` 表示设备本轮确实完成了 E-Ink 刷新；服务端会把对应图片发布记录从 `sent` 更新为 `displayed`
- `displayed_image_url` / `displayed_image_sha256` 表示设备本轮实际显示的图片标识；当命中 `304` 或内容未变化而跳过刷屏时，它们应为空
- 若设备本轮因 PMIC/I2C 异常上报 `-1`（缺失），服务端会保留上一轮有效值，并用“最终有效值”写入电池采样历史，
  避免控制台曲线出现长时间断点（同时可通过 `last_error` 观察是否存在硬件读数异常）。

## 3) 设备配置下发与同步

### 3.1 后台发布配置

`POST /api/v1/device-config`

```json
{
  "device_id": "pf-a1b2c3d4",
  "note": "公网 token 切换",
  "config": {
    "interval_minutes": 60,
    "image_url_template": "https://901.qingpei.me:40009/daily.bmp",
    "wifi_profiles": [
      { "ssid": "HomeWiFi", "password": "..." },
      { "ssid": "OfficeWiFi", "password": "..." }
    ],
    "photo_token": "..."
  }
}
```

- `device_id='*'` 表示全局配置
- 每次发布都会生成新的 `config_version`（即记录 ID）
- 服务默认保留每个目标最近 200 条配置历史

允许字段（白名单）：

- `orchestrator_enabled`
- `orchestrator_base_url`
- `orchestrator_token`
- `image_url_template`
- `photo_token`
- `wifi_profiles`（数组，最多 3 条：`[{ "ssid": "...", "password": "..." }]`）
- `interval_minutes`
- `retry_base_minutes`
- `retry_max_minutes`
- `max_failure_before_long_sleep`
- `display_rotation`
- `color_process_mode`
- `dither_mode`
- `six_color_tolerance`
- `timezone`

### 3.2 设备拉取配置

`GET /api/v1/device/config`

Header:

- `X-PhotoFrame-Token`：设备 token

Query:

- `device_id`（必填）
- `current_version`（可选，设备当前已应用版本）
- `now_epoch`（可选）

Response:

```json
{
  "device_id": "pf-a1b2c3d4",
  "server_epoch": 1760000000,
  "config_version": 18,
  "config": {
    "interval_minutes": 60,
    "image_url_template": "https://901.qingpei.me:40009/daily.bmp"
  },
  "note": "公网 token 切换"
}
```

### 3.3 设备回报配置应用结果

`POST /api/v1/device/config/applied`

Header:

- `X-PhotoFrame-Token`：设备 token

```json
{
  "device_id": "pf-a1b2c3d4",
  "config_version": 18,
  "applied": true,
  "error": "",
  "applied_epoch": 1760000030
}
```

### 3.4 查询配置历史

`GET /api/v1/device-configs`

Query:

- `device_id`：可选，不填或 `*` 表示全部
- `limit`：可选，1-200，默认 50

## 4) 设备按需日志上传

设计目标：

- 平时不引入“每轮必传”的固定耗电成本
- 仅在管理端显式请求时，设备下次唤醒才上传一小段诊断日志
- 日志上传为 best-effort，不应阻塞主周期的拉图、渲染和休眠

### 4.1 管理端创建日志采集请求

`POST /api/v1/device-log-requests`

Header:

- `X-PhotoFrame-Token`

```json
{
  "device_id": "pf-a1b2c3d4",
  "reason": "collect wake diagnostics",
  "max_lines": 800,
  "max_bytes": 65536,
  "expires_in_minutes": 1440
}
```

说明：

- 同一设备新的请求创建时，旧的 pending 请求会被取消。
- 设备只会拿到“最新且仍未过期”的一条请求。

### 4.2 管理端查看日志采集请求

`GET /api/v1/device-log-requests`

Query:

- `device_id`：可选
- `status`：可选，典型值 `pending|completed|cancelled|expired`
- `limit`：可选，默认 50

### 4.3 管理端取消 pending 请求

`DELETE /api/v1/device-log-requests/{request_id}`

### 4.4 设备上传日志

`POST /api/v1/device/log-upload`

Header:

- `X-PhotoFrame-Token`

```json
{
  "device_id": "pf-a1b2c3d4",
  "request_id": 21,
  "uploaded_epoch": 1760000123,
  "line_count": 3,
  "truncated": false,
  "uploaded_bytes": 412,
  "buffer_total_lines": 97,
  "buffer_total_bytes": 13884,
  "buffer_boot_id": 8,
  "lines": [
    "[1760000001][boot:8][seq:1][INFO] photoframe-rs: wakeup cause=TIMER",
    "[1760000005][boot:8][seq:2][INFO] photoframe-rs: wifi connected idx=0 ssid=HomeWiFi ip=192.168.1.8",
    "[1760000018][boot:8][seq:3][INFO] photoframe-rs: cycle exit=Sleep { seconds: 3600, timer_only: false } source=daily checkin_reported=true logs_uploaded=true"
  ]
}
```

说明：

- 上传成功后，请求会被标记为 `completed`。
- 相同 `request_id` 的重复上传按幂等更新处理。
- 取消或过期请求会拒绝上传。
- `uploaded_bytes` / `buffer_total_lines` / `buffer_total_bytes` / `buffer_boot_id` 用于判断本次上传拿到的是“整段”还是“环形缓冲尾部”。
- 设备会优先把受控重启 / deep sleep 前的日志块写入 TF 卡 10 MiB 环形段（20 个 segment）；下一次 boot 会先恢复这些历史块，再继续追加新日志。
- 若 TF 卡不可用，设备才退回到 RTC 保留区快照模式，因此日志上传仍可覆盖“上一轮睡前/重启前”的关键信息。

### 4.5 管理端查看已上传日志

`GET /api/v1/device-log-uploads`

Query:

- `device_id`：可选
- `limit`：可选，默认 20

### 4.6 设备调试阶段历史

`GET /api/v1/device-debug-stages`

用途：

- 查看设备通过 `/api/v1/device/debug-stage` 回传的阶段信标，便于 OTA 故障注入与阶段定位

Query:

- `device_id`：可选
- `stage`：可选
- `limit`：可选，默认 50

Header:

- `X-PhotoFrame-Token`

Response:

```json
{
  "now_epoch": 1770000000,
  "count": 1,
  "items": [
    {
      "id": 12,
      "device_id": "pf-a1b2c3d4",
      "stage": "ota_download_50",
      "stage_epoch": 1770000000,
      "created_at": 1770000000
    }
  ]
}
```

## 5) 固件 OTA 控制面

说明：

- OTA 只接受 `app.bin` 这类应用分区镜像，不接受整片镜像。
- 设备若仍是旧单分区布局，需先 USB 迁移到 `ota_0/ota_1` 分区表后，才能启用 ping-pong OTA。

### 5.1 固件制品上传

`POST /api/v1/firmware-artifacts/upload`

Header:

- `X-PhotoFrame-Token`

`multipart/form-data`：

- `file`：固件应用分区镜像（`.bin`）
- `version`：固件版本字符串
- `note`：可选备注

### 5.2 固件制品列表

`GET /api/v1/firmware-artifacts`

### 5.3 创建 rollout

`POST /api/v1/firmware-rollouts`

```json
{
  "device_id": "pf-a1b2c3d4",
  "firmware_artifact_id": 3,
  "min_battery_percent": 50,
  "requires_vbus": false,
  "note": "pmic fix rollout"
}
```

说明：

- 同一设备新的 rollout 创建时，旧的 enabled rollout 会被关闭。
- `device_id` 当前只支持单设备，不支持 `*` 批量升级。

### 5.4 查看 rollout

`GET /api/v1/firmware-rollouts`

### 5.5 取消 rollout

`DELETE /api/v1/firmware-rollouts/{rollout_id}`

## 6) Web 上传插播

`POST /api/v1/overrides/upload`（`multipart/form-data`）

- `file`：上传图片（服务端统一转 480x800 BMP）
- `duration_minutes`：持续分钟
- `device_id`：目标设备，`*` 代表全部设备
- `starts_at`：可选，ISO 日期时间，不填表示立即
- `note`：备注

### Response

```json
{
  "ok": true,
  "id": 12,
  "device_id": "pf-a1b2c3d4",
  "start_epoch": 1760003600,
  "end_epoch": 1760005400,
  "duration_minutes": 30,
  "start_policy": "next_wakeup",
  "will_expire_before_effective": false,
  "image_url": "http://192.168.58.113:18081/api/v1/assets/xxxx.bmp?device_id=pf-a1b2c3d4",
  "asset_sha256": "...",
  "expected_effective_epoch": 1760003600
}
```

`expected_effective_epoch` 用于前端提示“预计何时在屏幕上生效”。

插播开始时间规则：

- 显式填写 `starts_at`：按该时间开始（`start_policy=explicit`）
- `starts_at` 留空且是单设备：默认按该设备 `next_wakeup_epoch` 开始（`start_policy=next_wakeup`）
- `starts_at` 留空且是 `*`：立即开始（`start_policy=immediate`）

当 `will_expire_before_effective=true` 时，表示该窗口可能在设备真正生效前就过期。

## 7) 查询与管理

- `GET /api/v1/devices`：设备状态（含 `next_wakeup_epoch` 与配置同步状态）
- `GET /api/v1/power-samples`：电池采样历史（用于曲线展示与续航估算）
- `GET /api/v1/overrides`：插播列表（含状态 active/upcoming/expired）
- `DELETE /api/v1/overrides/{id}`：取消插播
- `GET /api/v1/device-tokens?pending_only=1`：查看待审批设备 token 列表
- `POST /api/v1/device-tokens/{device_id}/approve`：审批并信任设备 token
- `DELETE /api/v1/device-tokens/{device_id}`：移除设备 token 记录（重新配对）
- `GET /api/v1/daily-render-config`：查看当前 Daily Dither 算法与可选项
- `POST /api/v1/daily-render-config`：更新当前 Daily Dither 算法

以上管理接口都使用：

- `X-PhotoFrame-Token`：管理端 token（当 `PHOTOFRAME_TOKEN` 已配置时必填）

`/api/v1/devices` 额外字段：

- `firmware_version`：设备最近一次 `checkin.reported_config.firmware_version`
- `config_target_version`
- `config_seen_version`
- `config_last_query_epoch`
- `config_applied_version`
- `config_last_apply_epoch`
- `config_apply_ok`
- `config_apply_error`
- `reported_config_epoch`
- `reported_config`（设备上报配置快照，敏感值脱敏）
- `battery_mv` / `battery_percent` / `charging` / `vbus_good`

### 7.1) 电池采样历史（曲线/续航估算）

`GET /api/v1/power-samples`

Query：

- `device_id`：必填，不能为 `*`
- `from_epoch`：可选，默认 `now - 30 天`
- `to_epoch`：可选，默认 `now`
- `limit`：可选（1-20000，默认 5000）

Header：

- `X-PhotoFrame-Token`：管理端 token（当 `PHOTOFRAME_TOKEN` 已配置时必填）

Response：

```json
{
  "now_epoch": 1760000600,
  "device_id": "pf-a1b2c3d4",
  "from_epoch": 1757408600,
  "to_epoch": 1760000600,
  "count": 2,
  "items": [
    {
      "sample_epoch": 1760000000,
      "battery_mv": 3987,
      "battery_percent": 84,
      "charging": 1,
      "vbus_good": 1
    }
  ]
}
```

说明：

- 服务端会在每次设备 `POST /api/v1/device/checkin` 时追加采样（`sample_epoch` 取 `checkin_epoch`）。
- 服务端默认保留最近 365 天采样，超期数据会自动清理。

## 8) 当前下发图片预览（管理页）

`GET /api/v1/preview/current.bmp`

用途：

- 管理页面实时预览“设备此刻拉图会拿到什么图”（480x800 BMP）

Query：

- `device_id`：可选，默认 `*`
- `now_epoch`：可选

Header：

- `X-PhotoFrame-Token`
  - 管理页/管理端调试：可直接传 `PHOTOFRAME_TOKEN`
  - 设备侧调试：也可传已绑定该 `device_id` 的设备 token

响应头：

- `X-PhotoFrame-Source: override|daily`
- `X-PhotoFrame-Device: <device_id>`

## 7) 图片发布历史

`GET /api/v1/publish-history`

### Query

- `device_id`：可选，指定设备 ID；`*` 或不填表示全部设备
- `limit`：可选，返回条数（1-1000，默认 200）

### Response

```json
{
  "now_epoch": 1760000600,
  "count": 2,
  "items": [
    {
      "id": 101,
      "device_id": "pf-a1b2c3d4",
      "issued_epoch": 1760000500,
      "source": "override",
      "image_url": "http://192.168.58.113:18081/api/v1/assets/xxxx.bmp?device_id=pf-a1b2c3d4",
      "override_id": 12,
      "poll_after_seconds": 900,
      "valid_until_epoch": 1760001800,
      "status": "displayed",
      "displayed_epoch": 1760000512,
      "displayed_image_url": "http://192.168.58.113:18081/api/v1/assets/xxxx.bmp?device_id=pf-a1b2c3d4",
      "displayed_image_sha256": "abc123..."
    }
  ]
}
```

说明：

- 历史记录按 `issued_epoch` 倒序返回。
- `status=sent` 表示服务端已经下发该图片指令，但还没收到设备的成功显示确认。
- `status=displayed` 表示设备已经通过 `POST /api/v1/device/checkin` 明确确认“本轮图片已成功显示”。
- 命中 `304` / 内容未变化跳过刷屏 / 渲染失败的周期，会保留在 `sent` 状态，不会升级成 `displayed`。
- 当前实现自动保留最近 5000 条，超出后会清理最旧记录。

## 8) 公网只读日图（供外网相框拉取）

`GET /public/daily.bmp`
`GET /public/daily.jpg`

鉴权：

- 请求头：`X-Photo-Token: <token>`
- 或 query：`?token=<token>`
- 当 `PUBLIC_DAILY_BMP_TOKEN` 未配置时返回 `403`

可选 Query：

- `device_id`：设备 ID（默认 `*`）

行为：

1. 先判断 `device_id` 当前是否有 active override（设备专属优先，其次全局）。
2. 若有插播，直接返回插播图（`daily.bmp` 输出 BMP，`daily.jpg` 输出 JPEG）。
3. 若无插播，抓取 `DAILY_IMAGE_URL_TEMPLATE` 的当日图（推荐上游 JPG），按当前 Daily Dither 算法生成并输出 BMP/JPEG。

缓存与省电：

- 响应会携带 `ETag`，并支持 `If-None-Match` 命中 `304 Not Modified`（不返回正文），用于设备侧省流省电轮询。
- 服务端 daily 缓存会定期带 `If-None-Match` / `If-Modified-Since` 回源；若上游未变，或虽回源成功但最终渲染结果字节不变，则不会重写本地 daily 资产，便于设备继续命中稳定 ETag。
- 当提供 `device_id`（非 `*`）时，服务会更新该设备的 `last_seen`（控制台的 `last_checkin`），
  便于在设备仅能访问 `/public/daily.*` 的场景下也能确认“设备仍在活跃拉图”；并会用服务端已保存的最近电量值写入一条采样点
  （若尚无任何电量读数则跳过），让电池曲线能反映设备仍有活动。

响应头会包含：

- `X-PhotoFrame-Source: override|daily`
- `X-PhotoFrame-Device: <device_id>`

## 9) 设备历史补图

`GET /api/v1/device/history/daily.bmp`
`GET /api/v1/device/history/daily.jpg`

鉴权：

- 请求头：`X-PhotoFrame-Token: <device_token>`

必填 Query：

- `device_id`：设备 ID
- `date`：目标日期，格式 `YYYY-MM-DD`

行为：

1. 仅面向设备侧使用，不经过 publish history，也不会生成新的 `sent/displayed` 记录。
2. 服务端按显式 `date` 重写 `DAILY_IMAGE_URL_TEMPLATE`，向上游拉取对应日期图片。
3. 与常规 daily 一样，复用服务端裁剪、dither、palette profile 和资产缓存链路。
4. 返回 BMP 或 JPEG，取决于接口后缀；响应头会包含 `X-PhotoFrame-Date`、`ETag` 和 `X-PhotoFrame-Dither`，设备可按需复用缓存。

适用场景：

- 设备短按 `KEY` 回看历史日期时，若 TF 卡里缺该日期图片，可通过该接口即时补图并写回本地缓存。
- 设备长按 `KEY` 回到“当前 orchestrator 图片”时，若当前日期图片也不在 TF 中，同样走该接口兜底。

## 10) 认证

管理端（编辑页、插播、配置发布、审批）使用：

- `X-PhotoFrame-Token: <PHOTOFRAME_TOKEN>`

设备端（`/api/v1/device/*`）使用：

- `X-PhotoFrame-Token: <device_token>`
- 当配置了 `DEVICE_TOKEN_MAP_JSON` / `DEVICE_TOKEN_MAP` 时，按映射强校验。
- 当未配置映射时，设备首次会进入 pending，需管理端审批后放行。

常见管理接口：

- `POST /api/v1/overrides/upload`
- `DELETE /api/v1/overrides/{id}`
- `GET /api/v1/publish-history`
- `POST /api/v1/device-config`
- `GET /api/v1/device-configs`
- `GET /api/v1/device-tokens`
- `POST /api/v1/device-tokens/{device_id}/approve`
