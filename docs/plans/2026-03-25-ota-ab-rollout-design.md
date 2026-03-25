# 2026-03-25 A/B OTA 升级设计

## 目标

为 photoframe 增加一套由 orchestrator 托管的 OTA 升级机制，满足：

1. 管理端可上传新固件到 orchestrator。
2. 设备在下次唤醒时，通过既有 pull 协议得知“有升级可做”。
3. 设备下载固件到非当前运行分区，完成校验后切换启动分区并重启。
4. 若下载失败、写入失败、校验失败，则保持当前旧版本继续运行。
5. 新版本重启后若未通过首轮自检，bootloader 自动回滚到旧版本。

## 非目标

1. 本阶段不做 Secure Boot / Flash Encryption。
2. 本阶段不做差分升级，只传完整 app bin。
3. 本阶段不要求管理页 UI 一次完成，可先落 API 与协议。

## 当前事实

### 分区现状

当前 Rust 固件分区表仅有单 `factory` 应用分区：

- `nvs`
- `otadata`
- `phy_init`
- `factory`

这意味着当前设备无法直接做“下载到另一个 app 槽后切换启动分区”的 A/B OTA。

### 已有能力

- orchestrator 与设备已有稳定唤醒主链：
  - `GET /api/v1/device/next`
  - `POST /api/v1/device/checkin`
  - `GET /api/v1/device/config`
- 构建脚本已能产出：
  - `photoframe-rs-app.bin`：应用分区镜像
  - `photoframe-rs-fullchip.bin`：整片镜像

其中 OTA 只能使用 `app.bin`；`fullchip.bin` 只适合 USB 首刷/迁移。

## 总体方案

### 阶段 1：A/B 分区与回滚基础

目标：先把设备从“单 factory”迁到“双 OTA 槽 + otadata”的布局。

做法：

1. 将 `partitions.csv` 改为 `ota_0 + ota_1` 双槽布局。
2. 打开 bootloader / app rollback 配置：
   - `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y`
   - `CONFIG_APP_ROLLBACK_ENABLE=y`
3. 调整本地构建与整片镜像输出逻辑，使 USB 迁移首刷时默认把应用落到 `ota_0`。

风险：

- 这一步不能通过当前 OTA 自举完成，必须先做一次 USB 迁移刷机。

### 阶段 2：orchestrator 固件制品与 rollout

目标：让 orchestrator 成为 OTA 控制面。

做法：

1. 新增 `firmware_artifacts` 表：
   - `id`
   - `version`
   - `asset_name`
   - `asset_sha256`
   - `size_bytes`
   - `note`
   - `created_epoch`
2. 新增 `firmware_rollouts` 表：
   - `id`
   - `device_id`
   - `artifact_id`
   - `min_battery_percent`
   - `requires_vbus`
   - `enabled`
   - `created_epoch`
3. 新增管理 API：
   - 上传固件 artifact
   - 创建 rollout
   - 查看/取消 rollout
4. 在 `device/next` 返回可选 `firmware_update` 字段。

### 阶段 3：设备侧下载、写槽、切换与回滚

目标：设备能在不影响当前运行版本的前提下安装新版本。

做法：

1. 固件解析 `firmware_update` 指令。
2. 在本地判定是否允许升级：
   - 当前版本是否已等于目标版本
   - 电量是否达到门槛
   - 是否要求 `vbus_good=1`
3. 使用 ESP-IDF OTA API：
   - `esp_ota_get_next_update_partition`
   - `esp_ota_begin`
   - `esp_ota_write`
   - `esp_ota_end`
   - `esp_ota_set_boot_partition`
4. 下载过程中边写边计算 SHA256，与 orchestrator 下发值比对。
5. 只有校验通过才切换 boot partition 并重启。

失败策略：

- 下载失败：继续运行旧版本
- 写入失败：继续运行旧版本
- SHA256 不匹配：继续运行旧版本

### 阶段 4：新版本首启确认

目标：把“能启动”和“通过基本自检”区分开。

做法：

1. 新版本首启时，若当前分区状态为 `PENDING_VERIFY`，暂不立刻确认。
2. 等到主周期至少完成一次基本闭环后，再调用：
   - `esp_ota_mark_app_valid_cancel_rollback`
3. 若在此之前崩溃/复位，bootloader 自动回滚到旧版本。

建议的“通过自检”判定：

- NVS 读写正常
- Wi‑Fi 初始化正常
- 至少完成一次 `device/next` 或 `device/checkin`
- 主循环能走到休眠决策

### 阶段 5：升级状态可观测

目标：管理端能看见设备升级结果，不用猜。

做法：

1. 设备 `checkin` 增加 OTA 状态字段：
   - `running_partition`
   - `ota_state`
   - `ota_target_version`
   - `ota_last_error`
   - `ota_last_attempt_epoch`
2. orchestrator 持久化并展示这些状态。

## 协议建议

在 `DeviceNextResponse` 中新增：

```json
{
  "firmware_update": {
    "rollout_id": 12,
    "version": "0.2.0+abcd1234",
    "app_bin_url": "https://.../api/v1/assets/<sha>.bin?device_id=pf-a1b2c3d4",
    "sha256": "...",
    "size_bytes": 1677721,
    "min_battery_percent": 50,
    "requires_vbus": false,
    "created_epoch": 1760000000
  }
}
```

## Todo

### P0

- [ ] 改 A/B 分区表
- [ ] 打开 rollback 配置
- [ ] 调整构建脚本让整片镜像默认落到 `ota_0`
- [ ] 在 contracts 中定义 `firmware_update` 协议

### P1

- [ ] orchestrator 增加 firmware artifact 存储
- [ ] orchestrator 增加 rollout 管理 API
- [ ] `device/next` 下发 `firmware_update`

### P2

- [ ] 设备侧实现 OTA 下载到 inactive slot
- [ ] 设备侧实现 SHA256 校验
- [ ] 设备侧成功后切换 boot partition 并重启
- [ ] 设备侧失败时保留旧版本运行

### P3

- [ ] 新版本首启自检后 `mark_app_valid_cancel_rollback`
- [ ] `checkin` 回报 OTA 状态
- [ ] orchestrator 展示 OTA 状态与历史

## 实施顺序

1. 先完成 P0，准备一次 USB 迁移刷机。
2. 再完成 P1，让 orchestrator 能托管升级控制面。
3. 再完成 P2/P3，做端到端 OTA。

