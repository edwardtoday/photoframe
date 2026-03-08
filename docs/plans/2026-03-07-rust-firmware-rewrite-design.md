# 固件 Rust 重写设计说明（2026-03-07）

## 目标

1. 用 `Rust + ESP-IDF` 重写当前固件，优先提升稳定性、可测试性和可维护性。
2. 保持现有产品能力不倒退：配网、拉图、渲染、深睡、远端配置同步、状态上报都必须保留。
3. 在不改变 orchestrator 对外协议的前提下，把“高频反复出问题的业务状态机”从硬件细节中剥离出来，并建立可自动化验收的测试基线。

## 非目标

- V1 **不追求**纯 `no_std` 或纯 `esp-hal` 重写。
- V1 **不要求**立刻把 PMIC / EPD 底层驱动 100% 改成 Rust；允许先通过薄 FFI 过渡。
- V1 **不改变**现有 orchestrator API 语义，只在设备端实现上重构。
- V1 **不新增**与当前需求无关的产品功能。

## 现状基线

当前 C++ 固件已经具备完整产品能力，但主流程耦合严重：

- 总控与状态机集中在 `firmware/photoframe-fw/main/main.cpp:1134`
- 配置持久化在 `firmware/photoframe-fw/main/config_store.cpp:87`
- 本地 Portal 在 `firmware/photoframe-fw/main/portal_server.cpp:205`
- 编排服务交互在 `firmware/photoframe-fw/main/orchestrator_client.cpp:341`
- 拉图与条件请求在 `firmware/photoframe-fw/main/image_client.cpp:131`
- JPEG 解码在 `firmware/photoframe-fw/main/jpeg_decoder.cpp:47`
- PMIC / I2C 恢复在 `firmware/photoframe-fw/main/power_manager.cpp:271`
- 墨水屏渲染在 `firmware/photoframe-fw/components/photopainter_epd/photopainter_epd.cpp:483`

已确认的主要问题：

1. `main.cpp` 同时承担状态机、网络、时间、电源、渲染与休眠控制，复杂度过高。
2. Wi‑Fi profile、显示参数应用、HTTP/TLS/JSON 客户端逻辑存在重复实现。
3. 配置应用、运行态与持久态之间缺少统一事务边界，局部失败时容易漂移。
4. 几乎没有固件自己的自动化测试基线，导致回归依赖真机与日志观察。

## 功能基线

Rust 重写 V1 必须完整保留以下能力：

1. **配网与本地配置**
   - 无 Wi‑Fi 凭据时进入 AP Portal。
   - 长按 `BOOT` 清 Wi‑Fi 并进入 AP 配网。
   - 长按 `KEY` 打开 120 秒 STA 配置窗口。
   - 最多保存 3 组 Wi‑Fi profile，并优先尝试上次成功网络。

2. **拉图与显示**
   - 支持 orchestrator `/api/v1/device/next` 与 `image_url_template` 双路径。
   - 支持 `BMP` / `JPEG`。
   - 支持 `ETag` / `If-None-Match` 与 `Last-Modified` / `If-Modified-Since`。
   - 命中 `304` 时跳过正文下载与重刷。
   - `BOOT` 强刷时必须绕过条件请求。
   - 保持严格分辨率限制：仅接受 `800x480` 或 `480x800`。

3. **唤醒与省电**
   - 区分 `TIMER` / `KEY` / `BOOT` / `SPURIOUS_EXT1`。
   - `KEY` 短按执行一次手动同步，不默认开启 120 秒窗口。
   - 误唤醒时跳过联网与拉图，走 timer-only 深睡兜底。
   - 失败采用指数退避；PMIC 软失败不放大退避。
   - USB / VBUS 调试场景下允许阻止深睡。

4. **远端配置与状态上报**
   - 支持 `/api/v1/device/config` 轮询与 `config_version` 版本化应用。
   - 支持 `/api/v1/device/config/applied` 回报结果。
   - 支持 `/api/v1/device/checkin` 上报网络、拉图、电源、配置与错误信息。
   - 支持设备 token、photo token 与 server time fallback 相关协议字段。

5. **电源与遥测**
   - 读取 `battery_mv` / `battery_percent` / `charging` / `vbus_good`。
   - PMIC 读数失败时允许 RTC 缓存兜底。
   - 需要保留“按电压估算百分比”的保护逻辑。

## 验收清单

### A. 功能验收

- 无 Wi‑Fi 凭据时，设备进入 `PhotoFrame-Setup` AP 并可完成配网。
- `KEY` 短按后执行一次联网→拉图→check-in→深睡流程。
- `BOOT` 短按后必须拿到正文并重渲染，不能命中 `304` 后直接跳过。
- 长按 `KEY` 后出现 120 秒 STA Portal 窗口；长按 `BOOT` 后清 Wi‑Fi 并进 AP。
- `device/next` 返回 JPEG 时可正确下载、解码并渲染。
- orchestrator 不可达或下发 daily URL 不可达时，自动回退 `image_url_template`。
- `date=YYYY-MM-DD` 返回 `404` 时自动尝试前一天日期。

### B. 协议验收

- `/api/v1/device/next` 请求包含 `device_id`、`now_epoch`、`default_poll_seconds`、`failure_count`、`accept_formats=jpeg,bmp`。
- `/api/v1/device/config` 应用新版本后必须写入持久层并回报 `config/applied`。
- `/api/v1/device/checkin` 必须包含 `battery_mv`、`battery_percent`、`charging`、`vbus_good`、`sta_ip`、`reported_config`。
- 设备时钟不可信时，仍能正确使用服务端时间相关字段进行调度。

### C. 功耗验收

- 正常运行时，`publish_history` 与 `power_samples` 间隔应回到配置轮询值附近，不再稳定落在约 133 秒。
- `SPURIOUS_EXT1` 不应触发整轮联网逻辑。
- 命中 `304` 的轮次应显著缩短下载时长与传输字节数。

### D. 工程验收

- 核心状态机具备宿主机可运行测试。
- orchestrator 协议具备 JSON 契约测试。
- Rust workspace 能在宿主机执行 `cargo test`。
- 设备端最小 smoke 流程可编译并链接到 ESP-IDF。

## 技术路线

### 选型

- 语言：Rust
- 设备框架：ESP-IDF 绑定路线
- 推荐依赖方向：`esp-idf-hal`、`esp-idf-svc`、`embedded-svc`、`serde`
- 验证依据：官方 `esp-rs` 文档与仓库
  - `https://docs.esp-rs.org/book/`
  - `https://github.com/esp-rs/esp-idf-svc`
  - `https://github.com/esp-rs/esp-hal`

### 目录规划

计划新增 `firmware/photoframe-rs/`，结构如下：

```text
firmware/photoframe-rs/
  Cargo.toml                 # workspace
  crates/
    domain/                  # 纯业务状态机与策略（宿主机可测）
    contracts/               # orchestrator 协议模型与 JSON 契约
    app/                     # 设备应用编排
    platform-espidf/         # Wi‑Fi / HTTP / NVS / 时间 / 睡眠适配
    drivers-ffi/             # EPD / PMIC 过渡层（先复用现有实现）
```

### 分层原则

1. **domain**
   - 不依赖 ESP-IDF。
   - 只负责状态、策略与决策。
   - 覆盖：唤醒原因判定、按键语义、退避策略、刷新策略、配置应用决策。

2. **contracts**
   - 不依赖硬件。
   - 专门定义 `device/next`、`device/config`、`checkin` 等协议模型。
   - 所有字段与白名单约束集中维护，禁止分散解析。

3. **platform-espidf**
   - 封装 ESP-IDF 相关能力：Wi‑Fi、HTTP、TLS、SNTP、NVS、深睡。
   - 对上暴露 trait / adapter，避免业务层直接碰 IDF 细节。

4. **drivers-ffi**
   - 第一阶段不直接重写 EPD / PMIC 底层驱动。
   - 先通过最薄边界复用现有成熟实现，降低迁移风险。

5. **app**
   - 只负责组织每轮执行顺序。
   - 业务判断交给 `domain`，硬件动作交给 `platform-espidf` 与 `drivers-ffi`。

## 迁移顺序

### 阶段 1：先迁移“易出错但易测试”的决策逻辑

- 唤醒分类
- 长按 / 短按语义
- 重试退避与 PMIC 软失败策略
- 强刷 / 条件 GET 策略
- 配置版本应用策略

### 阶段 2：迁移协议与持久化边界

- orchestrator JSON 模型
- 配置白名单
- NVS 映射
- reported_config 生成

### 阶段 3：迁移联网与拉图主链路

- `device/next`
- `device/config`
- `device/checkin`
- 模板 URL 展开
- origin 偏好与前一天回退

### 阶段 4：接入屏幕与 PMIC

- 先保留 FFI
- 再评估是否有必要继续纯 Rust 化

### 阶段 5：迁移 Portal

- 复用现有配置数据模型
- 保证 AP/STA 双模式行为一致

## 测试策略

### 宿主机测试

- `domain`：纯单元测试
- `contracts`：JSON 反序列化 / 序列化契约测试
- `app`：状态流 smoke 测试

### 真机验证

- 配网 smoke
- 定时唤醒 smoke
- 304 命中 smoke
- JPEG / BMP 渲染 smoke
- PMIC 读数与 check-in smoke
- 长按 / 短按语义 smoke

## 文档更新要求

在重写过程中，以下文档必须同步维护：

- `README.md`
- `docs/firmware-photoframe-fw.md`
- `docs/orchestrator-api.md`
- `docs/featured-photo-integration.md`
- 本设计文档与实施计划

## 本次结论

采用 **Rust + ESP-IDF + 先状态机后驱动** 的迁移路线。

第一批落地目标不是“全量替换”，而是先建立一个可测试、可扩展、能承载旧协议的 Rust 固件骨架，并优先迁移最容易反复出错的主决策逻辑。
