# photoframe

基于 Waveshare **ESP32-S3-PhotoPainter** 相框（7.3" 彩色墨水屏）做二次开发。

## 目标（当前）

- 固件支持配网连接家里 Wi-Fi
- 每小时（可配置）拉取并显示 `800x480/480x800` 图片（BMP/JPEG）
- 支持局域网“插播”任务（指定时长，过期自动回到日常图）
- 支持设备远端配置下发（轮询参数、URL、token 等）
- 按键语义：`KEY` 短按手动同步，长按 `KEY` 开 120 秒本地配置窗口，长按 `BOOT` 清 Wi-Fi 进入配网，其余时间深度睡眠省电

## 功能清单（当前实现）

### 设备固件（Rust）

- [x] 设备身份生成与本地 NVS 配置持久化
- [x] 多 Wi‑Fi 轮询连接（内置 `OpenWrt` / `Qing-IoT` / `Qing-AP`）
- [x] 支持通过 orchestrator 下发 `wifi_profiles`（设备侧完整替换）
- [x] SNTP 校时与设备时钟兜底协同（`now_epoch` 上报）
- [x] 拉图与渲染：`BMP/JPEG`（`800x480` / `480x800`）
- [x] 图片请求支持 token / 条件请求（`ETag` / `If-None-Match`）
- [x] 远端配置下发与应用回报（`device/config`）
- [x] 任务拉取与状态上报（`device/next` / `device/checkin`）
- [x] 显示参数可配置（如 `display_rotation=0/2`、色彩处理、抖动）
- [x] 省电策略：失败退避、深睡、误唤醒兜底（`SPURIOUS_EXT1`）
- [x] 按键行为：短按手动同步、长按进入配置/清网
- [x] AP/STA Portal 本地配置页面

### 编排服务（Orchestrator）

- [x] 设备注册、令牌审批与鉴权（设备 token）
- [x] 设备配置增删改查与版本化下发
- [x] 设备在线状态、最近 checkin、下次唤醒、错误状态可视化
- [x] 日常图片下发与局域网插播任务调度
- [x] `daily.bmp` / `daily.jpg` 与内部 `preview` 端点下发
- [x] 图片端点支持条件请求（`ETag` / `304`）以省流省电

## 仓库结构

- `upstream/ESP32-S3-PhotoPainter/`：Waveshare 开源固件（Git submodule）
- `firmware/photoframe-fw/`：你的私有固件实现（不改 upstream）
- `firmware/photoframe-rs/`：Rust 重写中的新固件工程（分阶段迁移）
- `services/photoframe-orchestrator/`：NAS 侧编排服务（Docker + Web）
- `references/waveshare/wiki/`：Wiki 页面快照/链接索引
- `references/waveshare/downloads/`：从 Wiki 相关链接下载的资料（默认不入 git，可脚本重拉）
- `scripts/`：资料下载、编译、烧录脚本

## 初始化

1) 拉取 submodule：

```bash
git submodule update --init --recursive
```

2) 下载/整理 Waveshare 资料：

```bash
python3 scripts/fetch_waveshare_assets.py
```

3) 一键更新 Waveshare 官方 submodule + 重拉 releases：

```bash
scripts/sync-waveshare-official.sh
```

4) 查看 ESP-IDF Docker 工作流（编译/烧录/调试）：

```bash
cat docs/workflow-esp-idf-docker.md
```

## 固件开发进度

- A：Docker 编译 + 宿主机烧录/调试链路 ✅
- B：上游 Waveshare 固件编译验证 ✅
- C：自有固件（配网、拉图、深睡）✅
- D：增强能力（重试退避、按键强刷、断网恢复、配置接口）✅
- E：编排服务接入（插播 + 预计生效时间 + 图片发布历史）✅
- F：设备远端配置同步（下发/查询/应用回报）✅
- G：按键唤醒 120 秒局域网配置窗口 ✅
- H：电池/充电状态采集与 orchestrator 上报 ✅

## Rust 重写计划（进行中）

- 当前量产路径仍为 `firmware/photoframe-fw/`（C++ / ESP-IDF）
- 新路径 `firmware/photoframe-rs/` 已能通过 **Docker 工具链** 编译并导出可刷机镜像：
  - 构建：`scripts/build-photoframe-rs.sh`
  - 产物：`firmware/photoframe-rs/dist/photoframe-rs-app.bin`、`firmware/photoframe-rs/dist/photoframe-rs-fullchip.bin`
- Rust 固件当前已接通：NVS 配置、设备身份生成、多 Wi‑Fi 轮询连接、SNTP 校时、orchestrator 配置同步 / 指令拉取、图片下载、BMP/JPEG 渲染、checkin 上报、按键唤醒判定、AP/STA Portal、深睡进入；自研固件代码已改为 Rust 实现
- 当前阶段目标已从“只能编译骨架”推进到“自研固件全 Rust 化、可编译、可出包、主闭环打通”；2026-03-09 已完成真机 AP Portal smoke + 联网闭环验收（`device/config` / `device/next` / `device/checkin`）
- Rust 固件构建配置已收敛：`sdkconfig.defaults` 通过 `.cargo/config.toml` 注入、主任务栈提升到 `16384`、分区表路径对齐 Docker 工作目录
- Rust 固件每次启动都会确保内置 3 条网络配置（`OpenWrt` / `Qing-IoT` / `Qing-AP`）存在；Wi‑Fi 列表容量已扩至 8 条，并已支持通过 orchestrator 对 `wifi_profiles` 做增删改查（完整替换语义）
- Rust 现场升级默认使用 `scripts/flash-photoframe-rs.sh` 分段刷写，保留 NVS；`photoframe-rs-fullchip.bin` 只用于空片首刷，误刷会清空设备本地 token / Wi‑Fi / orchestrator 配置
- 下一阶段的主要工作是 **真机联调与行为验收**（按键/Portal 提交链路/EPD/PMIC/功耗/联网闭环）
- 重写基线文档：`docs/plans/2026-03-07-rust-firmware-rewrite-design.md`
- 实施计划：`docs/plans/2026-03-07-rust-firmware-rewrite.md`
- 里程碑发布说明：`docs/releases/v0.1.0-rust-fw.md`（tag：`v0.1.0-rust-fw`）
- 本次重写第一优先级仍是：稳定性、可测试性、省电行为不回退

## 启动 NAS 编排服务（镜像拉取模式）

生产推荐：NAS 只保存 compose 和 data，镜像从 Docker Hub 拉取。

```bash
docker compose -f docker-compose.photoframe-orchestrator.prod.yml pull
docker compose -f docker-compose.photoframe-orchestrator.prod.yml up -d
```

## 发布 multi-arch 镜像

```bash
# 默认推送 edwardtoday/photoframe-orchestrator:<git短sha> + latest
scripts/release-orchestrator-image.sh

# 或指定 tag
scripts/release-orchestrator-image.sh 0.1.0
```

- NAS Web：`http://<NAS_IP>:18081/`
- API：见 `docs/orchestrator-api.md`
- 服务说明：`services/photoframe-orchestrator/README.md`
