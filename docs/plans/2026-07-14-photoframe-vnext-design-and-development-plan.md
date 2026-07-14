# PhotoFrame vNext 产品设计与开发计划

状态：执行中  
创建日期：2026-07-14  
目标版本：管理台 vNext / Admin API v2  
原则：设备协议稳定、控制面重做、SQLite 保留、单容器部署保留、省电优先

## 1. 目标

PhotoFrame vNext 要把当前“按技术模块组织的研发控制台”重构为一个以用户意图为中心的个人电子相框产品：

1. 用户在首页 5 秒内知道相框是否正常、当前实际显示什么、下一次何时更新。
2. 用户可用一次选择和一次确认完成“换一张照片”，无需理解 override、poll、dither 等内部概念。
3. 设备异常时，系统给出状态、证据、最可能原因和下一步，而不是只展示原始遥测字段。
4. 图片发送、下载、屏幕刷新和设备回报必须明确区分，避免把 preview 或 sent 误认为 displayed。
5. 低频工程能力保留，但移入“实验室”，不干扰日常使用。
6. 后端由单文件原型演进为模块化单体，同时保持现有设备 API 兼容。

## 2. 非目标

本版本不做以下事项：

- 不重写 Rust 固件主流程。
- 不破坏 `/api/v1/device/*`、`/public/daily.*` 和现有图片端点。
- 不引入 PostgreSQL、Redis、Celery、Kubernetes 或微服务。
- 不建设多租户、社交分享、复杂账号体系。
- 不为了前端改版改变设备每轮唤醒的固定动作。
- 不自动启用实验性渲染方案影响设备日常显示。

## 3. 产品定义

PhotoFrame 包含三个使用层次：

### 3.1 日常产品

- 查看当前显示照片。
- 判断设备是否健康。
- 临时换图。
- 查看下一次更新时间。
- 浏览最近显示过的照片。

### 3.2 内容工作室

- 管理照片资产。
- 调整裁剪焦点和渲染方案。
- 对同一照片比较不同 Dither / Palette。
- 记录喜欢、不再显示、裁剪错误、色彩错误等反馈。

### 3.3 设备实验室

- 查看完整遥测和原始事件。
- 下发设备配置。
- 请求日志采集。
- 管理固件制品和 OTA rollout。
- 查看原始 API、SHA、版本和诊断数据。

默认 UI 只暴露日常产品；内容工作室和设备实验室使用渐进披露。

## 4. 信息架构

vNext 一级工作区固定为四个：

### 4.1 今日

首页必须直接展示：

- 综合状态：正常、按计划休眠、更新逾期、低电量、联网失败、配置卡住、显示未确认、OTA 阻塞。
- 当前设备和最近一次完整成功周期。
- 当前实际显示照片；无法确认时明确标注“预计显示”或“仅已发送”。
- 下一次预计唤醒和距离当前时间。
- 电量、供电状态和数据新鲜度。
- 主操作“换一张”；次操作“立即同步”“查看诊断”。
- 最近事件摘要。

### 4.2 照片

- 最近显示。
- 今日候选。
- 收藏、不再显示、重新显示。
- 原图、裁剪图、设备渲染图关联。
- 显示结果和用户反馈。

### 4.3 设备

- 健康判定和证据。
- 最近成功周期。
- 电池趋势和预计续航。
- Wi-Fi、固件、配置版本。
- 设备事件时间线。
- 意图化操作：更换 Wi-Fi、离家模式、调整刷新频率、收集诊断。

### 4.4 实验室

- Dither / Palette 对比。
- 原始配置发布。
- 日志采集与上传。
- OTA 制品和 rollout。
- 设备 token 审批。
- 原始表格和调试信息。

## 5. 核心领域模型

### 5.1 DeviceSnapshot

某一时刻设备的最新物化状态：

- `device_id`
- `last_seen_epoch`
- `next_wakeup_epoch`
- `firmware_version`
- `battery_percent` / `battery_mv`
- `vbus_present` / `charging`
- `last_error`
- `config_target_version` / `config_seen_version` / `config_applied_version`
- `ota_state` / `ota_target_version`
- `last_publish_state`

### 5.2 DeviceHealthAssessment

由纯函数根据 DeviceSnapshot 和当前时间得出：

- `status`: `healthy | sleeping | warning | critical | unknown`
- `code`: 稳定机器可读原因码。
- `title`: 用户可读结论。
- `summary`: 一句话解释。
- `evidence[]`: 支撑结论的事实。
- `actions[]`: 建议操作。
- `freshness_seconds`: 最新数据年龄。

首批原因码：

- `never_seen`
- `sleeping_as_expected`
- `healthy_recent_checkin`
- `wake_overdue`
- `battery_low`
- `battery_critical`
- `device_error`
- `display_unconfirmed`
- `config_pending`
- `ota_blocked_low_battery`
- `ota_pending`

判定优先级：明确设备错误 > 严重逾期 > 严重低电量 > OTA/配置阻塞 > 显示未确认 > 正常休眠。

### 5.3 DeviceEvent

统一时间线事件：

- `id`
- `device_id`
- `epoch`
- `kind`
- `severity`
- `title`
- `detail`
- `source`
- `metadata`

首批事件来源：

- `publish_history`
- `device_config_plans`
- `device_config_status`
- `device_log_upload_requests`
- `device_log_uploads`
- `firmware_rollouts`
- `device_debug_stages`
- 设备最新 checkin 物化状态

### 5.4 PhotoAsset / RenderVariant / Delivery

本阶段先定义接口，后续迁移数据：

- PhotoAsset：原始照片及来源。
- RenderVariant：裁剪、焦点、算法、Palette 和生成文件。
- Delivery：计划发送、已发送、已下载、已显示、失败。

现有 `assets + publish_history + overrides` 先作为兼容数据源，不立即重建全部表。

## 6. Admin API v2

v2 是面向管理台的聚合 API，不提供给固件调用。

### 6.1 `GET /api/v2/admin/dashboard`

参数：

- `device_id`：可选；缺省选择最近设备。
- `event_limit`：默认 12。

响应：

- `now_epoch`
- `device`
- `health`
- `current_delivery`
- `next_wakeup`
- `recent_events`
- `available_devices`
- `service`

该接口必须在一次请求中提供首页所需信息，避免浏览器并行请求十余个端点后自行拼装。

### 6.2 `GET /api/v2/admin/devices/{device_id}/timeline`

参数：

- `limit`
- `before_epoch`
- `kinds`

响应为统一 DeviceEvent 列表。

### 6.3 后续接口

- `POST /api/v2/admin/devices/{device_id}/actions/sync`
- `POST /api/v2/admin/devices/{device_id}/actions/diagnostics`
- `GET /api/v2/admin/photos`
- `POST /api/v2/admin/photos/{photo_id}/deliver`
- `POST /api/v2/admin/photos/{photo_id}/feedback`

## 7. 后端架构

目标目录：

```text
app/
  main.py                         # 应用装配与 v1 兼容入口
  domains/
    health.py                     # 纯健康判定
    events.py                     # 统一事件模型与排序
  admin_v2/
    dashboard.py                  # Dashboard 聚合服务
    timeline.py                   # 时间线读取与转换
  infrastructure/
    database.py                   # 后续抽取连接与迁移
```

第一阶段采用“抽取而非重写”：

- 新领域逻辑放入独立模块。
- `main.py` 继续持有现有 v1 路由和数据库初始化。
- v2 route 在 `main.py` 中完成薄装配，领域判断不写回单文件。
- 后续逐领域把 v1 实现迁出，不建立重复逻辑层。

## 8. 前端架构

### 8.1 最终方案

- 使用零构建依赖的语义 HTML、CSS variables 和原生 JavaScript。
- Admin API v2 承担服务端聚合，浏览器不再拼接十余个原始端点。
- 四个工作区由轻量客户端状态切换，不引入路由框架。
- 图片和需要鉴权的资源统一通过 `fetch + Blob URL` 展示，避免 Token 进入 DOM URL。
- 静态资源由 FastAPI 直接托管，继续使用单容器离线部署。

本项目只有一位用户、四个固定工作区和有限的前端状态。React/Vite 会增加 Node 工具链、依赖升级、镜像构建和离线部署成本，却不会显著降低当前复杂度。因此本版明确选择零构建前端；只有在出现多人协作、复杂路由、可复用组件库或自动化前端测试规模显著增长时再迁移 React。

### 8.2 迁移方案

为避免覆盖当前尚未提交的控制台工作，采用并行入口：

- 现有控制台保留在 `/legacy`。
- vNext 初期放在 `/vnext`。
- vNext 达到验收标准后将 `/` 切换到新版，旧版继续保留一个发布周期。
- 零构建静态实现已完成产品闭环并通过桌面/移动端验收，因此直接作为正式实现，不再为技术栈迁移而迁移。

### 8.3 设计原则

- 照片优先，状态第二，原始数据最后。
- 默认只展示一个明确主操作。
- 所有状态包含数据时间和可信度。
- `sent`、`displayed`、`preview` 使用不同视觉和文案。
- 危险操作说明影响范围，执行后展示真实 ack。
- 专家字段不得占据默认首屏。

## 9. 安全与可运维性

- 管理端不在 DOM、历史链接或日志中展示完整 token。
- 后续由反向代理提供 HTTPS 和 HttpOnly session。
- 图片历史打开动作通过管理端代理或短时签名 URL。
- 构建和部署必须绑定 commit SHA；正式部署拒绝 `dirty` 标记。
- 数据库提供备份、恢复和 schema version。
- 每次部署验证 `/healthz`、Dashboard API、设备协议兼容和真实图片端点。

## 10. 可观测指标

### 10.1 产品指标

- 首页健康结论生成时间。
- 用户完成换图所需步骤。
- 最近显示成功率。
- sent 到 displayed 的确认率与耗时。
- 设备逾期后被识别的时间。

### 10.2 设备与省电指标

- 每轮唤醒时长。
- 每轮传输字节数。
- 304 命中率。
- 失败重试次数。
- 日均唤醒次数。
- 电池日均下降幅度。

### 10.3 工程指标

- v2 首页请求数。
- Dashboard API P95。
- 健康判定单元测试覆盖。
- 部署版本可追溯率。
- `dirty` 生产部署次数。

## 11. 分阶段开发计划

### Phase 0：设计与兼容基线

- [x] 固化产品定义、架构和验收标准。
- [x] 在 API 文档中记录 v1 路由兼容边界。
- [x] 建立 vNext 入口与 `/legacy` 回滚入口。
- [ ] 记录当前线上健康和设备状态基线。

验收：文档可直接作为实现和评审清单；现有设备协议无修改。

### Phase 1：健康诊断与聚合 API

- [x] 新增 DeviceHealthAssessment 纯领域模块。
- [x] 新增统一 DeviceEvent 模型。
- [x] 新增时间线聚合查询。
- [x] 新增 `/api/v2/admin/dashboard`。
- [x] 新增 `/api/v2/admin/devices/{device_id}/timeline`。
- [x] 覆盖正常休眠、逾期、低电量、设备错误、OTA 阻塞等测试。

验收：一次 Dashboard 请求能够解释线上当前设备为何异常或正常。

### Phase 2：vNext 今日页

- [x] 新建 `/vnext` 入口。
- [x] 实现设备选择器和健康主卡。
- [x] 实现当前显示 / 预计显示明确区分。
- [x] 实现下一次唤醒、电量、版本和数据新鲜度。
- [x] 实现最近事件时间线。
- [x] 提供跳转旧控制台的专家入口。

验收：默认首屏无需横向滚动；5 秒内可回答设备是否正常和下一步。

### Phase 3：照片工作区

- [x] 建立 PhotoAsset / RenderVariant / Delivery schema。
- [x] 迁移现有 override 资产映射。
- [x] 实现最近投送、重新显示和快速换图。
- [x] 实现反馈：喜欢、不再显示、裁剪问题、色彩问题。
- [x] 将 Dither 比较移入实验室。

验收：常规换图不要求用户理解 override 或 Dither。

### Phase 4：设备与实验室

- [x] 意图化设备设置。
- [x] 完整设备事件筛选和诊断摘要。
- [x] OTA 创建向导与状态视图。
- [x] 原始配置、日志和 token 工具归入实验室。
- [x] 隐藏历史中的完整 token URL。

验收：高风险操作具备影响说明、确认、执行 ack 和最终状态。

### Phase 5：根入口切换与部署治理

- [x] 确认零构建前端并接入单容器静态托管。
- [x] `/` 切换到 vNext，`/legacy` 保留。
- [ ] 建立干净 commit 构建约束。
- [ ] 完成本地测试、离线部署和延时 live verification。
- [x] 更新 README 和 API 文档。
- [ ] 更新部署 runbook 与回滚记录。

验收：线上版本可追溯，设备 v1 协议、图片 304 和管理台核心流程全部通过。

## 12. 测试策略

### 12.1 领域测试

- 健康判定使用纯字典输入，不依赖 DB 或 FastAPI。
- 每个原因码至少有一个正例和一个优先级冲突例。
- 时间线排序、去重、severity 和 detail 映射独立测试。

### 12.2 API 测试

- Dashboard 无设备、单设备和指定设备。
- 鉴权失败。
- 陈旧设备、逾期唤醒和低电量。
- current_delivery 的 sent/displayed 区分。
- timeline limit 和 before_epoch。

### 12.3 UI 测试

- 首屏 loading、error、empty、stale 和 healthy。
- 窄屏不横向溢出。
- 键盘可访问和明确 focus。
- 不把 token 写入 DOM 或 URL。

### 12.4 兼容验证

- `python3 -m compileall -q services/photoframe-orchestrator/app`
- orchestrator pytest。
- `GET /api/v1/device/next` 行为不变。
- `POST /api/v1/device/checkin` 行为不变。
- `/public/daily.bmp` 和 `/public/daily.jpg` ETag/304 不变。
- 固件编译在涉及协议变更时才要求；本阶段不改固件协议。

## 13. 完成定义

vNext 只有同时满足以下条件才算完成：

- 新管理台成为默认入口。
- 首页可给出设备健康结论和证据。
- 当前显示状态不再混淆 preview、sent 和 displayed。
- 常规换图流程不暴露内部调度概念。
- 设备协议兼容测试通过。
- 线上部署对应干净 commit SHA。
- 真实设备完成一次 next → download → display → checkin 闭环。
- 文档、部署脚本、回滚方案和剩余风险已更新。

## 14. 当前执行顺序

当前进入发布收口：

1. 运行 Python、JavaScript、Rust 与设备兼容测试。
2. 记录线上部署前基线。
3. 整理干净 commit 并构建离线镜像。
4. 部署到 `tvs675-lan`，验证根入口、Dashboard、设备 v1 API 和图片 304。
5. 等待真实设备下一次唤醒，核对 next → download → display → checkin 闭环。
