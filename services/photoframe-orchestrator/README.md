# photoframe-orchestrator

用于 NAS 侧托管“每日图片 + 插播任务”的编排服务。

## 能力

- 设备拉取：`GET /api/v1/device/next`
- 设备心跳：`POST /api/v1/device/checkin`
- Web 上传插播图并设置播放窗口：`POST /api/v1/overrides/upload`
- 管理插播列表：`GET /api/v1/overrides`、`DELETE /api/v1/overrides/{id}`
- Web 管理页：`GET /`（含发布历史时间线）

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
- `TZ`：服务端时区


发布历史数据文件：`services/photoframe-orchestrator/app/release_history.json`
