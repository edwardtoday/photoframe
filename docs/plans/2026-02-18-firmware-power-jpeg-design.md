# 固件耗电优化 + JPEG 支持：设计说明（2026-02-18）

**目标**

1. 优先解决“设备一天耗光电/耗电过快”的问题，保证设备按预期进入深度睡眠并稳定按配置周期唤醒。
2. 让设备在 BMP 之外支持至少 JPEG（JPG）拉取与显示，以显著降低下载体积。
3. 借鉴 Waveshare upstream（ESP32-S3-PhotoPainter）中已验证的低功耗与多格式图片设计点，落地到自有固件（`firmware/photoframe-fw/`），并用清晰 commit 与文档记录。

---

## 现象与证据（已验证）

### 1) 设备实际唤醒频率远高于配置值

从 NAS orchestrator 数据库（`/share/ZFS19_DATA/Container/docker/photoframe-orchestrator/data/orchestrator.db`）拉取到本地分析，设备 `pf-d9369c80` 的关键字段：

- `poll_interval_seconds=3600`、`sleep_seconds=3600`、`next_wakeup_epoch` 也按 1 小时推算
- 但 `publish_history` 里同一设备的 `issued_epoch` 间隔约 132-134 秒
- `device_power_samples` 的 `sample_epoch` 间隔也稳定在约 132-134 秒

结论：设备“自认为”要睡 3600 秒，但实际并未进入有效深睡（几乎是立即再次被唤醒），导致每约 2 分钟跑一轮联网逻辑与（可能的）刷新逻辑，耗电陡增。

### 2) 周期约 133 秒与“KEY 唤醒 120 秒窗口”强相关

固件实现里，当 WakeSource 被判定为 `KEY` 时，会在 STA 模式下打开 `120s` 本地配置窗口（忙等循环，`200ms` tick）。这会把单轮时长拉长到接近 `120s + 联网/拉图开销`，与观测到的 `~133s` 间隔高度吻合。

因此优先怀疑：深睡唤醒源 EXT1 在深睡阶段因为按键引脚浮空/不稳定导致误触发，从而被误判为 KEY 唤醒，进入 120 秒窗口并循环。

---

## 根因假设（可验证）

**假设 A（优先）：EXT1 唤醒引脚在深睡阶段未正确配置 RTC 上拉，导致浮空触发 ANY_LOW。**

依据：
- 当前固件深睡使用 `esp_sleep_enable_ext1_wakeup(..., ESP_EXT1_WAKEUP_ANY_LOW)`，但未对 EXT1 引脚在 RTC 域显式启用上拉。
- upstream 示例里使用 `esp_sleep_enable_ext1_wakeup_io(...)` 并且对唤醒按键 GPIO 显式 `rtc_gpio_pullup_en()`，以避免深睡阶段浮空误唤醒。

**假设 B（加剧耗电）：即使 EXT1 误唤醒发生，当前 WakeSource 判定逻辑未二次确认“按键是否仍处于按下状态”，导致误入 KEY 120 秒窗口或 BOOT 强刷。**

依据：
- 当前逻辑对 `ESP_SLEEP_WAKEUP_EXT1` 只看 `esp_sleep_get_ext1_wakeup_status()`，没有再读一次 GPIO 电平确认。

---

## 方案候选（2-3 个）

### 方案 1（推荐）：EXT1 走 *_wakeup_io + RTC 上拉 + 二次确认按键仍按下

改动点：
- 深睡前使用 `esp_sleep_enable_ext1_wakeup_io(...)`（如果当前 IDF 提供）替代 `esp_sleep_enable_ext1_wakeup(...)`
- 对 KEY(GPIO4)（必要）以及 BOOT(GPIO0)（可选）启用 RTC 域上拉：`rtc_gpio_pullup_en()` + `rtc_gpio_pulldown_dis()`
- 唤醒后判定 WakeSource 时，若 `cause==EXT1`，只有当对应 GPIO 在运行态仍读到 `0`（仍按下）才认为是 KEY/BOOT；否则归类为 OTHER，避免进入 120 秒窗口/强刷

优点：
- 对症：直接解决“误唤醒 + 误判按键”的核心问题
- 改动面集中，风险可控
- 与 upstream 已验证实践一致

缺点：
- 需要依赖 RTC GPIO API；若引脚不是 RTC 可用，需要兜底（不过当前已能用 EXT1，基本满足）

### 方案 2：禁用 EXT1（只保留 TIMER 唤醒）

优点：
- 最简单，误唤醒直接消失

缺点：
- 丢失深睡期间按键唤醒能力（KEY/BOOT 唤醒），影响现场操作体验

### 方案 3：EXT1 仅保留 KEY(GPIO4)，不把 BOOT(GPIO0) 作为深睡唤醒源

优点：
- 降低 BOOT 这种 strapping pin 带来的不确定性

缺点：
- BOOT 深睡唤醒能力丢失（但仍可在唤醒后作为“强刷语义”保留）

---

## 低功耗增强（第二优先级，但建议一起做）

即便深睡正确，外围 IC 仍可能通过 PMIC 供电导致静态耗电偏大。当前 `PowerManager::Init()` 会开启并固定 ALDO3/ALDO4=3300mV（寄存器 `0x90/0x94/0x95`）。

借鉴 upstream “外围 IC 进入睡眠”的方向，增加：

- 深睡前关闭 ALDO3/ALDO4（仅外围供电，非 ESP 核心供电），降低外围静态耗电
- 唤醒后在 `PowerManager::Init()` 再打开即可

这部分属于“可回滚优化”：若出现外设异常，可单独回退该 commit。

---

## JPEG 支持设计

### 目标与边界

- 支持 `image/jpeg`（以及常见 `.jpg/.jpeg`）输入。
- 继续保持对输入分辨率的严格要求：只接受 `800x480` 或 `480x800`（与现有 BMP 路径一致），不在设备端做缩放。
- 解码输出统一为 `RGB888`，并复用现有 6 色量化 + 抖动流程。

### 组件与依赖选择

借鉴 upstream 的实现，使用 `espressif/esp_jpeg`（通过 ESP-IDF Component Manager）提供 `esp_jpeg_dec` 解码能力：

- 在 `firmware/photoframe-fw/main/idf_component.yml` 声明依赖 `espressif/esp_jpeg`
- 增加一层轻量 wrapper（参考 upstream `test_decoder.c`）：
  - 输入：JPEG bytes
  - 输出：PSRAM 中的 RGB888 buffer + width/height

### 渲染链路改造

现有渲染入口仅支持 `PhotoPainterEpd::DrawBmp24(bmp_bytes)`。

改造方向：

- 在 `photopainter_epd` 组件新增 `DrawRgb24(const uint8_t* rgb, int w, int h, ...)`（或等价函数）
- 把 `DrawBmp24` 中“像素读取 + 量化 + 抖动 + 打包写 display_buf_”的核心循环抽成可复用逻辑
- 主循环根据 Content-Type 或文件 magic 判断：
  - BMP：走旧路径
  - JPEG：先解码到 RGB888，再走 `DrawRgb24`

---

## 验证方法（可操作）

### 1) 验证是否还在 2 分钟循环

升级固件后，观察 orchestrator：
- `publish_history` 同设备的下发记录间隔应回到 `~3600s`（或配置值）
- `power_samples` 采样间隔应随设备唤醒周期变化，不再稳定 `~133s`

### 2) 验证 KEY/BOOT 语义未被误触发

在串口日志中新增/观察：
- `wakeup cause`、`ext1 pins mask`、`key/boot level`
- 确认正常 timer 唤醒不再误判为 KEY/BOOT

### 3) JPEG 拉取验证

将 `image_url_template` 指向 JPEG 资源：
- Content-Type 为 `image/jpeg`
- 分辨率为 `800x480` 或 `480x800`

观察：
- 固件日志输出“识别为 jpeg + 解码尺寸”
- 成功刷新屏幕且进入深睡

---

## 实施拆分（commit 粒度）

1. 固件：修复深睡误唤醒（RTC 上拉 + WakeSource 二次确认 + 关键日志）
2. 固件：深睡前关闭外围供电（ALDO3/ALDO4 等）以降低静态耗电（可回滚）
3. 固件：接入 esp_jpeg + JPEG 下载/解码/渲染链路
4. 文档：更新 `docs/firmware-photoframe-fw.md`，记录根因、验证、以及 upstream 借鉴点

