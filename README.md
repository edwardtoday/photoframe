# photoframe

基于 Waveshare **ESP32-S3-PhotoPainter** 相框（7.3" 彩色墨水屏）做二次开发。

## 目标（当前）

- 固件支持配网连接家里 Wi-Fi
- 每小时（可配置）拉取并显示 `480x800` BMP
- 支持局域网“插播”任务（指定时长，过期自动回到日常图）
- 支持设备远端配置下发（轮询参数、URL、token 等）
- 按键唤醒后可开启 120 秒本地配置窗口，其余时间深度睡眠省电

## 仓库结构

- `upstream/ESP32-S3-PhotoPainter/`：Waveshare 开源固件（Git submodule）
- `firmware/photoframe-fw/`：你的私有固件实现（不改 upstream）
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
