# photoframe-orchestrator

用于 NAS 侧托管“每日图片 + 插播任务 + 设备配置下发”的编排服务。

## 能力

- 设备拉取：`GET /api/v1/device/next`
- 设备历史补图：`GET /api/v1/device/history/daily.bmp` / `GET /api/v1/device/history/daily.jpg`
- 设备心跳：`POST /api/v1/device/checkin`
- 电池采样历史：`GET /api/v1/power-samples`（用于控制台曲线/续航估算）
- 设备配置同步：
  - 管理端发布：`POST /api/v1/device-config`
  - 设备查询：`GET /api/v1/device/config`
  - 设备回报：`POST /api/v1/device/config/applied`
  - 历史查询：`GET /api/v1/device-configs`
- 设备按需日志上传：
  - 管理端下发：`POST /api/v1/device-log-requests`
  - 设备回传：`POST /api/v1/device/log-upload`
  - 管理端查看：`GET /api/v1/device-log-requests` / `GET /api/v1/device-log-uploads`
- 固件 OTA 控制面：
  - 固件制品上传：`POST /api/v1/firmware-artifacts/upload`
  - rollout 管理：`POST /api/v1/firmware-rollouts`、`GET /api/v1/firmware-rollouts`
  - 设备通过 `device/next` 获取 `firmware_update` 指令
- Web 上传插播图并设置播放窗口：`POST /api/v1/overrides/upload`
  - 上传时可选服务端 dither：`Bayer / Floyd-Steinberg / Jarvis / Stucki / Lab + CIEDE2000 / Atkinson / Sierra`
  - 选中后，服务端会先按相框 6 色调色板生成实际下发的 BMP/JPEG 资产
  - 管理页“快速送图”复用同一接口：默认面向顶部当前设备、开始时间留空、推荐使用 `sierra`，降低临时送图操作成本
- 管理插播列表：`GET /api/v1/overrides`、`DELETE /api/v1/overrides/{id}`
- 图片发布历史（同一条记录跟踪 `sent -> displayed`）：`GET /api/v1/publish-history`
- 管理页预览当前下发图：`GET /api/v1/preview/current.bmp`
  - 管理页可直接使用 `PHOTOFRAME_TOKEN` 预览，不要求再填设备 token
  - 若上游是 `immich-featured-today`，预览响应会透传 `X-IFT-*` 元数据（asset/layout/crop/display_score），控制台可直接看到当前成片来自哪张图、用的什么构图策略
- 公网日图代理：`GET /public/daily.bmp` / `GET /public/daily.jpg`（token 保护，且优先返回当前生效插播）
- 历史补图接口会按显式 `date=YYYY-MM-DD` 使用与 daily 相同的裁剪 / dither / 缓存链路，供设备在本地 TF 缺少某天图片时按需回源补齐
- Web 管理页：`GET /`（含图片发布历史 + 设备配置发布历史 + 当前下发预览 + 设备 token 审批）
- 设备状态页会直接显示设备最近一次 checkin 上报的 `firmware_version`
- 设备状态页会给出电源告警：区分 `USB debug mode` 导致的连续高频活跃，与“电池下平均电流偏高”的待机底流异常
- 控制台顶部会显示 orchestrator 自身的服务版本与 git sha，便于确认当前部署的是哪次构建
- 设备配置“填空式表单”：不再手写 JSON，灰字提示来自设备最近上报配置
- 设备配置页提供 daily.bmp/daily.jpg URL 快捷填入（当前服务 / 公网示例）
- 创建插播后给出“预计生效时间/可能过期”可读提示，便于和设备唤醒周期对齐
- 支持“下次唤醒时上传日志”的一次性诊断请求：仅在显式请求时上传有限日志，不引入每轮固定成本
- 已预留 A/B OTA 升级控制面；注意当前设备若仍是旧单分区布局，需先做一次 USB 迁移刷机后才能启用 ping-pong OTA

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

QNAP/NAS 生产环境默认使用 `network_mode: host`，并把 Daily upstream 指到
`http://127.0.0.1:8000/image/480x800.jpg?date=%DATE%`。
原因是 `immich-featured-today` 通常跑在同机宿主网络；若 orchestrator 留在 bridge
网络里，容器内未必能直接访问宿主的 `8000`，也容易误把 daily 源写成公网地址后再回源失败。

## 生产 Docker（NAS，离线投送，不 pull）

适合 NAS 外网不稳定、或希望明确绕过 `docker compose pull` 的场景。

```bash
# 默认目标：tvs675-lan（ssh config）
# 默认 tag：当前 git 短 SHA
scripts/deploy-orchestrator-offline-to-tvs675-lan.sh

# 指定 tag
scripts/deploy-orchestrator-offline-to-tvs675-lan.sh 0.1.0

# 只预演不执行
DRY_RUN=1 scripts/deploy-orchestrator-offline-to-tvs675-lan.sh
```

如遇 ssh-agent 不可用/签名失败，可显式指定私钥并强制仅使用该身份：

```bash
SSH_IDENTITY_FILE=~/.ssh/id_rsa scripts/deploy-orchestrator-offline-to-tvs675-lan.sh
```

脚本关键行为：
- 本机构建 `linux/amd64` 镜像并导出 tar
- 通过 `scp` 投送到 NAS
- NAS 执行 `docker load`
- 默认把远端 `docker-compose.yml` 的 `image:` 固定到本次发布的 `IMAGE_REPO:TAG`，避免 watchtower 被 Docker Hub 上的 `latest` 覆盖
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

- `DAILY_IMAGE_URL_TEMPLATE`：无插播时的每日图片模板，支持 `%DATE%`；推荐指向 `immich-featured-today` 的 `480x800.jpg`
  同机部署时优先用 `http://127.0.0.1:8000/image/480x800.jpg?date=%DATE%`
- `PUBLIC_BASE_URL`：返回给设备的资源 URL 前缀
- `DEFAULT_POLL_SECONDS`：默认轮询周期（秒）
- `PHOTOFRAME_TOKEN`：管理接口 token（Web 管理页、插播编辑、设备 token 审批等后台接口）
- `DEVICE_TOKEN_MAP_JSON`：设备 token 映射（JSON 对象，例：`{"pf-d9369c80":"devtoken-xxx"}`）
- `DEVICE_TOKEN_MAP`：设备 token 映射（CSV 兼容写法，例：`pf-d9369c80=devtoken-xxx,pf-guest=devtoken-yyy`）
- `DEVICE_TOKEN_MAP_JSON` / `DEVICE_TOKEN_MAP` 可留空：此时设备首次携带 token 请求会进入待审批，需在管理页点击“信任”后放行
- `PUBLIC_DAILY_BMP_TOKEN`：公网日图接口口令（`/public/daily.bmp`，为空则禁用）
- `PHOTOFRAME_ASSET_JPEG_QUALITY`：插播上传时生成 `.jpg` 派生资源的质量参数（40-95，默认 85）
- `DAILY_FETCH_TIMEOUT_SECONDS`：公网日图代理拉取上游超时（秒，默认 10）
- `DAILY_UPSTREAM_REVALIDATE_SECONDS`：daily 缓存向上游做条件校验的最小间隔（秒，默认 300）；到期后会带 `If-None-Match` / `If-Modified-Since` 回源确认是否变更
- `DAILY_ASSET_RETENTION_DAYS`：Daily 静态缓存保留天数（默认 14；仅清理 `daily-*.bmp/.jpg`）
- `POWER_ALERT_BATTERY_CAPACITY_MAH`：控制台估算电池平均电流时使用的电池容量假设（默认 1500）
- `POWER_ALERT_HIGH_STANDBY_MA`：判定“待机底流偏高”的平均电流阈值（默认 3.0mA）
- `POWER_ALERT_ACTIVE_GAP_SECONDS`：判定“连续活跃窗口”的相邻下发最大间隔（默认 120 秒）
- `POWER_ALERT_ACTIVE_RECENT_SECONDS`：仅当最近一次高频活跃仍落在这个窗口内时，控制台才显示连续活跃告警（默认 1800 秒）
- `TZ`：服务端时区

## 服务端 Dither（Daily + 插播）

- 控制台“当前下发预览”支持选择并保存 Daily Dither 算法；预览会即时试算，保存后 `/public/daily.*` 与 `/api/v1/preview/current.*` 会统一使用该算法。
- Daily 链路现在默认从上游 JPG 抓图，服务端会统一裁剪到 `480x800`，再量化到设备 6 色调色板并生成 `.bmp` / `.jpg` 缓存。
- 若上游响应带 `X-IFT-*` 元数据头，Daily 缓存会一并记录并在预览接口透传，便于控制台确认当前 full-bleed 布局、crop 策略和上游 asset_id。
- Daily 缓存会记住上游 `ETag` / `Last-Modified`；到达 `DAILY_UPSTREAM_REVALIDATE_SECONDS` 后会做条件回源。若上游未变，或虽回源成功但最终渲染结果字节不变，则不会重写本地文件，便于设备继续命中稳定 ETag。
- 控制台“创建插播”也支持选择服务端 dither 算法。
- `保持原图`：维持当前行为，只做裁剪缩放，不做服务端预抖动。
- 其余选项会在 orchestrator 侧先量化到设备 6 色调色板，再生成 `.bmp` / `.jpg` 派生资源。
- 当前实现覆盖文章里提到的主要算法族：`Bayer 4x4`、`Floyd-Steinberg`、`Jarvis (JJN)`、`Stucki`、`Lab + CIEDE2000`、`Atkinson`、`Sierra`。
- `Lab + CIEDE2000`：按 issue 10 的思路，引入设备目标色表、`RGB -> Lab -> ΔE00` 最近色匹配，并对绿色候选施加轻微惩罚；误差扩散仍沿用现有 RGB 空间扩散链路。
- 设备端建议使用 `color_process_mode=2`（认为输入已是 6 色），或至少使用新的 `color_process_mode=0` 直转热路径，避免旧式全屏预扫描带来的固定时延。

## 公网日图接口

- `GET /public/daily.bmp` / `GET /public/daily.jpg`
- 鉴权：请求头 `X-Photo-Token` 或 query `?token=`
- 可选：`device_id`（用于按设备匹配插播）
- 额外行为：当 `device_id` 非 `*` 时，会更新该设备的 `last_seen`（控制台 `last_checkin`），并用服务端已保存的最近电量值写入一条采样点（若无则跳过），用于判断设备是否仍在活跃拉图。
- 行为：
  1. 优先返回该设备当前生效的插播图（若存在）
  2. 否则抓取 `DAILY_IMAGE_URL_TEMPLATE` 的当日图（推荐 JPG），按当前 Daily Dither 算法生成并返回 BMP/JPEG
- 设备下发策略补充：
  - `/api/v1/device/next` 现在统一下发静态 `/api/v1/assets/daily-...` 资源；URL 会带 `device_id`，设备继续使用自己的 `X-PhotoFrame-Token` 拉取即可。
  - Daily 静态缓存会按“日期 + 算法 + palette profile”分桶写入 `data/assets/daily-cache/`，并自动清理超出保留天数的旧日图缓存。
  - 仅当设备不支持 BMP 时，daily 才回退到 JPEG 派生资源。

更多字段与示例见 `docs/orchestrator-api.md`。

## Wi-Fi 列表管理（设备配置）

- 控制台“设备配置下发”中的 Wi-Fi 区域支持对设备已知 Wi-Fi 做增删改查。
- 语义为“完整替换设备端列表”（最多 8 条）：
  - 勾选“替换设备 Wi-Fi 列表”后，下发内容会覆盖设备当前列表。
  - 提交空列表会清空设备 Wi-Fi 列表。
  - 某条仅填 SSID、不填密码时：设备端会保留该 SSID 现有密码（若该 SSID 已存在）。
- 设备每轮 `checkin` 上报 `reported_config.wifi_profiles`（密码不会明文回显），控制台可查看当前已记住 SSID。

## 设备离家场景建议

- 若设备可能不在家，请把 `image_url_template` 配成公网 `daily.bmp` 地址（含 token）。
- 即使设备访问不到本地 orchestrator（无法实时下发配置），仍可在每次联网唤醒时拉到应显示图片。

## 公网暴露建议（仅设备接口）

建议仅对公网放行以下路径，管理页与编辑接口继续只在内网开放：

- `GET /public/daily.bmp`
- `POST /api/v1/device/checkin`（设备回报本轮是否真正完成显示，用于把同一条发布记录从 `sent` 更新为 `displayed`）
- `GET /api/v1/device/config`
- `POST /api/v1/device/config/applied`

其中 `/api/v1/device/*` 通过 `X-PhotoFrame-Token` 做设备身份校验（优先按 `DEVICE_TOKEN_MAP_JSON` / `DEVICE_TOKEN_MAP`，否则走“首次请求待审批”模式）。

若你需要公网侧直接走 `/api/v1/device/next` 指令流，还需额外放行 `/api/v1/assets/*`，并保留设备请求头里的 `X-PhotoFrame-Token`。

## 插播开始时间规则

- 指定 `starts_at`：按指定时间开始。
- `starts_at` 为空且是单设备：按设备 `next_wakeup_epoch` 开始，避免设备睡眠期间把窗口耗尽。
- `starts_at` 为空且 `device_id=*`：立即开始。
