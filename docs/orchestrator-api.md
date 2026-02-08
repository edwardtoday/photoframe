# 编排服务接口定义（省电优先）

本接口用于配合相框固件实现：

- 设备只在唤醒窗口主动拉取任务（pull）
- Web 提交插播后，服务根据设备 `next_wakeup_epoch` 给出预计生效时间

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

## 3) Web 上传插播

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

## 4) 查询与管理

- `GET /api/v1/devices`：设备状态（含 `next_wakeup_epoch`）
- `GET /api/v1/overrides`：插播列表（含状态 active/upcoming/expired）
- `DELETE /api/v1/overrides/{id}`：取消插播

## 5) 可选认证

设置环境变量 `PHOTOFRAME_TOKEN` 后，接口需要请求头：

- `X-PhotoFrame-Token: <token>`

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
    },
    {
      "id": 100,
      "device_id": "pf-a1b2c3d4",
      "issued_epoch": 1760000000,
      "source": "daily",
      "image_url": "http://192.168.58.113:8000/image/480x800?date=2026-02-08",
      "override_id": null,
      "poll_after_seconds": 3600,
      "valid_until_epoch": 1760003600
    }
  ]
}
```

说明：

- 历史记录按 `issued_epoch` 倒序返回。
- 当前实现自动保留最近 5000 条，超出后会清理最旧记录。
