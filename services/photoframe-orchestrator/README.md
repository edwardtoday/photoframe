# photoframe-orchestrator

用于 NAS 侧托管“每日图片 + 插播任务 + 设备配置下发”的编排服务。

## 能力

- 设备拉取：`GET /api/v1/device/next`
- 设备心跳：`POST /api/v1/device/checkin`
- 电池采样历史：`GET /api/v1/power-samples`（用于控制台曲线/续航估算）
- 设备配置同步：
  - 管理端发布：`POST /api/v1/device-config`
  - 设备查询：`GET /api/v1/device/config`
  - 设备回报：`POST /api/v1/device/config/applied`
  - 历史查询：`GET /api/v1/device-configs`
- Web 上传插播图并设置播放窗口：`POST /api/v1/overrides/upload`
- 管理插播列表：`GET /api/v1/overrides`、`DELETE /api/v1/overrides/{id}`
- 图片下发历史：`GET /api/v1/publish-history`
- 管理页预览当前下发图：`GET /api/v1/preview/current.bmp`
- 公网日图代理：`GET /public/daily.bmp`（token 保护，且优先返回当前生效插播）
- Web 管理页：`GET /`（含图片发布历史 + 设备配置发布历史 + 当前下发预览 + 设备 token 审批）
- 设备配置“填空式表单”：不再手写 JSON，灰字提示来自设备最近上报配置
- 设备配置页提供 daily.bmp URL 快捷填入（当前服务 / 公网示例）
- 创建插播后给出“预计生效时间/可能过期”可读提示，便于和设备唤醒周期对齐

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

## 生产 Docker（NAS，离线投送，不 pull）

适合 NAS 外网不稳定、或希望明确绕过 `docker compose pull` 的场景。

```bash
# 默认目标：tvs675（ssh config）
# 默认 tag：当前 git 短 SHA
scripts/deploy-orchestrator-offline-to-tvs675.sh

# 指定 tag
scripts/deploy-orchestrator-offline-to-tvs675.sh 0.1.0

# 只预演不执行
DRY_RUN=1 scripts/deploy-orchestrator-offline-to-tvs675.sh
```

如遇 ssh-agent 不可用/签名失败，可显式指定私钥并强制仅使用该身份：

```bash
SSH_IDENTITY_FILE=~/.ssh/id_rsa scripts/deploy-orchestrator-offline-to-tvs675.sh
```

脚本关键行为：
- 本机构建 `linux/amd64` 镜像并导出 tar
- 通过 `scp` 投送到 NAS
- NAS 执行 `docker load`
- 在远端 compose 目录执行 `docker compose up -d --pull never --force-recreate`

## 发布 multi-arch 镜像

```bash
# 默认: edwardtoday/photoframe-orchestrator:<git短sha> + latest
scripts/release-orchestrator-image.sh

# 指定 tag
scripts/release-orchestrator-image.sh 0.1.0

# 指定 buildx builder（例如带 DNS 配置的 builder）
BUILDER_NAME=photoframe-dns scripts/release-orchestrator-image.sh

# 关闭“rebase latest”失败兜底路径
ENABLE_REBASE_FALLBACK=0 scripts/release-orchestrator-image.sh
```

访问：`http://<NAS_IP>:18081/`

管理页里的 `PHOTOFRAME_TOKEN` 输入框会保存在浏览器本地（localStorage），避免每次刷新重复输入。

## 环境变量

- `DAILY_IMAGE_URL_TEMPLATE`：无插播时的每日图片模板，支持 `%DATE%`
- `PUBLIC_BASE_URL`：返回给设备的资源 URL 前缀
- `DEFAULT_POLL_SECONDS`：默认轮询周期（秒）
- `PHOTOFRAME_TOKEN`：管理接口 token（Web 管理页、插播编辑、设备 token 审批等后台接口）
- `DEVICE_TOKEN_MAP_JSON`：设备 token 映射（JSON 对象，例：`{"pf-d9369c80":"devtoken-xxx"}`）
- `DEVICE_TOKEN_MAP`：设备 token 映射（CSV 兼容写法，例：`pf-d9369c80=devtoken-xxx,pf-guest=devtoken-yyy`）
- `DEVICE_TOKEN_MAP_JSON` / `DEVICE_TOKEN_MAP` 可留空：此时设备首次携带 token 请求会进入待审批，需在管理页点击“信任”后放行
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


## 设备离家场景建议

- 若设备可能不在家，请把 `image_url_template` 配成公网 `daily.bmp` 地址（含 token）。
- 即使设备访问不到本地 orchestrator（无法实时下发配置），仍可在每次联网唤醒时拉到应显示图片。

## 公网暴露建议（仅设备接口）

建议仅对公网放行以下路径，管理页与编辑接口继续只在内网开放：

- `GET /public/daily.bmp`
- `POST /api/v1/device/checkin`
- `GET /api/v1/device/config`
- `POST /api/v1/device/config/applied`

其中 `/api/v1/device/*` 通过 `X-PhotoFrame-Token` 做设备身份校验（优先按 `DEVICE_TOKEN_MAP_JSON` / `DEVICE_TOKEN_MAP`，否则走“首次请求待审批”模式）。

若你需要公网侧直接走 `/api/v1/device/next` 指令流，还需额外放行 `/api/v1/assets/*`。

## 插播开始时间规则

- 指定 `starts_at`：按指定时间开始。
- `starts_at` 为空且是单设备：按设备 `next_wakeup_epoch` 开始，避免设备睡眠期间把窗口耗尽。
- `starts_at` 为空且 `device_id=*`：立即开始。
