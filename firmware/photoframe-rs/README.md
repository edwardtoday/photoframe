# photoframe-rs

Rust 重写中的新固件工程。

## 当前状态

- 已建立 Cargo workspace，并补齐 `domain` / `contracts` / `app` / `platform-espidf` / `drivers-ffi` / `firmware-device` 六层结构
- 已有宿主机可验证策略：唤醒动作、长按语义、失败退避、URL 候选、配置应用、电源纠偏
- 已接入真实设备主链：NVS 配置、设备身份、多 Wi‑Fi 轮询连接、SNTP 校时、orchestrator `device/config` / `device/next` / `device/checkin`、图片下载、BMP/JPEG 渲染、深睡
- 当前仓库内自研固件逻辑已迁为 Rust；仅保留 Espressif `esp_new_jpeg` 作为外部 vendor 库链接
- 已能通过 Docker 工具链导出可刷机镜像；当前自研固件代码已完成 Rust 收口，下一阶段主要缺口转为真机刷写与行为验证
- 当前量产路径仍是 `../photoframe-fw/`；Rust 路径现已进入真机验证前的收口阶段

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
