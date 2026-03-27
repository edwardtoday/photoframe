# OTA 真机验证 Runbook

本文档记录 photoframe 的 A/B OTA 真机验收流程，目标是让后续验证不依赖口头经验。

## 前提

- 设备已经完成 A/B 分区迁移，且能正常通过 orchestrator 联网
- orchestrator 已部署到目标环境
- 本机已具备：
  - `scripts/build-photoframe-rs.sh`
  - `scripts/flash-photoframe-rs.sh`
  - `scripts/validate-ota-host.py`
  - `.venv-host-tools/bin/python -m esptool`

本轮验证使用的典型参数：

```bash
BASE_URL='https://901.qingpei.me:40009'
ADMIN_TOKEN='...'
DEVICE_ID='pf-d9369c80'
PORT='/dev/cu.usbmodem111201'
```

## 结果判定

成功升级的最低验收标准：

- `firmware_version` 变为目标版本
- `running_partition` 从当前槽切到另一槽
- `ota_state = valid`
- `ota_last_error = ""`

失败类场景的最低验收标准：

- 设备仍停留在原版本和原分区
- `running_partition` 不变
- `ota_state` 仍为 `valid`
- 对应 `device-debug-stages` 中出现预期阶段名

## 先看状态

```bash
curl -ksS -H "X-PhotoFrame-Token: ${ADMIN_TOKEN}" \
  "${BASE_URL}/api/v1/devices" \
| jq '.devices[] | select(.device_id=="'"${DEVICE_ID}"'")'
```

查看最近 debug stage：

```bash
curl -ksS -H "X-PhotoFrame-Token: ${ADMIN_TOKEN}" \
  "${BASE_URL}/api/v1/device-debug-stages?device_id=${DEVICE_ID}&limit=20" \
| jq '.items'
```

## 构建规则

正常 OTA 目标镜像：

```bash
PHOTOFRAME_FIRMWARE_VERSION='0.1.0+<tag>' \
scripts/build-photoframe-rs.sh
```

如果要让固件发出 OTA 阶段信标：

```bash
PHOTOFRAME_FIRMWARE_VERSION='0.1.0+<tag>' \
PHOTOFRAME_DEBUG_STAGE_BEACON=1 \
scripts/build-photoframe-rs.sh
```

如果要做测试电源注入：

```bash
PHOTOFRAME_FIRMWARE_VERSION='0.1.0+<tag>' \
PHOTOFRAME_DEBUG_STAGE_BEACON=1 \
PHOTOFRAME_TEST_POWER_OVERRIDE_JSON='{"battery_mv":4000,"battery_percent":80,"charging":0,"vbus_good":0}' \
scripts/build-photoframe-rs.sh
```

说明：

- `PHOTOFRAME_TEST_POWER_OVERRIDE_JSON` 仅在显式设置时生效，生产固件默认不启用
- 刷机脚本默认保留 NVS，只更新 bootloader / partition table / otadata / app

## 刷入基线

```bash
scripts/flash-photoframe-rs.sh "${PORT}" 115200
```

## USB Debug Mode 说明

- 设备检测到 USB Serial/JTAG 已接入时，一轮主周期结束后不会立即进入深睡，而会等待 5 秒后再跑下一轮联网/拉图/上报
- 该模式只用于调试提速，不改变“仅 VBUS 供电但没有串口主机接入”时的省电语义
- 真机观察建议：

```bash
MONITOR_AUTO_RECONNECT_ON_CLEAN_EXIT=1 \
scripts/monitor-host.sh "${PORT}" 115200
```

判定标准：

- 一轮成功后出现 `usb debug mode active, rerun cycle in 5s`
- 约 5 秒后再次出现 `wifi try idx=...`
- 设备不会在两轮之间进入深睡

## 通用 OTA 验证脚本

```bash
python3 scripts/validate-ota-host.py \
  --base-url "${BASE_URL}" \
  --admin-token "${ADMIN_TOKEN}" \
  --device-id "${DEVICE_ID}" \
  --port "${PORT}" \
  --artifact-path firmware/photoframe-rs/dist/photoframe-rs-app.bin \
  --version '0.1.0+<target>' \
  --note '<note>' \
  --poll-interval-seconds 2 \
  --timeout-seconds 420
```

脚本默认会：

- 上传 artifact
- 创建单机 rollout
- 通过一次 USB reset 触发设备立即醒来
- 等待设备达到目标版本
- 清理 rollout

## 场景 1：正向升级

构建目标：

```bash
PHOTOFRAME_FIRMWARE_VERSION='0.1.0+<target>' \
PHOTOFRAME_DEBUG_STAGE_BEACON=1 \
scripts/build-photoframe-rs.sh
```

执行：

```bash
python3 scripts/validate-ota-host.py \
  --base-url "${BASE_URL}" \
  --admin-token "${ADMIN_TOKEN}" \
  --device-id "${DEVICE_ID}" \
  --port "${PORT}" \
  --artifact-path firmware/photoframe-rs/dist/photoframe-rs-app.bin \
  --version '0.1.0+<target>' \
  --note 'ota success validation'
```

期望：

- 设备切到另一槽
- `firmware_version = 0.1.0+<target>`
- `ota_state = valid`

## 场景 2：正向升级 + 日志采集

执行：

```bash
python3 scripts/validate-ota-host.py \
  --base-url "${BASE_URL}" \
  --admin-token "${ADMIN_TOKEN}" \
  --device-id "${DEVICE_ID}" \
  --port "${PORT}" \
  --artifact-path firmware/photoframe-rs/dist/photoframe-rs-app.bin \
  --version '0.1.0+<target>' \
  --note 'ota success with log upload' \
  --log-reason 'ota validation log capture'
```

期望：

- 正向升级成功
- `/api/v1/device-log-requests` 中对应请求状态变为 `completed`
- `/api/v1/device-log-uploads` 中出现对应上传记录

## 附：TF 持久日志链路验收

该项不属于 OTA 本身，但通常和 OTA/重启验证一起做，因为它依赖“受控重启前持久化，下一次 boot 后恢复”。

建议步骤：

1. 先在串口观察启动早期日志，确认 TF 已挂载，且不再出现坏 segment 报错。
2. 通过 `POST /api/v1/device-config` 下发空配置，触发一次 `RebootForConfig`。
3. 串口期望看到：
   - `photoframe-rs: tf persist snapshot boot=<n> lines=<m> truncated=0`
   - `photoframe-rs: tf persist success`
4. 设备重启并重新联网后，创建一次日志采集请求：

```bash
curl -ksS -X POST \
  -H "Content-Type: application/json" \
  -H "X-PhotoFrame-Token: ${ADMIN_TOKEN}" \
  "${BASE_URL}/api/v1/device-log-requests" \
  --data '{
    "device_id": "'"${DEVICE_ID}"'",
    "reason": "tf history validation",
    "max_lines": 200,
    "max_bytes": 65536,
    "expires_in_minutes": 30
  }'
```

5. 在 `/api/v1/device-log-uploads` 中确认：
   - 出现 `boot:1` 与 `boot:2` 两段日志
   - 出现 `photoframe-rs: tf history ready blocks=1 ...`
   - 出现 `photoframe-rs: preparing log upload history_blocks=1 ...`

说明：

- 当前真机已确认 TF 路径需使用 FAT 更稳妥的 8.3 文件名（例如 `pflog00.bin`）；更长文件名在该卡/挂载组合下会触发 `EINVAL`
- 若 TF 不可用，设备仍会回退到 RTC 保留区快照，但那只能保留较小的一段尾部日志，不能替代 TF 环形持久化

## 场景 3：下载中途 reset

执行：

```bash
python3 scripts/validate-ota-host.py \
  --base-url "${BASE_URL}" \
  --admin-token "${ADMIN_TOKEN}" \
  --device-id "${DEVICE_ID}" \
  --port "${PORT}" \
  --artifact-path firmware/photoframe-rs/dist/photoframe-rs-app.bin \
  --version '0.1.0+<target>' \
  --note 'ota reset at 50%' \
  --reset-stage ota_download_50
```

期望：

- `device-debug-stages` 出现 `ota_download_50`
- 注入 reset 后设备仍保持原版本和原分区

## 场景 4：`requires_vbus` 拒绝升级

先刷入带电源覆盖的基线：

```bash
PHOTOFRAME_FIRMWARE_VERSION='0.1.0+<base>' \
PHOTOFRAME_DEBUG_STAGE_BEACON=1 \
PHOTOFRAME_TEST_POWER_OVERRIDE_JSON='{"battery_mv":4000,"battery_percent":80,"charging":0,"vbus_good":0}' \
scripts/build-photoframe-rs.sh
scripts/flash-photoframe-rs.sh "${PORT}" 115200
```

执行：

```bash
python3 scripts/validate-ota-host.py \
  --base-url "${BASE_URL}" \
  --admin-token "${ADMIN_TOKEN}" \
  --device-id "${DEVICE_ID}" \
  --port "${PORT}" \
  --artifact-path firmware/photoframe-rs/dist/photoframe-rs-app.bin \
  --version '0.1.0+<target>' \
  --note 'requires_vbus gating test' \
  --expect-stage ota_skip_requires_vbus \
  --expect-version-unchanged
```

期望：

- `device-debug-stages` 出现 `ota_skip_requires_vbus`
- 设备版本和分区不变

## 场景 5：低电量拒绝升级

先刷入带电量覆盖的基线：

```bash
PHOTOFRAME_FIRMWARE_VERSION='0.1.0+<base>' \
PHOTOFRAME_DEBUG_STAGE_BEACON=1 \
PHOTOFRAME_TEST_POWER_OVERRIDE_JSON='{"battery_mv":3600,"battery_percent":10,"charging":0,"vbus_good":1}' \
scripts/build-photoframe-rs.sh
scripts/flash-photoframe-rs.sh "${PORT}" 115200
```

执行：

```bash
python3 scripts/validate-ota-host.py \
  --base-url "${BASE_URL}" \
  --admin-token "${ADMIN_TOKEN}" \
  --device-id "${DEVICE_ID}" \
  --port "${PORT}" \
  --artifact-path firmware/photoframe-rs/dist/photoframe-rs-app.bin \
  --version '0.1.0+<target>' \
  --note 'battery gating test' \
  --expect-stage ota_skip_battery \
  --expect-version-unchanged
```

期望：

- `device-debug-stages` 出现 `ota_skip_battery`
- 设备版本和分区不变

## 场景 6：SHA256 错配

先上传 artifact，然后把 orchestrator 数据库里的 `asset_sha256` 改成错误值，再创建 rollout。

数据库注入示例：

```bash
ssh tvs675-lan '/share/ZFS1_DATA/.qpkg/container-station/bin/docker exec photoframe-orchestrator \
  python -c "import sqlite3; conn=sqlite3.connect(\"/app/data/orchestrator.db\"); \
  conn.execute(\"update firmware_artifacts set asset_sha256=? where id=?\", \
  (\"0000000000000000000000000000000000000000000000000000000000000000\", <artifact_id>)); \
  conn.commit()"'
```

期望：

- `device-debug-stages` 出现 `ota_fail_sha`
- 设备仍保留原版本和原分区
- `ota_last_error` 包含 `firmware sha256 mismatch`

## 场景 7：首启确认前 reset，验证自动回滚

构建目标固件时开启 `PHOTOFRAME_DEBUG_STAGE_BEACON=1`。

执行：

```bash
python3 scripts/validate-ota-host.py \
  --base-url "${BASE_URL}" \
  --admin-token "${ADMIN_TOKEN}" \
  --device-id "${DEVICE_ID}" \
  --port "${PORT}" \
  --artifact-path firmware/photoframe-rs/dist/photoframe-rs-app.bin \
  --version '0.1.0+<target>' \
  --note 'rollback before confirm test' \
  --reset-stage after_fetch_ok
```

说明：

- `after_fetch_ok` 出现在新分区首轮主循环里，早于 `mark_app_valid_cancel_rollback`
- 在这个阶段 reset，若 rollback 正常，设备应回到旧的 `valid` 分区

期望：

- 新版本一度进入新槽执行
- 注入 reset 后最终恢复到初始版本和初始分区

## 恢复到正式固件

验证结束后，建议再跑一轮普通 OTA，把设备恢复到正式非测试版本。

```bash
PHOTOFRAME_FIRMWARE_VERSION='0.1.0+<final>' \
scripts/build-photoframe-rs.sh

python3 scripts/validate-ota-host.py \
  --base-url "${BASE_URL}" \
  --admin-token "${ADMIN_TOKEN}" \
  --device-id "${DEVICE_ID}" \
  --port "${PORT}" \
  --artifact-path firmware/photoframe-rs/dist/photoframe-rs-app.bin \
  --version '0.1.0+<final>' \
  --note 'restore device to final non-test firmware'
```

恢复后确认：

- `firmware_version = 0.1.0+<final>`
- `ota_state = valid`
- 没有 enabled rollout

## 验证后清理

检查没有 enabled rollout：

```bash
curl -ksS -H "X-PhotoFrame-Token: ${ADMIN_TOKEN}" \
  "${BASE_URL}/api/v1/firmware-rollouts?device_id=${DEVICE_ID}" \
| jq '[.items[] | select(.enabled==true)]'
```

必要时取消 rollout：

```bash
curl -ksS -X DELETE -H "X-PhotoFrame-Token: ${ADMIN_TOKEN}" \
  "${BASE_URL}/api/v1/firmware-rollouts/<rollout_id>"
```
