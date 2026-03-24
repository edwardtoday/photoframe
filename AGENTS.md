# photoframe 项目专用约定（仅本仓库适用）

本文件只记录 photoframe 项目中的“落地约束 / 接口约定 / 操作脚本”，不重复 `~/.codex/AGENTS.md` 里的通用方法论与工作流。

## 省电优先的产品约束

- 第一优先级：降低耗电（减少误唤醒、缩短唤醒时长、减少传输字节数）。
- 任何“每轮唤醒必做”的动作都需要明确收益与可观测指标，否则默认不引入（固定成本会被无限放大）。

## 端到端能力协商（图片格式）

- 设备拉图前会向 orchestrator `GET /api/v1/device/next` 传 `accept_formats=jpeg,bmp`（逗号分隔）。
- orchestrator 在 override 资源存在 `<sha256>.jpg` 派生文件时，优先下发 `.jpg` URL；否则回退 `.bmp`。
- 公网只读日图端点：`/public/daily.bmp` 与 `/public/daily.jpg`（均需 token，均支持 `ETag` + `If-None-Match` 命中 `304` 省流省电）。

## 设备时钟兜底（避免 1970/未来时间）

- 设备请求 `GET /api/v1/device/next` / `GET /api/v1/device/config` 可携带 `now_epoch`（设备当前 epoch）。
- orchestrator 会校验 `now_epoch` 是否可信：过小（常见 1970）、过大（未来）或漂移过大时，回退到服务端时间。
- 接口返回字段约定：
  - `server_epoch`：服务端当前时间（设备可用它校时）。
  - `device_epoch`：设备上报的原始时间戳。
  - `device_clock_ok`：设备时钟是否可信。
  - `effective_epoch`：本次决策实际使用的时间（用于日图 date 与 override window）。

## 部署约定（离线投送到 tvs675-lan）

- 使用 `scripts/deploy-orchestrator-offline-to-tvs675-lan.sh`：
  - 本机构建镜像并导出 tar
  - scp 到 NAS
  - 远端 `docker load`
  - `docker compose up -d --pull never --force-recreate`（绕过 `docker compose pull` 的离线部署）

## 最小验证（相框不在手时）

- 固件编译：`scripts/build-photoframe-rs.sh`
- orchestrator 语法检查：`python3 -m compileall -q services/photoframe-orchestrator/app`
