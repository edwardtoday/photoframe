#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

#[cfg(target_os = "espidf")]
use std::{
    ffi::{CString, c_char, c_void},
    net::UdpSocket,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[cfg(target_os = "espidf")]
use esp_idf_sys as sys;
#[cfg(target_os = "espidf")]
use photoframe_app::{DeviceRuntimeConfig, LocalConfigPatch, PowerSample, Storage};
#[cfg(target_os = "espidf")]
use photoframe_platform_espidf::EspIdfStorage;
#[cfg(target_os = "espidf")]
use serde_json::{Value, json};

#[cfg(target_os = "espidf")]
const AP_SSID: &str = "PhotoFrame-Setup";
#[cfg(target_os = "espidf")]
const AP_PASSWORD: &str = "12345678";
#[cfg(target_os = "espidf")]
const AP_IP: [u8; 4] = [192, 168, 73, 1];
#[cfg(target_os = "espidf")]
const PORTAL_LOOP_STEP_MS: u64 = 200;
#[cfg(target_os = "espidf")]
const HTTPD_TASK_NO_AFFINITY: i32 = 0x7fff_ffff;

#[cfg(target_os = "espidf")]
static ROOT_URI: &[u8] = b"/\0";
#[cfg(target_os = "espidf")]
static API_CONFIG_URI: &[u8] = b"/api/config\0";
#[cfg(target_os = "espidf")]
static API_WIFI_SCAN_URI: &[u8] = b"/api/wifi/scan\0";
#[cfg(target_os = "espidf")]
static CONTENT_TYPE_HTML: &[u8] = b"text/html; charset=utf-8\0";
#[cfg(target_os = "espidf")]
static CONTENT_TYPE_JSON: &[u8] = b"application/json\0";

#[cfg(target_os = "espidf")]
static PORTAL_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>PhotoFrame 配置门户</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; margin: 1rem; }
    input, button, select { width: 100%; margin: .4rem 0; padding: .6rem; font-size: 1rem; }
    .card { border: 1px solid #ddd; border-radius: .5rem; padding: 1rem; margin-bottom: 1rem; }
    .muted { color: #666; font-size: .9rem; }
    pre { white-space: pre-wrap; }
  </style>
</head>
<body>
  <h2>PhotoFrame 配置门户</h2>
  <p class="muted">保存后设备会自动重启并尝试联网。</p>

  <div class="card">
    <h3>Wi-Fi</h3>
    <button onclick="scanWifi()">扫描 Wi-Fi</button>
    <select id="ssidSelect" onchange="fillSsid()"><option value="">手动输入 SSID</option></select>
    <input id="ssid" placeholder="SSID" />
    <input id="password" type="password" placeholder="Password（留空则保持不变）" />
    <p class="muted">仅在需要修改 Wi-Fi 密码时填写；留空不会覆盖当前密码。</p>
  </div>

  <div class="card">
    <h3>拉图配置</h3>
    <input id="urlTemplate" placeholder="URL 模板，例如 http://host/image/480x800?date=%DATE%" />
    <select id="orchEnabled">
      <option value="1">编排服务：启用（推荐）</option>
      <option value="0">编排服务：关闭</option>
    </select>
    <input id="orchBaseUrl" placeholder="编排服务地址，例如 http://192.168.233.11:8081" />
    <input id="deviceId" placeholder="设备 ID（留空则自动生成）" />
    <input id="orchToken" placeholder="编排服务 Token（可选）" />
    <input id="photoToken" placeholder="图片 Token（可选）" />
    <input id="interval" type="number" min="1" placeholder="刷新间隔（分钟）" />
    <input id="retryBase" type="number" min="1" placeholder="失败重试基数（分钟）" />
    <input id="retryMax" type="number" min="1" placeholder="失败重试上限（分钟）" />
    <input id="maxFail" type="number" min="1" placeholder="连续失败阈值" />
    <select id="rotation">
      <option value="0">旋转 0（推荐）</option>
      <option value="2">旋转 180</option>
    </select>
    <select id="colorMode">
      <option value="0">色彩模式：自动判断</option>
      <option value="1">总是转换为 6 色</option>
      <option value="2">输入已是 6 色</option>
    </select>
    <select id="ditherMode">
      <option value="1">有序抖动（推荐）</option>
      <option value="0">关闭</option>
    </select>
    <input id="colorTol" type="number" min="0" max="64" placeholder="6 色容差（0-64）" />
    <input id="timezone" placeholder="时区，例如 Asia/Shanghai" />
  </div>

  <button onclick="saveAll()">保存配置并重启</button>
  <pre id="out"></pre>

  <script>
    const out = (msg) => document.getElementById('out').textContent = msg;
    async function api(path, opt = {}) {
      const r = await fetch(path, {headers: {'Content-Type': 'application/json'}, ...opt});
      const t = await r.text();
      let j = null;
      try { j = JSON.parse(t); } catch {}
      if (!r.ok) throw new Error((j && j.error) || t || ('HTTP ' + r.status));
      return j;
    }
    function fillSsid() {
      const select = document.getElementById('ssidSelect');
      document.getElementById('ssid').value = select.value;
    }
    async function scanWifi() {
      try {
        const data = await api('/api/wifi/scan');
        const select = document.getElementById('ssidSelect');
        select.innerHTML = '<option value="">手动输入 SSID</option>';
        (data.networks || []).forEach((ap) => {
          const opt = document.createElement('option');
          opt.value = ap.ssid;
          opt.textContent = `${ap.ssid} (RSSI ${ap.rssi})`;
          select.appendChild(opt);
        });
        out('扫描完成');
      } catch (err) {
        out('扫描失败: ' + err.message);
      }
    }
    async function loadConfig() {
      const cfg = await api('/api/config');
      document.getElementById('ssid').value = cfg.wifi_ssid || '';
      document.getElementById('urlTemplate').value = cfg.image_url_template || '';
      document.getElementById('orchEnabled').value = String(cfg.orchestrator_enabled ? 1 : 0);
      document.getElementById('orchBaseUrl').value = cfg.orchestrator_base_url || '';
      document.getElementById('deviceId').value = cfg.device_id || '';
      document.getElementById('orchToken').value = cfg.orchestrator_token || '';
      document.getElementById('photoToken').value = cfg.photo_token || '';
      document.getElementById('interval').value = cfg.interval_minutes || 60;
      document.getElementById('retryBase').value = cfg.retry_base_minutes || 5;
      document.getElementById('retryMax').value = cfg.retry_max_minutes || 240;
      document.getElementById('maxFail').value = cfg.max_failure_before_long_sleep || 24;
      document.getElementById('rotation').value = cfg.display_rotation || 0;
      document.getElementById('colorMode').value = cfg.color_process_mode || 0;
      document.getElementById('ditherMode').value = cfg.dither_mode || 1;
      document.getElementById('colorTol').value = cfg.six_color_tolerance || 0;
      document.getElementById('timezone').value = cfg.timezone || 'UTC';
      out('配置已加载');
    }
    async function saveAll() {
      const body = {
        wifi_ssid: document.getElementById('ssid').value,
        wifi_password: document.getElementById('password').value,
        image_url_template: document.getElementById('urlTemplate').value,
        orchestrator_enabled: Number(document.getElementById('orchEnabled').value),
        orchestrator_base_url: document.getElementById('orchBaseUrl').value,
        device_id: document.getElementById('deviceId').value,
        orchestrator_token: document.getElementById('orchToken').value,
        photo_token: document.getElementById('photoToken').value,
        interval_minutes: Number(document.getElementById('interval').value),
        retry_base_minutes: Number(document.getElementById('retryBase').value),
        retry_max_minutes: Number(document.getElementById('retryMax').value),
        max_failure_before_long_sleep: Number(document.getElementById('maxFail').value),
        display_rotation: Number(document.getElementById('rotation').value),
        color_process_mode: Number(document.getElementById('colorMode').value),
        dither_mode: Number(document.getElementById('ditherMode').value),
        six_color_tolerance: Number(document.getElementById('colorTol').value),
        timezone: document.getElementById('timezone').value,
      };
      try {
        const data = await api('/api/config', {method: 'POST', body: JSON.stringify(body)});
        out(JSON.stringify(data, null, 2) + '\n设备将自动重启。');
      } catch (err) {
        out('保存失败: ' + err.message);
      }
    }
    loadConfig().catch((err) => out('加载失败: ' + err.message));
  </script>
</body>
</html>
"#;

#[cfg(target_os = "espidf")]
#[derive(Clone, Copy)]
struct PortalRuntimeStatus {
    wifi_connected: bool,
    force_refresh: bool,
    last_http_status: i32,
    image_changed: bool,
    battery_mv: i32,
    battery_percent: i32,
    charging: i32,
    vbus_good: i32,
}

#[cfg(target_os = "espidf")]
struct PortalState {
    storage: Mutex<EspIdfStorage>,
    config: Mutex<DeviceRuntimeConfig>,
    status: PortalRuntimeStatus,
    should_reboot: AtomicBool,
}

#[cfg(target_os = "espidf")]
struct RustPortalServer {
    handle: sys::httpd_handle_t,
    state: *mut PortalState,
    dns_running: Arc<AtomicBool>,
    dns_thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "espidf")]
fn httpd_default_config() -> sys::httpd_config_t {
    sys::httpd_config_t {
        task_priority: 5,
        stack_size: 4096,
        // ESP-IDF 5.5 下 tskNO_AFFINITY 仍是 0x7fffffff，不能写 -1。
        core_id: HTTPD_TASK_NO_AFFINITY,
        task_caps: sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT,
        max_req_hdr_len: sys::CONFIG_HTTPD_MAX_REQ_HDR_LEN as usize,
        max_uri_len: sys::CONFIG_HTTPD_MAX_URI_LEN as usize,
        server_port: 80,
        ctrl_port: sys::ESP_HTTPD_DEF_CTRL_PORT as u16,
        max_open_sockets: 7,
        max_uri_handlers: 8,
        max_resp_headers: 8,
        backlog_conn: 5,
        lru_purge_enable: false,
        recv_wait_timeout: 5,
        send_wait_timeout: 5,
        global_user_ctx: ptr::null_mut(),
        global_user_ctx_free_fn: None,
        global_transport_ctx: ptr::null_mut(),
        global_transport_ctx_free_fn: None,
        enable_so_linger: false,
        linger_timeout: 0,
        keep_alive_enable: false,
        keep_alive_idle: 0,
        keep_alive_interval: 0,
        keep_alive_count: 0,
        open_fn: None,
        close_fn: None,
        uri_match_fn: None,
    }
}

#[cfg(target_os = "espidf")]
fn esp_to_result(err: i32, op: &str) -> Result<(), String> {
    if err == 0 || err == sys::ESP_ERR_INVALID_STATE {
        Ok(())
    } else {
        Err(format!("{op} failed: {err}"))
    }
}

#[cfg(target_os = "espidf")]
fn ensure_wifi_stack_for_portal() -> Result<(*mut sys::esp_netif_t, *mut sys::esp_netif_t), String>
{
    esp_to_result(unsafe { sys::esp_netif_init() }, "esp_netif_init")?;
    esp_to_result(
        unsafe { sys::esp_event_loop_create_default() },
        "esp_event_loop_create_default",
    )?;

    let sta_netif = unsafe { sys::esp_netif_create_default_wifi_sta() };
    let ap_netif = unsafe { sys::esp_netif_create_default_wifi_ap() };
    if sta_netif.is_null() || ap_netif.is_null() {
        return Err("create default wifi netif failed".into());
    }

    let init_config = crate::wifi::wifi_init_config_default();
    esp_to_result(unsafe { sys::esp_wifi_init(&init_config) }, "esp_wifi_init")?;
    esp_to_result(
        unsafe { sys::esp_wifi_set_storage(sys::wifi_storage_t_WIFI_STORAGE_RAM) },
        "esp_wifi_set_storage",
    )?;
    Ok((sta_netif, ap_netif))
}

#[cfg(target_os = "espidf")]
fn configure_ap_network(ap_netif: *mut sys::esp_netif_t) -> Result<(), String> {
    let err = unsafe { sys::esp_netif_dhcps_stop(ap_netif) };
    if err != 0 && err != sys::ESP_ERR_ESP_NETIF_DHCP_ALREADY_STOPPED {
        return Err(format!("esp_netif_dhcps_stop failed: {err}"));
    }

    let mut ip_info = sys::esp_netif_ip_info_t::default();
    ip_info.ip.addr = u32::from_le_bytes(AP_IP);
    ip_info.gw.addr = u32::from_le_bytes(AP_IP);
    ip_info.netmask.addr = u32::from_le_bytes([255, 255, 255, 0]);

    esp_to_result(
        unsafe { sys::esp_netif_set_ip_info(ap_netif, &ip_info) },
        "esp_netif_set_ip_info",
    )?;
    let err = unsafe { sys::esp_netif_dhcps_start(ap_netif) };
    if err != 0 && err != sys::ESP_ERR_ESP_NETIF_DHCP_ALREADY_STARTED {
        return Err(format!("esp_netif_dhcps_start failed: {err}"));
    }
    Ok(())
}

#[cfg(target_os = "espidf")]
fn copy_bytes_to_array<const N: usize>(dst: &mut [u8; N], value: &str) {
    dst.fill(0);
    let bytes = value.as_bytes();
    let len = bytes.len().min(N.saturating_sub(1));
    dst[..len].copy_from_slice(&bytes[..len]);
}

#[cfg(target_os = "espidf")]
fn start_config_ap_mode(
    _sta_netif: *mut sys::esp_netif_t,
    ap_netif: *mut sys::esp_netif_t,
) -> Result<(), String> {
    configure_ap_network(ap_netif)?;

    let mut ap_config = sys::wifi_ap_config_t::default();
    copy_bytes_to_array(&mut ap_config.ssid, AP_SSID);
    copy_bytes_to_array(&mut ap_config.password, AP_PASSWORD);
    ap_config.ssid_len = AP_SSID.len() as u8;
    ap_config.channel = 1;
    ap_config.max_connection = 4;
    ap_config.authmode = if AP_PASSWORD.len() < 8 {
        sys::wifi_auth_mode_t_WIFI_AUTH_OPEN
    } else {
        sys::wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK
    };
    ap_config.pmf_cfg = sys::wifi_pmf_config_t {
        capable: false,
        required: false,
    };

    let mut wifi_config = sys::wifi_config_t { ap: ap_config };
    esp_to_result(
        unsafe { sys::esp_wifi_set_mode(sys::wifi_mode_t_WIFI_MODE_APSTA) },
        "esp_wifi_set_mode(APSTA)",
    )?;
    esp_to_result(
        unsafe { sys::esp_wifi_set_config(sys::wifi_interface_t_WIFI_IF_AP, &mut wifi_config) },
        "esp_wifi_set_config(AP)",
    )?;
    let err = unsafe { sys::esp_wifi_start() };
    if err != 0 && err != sys::ESP_ERR_WIFI_CONN {
        return Err(format!("esp_wifi_start failed: {err}"));
    }
    Ok(())
}

#[cfg(target_os = "espidf")]
fn send_json(req: *mut sys::httpd_req_t, body: &str) -> i32 {
    unsafe {
        let _ = sys::httpd_resp_set_type(req, CONTENT_TYPE_JSON.as_ptr() as *const c_char);
        sys::httpd_resp_send(req, body.as_ptr() as *const c_char, body.len() as isize)
    }
}

#[cfg(target_os = "espidf")]
fn send_html(req: *mut sys::httpd_req_t, body: &str) -> i32 {
    unsafe {
        let _ = sys::httpd_resp_set_type(req, CONTENT_TYPE_HTML.as_ptr() as *const c_char);
        sys::httpd_resp_send(
            req,
            body.as_ptr() as *const c_char,
            sys::HTTPD_RESP_USE_STRLEN as isize,
        )
    }
}

#[cfg(target_os = "espidf")]
fn send_err(req: *mut sys::httpd_req_t, code: u32, message: &str) -> i32 {
    let message = CString::new(message).unwrap_or_else(|_| CString::new("error").unwrap());
    unsafe { sys::httpd_resp_send_err(req, code, message.as_ptr()) }
}

#[cfg(target_os = "espidf")]
fn read_body(req: *mut sys::httpd_req_t) -> Result<String, String> {
    let content_len = unsafe { (*req).content_len };
    let mut body = vec![0u8; content_len];
    let mut offset = 0usize;
    while offset < content_len {
        let received = unsafe {
            sys::httpd_req_recv(
                req,
                body[offset..].as_mut_ptr() as *mut c_char,
                (content_len - offset) as usize,
            )
        };
        if received <= 0 {
            return Err(format!("httpd_req_recv failed: {received}"));
        }
        offset += received as usize;
    }
    String::from_utf8(body).map_err(|err| err.to_string())
}

#[cfg(target_os = "espidf")]
fn state_from_req<'a>(req: *mut sys::httpd_req_t) -> &'a PortalState {
    unsafe { &*((*req).user_ctx as *const PortalState) }
}

#[cfg(target_os = "espidf")]
fn build_config_json(config: &DeviceRuntimeConfig, status: PortalRuntimeStatus) -> String {
    json!({
        "wifi_ssid": config.primary_wifi_ssid,
        "wifi_profile_count": config.wifi_profiles.len(),
        "last_connected_wifi_index": config.last_connected_wifi_index.map(|index| index as i32).unwrap_or(-1),
        "wifi_profiles": config.wifi_profiles.iter().map(|item| json!({
            "ssid": item.ssid,
            "password_len": item.password.len(),
        })).collect::<Vec<_>>(),
        "image_url_template": config.image_url_template,
        "orchestrator_enabled": config.orchestrator_enabled,
        "orchestrator_base_url": config.orchestrator_base_url,
        "device_id": config.device_id,
        "orchestrator_token": config.orchestrator_token,
        "photo_token": config.photo_token,
        "timezone": config.timezone,
        "interval_minutes": config.interval_minutes,
        "retry_base_minutes": config.retry_base_minutes,
        "retry_max_minutes": config.retry_max_minutes,
        "max_failure_before_long_sleep": config.max_failure_before_long_sleep,
        "display_rotation": config.display_rotation,
        "color_process_mode": config.color_process_mode,
        "dither_mode": config.dither_mode,
        "six_color_tolerance": config.six_color_tolerance,
        "wifi_connected": status.wifi_connected,
        "force_refresh": status.force_refresh,
        "last_http_status": status.last_http_status,
        "image_changed": status.image_changed,
        "image_source": "daily",
        "next_wakeup_epoch": 0,
        "battery_mv": status.battery_mv,
        "battery_percent": status.battery_percent,
        "charging": status.charging,
        "vbus_good": status.vbus_good,
        "last_error": "",
    }).to_string()
}

#[cfg(target_os = "espidf")]
fn parse_local_patch(body: &str) -> Result<LocalConfigPatch, String> {
    let value: Value = serde_json::from_str(body).map_err(|err| err.to_string())?;
    let read_u32 = |key: &str| {
        value
            .get(key)
            .and_then(|item| {
                item.as_u64()
                    .or_else(|| item.as_i64().map(|n| n.max(0) as u64))
            })
            .map(|number| number as u32)
    };
    let read_i32 = |key: &str| {
        value
            .get(key)
            .and_then(|item| item.as_i64())
            .map(|number| number as i32)
    };
    let read_string = |key: &str| {
        value
            .get(key)
            .and_then(|item| item.as_str())
            .map(|text| text.to_string())
    };
    let read_bool = |key: &str| {
        value.get(key).and_then(|item| {
            item.as_bool()
                .or_else(|| item.as_i64().map(|number| number != 0))
        })
    };

    Ok(LocalConfigPatch {
        wifi_ssid: read_string("wifi_ssid"),
        wifi_password: read_string("wifi_password"),
        image_url_template: read_string("image_url_template"),
        orchestrator_enabled: read_bool("orchestrator_enabled"),
        orchestrator_base_url: read_string("orchestrator_base_url"),
        device_id: read_string("device_id"),
        orchestrator_token: read_string("orchestrator_token"),
        photo_token: read_string("photo_token"),
        timezone: read_string("timezone"),
        interval_minutes: read_u32("interval_minutes"),
        retry_base_minutes: read_u32("retry_base_minutes"),
        retry_max_minutes: read_u32("retry_max_minutes"),
        max_failure_before_long_sleep: read_u32("max_failure_before_long_sleep"),
        display_rotation: read_i32("display_rotation"),
        color_process_mode: read_i32("color_process_mode"),
        dither_mode: read_i32("dither_mode"),
        six_color_tolerance: read_i32("six_color_tolerance"),
    })
}

#[cfg(target_os = "espidf")]
unsafe extern "C" fn handle_root(req: *mut sys::httpd_req_t) -> i32 {
    send_html(req, PORTAL_HTML)
}

#[cfg(target_os = "espidf")]
unsafe extern "C" fn handle_get_config(req: *mut sys::httpd_req_t) -> i32 {
    let state = state_from_req(req);
    let config = state.config.lock().unwrap().clone();
    let body = build_config_json(&config, state.status);
    send_json(req, &body)
}

#[cfg(target_os = "espidf")]
unsafe extern "C" fn handle_post_config(req: *mut sys::httpd_req_t) -> i32 {
    let state = state_from_req(req);
    let body = match read_body(req) {
        Ok(body) => body,
        Err(err) => return send_err(req, sys::httpd_err_code_t_HTTPD_400_BAD_REQUEST, &err),
    };

    let patch = match parse_local_patch(&body) {
        Ok(patch) => patch,
        Err(err) => return send_err(req, sys::httpd_err_code_t_HTTPD_400_BAD_REQUEST, &err),
    };

    let mut config = state.config.lock().unwrap();
    let outcome = config.apply_local_config_patch(&patch);
    let save_ok = {
        let mut storage = state.storage.lock().unwrap();
        storage.save_config(&config).is_ok()
    };

    let response = json!({
        "ok": save_ok,
        "reboot_required": outcome.wifi_changed,
        "error": if save_ok { Value::Null } else { Value::String("save failed".into()) },
    })
    .to_string();

    if save_ok {
        state.should_reboot.store(true, Ordering::SeqCst);
    }
    send_json(req, &response)
}

#[cfg(target_os = "espidf")]
unsafe extern "C" fn handle_scan_wifi(req: *mut sys::httpd_req_t) -> i32 {
    let mut scan_cfg = sys::wifi_scan_config_t::default();
    scan_cfg.show_hidden = false;
    let err = unsafe { sys::esp_wifi_scan_start(&scan_cfg, true) };
    if err != 0 {
        return send_err(
            req,
            sys::httpd_err_code_t_HTTPD_500_INTERNAL_SERVER_ERROR,
            "scan failed",
        );
    }

    let mut count: u16 = 20;
    let mut records = vec![sys::wifi_ap_record_t::default(); count as usize];
    let err = unsafe { sys::esp_wifi_scan_get_ap_records(&mut count, records.as_mut_ptr()) };
    if err != 0 {
        return send_err(
            req,
            sys::httpd_err_code_t_HTTPD_500_INTERNAL_SERVER_ERROR,
            "scan read failed",
        );
    }

    let networks = records
        .into_iter()
        .take(count as usize)
        .map(|record| {
            let len = record
                .ssid
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(record.ssid.len());
            let ssid = String::from_utf8_lossy(&record.ssid[..len]).into_owned();
            json!({
                "ssid": ssid,
                "rssi": record.rssi,
                "authmode": record.authmode,
            })
        })
        .collect::<Vec<_>>();

    let body = json!({ "networks": networks }).to_string();
    send_json(req, &body)
}

#[cfg(target_os = "espidf")]
impl RustPortalServer {
    fn start(
        config: DeviceRuntimeConfig,
        status: PortalRuntimeStatus,
        enable_dns: bool,
    ) -> Result<Self, String> {
        let storage = EspIdfStorage::new()?;
        let state = Box::new(PortalState {
            storage: Mutex::new(storage),
            config: Mutex::new(config),
            status,
            should_reboot: AtomicBool::new(false),
        });
        let state_ptr = Box::into_raw(state);

        let mut handle = ptr::null_mut();
        let config = httpd_default_config();
        esp_to_result(
            unsafe { sys::httpd_start(&mut handle, &config) },
            "httpd_start",
        )?;

        for uri in [
            sys::httpd_uri_t {
                uri: ROOT_URI.as_ptr() as *const c_char,
                method: sys::http_method_HTTP_GET,
                handler: Some(handle_root),
                user_ctx: state_ptr as *mut c_void,
            },
            sys::httpd_uri_t {
                uri: API_CONFIG_URI.as_ptr() as *const c_char,
                method: sys::http_method_HTTP_GET,
                handler: Some(handle_get_config),
                user_ctx: state_ptr as *mut c_void,
            },
            sys::httpd_uri_t {
                uri: API_CONFIG_URI.as_ptr() as *const c_char,
                method: sys::http_method_HTTP_POST,
                handler: Some(handle_post_config),
                user_ctx: state_ptr as *mut c_void,
            },
            sys::httpd_uri_t {
                uri: API_WIFI_SCAN_URI.as_ptr() as *const c_char,
                method: sys::http_method_HTTP_GET,
                handler: Some(handle_scan_wifi),
                user_ctx: state_ptr as *mut c_void,
            },
        ] {
            esp_to_result(
                unsafe { sys::httpd_register_uri_handler(handle, &uri) },
                "httpd_register_uri_handler",
            )?;
        }

        let dns_running = Arc::new(AtomicBool::new(enable_dns));
        let dns_thread = if enable_dns {
            let running = dns_running.clone();
            Some(thread::spawn(move || run_dns_server(running)))
        } else {
            None
        };

        Ok(Self {
            handle,
            state: state_ptr,
            dns_running,
            dns_thread,
        })
    }

    fn should_reboot(&self) -> bool {
        unsafe { (*self.state).should_reboot.load(Ordering::SeqCst) }
    }

    fn stop(&mut self) {
        self.dns_running.store(false, Ordering::SeqCst);
        if let Some(join) = self.dns_thread.take() {
            let _ = join.join();
        }
        if !self.handle.is_null() {
            unsafe {
                let _ = sys::httpd_stop(self.handle);
            }
            self.handle = ptr::null_mut();
        }
        if !self.state.is_null() {
            unsafe {
                drop(Box::from_raw(self.state));
            }
            self.state = ptr::null_mut();
        }
    }
}

#[cfg(target_os = "espidf")]
impl Drop for RustPortalServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "espidf")]
fn run_dns_server(running: Arc<AtomicBool>) {
    let socket = match UdpSocket::bind(("0.0.0.0", 53)) {
        Ok(socket) => socket,
        Err(_) => return,
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));

    let mut req = [0u8; 512];
    let mut resp = [0u8; 512];
    while running.load(Ordering::SeqCst) {
        let (size, from) = match socket.recv_from(&mut req) {
            Ok(result) => result,
            Err(_) => continue,
        };
        if size <= 12 {
            continue;
        }

        let mut q_end = 12usize;
        while q_end < size && req[q_end] != 0 {
            q_end += req[q_end] as usize + 1;
        }
        if q_end + 5 >= size {
            continue;
        }
        let question_len = q_end + 5 - 12;

        resp.fill(0);
        resp[0] = req[0];
        resp[1] = req[1];
        resp[2] = 0x81;
        resp[3] = 0x80;
        resp[4] = 0x00;
        resp[5] = 0x01;
        resp[6] = 0x00;
        resp[7] = 0x01;
        resp[12..12 + question_len].copy_from_slice(&req[12..12 + question_len]);
        let mut offset = 12 + question_len;
        for byte in [
            0xC0, 0x0C, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3C, 0x00, 0x04,
        ] {
            resp[offset] = byte;
            offset += 1;
        }
        for byte in AP_IP {
            resp[offset] = byte;
            offset += 1;
        }
        let _ = socket.send_to(&resp[..offset], from);
    }
}

#[cfg(target_os = "espidf")]
fn portal_loop(server: &RustPortalServer, window_seconds: Option<i32>) -> Result<(), String> {
    let deadline_us = window_seconds.map(
        |seconds| unsafe { sys::esp_timer_get_time() } + i64::from(seconds.max(0)) * 1_000_000,
    );

    loop {
        if server.should_reboot() {
            thread::sleep(Duration::from_millis(300));
            unsafe {
                sys::esp_restart();
            }
        }
        if let Some(deadline_us) = deadline_us
            && unsafe { sys::esp_timer_get_time() } >= deadline_us
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(PORTAL_LOOP_STEP_MS));
    }
}

#[cfg(target_os = "espidf")]
pub fn run_ap_portal_forever() -> Result<(), String> {
    let (sta_netif, ap_netif) = ensure_wifi_stack_for_portal()?;
    start_config_ap_mode(sta_netif, ap_netif)?;

    let mut storage = EspIdfStorage::new()?;
    let config = storage.load_config()?;
    let status = PortalRuntimeStatus {
        wifi_connected: false,
        force_refresh: false,
        last_http_status: 0,
        image_changed: false,
        battery_mv: -1,
        battery_percent: -1,
        charging: -1,
        vbus_good: -1,
    };
    let server = RustPortalServer::start(config, status, true)?;
    portal_loop(&server, None)
}

#[cfg(target_os = "espidf")]
pub fn run_sta_portal_window(power_sample: PowerSample, force_refresh: bool) -> Result<(), String> {
    let mut storage = EspIdfStorage::new()?;
    let config = storage.load_config()?;
    let status = PortalRuntimeStatus {
        wifi_connected: true,
        force_refresh,
        last_http_status: 0,
        image_changed: false,
        battery_mv: power_sample.battery_mv,
        battery_percent: power_sample.battery_percent,
        charging: power_sample.charging,
        vbus_good: power_sample.vbus_good,
    };
    let server = RustPortalServer::start(config, status, false)?;
    portal_loop(&server, Some(120))
}

#[cfg(not(target_os = "espidf"))]
pub fn run_ap_portal_forever() -> Result<(), String> {
    Err("portal only works on espidf target".into())
}

#[cfg(not(target_os = "espidf"))]
pub fn run_sta_portal_window(_power_sample: (), _force_refresh: bool) -> Result<(), String> {
    Err("portal only works on espidf target".into())
}
