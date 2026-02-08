# Featured Photo 集成说明（编排版）

## 背景

- 设备：Waveshare `ESP32-S3-PhotoPainter`（480x800 彩色墨水屏）
- 固件：`firmware/photoframe-fw/`
- 每日图服务：`immich-featured-today`（示例 `http://192.168.58.113:8000`）
- 新增编排服务：`services/photoframe-orchestrator/`

## 目标

1. 设备保持省电：唤醒窗口 pull，其他时间 deep sleep。
2. 日常展示：按日拉取 featured photo。
3. 插播展示：NAS Web 上传图片并指定播放时段。
4. 到期后自动回到每日图。

## 架构

```text
PhotoFrame(ESP32)
  ├─ GET /api/v1/device/next  ---> Orchestrator(NAS)
  │       ├─ 有插播: 返回本地 BMP 资源 URL
  │       └─ 无插播: 返回 daily URL（immich-featured-today）
  └─ POST /api/v1/device/checkin ---> Orchestrator(NAS)
          (携带 next_wakeup_epoch, failure_count 等)
```

Web 管理页根据 `next_wakeup_epoch` 估算“插播预计生效时间”。

## 时效与省电边界

- deep sleep 期间设备离线，无法被动推送。
- 插播生效时间 = `max(插播开始时间, 设备下一次唤醒时间)`。
- 如需更快生效，可缩短轮询周期（会增加功耗）。

## 接口定义

详见：`docs/orchestrator-api.md`
