# 编排服务接口定义（省电优先）

本接口用于配合相框固件实现：

- 设备只在唤醒窗口主动拉取任务（pull）
- Web 提交插播后，服务根据设备 `next_wakeup_epoch` 给出预计生效时间
- 设备每次唤醒可同步“待生效配置”，并上报已应用状态

## 1) 设备拉取当前任务

`GET /api/v1/device/next`

### Query

- `device_id`：设备唯一标识（必填）
- `now_epoch`：当前时间戳（可选）
- `default_poll_seconds`：设备默认轮询间隔（可选）
- `failure_count`：设备失败计数（可选）

### Response

```json
{
  "device_id": "pf-a1b2c3d4",
  "server_epoch": 1760000000,
  "source": "override",
  "image_url": "http://192.168.58.113:18081/api/v1/assets/xxxx.bmp",
  "valid_until_epoch": 1760001800,
  "poll_after_seconds": 900,
  "default_poll_seconds": 3600,
  "active_override_id": 12
}
```

说明：

- `source=override`：有插播生效
- `source=daily`：无插播，回退到每日图
- `poll_after_seconds`：服务建议下次唤醒间隔（用于平衡省电和插播时效）
- 服务会在每次 `device/next` 响应后记录一条图片下发历史

## 2) 设备上报心跳

`POST /api/v1/device/checkin`

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
  "image_source": "override",
  "last_error": ""
}
```

## 3) 设备配置下发与同步

### 3.1 后台发布配置

`POST /api/v1/device-config`

```json
{
  "device_id": "pf-a1b2c3d4",
  "note": "公网 token 切换",
  "config": {
    "interval_minutes": 60,
    "image_url_template": "https://901.qingpei.me:40009/daily.bmp?device_id=%DEVICE_ID%",
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
    "image_url_template": "https://901.qingpei.me:40009/daily.bmp?device_id=%DEVICE_ID%"
  },
  "note": "公网 token 切换"
}
```

### 3.3 设备回报配置应用结果

`POST /api/v1/device/config/applied`

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

## 4) Web 上传插播

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
  "start_epoch": 1760000100,
  "end_epoch": 1760001900,
  "duration_minutes": 30,
  "image_url": "http://192.168.58.113:18081/api/v1/assets/xxxx.bmp",
  "asset_sha256": "...",
  "expected_effective_epoch": 1760003600
}
```

`expected_effective_epoch` 用于前端提示“预计何时在屏幕上生效”。

## 5) 查询与管理

- `GET /api/v1/devices`：设备状态（含 `next_wakeup_epoch` 与配置同步状态）
- `GET /api/v1/overrides`：插播列表（含状态 active/upcoming/expired）
- `DELETE /api/v1/overrides/{id}`：取消插播

`/api/v1/devices` 额外字段：

- `config_target_version`
- `config_seen_version`
- `config_last_query_epoch`
- `config_applied_version`
- `config_last_apply_epoch`
- `config_apply_ok`
- `config_apply_error`

## 6) 图片发布历史

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
      "image_url": "http://192.168.58.113:18081/api/v1/assets/xxxx.bmp",
      "override_id": 12,
      "poll_after_seconds": 900,
      "valid_until_epoch": 1760001800
    }
  ]
}
```

说明：

- 历史记录按 `issued_epoch` 倒序返回。
- 当前实现自动保留最近 5000 条，超出后会清理最旧记录。

## 7) 公网只读日图（供外网相框拉取）

`GET /public/daily.bmp`

鉴权：

- 请求头：`X-Photo-Token: <token>`
- 或 query：`?token=<token>`
- 当 `PUBLIC_DAILY_BMP_TOKEN` 未配置时返回 `403`

可选 Query：

- `device_id`：设备 ID（默认 `*`）

行为：

1. 先判断 `device_id` 当前是否有 active override（设备专属优先，其次全局）。
2. 若有插播，直接返回插播 BMP。
3. 若无插播，回退到 `DAILY_IMAGE_URL_TEMPLATE` 的当日图。

响应头会包含：

- `X-PhotoFrame-Source: override|daily`
- `X-PhotoFrame-Device: <device_id>`

## 8) 认证

设置环境变量 `PHOTOFRAME_TOKEN` 后，以下接口需要请求头：

- `X-PhotoFrame-Token: <token>`

涉及：

- `POST /api/v1/device/checkin`
- `GET /api/v1/device/next`
- `POST /api/v1/overrides/upload`
- `DELETE /api/v1/overrides/{id}`
- `GET /api/v1/publish-history`
- `GET /api/v1/device/config`
- `POST /api/v1/device/config/applied`
- `POST /api/v1/device-config`
- `GET /api/v1/device-configs`
