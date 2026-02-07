# photoframe

基于 Waveshare **ESP32-S3-PhotoPainter** 相框（7.3" 彩色墨水屏）做二次开发。

## 目标（第一阶段）

- 固件支持配网连接家里 Wi-Fi
- 每小时（可配置）从指定 HTTP 地址拉取一张 **480x800 的 BMP（24-bit）**
  - 例如：`http://192.168.58.113:8000/image/480x800?date=2026-02-07`
- 将图片刷新到墨水屏
- 其他时间进入深度睡眠以省电（墨水屏断电保持显示）

## 仓库结构

- `upstream/ESP32-S3-PhotoPainter/`：Waveshare 开源固件（Git submodule）
- `firmware/photoframe-fw/`：你的私有固件实现（不改 upstream）
- `references/waveshare/wiki/`：Wiki 页面快照/链接索引
- `references/waveshare/downloads/`：从 Wiki 相关链接下载的资料（默认不入 git，可用脚本重拉）
- `scripts/`：拉取资料、生成索引等脚本

## 初始化

1) 拉取 submodule：

```bash
git submodule update --init --recursive
```

2) 下载/整理 Waveshare 资料：

```bash
python3 scripts/fetch_waveshare_assets.py
```

3) 查看 ESP-IDF Docker 工作流（编译/烧录/调试）：

```bash
cat docs/workflow-esp-idf-docker.md
```

## 开发阶段规划

- A：打通 Docker 编译 + 宿主机烧录/调试链路
- B：先编译通过上游 Waveshare 固件
- C：在本仓库新增自有固件（不改 upstream）
- D：补齐增强能力（重试退避、按键强制刷新、断网恢复、配置接口）

当前状态：A/B/C/D 已落地并完成本地编译验证。
