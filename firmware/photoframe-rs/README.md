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
- 2026-03-09 已修复 daily 图片兼容链路：同源图片请求会自动携带 `X-PhotoFrame-Token`，并与 orchestrator 的 BMP 代理下发策略配合，避免上游 progressive JPEG 导致 `render failed`
- 2026-03-09 已修复 `device/checkin` 回归：上报失败改回 best-effort，不再把整轮主周期打成 `cycle failed`；同时补充按 base URL / attempt 打印的 POST 诊断日志，便于串口定位为何“能拉图但不报电量”
- 已固化构建配置：`ESP_IDF_SDKCONFIG_DEFAULTS` 生效、主任务栈提升到 `16384`、分区表路径对齐 Docker 工作目录
- USB hold 模式已改为低频电源采样（3 秒一次），降低 PMIC I2C 报错噪音并避免高频采样抖动
- 固件启动时会自动补齐 3 条内置 Wi‑Fi 配置（`OpenWrt`、`Qing-IoT`、`Qing-AP`），并将 Wi‑Fi 列表容量扩展到 8 条，确保在目标环境可联网且可继续扩展
- 远端 `wifi_profiles` 配置语义已改为“完整替换设备列表”，支持通过 orchestrator 做增删改（提交空数组可清空）
- 已补充恢复机制：当设备因误刷整片镜像导致 NVS 丢失时，可通过 `PHOTOFRAME_BOOTSTRAP_CONFIG_JSON` 构建一版恢复固件，把 orchestrator / photo token 等关键配置写回
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
- 整片镜像：`firmware/photoframe-rs/dist/photoframe-rs-fullchip.bin`（会覆盖 NVS，仅限空片首刷）

## 真机刷写与串口

- 构建继续使用 Docker：`scripts/build-photoframe-rs.sh`
- 在 macOS 上，Docker Desktop 不能直接透传 USB 串口，因此真机刷写使用主机上**已存在**的 ESP Python 环境：`~/.espressif/python_env/.../bin/esptool.py`
- 本轮验证使用端口：`/dev/cu.usbmodem111201`
- 现场升级默认使用：`scripts/flash-photoframe-rs.sh /dev/cu.usbmodem111201 115200`（默认刷 `dist/photoframe-rs-app.bin`，分段刷写，保留 NVS）
- 刷机脚本会校验：若 `dist/photoframe-rs-app.bin` 早于当前 ELF 产物，会直接报错并要求先执行 `scripts/build-photoframe-rs.sh`，避免静默刷入陈旧 app 镜像
- 仅空片首刷才使用整片镜像，例如：`~/.espressif/python_env/idf5.0_py3.13_env/bin/esptool.py --chip esp32s3 --port /dev/cu.usbmodem111201 --baud 115200 write_flash -z 0x0 firmware/photoframe-rs/dist/photoframe-rs-fullchip.bin`
- 串口抓日志可复用同一 Python 环境中的 `pyserial`，避免额外安装宿主机工具链
- 串口监控建议用 `scripts/monitor-host.sh --once /dev/cu.usbmodem111201 115200`；如启用自动重连，反复打开串口会触发 `USB_UART_CHIP_RESET`，看起来像“重启循环”

## NVS 恢复

- 误刷整片镜像会把 `0x9000` 开始的 NVS 分区抹成 `0xFF`，设备会丢失 `orch_url` / `orch_tok` / `photo_tok` 等运行配置
- 恢复时可在构建前注入：

```bash
export PHOTOFRAME_BOOTSTRAP_CONFIG_JSON='{"orchestrator_base_url":"https://901.qingpei.me:40009","orchestrator_token":"...","photo_token":"...","image_url_template":"https://901.qingpei.me:40009/public/daily.bmp?device_id=%DEVICE_ID%","timezone":"CST-8","display_rotation":2}'
scripts/build-photoframe-rs.sh
scripts/flash-photoframe-rs.sh /dev/cu.usbmodem111201 115200 --app-bin firmware/photoframe-rs/dist/photoframe-rs-recovery-app.bin
```

- 该 bootstrap 仅在设备看起来像“配置丢失”（`remote_config_version == 0` 且 orchestrator / photo token / 默认地址缺失）时才应用，不会覆盖已正常运行的设备
