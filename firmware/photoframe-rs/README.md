# photoframe-rs

Rust 重写中的新固件工程。

## 当前状态

- 已建立 Cargo workspace，并补齐 `domain` / `contracts` / `app` / `platform-espidf` / `drivers-ffi` / `firmware-device` 六层结构
- 已有宿主机可验证策略：唤醒动作、长按语义、失败退避、URL 候选、配置应用、电源纠偏
- 已接入真实设备主链：NVS 配置、设备身份、多 Wi‑Fi 轮询连接、SNTP 校时、orchestrator `device/config` / `device/next` / `device/checkin`、图片下载、BMP/JPEG 渲染、深睡
- 当前仓库内自研固件逻辑已迁为 Rust；仅保留 Espressif `esp_new_jpeg` 作为外部 vendor 库链接
- 已能通过 Docker 工具链导出可刷机镜像；当前自研固件代码已完成 Rust 收口
- 2026-03-09 已完成首轮真机 smoke：刷机启动、空 Wi‑Fi 场景进入 AP Portal、`GET /`、`GET /api/config`、`GET /api/wifi/scan`、`POST /api/config` 均已打通
- 2026-03-09 已完成联网闭环真机验收：STA 连接获取 IP、`device/config` / `device/next` / `device/checkin` 联调通过（200）、主周期可完成后进入休眠决策
- 已固化构建配置：`ESP_IDF_SDKCONFIG_DEFAULTS` 生效、主任务栈提升到 `16384`、分区表路径对齐 Docker 工作目录
- USB hold 模式已改为低频电源采样（3 秒一次），降低 PMIC I2C 报错噪音并避免高频采样抖动
- 当前量产路径仍是 `../photoframe-fw/`；Rust 路径现已进入真机联调阶段，后续重点转为按键 / EPD / PMIC / 深睡 / 联网闭环验收

## 目录

- `crates/domain/`：宿主机可测的业务状态机与策略
- `crates/contracts/`：与 orchestrator 对齐的协议模型
- `crates/app/`：设备配置模型、URL 策略、主周期编排
- `crates/platform-espidf/`：ESP-IDF 平台适配占位层
- `crates/drivers-ffi/`：底层驱动 FFI 占位层
- `crates/firmware-device/`：设备入口 crate（host stub + espidf 入口位置）

## 本地验证

```bash
cargo test --manifest-path firmware/photoframe-rs/Cargo.toml
scripts/rust-idf-docker.sh 'cargo build --release'
scripts/build-photoframe-rs.sh
```

## 当前产物

- 应用镜像：`firmware/photoframe-rs/dist/photoframe-rs-app.bin`
- 合并镜像：`firmware/photoframe-rs/dist/photoframe-rs.bin`

## 真机刷写与串口

- 构建继续使用 Docker：`scripts/build-photoframe-rs.sh`
- 在 macOS 上，Docker Desktop 不能直接透传 USB 串口，因此真机刷写使用主机上**已存在**的 ESP Python 环境：`~/.espressif/python_env/.../bin/esptool.py`
- 本轮验证使用端口：`/dev/cu.usbmodem111201`
- 刷写示例（推荐更稳的 115200 波特）：`~/.espressif/python_env/idf5.0_py3.13_env/bin/esptool.py --chip esp32s3 --port /dev/cu.usbmodem111201 --baud 115200 write_flash -z 0x0 firmware/photoframe-rs/dist/photoframe-rs.bin`
- 串口抓日志可复用同一 Python 环境中的 `pyserial`，避免额外安装宿主机工具链
- 串口监控建议用 `scripts/monitor-host.sh --once /dev/cu.usbmodem111201 115200`；如启用自动重连，反复打开串口会触发 `USB_UART_CHIP_RESET`，看起来像“重启循环”
