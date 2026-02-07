# ESP-IDF（Docker）编译 / 宿主机烧录 / 调试工作流

本仓库目标：在 macOS 上尽量“零污染”环境。

- **容器内**：ESP-IDF 编译（固定版本，便于复现）
- **宿主机**：串口监控、烧录（Docker Desktop 对 USB 透传不稳定/不可用）

> 说明：上游 `sdkconfig.defaults` 标注了 ESP-IDF `5.5.1`，见 `upstream/ESP32-S3-PhotoPainter/01_Example/xiaozhi-esp32/sdkconfig.defaults:2`。

## 阶段 A：工具链准备

### 1) 拉取 ESP-IDF Docker 镜像

```bash
scripts/idf-docker.sh idf.py --version
```

预期：输出 `ESP-IDF v5.5.1`。

### 2) 安装宿主机最小工具（烧录/串口）

```bash
scripts/setup-host-tools.sh
```

会在仓库根目录创建 `.venv-host-tools/`，并安装：
- `esptool`
- `pyserial`

## 阶段 B：编译 Waveshare 上游固件

```bash
scripts/build-upstream.sh
```

预期：成功生成 `upstream/ESP32-S3-PhotoPainter/01_Example/xiaozhi-esp32/build/flasher_args.json` 等构建产物。

> 本仓库在 2026-02-07 已验证该步骤可编译通过，并生成 `xiaozhi.bin`。

## 阶段 C：编译自有固件

```bash
scripts/build-photoframe-fw.sh
```

预期：成功生成 `firmware/photoframe-fw/build/flasher_args.json` 与 `photoframe_fw.bin`。

> 本仓库在 2026-02-07 已验证该步骤可编译通过。

## 烧录（宿主机）

1) 先找到串口：

```bash
ls -1 /dev/cu.*
```

2) 烧录：

```bash
scripts/flash-host.py \
  --port /dev/cu.usbmodemXXXX \
  --project-dir firmware/photoframe-fw
```

若仅检查命令拼装是否正确（不实际烧录）：

```bash
scripts/flash-host.py \
  --port /dev/cu.usbmodemXXXX \
  --project-dir firmware/photoframe-fw \
  --dry-run
```

> 这里会自动读取 `build/flasher_args.json` 组装 esptool 命令，以减少手工抄地址的错误。

## 串口监控（宿主机）

```bash
scripts/monitor-host.sh /dev/cu.usbmodemXXXX 115200
```

## 调试建议（后续增强）

- 先把 `monitor` 跑通，确保能抓到崩溃日志。
- 若需要更强的定位能力：
  - 开启 core dump（Flash 或 UART）
  - 使用 `xtensa-esp32s3-elf-addr2line` 或 `esp-idf` 的 `idf.py monitor` 进行 backtrace 解析
