# photoframe-orchestrator

用于 NAS 侧托管“每日图片 + 插播任务 + 设备配置下发”的编排服务。

## 能力

- 设备拉取：`GET /api/v1/device/next`
- 设备心跳：`POST /api/v1/device/checkin`
- 设备配置同步：
  - 管理端发布：`POST /api/v1/device-config`
  - 设备查询：`GET /api/v1/device/config`
  - 设备回报：`POST /api/v1/device/config/applied`
  - 历史查询：`GET /api/v1/device-configs`
- Web 上传插播图并设置播放窗口：`POST /api/v1/overrides/upload`
- 管理插播列表：`GET /api/v1/overrides`、`DELETE /api/v1/overrides/{id}`
- 图片下发历史：`GET /api/v1/publish-history`
- 公网日图代理：`GET /public/daily.bmp`（token 保护，且优先返回当前生效插播）
- Web 管理页：`GET /`（含图片发布历史 + 设备配置发布历史）

## 本地运行（源码）

```bash
cd services/photoframe-orchestrator
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
uvicorn app.main:app --host 0.0.0.0 --port 8081
```

## 本地 Docker（开发模式，按源码构建）

```bash
docker compose -f docker-compose.photoframe-orchestrator.yml up -d --build
```

## 生产 Docker（NAS，镜像拉取）

```bash
docker compose -f docker-compose.photoframe-orchestrator.prod.yml pull
docker compose -f docker-compose.photoframe-orchestrator.prod.yml up -d
```

## 发布 multi-arch 镜像

```bash
# 默认: edwardtoday/photoframe-orchestrator:<git短sha> + latest
scripts/release-orchestrator-image.sh

# 指定 tag
scripts/release-orchestrator-image.sh 0.1.0
```

访问：`http://<NAS_IP>:18081/`

## 环境变量

- `DAILY_IMAGE_URL_TEMPLATE`：无插播时的每日图片模板，支持 `%DATE%`
- `PUBLIC_BASE_URL`：返回给设备的资源 URL 前缀
- `DEFAULT_POLL_SECONDS`：默认轮询周期（秒）
- `PHOTOFRAME_TOKEN`：可选认证 token（设备与 Web 请求都需带 `X-PhotoFrame-Token`）
- `PUBLIC_DAILY_BMP_TOKEN`：公网日图接口口令（`/public/daily.bmp`，为空则禁用）
- `DAILY_FETCH_TIMEOUT_SECONDS`：公网日图代理拉取上游超时（秒，默认 10）
- `TZ`：服务端时区

## 公网日图接口

- `GET /public/daily.bmp`
- 鉴权：请求头 `X-Photo-Token` 或 query `?token=`
- 可选：`device_id`（用于按设备匹配插播）
- 行为：
  1. 优先返回该设备当前生效的插播图（若存在）
  2. 否则回退到 `DAILY_IMAGE_URL_TEMPLATE` 的当日 BMP

更多字段与示例见 `docs/orchestrator-api.md`。
