# Rust Firmware Rewrite Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 `photoframe` 建立一套可测试、可逐步替换现有 C++ 固件的 Rust 固件工程骨架，并优先迁移高风险状态机逻辑。

**Architecture:** 采用 `Rust + ESP-IDF` 路线，先把协议模型与业务决策拆到宿主机可测的 crate，再用 `platform-espidf` / `drivers-ffi` 接入硬件能力。V1 不追求纯 Rust 驱动，优先保证功能等价、协议兼容与功耗行为不回退。

**Tech Stack:** Rust、Cargo workspace、`serde`、`thiserror`、`esp-idf-hal`、`esp-idf-svc`、宿主机 `cargo test`

---

## 分阶段任务

### Task 1: 建立 Rust workspace 与首个 domain crate

**Files:**
- Create: `firmware/photoframe-rs/Cargo.toml`
- Create: `firmware/photoframe-rs/crates/domain/Cargo.toml`
- Create: `firmware/photoframe-rs/crates/domain/src/lib.rs`
- Create: `firmware/photoframe-rs/crates/domain/tests/wake_policy.rs`
- Modify: `.gitignore`

**Step 1: 写失败测试**

- 在 `firmware/photoframe-rs/crates/domain/tests/wake_policy.rs` 写 3 个测试：
  - `spurious_ext1_skips_network_cycle`
  - `key_wake_triggers_manual_sync`
  - `boot_wake_triggers_force_refresh`

**Step 2: 运行测试验证 RED**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-domain`

Expected: 编译失败，提示缺少 `WakeSource` / `CycleAction` / `decide_cycle_action`。

**Step 3: 写最小实现**

- 在 `firmware/photoframe-rs/crates/domain/src/lib.rs` 定义：
  - `WakeSource`
  - `CycleAction`
  - `decide_cycle_action()`

**Step 4: 运行测试验证 GREEN**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-domain`

Expected: 上述 3 个测试通过。

### Task 2: 迁移按键长按语义与窗口决策

**Files:**
- Modify: `firmware/photoframe-rs/crates/domain/src/lib.rs`
- Create: `firmware/photoframe-rs/crates/domain/tests/button_policy.rs`

**Step 1: 写失败测试**

- 覆盖以下行为：
  - `long_press_key_opens_sta_portal`
  - `long_press_boot_clears_wifi_and_enters_ap_portal`
  - `short_press_key_does_not_open_sta_portal`

**Step 2: 运行测试验证 RED**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-domain button_policy`

Expected: 编译失败或断言失败，说明长按语义尚未实现。

**Step 3: 写最小实现**

- 增加 `LongPressAction` 与判定函数。

**Step 4: 运行测试验证 GREEN**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-domain button_policy`

Expected: 测试通过。

### Task 3: 迁移退避与 PMIC 软失败策略

**Files:**
- Modify: `firmware/photoframe-rs/crates/domain/src/lib.rs`
- Create: `firmware/photoframe-rs/crates/domain/tests/backoff_policy.rs`

**Step 1: 写失败测试**

- 覆盖以下行为：
  - 正常失败按指数退避
  - 达到最大退避时钳制
  - PMIC 软失败保持常规间隔
  - 成功后失败计数清零

**Step 2: 运行测试验证 RED**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-domain backoff_policy`

Expected: 失败，因为退避函数不存在或行为未满足。

**Step 3: 写最小实现**

- 增加 `BackoffPolicy` / `FailureKind` / `next_sleep_seconds()`。

**Step 4: 运行测试验证 GREEN**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-domain backoff_policy`

Expected: 测试通过。

### Task 4: 建立 orchestrator contracts crate

**Files:**
- Create: `firmware/photoframe-rs/crates/contracts/Cargo.toml`
- Create: `firmware/photoframe-rs/crates/contracts/src/lib.rs`
- Create: `firmware/photoframe-rs/crates/contracts/tests/device_next.rs`
- Create: `firmware/photoframe-rs/crates/contracts/tests/device_config.rs`

**Step 1: 写失败测试**

- `device_next.rs` 覆盖字段：`image_url`、`source`、`poll_after_seconds`、`device_clock_ok`、`effective_epoch`。
- `device_config.rs` 覆盖字段：`config_version`、`config`、`server_epoch`、`device_epoch`。

**Step 2: 运行测试验证 RED**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-contracts`

Expected: 缺少协议模型或序列化实现。

**Step 3: 写最小实现**

- 定义与现有 orchestrator 协议兼容的 Rust 结构体与 serde 映射。

**Step 4: 运行测试验证 GREEN**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-contracts`

Expected: 协议测试通过。

### Task 5: 建立 app / platform 边界骨架

**Files:**
- Create: `firmware/photoframe-rs/crates/app/Cargo.toml`
- Create: `firmware/photoframe-rs/crates/app/src/lib.rs`
- Create: `firmware/photoframe-rs/crates/platform-espidf/Cargo.toml`
- Create: `firmware/photoframe-rs/crates/platform-espidf/src/lib.rs`
- Create: `firmware/photoframe-rs/crates/app/tests/cycle_smoke.rs`

**Step 1: 写失败测试**

- 用 fake adapter 覆盖最小周期：
  - 根据 `WakeSource` 决定是否联网
  - 根据 `device/next` 决定图片来源与 sleep seconds
  - 生成 check-in payload

**Step 2: 运行测试验证 RED**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-app`

Expected: 缺少 app service / adapter trait。

**Step 3: 写最小实现**

- 定义 `Clock` / `Storage` / `Network` / `Display` / `Power` trait。
- 写一个最小 `CycleRunner`。

**Step 4: 运行测试验证 GREEN**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml -p photoframe-app`

Expected: smoke 测试通过。

### Task 6: 接入 ESP-IDF 工程与 FFI 过渡层

**Files:**
- Create: `firmware/photoframe-rs/crates/drivers-ffi/Cargo.toml`
- Create: `firmware/photoframe-rs/crates/drivers-ffi/src/lib.rs`
- Create: `firmware/photoframe-rs/README.md`
- Modify: `README.md`
- Modify: `docs/firmware-photoframe-fw.md`

**Step 1: 写失败测试 / 编译检查**

- 先添加宿主机可跑的 FFI 边界 smoke 测试或最小编译检查。

**Step 2: 运行 RED/编译验证**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml`

Expected: 未接入平台层或 FFI 时失败。

**Step 3: 写最小实现**

- 把现有 EPD / PMIC 能力先封装成最薄接口。
- 让 `app` 侧不直接依赖底层细节。

**Step 4: 运行 GREEN**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml`

Expected: 宿主机测试通过。

### Task 7: 真机 smoke 与文档收敛

**Files:**
- Modify: `README.md`
- Modify: `docs/firmware-photoframe-fw.md`
- Modify: `docs/orchestrator-api.md`
- Modify: `docs/featured-photo-integration.md`

**Step 1: 整理真机 checklist**

- 配网、拉图、304、JPEG、check-in、深睡、误唤醒、长按/短按语义、电量上报。

**Step 2: 运行验证命令**

Run: `cargo test --manifest-path firmware/photoframe-rs/Cargo.toml`

Expected: 宿主机测试全绿。

**Step 3: 更新文档**

- 保证 README 和设计文档反映当前新旧两套固件关系、迁移状态与验证方法。

## 当前推荐执行顺序

1. Task 1
2. Task 2
3. Task 3
4. Task 4
5. Task 5
6. Task 6
7. Task 7

Plan complete and saved to `docs/plans/2026-03-07-rust-firmware-rewrite.md`. 当前自检结论：需求边界、功能基线、验收清单与首阶段落点已明确，可进入实现模式，从 Task 1 开始。
