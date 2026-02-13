function authHeaders() {
  const token = document.getElementById('token').value.trim();
  if (!token) {
    return {};
  }
  return { 'X-PhotoFrame-Token': token };
}

let previewBlobUrl = null;
let deviceMap = new Map();
let powerChartCache = null;
let powerResizeTimer = null;

const PUBLIC_DAILY_EXAMPLE_URL = 'https://example.com/daily.bmp';
const TOKEN_STORAGE_KEY = 'photoframe.console.token';
const TOKEN_COOKIE_KEY = 'photoframe_console_token';

function readCookie(name) {
  const encodedName = `${name}=`;
  const parts = document.cookie.split(';');
  for (const part of parts) {
    const item = part.trim();
    if (item.startsWith(encodedName)) {
      return decodeURIComponent(item.slice(encodedName.length));
    }
  }
  return '';
}

function writeCookie(name, value, maxAgeSeconds) {
  document.cookie = `${name}=${encodeURIComponent(value)}; path=/; max-age=${maxAgeSeconds}; samesite=lax`;
}

function loadStoredConsoleToken() {
  const input = document.getElementById('token');
  if (!input) return;

  let saved = '';
  try {
    saved = window.localStorage.getItem(TOKEN_STORAGE_KEY) || '';
  } catch (_) {
    // localStorage 受限时退化到 cookie
  }

  if (!saved) {
    saved = readCookie(TOKEN_COOKIE_KEY);
  }

  if (saved) {
    input.value = saved;
  }
}

function persistConsoleToken() {
  const input = document.getElementById('token');
  if (!input) return;
  const token = input.value.trim();

  try {
    if (token) {
      window.localStorage.setItem(TOKEN_STORAGE_KEY, token);
    } else {
      window.localStorage.removeItem(TOKEN_STORAGE_KEY);
    }
  } catch (_) {
    // ignore
  }

  if (token) {
    writeCookie(TOKEN_COOKIE_KEY, token, 180 * 24 * 3600);
  } else {
    writeCookie(TOKEN_COOKIE_KEY, '', 0);
  }
}

function escapeHtml(value) {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

function fmtEpoch(ts) {
  if (!ts) return '-';
  const d = new Date(ts * 1000);
  return d.toLocaleString();
}

function fmtDuration(seconds) {
  if (seconds == null) return '-';
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rest = m % 60;
  return rest ? `${h}h ${rest}m` : `${h}h`;
}

function startPolicyLabel(policy) {
  if (policy === 'next_wakeup') return '按设备下次唤醒';
  if (policy === 'explicit') return '按指定开始时间';
  if (policy === 'immediate') return '立即生效（等待设备来拉取）';
  return policy || '-';
}

function formatOverrideCreateResult(data) {
  const lines = [
    `插播 #${data.id} 已创建`,
    `目标设备: ${data.device_id}`,
    `开始策略: ${startPolicyLabel(data.start_policy)}`,
    `开始时间: ${fmtEpoch(data.start_epoch)}`,
    `结束时间: ${fmtEpoch(data.end_epoch)}`,
    `预计生效: ${fmtEpoch(data.expected_effective_epoch)}`,
  ];

  if (data.will_expire_before_effective) {
    lines.push('提示: 当前持续时间可能在设备下次唤醒前过期，建议延长持续分钟。');
  }

  return `${lines.join('\n')}\n\n详情:\n${JSON.stringify(data, null, 2)}`;
}

function shorten(text, maxLen = 72) {
  const s = String(text ?? '');
  if (s.length <= maxLen) {
    return s;
  }
  return `${s.slice(0, maxLen - 1)}…`;
}

function optionLabel(value, mapping) {
  if (value == null || value === '') {
    return '-';
  }
  const key = String(value);
  return mapping[key] || key;
}

function batteryStatusText(device) {
  const percent = Number(device?.battery_percent);
  const mv = Number(device?.battery_mv);

  if (Number.isFinite(percent) && percent >= 0) {
    if (Number.isFinite(mv) && mv > 0) {
      return `${percent}% / ${mv}mV`;
    }
    return `${percent}%`;
  }

  if (Number.isFinite(mv) && mv > 0) {
    return `${mv}mV`;
  }

  return '-';
}

function powerSourceText(device) {
  const vbus = Number(device?.vbus_good);
  const charging = Number(device?.charging);

  const vbusText = vbus === 1 ? 'USB' : vbus === 0 ? 'Battery' : '-';
  const chargeText = charging === 1 ? '充电中' : charging === 0 ? '未充电' : '-';

  if (vbusText === '-' && chargeText === '-') {
    return '-';
  }
  return `${vbusText} / ${chargeText}`;
}

function normalizeBinaryFlag(value) {
  const v = Number(value);
  if (v === 0 || v === 1) return v;
  return null;
}

function medianInt(values) {
  if (!values || values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}

function estimateSampleIntervalSeconds(items) {
  if (!items || items.length < 2) return null;
  const diffs = [];
  for (let i = 1; i < items.length; i++) {
    const prev = Number(items[i - 1]?.sample_epoch);
    const cur = Number(items[i]?.sample_epoch);
    if (!Number.isFinite(prev) || !Number.isFinite(cur)) continue;
    const diff = cur - prev;
    if (diff > 0 && diff < 365 * 24 * 3600) {
      diffs.push(diff);
    }
  }
  return medianInt(diffs);
}

function fmtEpochCompact(ts) {
  if (!ts) return '-';
  const d = new Date(ts * 1000);
  // 避免表格太长：优先展示 月-日 时:分
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  const hh = String(d.getHours()).padStart(2, '0');
  const mi = String(d.getMinutes()).padStart(2, '0');
  return `${mm}-${dd} ${hh}:${mi}`;
}

function analyzeLatestDischarge(items, thresholdPercent) {
  // 口径：最近一次 vbus_good 1->0（拔 USB）到 battery_percent <= 阈值 的时长（估算）。
  if (!items || items.length === 0) {
    return { ok: false, reason: '暂无采样数据' };
  }

  let prevVbus = null;
  let unplugIdx = null;
  for (let i = 0; i < items.length; i++) {
    const vbus = normalizeBinaryFlag(items[i]?.vbus_good);
    if (prevVbus === 1 && vbus === 0) {
      unplugIdx = i;
    }
    if (vbus != null) {
      prevVbus = vbus;
    }
  }

  if (unplugIdx == null) {
    const anyBattery = items.some((it) => normalizeBinaryFlag(it?.vbus_good) === 0);
    if (anyBattery) {
      return { ok: false, reason: '窗口内一直处于电池供电或缺少 USB→电池 转折点，可尝试扩大时间窗' };
    }
    return { ok: false, reason: '未检测到拔 USB(vbus_good 1→0) 事件（可能 vbus_good 未上报）' };
  }

  const start = items[unplugIdx];
  const startEpoch = Number(start.sample_epoch);
  const startPercentRaw = Number(start.battery_percent);
  const startPercent = Number.isFinite(startPercentRaw) && startPercentRaw >= 0 ? startPercentRaw : null;

  let minPercent = null;
  let endEpoch = null;
  let endPercent = null;
  let endReason = 'ongoing';
  let plugEpoch = null;

  for (let i = unplugIdx; i < items.length; i++) {
    const item = items[i];
    const vbus = normalizeBinaryFlag(item?.vbus_good);
    if (i > unplugIdx && vbus === 1) {
      plugEpoch = Number(item.sample_epoch);
      endEpoch = plugEpoch;
      endReason = 'plugged';
      break;
    }

    const pRaw = Number(item?.battery_percent);
    if (Number.isFinite(pRaw) && pRaw >= 0) {
      minPercent = minPercent == null ? pRaw : Math.min(minPercent, pRaw);
      if (pRaw <= thresholdPercent) {
        endEpoch = Number(item.sample_epoch);
        endPercent = pRaw;
        endReason = 'threshold';
        break;
      }
    }
  }

  if (endEpoch == null) {
    const last = items[items.length - 1];
    endEpoch = Number(last.sample_epoch) || startEpoch;
  }

  const durationSeconds = Math.max(0, endEpoch - startEpoch);
  const last = items[items.length - 1];
  const lastEpoch = Number(last.sample_epoch) || null;
  const lastPercentRaw = Number(last.battery_percent);
  const lastPercent = Number.isFinite(lastPercentRaw) && lastPercentRaw >= 0 ? lastPercentRaw : null;

  return {
    ok: true,
    start_epoch: startEpoch,
    start_percent: startPercent,
    end_epoch: endEpoch,
    end_percent: endPercent,
    end_reason: endReason,
    plug_epoch: plugEpoch,
    duration_seconds: durationSeconds,
    min_percent: minPercent,
    last_epoch: lastEpoch,
    last_percent: lastPercent,
  };
}

function prepareHiDPICanvas(canvas) {
  const rect = canvas.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const cssW = Math.max(1, Math.floor(rect.width));
  const cssH = Math.max(1, Math.floor(rect.height));
  const pixW = Math.max(1, Math.floor(cssW * dpr));
  const pixH = Math.max(1, Math.floor(cssH * dpr));

  if (canvas.width !== pixW || canvas.height !== pixH) {
    canvas.width = pixW;
    canvas.height = pixH;
  }

  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return { ctx, width: cssW, height: cssH };
}

function drawPowerChart(canvas, items, opts) {
  const { ctx, width, height } = prepareHiDPICanvas(canvas);
  ctx.clearRect(0, 0, width, height);

  const pad = { l: 54, r: 68, t: 26, b: 30 };
  const x0 = pad.l;
  const x1 = width - pad.r;
  const y0 = pad.t;
  const y1 = height - pad.b;

  ctx.fillStyle = '#ffffff';
  ctx.fillRect(0, 0, width, height);

  if (!items || items.length === 0) {
    ctx.fillStyle = '#5f6980';
    ctx.font = '12px ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, Segoe UI';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText('暂无电池采样数据（等待设备 checkin 上报）', width / 2, height / 2);
    return;
  }

  const fromEpoch = Number(opts?.fromEpoch);
  const toEpoch = Number(opts?.toEpoch);
  const xMin = Number.isFinite(fromEpoch) ? fromEpoch : Number(items[0].sample_epoch);
  const xMaxRaw = Number.isFinite(toEpoch) ? toEpoch : Number(items[items.length - 1].sample_epoch);
  const xMax = xMaxRaw > xMin ? xMaxRaw : xMin + 1;

  const mvValues = items
    .map((it) => Number(it?.battery_mv))
    .filter((mv) => Number.isFinite(mv) && mv > 0);
  let mvMin = null;
  let mvMax = null;
  if (mvValues.length > 0) {
    mvMin = Math.min(...mvValues);
    mvMax = Math.max(...mvValues);
    if (mvMax === mvMin) {
      mvMax = mvMin + 1;
    }
    // 适当加一点 padding，避免线条顶到边。
    mvMin = Math.max(0, mvMin - 60);
    mvMax = mvMax + 60;
  }

  const xScale = (epoch) => x0 + ((epoch - xMin) / (xMax - xMin)) * (x1 - x0);
  const yPercent = (p) => y1 - (p / 100) * (y1 - y0);
  const yMv = (mv) => y1 - ((mv - mvMin) / (mvMax - mvMin)) * (y1 - y0);

  // 背景：USB / 电池 供电区间（按 vbus_good 分段）。
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    const vbus = normalizeBinaryFlag(item?.vbus_good);
    if (vbus == null) continue;
    const segStart = Number(item.sample_epoch);
    const segEnd = i + 1 < items.length ? Number(items[i + 1].sample_epoch) : xMax;
    if (!Number.isFinite(segStart) || !Number.isFinite(segEnd)) continue;

    ctx.fillStyle = vbus === 1 ? 'rgba(53, 88, 229, 0.06)' : 'rgba(212, 133, 28, 0.06)';
    const sx = xScale(segStart);
    const ex = xScale(Math.min(segEnd, xMax));
    ctx.fillRect(sx, y0, Math.max(0, ex - sx), y1 - y0);
  }

  // 顶部条：充电中/未充电（按 charging 分段）。
  const chargeBarH = 6;
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    const charging = normalizeBinaryFlag(item?.charging);
    if (charging == null) continue;
    const segStart = Number(item.sample_epoch);
    const segEnd = i + 1 < items.length ? Number(items[i + 1].sample_epoch) : xMax;
    if (!Number.isFinite(segStart) || !Number.isFinite(segEnd)) continue;

    ctx.fillStyle = charging === 1 ? 'rgba(47, 158, 100, 0.38)' : 'rgba(58, 66, 87, 0.22)';
    const sx = xScale(segStart);
    const ex = xScale(Math.min(segEnd, xMax));
    ctx.fillRect(sx, y0, Math.max(0, ex - sx), chargeBarH);
  }

  // 网格：百分比 0/25/50/75/100
  ctx.strokeStyle = 'rgba(230, 233, 240, 1)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (const p of [0, 25, 50, 75, 100]) {
    const y = yPercent(p);
    ctx.moveTo(x0, y);
    ctx.lineTo(x1, y);
  }
  ctx.stroke();

  // 轴与标注
  ctx.fillStyle = '#5f6980';
  ctx.font = '11px ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, Segoe UI';
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  for (const p of [0, 50, 100]) {
    ctx.fillText(`${p}%`, x0 - 8, yPercent(p));
  }

  if (mvMin != null && mvMax != null) {
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const mid = Math.round((mvMin + mvMax) / 2);
    for (const mv of [mvMin, mid, mvMax]) {
      ctx.fillText(`${Math.round(mv)}mV`, x1 + 8, yMv(mv));
    }
  }

  // X 轴：起点/中点/终点
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  const xTicks = [xMin, Math.floor((xMin + xMax) / 2), xMax];
  for (const ts of xTicks) {
    const x = xScale(ts);
    ctx.fillText(fmtEpochCompact(ts), x, y1 + 6);
  }

  // 线条：battery_percent
  ctx.strokeStyle = 'rgba(53, 88, 229, 0.95)';
  ctx.lineWidth = 2;
  ctx.beginPath();
  let moved = false;
  for (const it of items) {
    const ts = Number(it?.sample_epoch);
    const p = Number(it?.battery_percent);
    if (!Number.isFinite(ts) || !Number.isFinite(p) || p < 0) {
      moved = false;
      continue;
    }
    const x = xScale(ts);
    const y = yPercent(Math.max(0, Math.min(100, p)));
    if (!moved) {
      ctx.moveTo(x, y);
      moved = true;
    } else {
      ctx.lineTo(x, y);
    }
  }
  ctx.stroke();

  // 线条：battery_mv
  if (mvMin != null && mvMax != null) {
    ctx.strokeStyle = 'rgba(212, 133, 28, 0.95)';
    ctx.lineWidth = 2;
    ctx.beginPath();
    moved = false;
    for (const it of items) {
      const ts = Number(it?.sample_epoch);
      const mv = Number(it?.battery_mv);
      if (!Number.isFinite(ts) || !Number.isFinite(mv) || mv <= 0) {
        moved = false;
        continue;
      }
      const x = xScale(ts);
      const y = yMv(mv);
      if (!moved) {
        ctx.moveTo(x, y);
        moved = true;
      } else {
        ctx.lineTo(x, y);
      }
    }
    ctx.stroke();
  }
}

async function loadPowerSamples() {
  const deviceId = (document.getElementById('powerDeviceId')?.value || '').trim();
  const daysRaw = Number(document.getElementById('powerDays')?.value || 30);
  const thresholdRaw = Number(document.getElementById('powerLowThreshold')?.value || 10);

  if (!deviceId) {
    setText('powerSummary', '请选择设备后再刷新曲线。');
    setText('powerHint', '');
    const canvas = document.getElementById('powerCanvas');
    if (canvas) {
      // 画一个空白提示，避免看起来像“没生效”。
      canvas.getContext('2d')?.clearRect(0, 0, canvas.width, canvas.height);
    }
    return;
  }

  const days = Math.max(1, Math.min(365, Math.floor(daysRaw)));
  const threshold = Math.max(0, Math.min(100, Math.floor(thresholdRaw)));

  setText('powerSummary', `加载中：${deviceId} · 最近 ${days} 天…`);
  setText('powerHint', '');

  const nowEpoch = Math.floor(Date.now() / 1000);
  const fromEpoch = nowEpoch - days * 24 * 3600;
  const query = `?device_id=${encodeURIComponent(deviceId)}&from_epoch=${fromEpoch}&to_epoch=${nowEpoch}&limit=20000`;
  const data = await fetchJson(`/api/v1/power-samples${query}`);

  const items = data.items || [];
  const interval = estimateSampleIntervalSeconds(items);
  const intervalText = interval ? `采样间隔≈${fmtDuration(interval)}（估算误差不超过一个采样周期）` : '采样间隔：-';

  const discharge = analyzeLatestDischarge(items, threshold);
  if (!discharge.ok) {
    setText('powerSummary', `${deviceId} · ${discharge.reason}`);
  } else {
    const startText = fmtEpoch(discharge.start_epoch);
    const durationText = fmtDuration(discharge.duration_seconds);
    const startPercent = discharge.start_percent != null ? `${discharge.start_percent}%` : '-';
    const minPercent = discharge.min_percent != null ? `${discharge.min_percent}%` : '-';

    if (discharge.end_reason === 'threshold') {
      const endText = fmtEpoch(discharge.end_epoch);
      const endPercent = discharge.end_percent != null ? `${discharge.end_percent}%` : '-';
      setText(
        'powerSummary',
        `最近一次放电：${startText} 拔 USB → ${endText} ≤ ${threshold}%（${durationText}，${startPercent} → ${endPercent}）`
      );
    } else if (discharge.end_reason === 'plugged') {
      const plugText = fmtEpoch(discharge.plug_epoch);
      setText(
        'powerSummary',
        `最近一次拔 USB：${startText}（起始 ${startPercent}，最低 ${minPercent}）· 在达到 ${threshold}% 前于 ${plugText} 接入 USB`
      );
    } else {
      const lastText = fmtEpoch(discharge.last_epoch);
      const lastPercent = discharge.last_percent != null ? `${discharge.last_percent}%` : '-';
      setText(
        'powerSummary',
        `最近一次拔 USB：${startText}（起始 ${startPercent}）· 已持续 ${durationText}，最新 ${lastText} 电量 ${lastPercent}（阈值 ${threshold}%）`
      );
    }
  }

  setText(
    'powerHint',
    `${items.length} 点 · ${fmtEpoch(data.from_epoch)} ~ ${fmtEpoch(data.to_epoch)} · ${intervalText}`
  );

  const canvas = document.getElementById('powerCanvas');
  if (canvas) {
    powerChartCache = {
      device_id: deviceId,
      from_epoch: data.from_epoch,
      to_epoch: data.to_epoch,
      items,
    };
    drawPowerChart(canvas, items, { fromEpoch: data.from_epoch, toEpoch: data.to_epoch });
  }
}

function setText(id, text) {
  const el = document.getElementById(id);
  if (el) {
    el.textContent = text;
  }
}

function setPlaceholder(id, text) {
  const el = document.getElementById(id);
  if (el) {
    el.placeholder = text;
  }
}

async function fetchJson(url, options = {}) {
  const headers = { ...authHeaders(), ...(options.headers || {}) };
  const resp = await fetch(url, { ...options, headers });
  const text = await resp.text();
  let data = null;
  try {
    data = JSON.parse(text);
  } catch (_) {
    data = { raw: text };
  }
  if (!resp.ok) {
    const msg = (data && (data.detail || data.error || data.raw)) || `HTTP ${resp.status}`;
    throw new Error(msg);
  }
  return data;
}

function renderStateTag(state) {
  if (state === 'active') return '<span class="tag active">active</span>';
  if (state === 'upcoming') return '<span class="tag upcoming">upcoming</span>';
  if (state === 'expired') return '<span class="tag expired">expired</span>';
  return `<span class="tag">${escapeHtml(state || '-')}</span>`;
}

function renderConfigApplyStatus(device) {
  const appliedVersion = Number(device.config_applied_version || 0);
  if (!appliedVersion) {
    return '<span class="tag">-</span>';
  }

  if (device.config_apply_ok) {
    return `<span class="tag active">ok v${appliedVersion}</span>`;
  }

  const err = device.config_apply_error
    ? `<div class="muted">${escapeHtml(shorten(device.config_apply_error, 80))}</div>`
    : '';
  return `<span class="tag expired">fail v${appliedVersion}</span>${err}`;
}

function appendDeviceOption(select, value) {
  const op = document.createElement('option');
  op.value = value;
  op.textContent = value;
  select.appendChild(op);
}

function normalizeReported(device) {
  if (!device || typeof device !== 'object') {
    return {};
  }
  if (!device.reported_config || typeof device.reported_config !== 'object') {
    return {};
  }
  return device.reported_config;
}

function updateConfigHints() {
  const selectedDevice = document.getElementById('configDeviceId').value || '*';
  const device = deviceMap.get(selectedDevice);
  const reported = normalizeReported(device);

  const reportedOr = (key, fallback = '-') => {
    if (Object.prototype.hasOwnProperty.call(reported, key)) {
      const value = reported[key];
      if (value === null || value === '') {
        return '-';
      }
      return String(value);
    }
    return fallback;
  };

  const intervalFallback = device ? String(Math.max(1, Math.floor((device.poll_interval_seconds || 3600) / 60))) : '-';

  const orchEnabled = reportedOr('orchestrator_enabled', '-');
  const rotation = reportedOr('display_rotation', '-');
  const colorMode = reportedOr('color_process_mode', '-');
  const ditherMode = reportedOr('dither_mode', '-');

  setText('cfgHintOrchEnabled', `当前: ${optionLabel(orchEnabled, { '0': '关闭', '1': '启用' })}`);
  setText('cfgHintDisplayRotation', `当前: ${optionLabel(rotation, { '0': '旋转 0', '2': '旋转 180' })}`);
  setText('cfgHintColorProcessMode', `当前: ${optionLabel(colorMode, {
    '0': '自动判断',
    '1': '总是转换为 6 色',
    '2': '认为输入已是 6 色',
  })}`);
  setText('cfgHintDitherMode', `当前: ${optionLabel(ditherMode, { '0': '关闭', '1': '有序抖动' })}`);

  setPlaceholder('cfgTimezone', `当前: ${reportedOr('timezone', '-')}`);
  setPlaceholder('cfgOrchBaseUrl', `当前: ${reportedOr('orchestrator_base_url', '-')}`);
  setPlaceholder('cfgImageUrlTemplate', `当前: ${reportedOr('image_url_template', '-')}`);
  setPlaceholder('cfgOrchToken', `当前: ${reportedOr('orchestrator_token', '未设置')}`);
  setPlaceholder('cfgPhotoToken', `当前: ${reportedOr('photo_token', '未设置')}`);
  setPlaceholder('cfgIntervalMinutes', `当前: ${reportedOr('interval_minutes', intervalFallback)}`);
  setPlaceholder('cfgRetryBaseMinutes', `当前: ${reportedOr('retry_base_minutes', '-')}`);
  setPlaceholder('cfgRetryMaxMinutes', `当前: ${reportedOr('retry_max_minutes', '-')}`);
  setPlaceholder('cfgMaxFailure', `当前: ${reportedOr('max_failure_before_long_sleep', '-')}`);
  setPlaceholder('cfgSixColorTolerance', `当前: ${reportedOr('six_color_tolerance', '-')}`);
}

function clearConfigPatchInputs() {
  const textIds = [
    'cfgTimezone',
    'cfgOrchBaseUrl',
    'cfgImageUrlTemplate',
    'cfgOrchToken',
    'cfgPhotoToken',
    'cfgIntervalMinutes',
    'cfgRetryBaseMinutes',
    'cfgRetryMaxMinutes',
    'cfgMaxFailure',
    'cfgSixColorTolerance',
  ];
  for (const id of textIds) {
    const input = document.getElementById(id);
    if (input) input.value = '';
  }

  const selectIds = [
    'cfgOrchEnabled',
    'cfgDisplayRotation',
    'cfgColorProcessMode',
    'cfgDitherMode',
  ];
  for (const id of selectIds) {
    const select = document.getElementById(id);
    if (select) select.value = '';
  }
}

function parseOptionalInteger(id, title, minValue, maxValue = null) {
  const raw = document.getElementById(id).value.trim();
  if (!raw) return null;
  const value = Number(raw);
  if (!Number.isInteger(value)) {
    throw new Error(`${title} 必须是整数`);
  }
  if (value < minValue) {
    throw new Error(`${title} 不能小于 ${minValue}`);
  }
  if (maxValue != null && value > maxValue) {
    throw new Error(`${title} 不能大于 ${maxValue}`);
  }
  return value;
}

function collectDeviceConfigPatch() {
  const patch = {};

  const addSelectNumber = (id, key) => {
    const raw = document.getElementById(id).value;
    if (raw === '') return;
    patch[key] = Number(raw);
  };

  const addText = (id, key) => {
    const raw = document.getElementById(id).value.trim();
    if (!raw) return;
    patch[key] = raw;
  };

  addSelectNumber('cfgOrchEnabled', 'orchestrator_enabled');
  addText('cfgTimezone', 'timezone');
  addText('cfgOrchBaseUrl', 'orchestrator_base_url');
  addText('cfgImageUrlTemplate', 'image_url_template');
  addText('cfgOrchToken', 'orchestrator_token');
  addText('cfgPhotoToken', 'photo_token');

  const interval = parseOptionalInteger('cfgIntervalMinutes', '刷新间隔', 1, 24 * 60);
  if (interval != null) patch.interval_minutes = interval;

  const retryBase = parseOptionalInteger('cfgRetryBaseMinutes', '失败重试基数', 1, 24 * 60);
  if (retryBase != null) patch.retry_base_minutes = retryBase;

  const retryMax = parseOptionalInteger('cfgRetryMaxMinutes', '失败重试上限', 1, 7 * 24 * 60);
  if (retryMax != null) patch.retry_max_minutes = retryMax;

  const maxFail = parseOptionalInteger('cfgMaxFailure', '连续失败阈值', 1, 1000);
  if (maxFail != null) patch.max_failure_before_long_sleep = maxFail;

  addSelectNumber('cfgDisplayRotation', 'display_rotation');
  addSelectNumber('cfgColorProcessMode', 'color_process_mode');
  addSelectNumber('cfgDitherMode', 'dither_mode');

  const tolerance = parseOptionalInteger('cfgSixColorTolerance', '6 色判断容差', 0, 64);
  if (tolerance != null) patch.six_color_tolerance = tolerance;

  return patch;
}

async function loadHealth() {
  const data = await fetchJson('/healthz');
  const version = data.app_version || '-';
  document.getElementById('appVersion').textContent = version;
  document.getElementById('appVersionStat').textContent = version;
}

async function loadDevices() {
  const data = await fetchJson('/api/v1/devices');
  const body = document.getElementById('devicesBody');
  const overrideDeviceSelect = document.getElementById('deviceId');
  const configDeviceSelect = document.getElementById('configDeviceId');
  const powerDeviceSelect = document.getElementById('powerDeviceId');
  const selectedOverrideBefore = overrideDeviceSelect.value;
  const selectedConfigBefore = configDeviceSelect.value;
  const selectedPowerBefore = powerDeviceSelect ? powerDeviceSelect.value : '';

  body.innerHTML = '';
  overrideDeviceSelect.innerHTML = '<option value="*">全部设备 (*)</option>';
  configDeviceSelect.innerHTML = '<option value="*">全部设备 (*)</option>';
  if (powerDeviceSelect) {
    powerDeviceSelect.innerHTML = '<option value="">请选择设备</option>';
  }

  const devices = data.devices || [];
  deviceMap = new Map();
  document.getElementById('deviceCount').textContent = String(devices.length);
  document.getElementById('serverNow').textContent = fmtEpoch(data.now_epoch);

  for (const d of devices) {
    deviceMap.set(d.device_id, d);

    const tr = document.createElement('tr');
    const cfgVersion = `${d.config_target_version || 0}/${d.config_seen_version || 0}/${d.config_applied_version || 0}`;
    const cfgQuery = fmtEpoch(d.config_last_query_epoch);

    tr.innerHTML = `
      <td><span class="tag">${escapeHtml(d.device_id)}</span></td>
      <td>${fmtEpoch(d.last_checkin_epoch)}</td>
      <td>${fmtEpoch(d.next_wakeup_epoch)}</td>
      <td>${fmtDuration(d.eta_seconds)}</td>
      <td>${fmtDuration(d.poll_interval_seconds)}</td>
      <td>${escapeHtml(d.failure_count)}</td>
      <td>${escapeHtml(d.image_source || 'daily')}</td>
      <td>${escapeHtml(batteryStatusText(d))}</td>
      <td>${escapeHtml(powerSourceText(d))}</td>
      <td>${escapeHtml(cfgVersion)}</td>
      <td>${cfgQuery}</td>
      <td>${renderConfigApplyStatus(d)}</td>
      <td>${escapeHtml(shorten(d.last_error || '', 88))}</td>
    `;
    body.appendChild(tr);

    appendDeviceOption(overrideDeviceSelect, d.device_id);
    appendDeviceOption(configDeviceSelect, d.device_id);
    if (powerDeviceSelect) {
      appendDeviceOption(powerDeviceSelect, d.device_id);
    }
  }

  if ([...overrideDeviceSelect.options].some((o) => o.value === selectedOverrideBefore)) {
    overrideDeviceSelect.value = selectedOverrideBefore;
  }
  if ([...configDeviceSelect.options].some((o) => o.value === selectedConfigBefore)) {
    configDeviceSelect.value = selectedConfigBefore;
  }
  if (powerDeviceSelect) {
    if ([...powerDeviceSelect.options].some((o) => o.value === selectedPowerBefore)) {
      powerDeviceSelect.value = selectedPowerBefore;
    } else if (powerDeviceSelect.options.length === 2) {
      // 只有一台设备时，默认选中，减少点击。
      powerDeviceSelect.value = powerDeviceSelect.options[1].value;
    }
  }

  updateConfigHints();
}

async function loadOverrides() {
  const data = await fetchJson('/api/v1/overrides');
  const body = document.getElementById('overridesBody');
  body.innerHTML = '';

  const overrides = data.overrides || [];
  document.getElementById('overrideCount').textContent = String(overrides.length);

  for (const item of overrides) {
    const tr = document.createElement('tr');
    const delBtn = `<button data-id="${item.id}" class="deleteBtn danger">取消</button>`;
    tr.innerHTML = `
      <td>${item.id}</td>
      <td>${escapeHtml(item.device_id)}</td>
      <td>${renderStateTag(item.state)}</td>
      <td>${fmtEpoch(item.start_epoch)}</td>
      <td>${fmtEpoch(item.end_epoch)}</td>
      <td>${fmtEpoch(item.expected_effective_epoch)}</td>
      <td>${escapeHtml(item.note || '')}</td>
      <td>${delBtn}</td>
    `;
    body.appendChild(tr);
  }

  for (const btn of document.querySelectorAll('.deleteBtn')) {
    btn.addEventListener('click', async () => {
      const id = btn.getAttribute('data-id');
      if (!confirm(`确认取消插播 #${id} ?`)) return;
      try {
        await fetchJson(`/api/v1/overrides/${id}`, { method: 'DELETE' });
        await refreshAll();
      } catch (err) {
        alert(`取消失败: ${err.message}`);
      }
    });
  }
}

async function loadDeviceTokens() {
  const data = await fetchJson('/api/v1/device-tokens?pending_only=1');
  const body = document.getElementById('deviceTokensBody');
  body.innerHTML = '';

  const items = data.items || [];
  if (items.length === 0) {
    body.innerHTML = '<tr><td colspan="5" class="muted">暂无待审批设备</td></tr>';
    document.getElementById('deviceTokensHint').textContent = '待审批: 0';
    return;
  }

  for (const item of items) {
    const tr = document.createElement('tr');
    const stateText = item.approved ? '已信任' : '待审批';
    const actions = item.approved
      ? `<button data-device="${escapeHtml(item.device_id)}" class="deleteTokenBtn danger">移除</button>`
      : `<button data-device="${escapeHtml(item.device_id)}" class="approveTokenBtn">信任</button> <button data-device="${escapeHtml(item.device_id)}" class="deleteTokenBtn danger">拒绝</button>`;

    tr.innerHTML = `
      <td><span class="tag">${escapeHtml(item.device_id)}</span></td>
      <td>${fmtEpoch(item.first_seen_epoch)}</td>
      <td>${fmtEpoch(item.last_seen_epoch)}</td>
      <td>${escapeHtml(stateText)}</td>
      <td>${actions}</td>
    `;
    body.appendChild(tr);
  }

  document.getElementById('deviceTokensHint').textContent = `待审批: ${items.length}`;

  for (const btn of document.querySelectorAll('.approveTokenBtn')) {
    btn.addEventListener('click', async () => {
      const deviceId = btn.getAttribute('data-device');
      if (!deviceId) return;
      try {
        await fetchJson(`/api/v1/device-tokens/${encodeURIComponent(deviceId)}/approve`, {
          method: 'POST',
        });
        await refreshAll();
      } catch (err) {
        alert(`审批失败: ${err.message}`);
      }
    });
  }

  for (const btn of document.querySelectorAll('.deleteTokenBtn')) {
    btn.addEventListener('click', async () => {
      const deviceId = btn.getAttribute('data-device');
      if (!deviceId) return;
      if (!confirm(`确认移除设备 ${deviceId} 的 token 记录？`)) return;
      try {
        await fetchJson(`/api/v1/device-tokens/${encodeURIComponent(deviceId)}`, {
          method: 'DELETE',
        });
        await refreshAll();
      } catch (err) {
        alert(`移除失败: ${err.message}`);
      }
    });
  }
}


function renderPublishHistoryItem(item) {
  const sourceTag = item.source === 'override'
    ? '<span class="tag active">override</span>'
    : '<span class="tag">daily</span>';
  const overrideText = item.override_id == null ? '-' : `#${item.override_id}`;
  const safeUrl = escapeHtml(item.image_url || '');
  const shortUrl = escapeHtml(shorten(item.image_url || '', 78));

  return `
    <article class="release-item">
      <div class="release-head">
        <p class="release-title">${sourceTag} <span class="tag">${escapeHtml(item.device_id)}</span></p>
        <span class="release-date">${fmtEpoch(item.issued_epoch)}</span>
      </div>
      <p class="release-summary">${shortUrl}</p>
      <ul>
        <li>override_id: ${escapeHtml(overrideText)}</li>
        <li>poll_after: ${fmtDuration(item.poll_after_seconds)}</li>
        <li>valid_until: ${fmtEpoch(item.valid_until_epoch)}</li>
        <li><a href="${safeUrl}" target="_blank" rel="noreferrer">打开原图</a></li>
      </ul>
    </article>
  `;
}

async function loadPublishHistory() {
  const selectedDevice = document.getElementById('deviceId').value || '*';
  const query = selectedDevice !== '*'
    ? `?device_id=${encodeURIComponent(selectedDevice)}&limit=120`
    : '?limit=120';
  const data = await fetchJson(`/api/v1/publish-history${query}`);

  const items = data.items || [];
  const body = document.getElementById('publishHistoryBody');
  if (items.length === 0) {
    body.innerHTML = '<p class="muted">暂无发布记录（等待设备下一次拉取）。</p>';
  } else {
    body.innerHTML = items.map(renderPublishHistoryItem).join('');
  }

  const scope = selectedDevice === '*' ? '全部设备' : selectedDevice;
  document.getElementById('publishHistoryHint').textContent = `${scope} · 最近 ${items.length} 条`;
}

async function loadCurrentPreview() {
  const selectedDevice = document.getElementById('deviceId').value || '*';
  const meta = document.getElementById('previewMeta');
  const img = document.getElementById('currentPreview');

  const headers = authHeaders();
  const resp = await fetch(`/api/v1/preview/current.bmp?device_id=${encodeURIComponent(selectedDevice)}`, {
    headers,
  });

  if (!resp.ok) {
    const textBody = await resp.text();
    let detail = textBody;
    try {
      const data = JSON.parse(textBody);
      detail = data.detail || data.error || textBody;
    } catch (_) {
      // keep raw text
    }
    throw new Error(detail || `HTTP ${resp.status}`);
  }

  const blob = await resp.blob();
  if (previewBlobUrl) {
    URL.revokeObjectURL(previewBlobUrl);
  }
  previewBlobUrl = URL.createObjectURL(blob);
  img.src = previewBlobUrl;

  const source = resp.headers.get('X-PhotoFrame-Source') || 'daily';
  const target = resp.headers.get('X-PhotoFrame-Device') || selectedDevice;
  meta.textContent = `设备 ${target} · 当前来源 ${source} · ${fmtEpoch(Math.floor(Date.now() / 1000))}`;
}

function renderConfigHistoryItem(item) {
  const configText = escapeHtml(JSON.stringify(item.config || {}, null, 2));
  return `
    <article class="release-item">
      <div class="release-head">
        <p class="release-title"><span class="tag">${escapeHtml(item.device_id)}</span> 配置版本 #${item.id}</p>
        <span class="release-date">${fmtEpoch(item.created_epoch)}</span>
      </div>
      <p class="release-summary">${escapeHtml(item.note || '-')}</p>
      <pre>${configText}</pre>
    </article>
  `;
}

async function loadDeviceConfigs() {
  const selectedDevice = document.getElementById('configDeviceId').value || '*';
  const query = selectedDevice !== '*'
    ? `?device_id=${encodeURIComponent(selectedDevice)}&limit=80`
    : '?limit=80';
  const data = await fetchJson(`/api/v1/device-configs${query}`);

  const items = data.items || [];
  const body = document.getElementById('configHistoryBody');
  if (items.length === 0) {
    body.innerHTML = '<p class="muted">暂无设备配置发布记录。</p>';
  } else {
    body.innerHTML = items.map(renderConfigHistoryItem).join('');
  }

  const scope = selectedDevice === '*' ? '全部设备' : selectedDevice;
  document.getElementById('configHistoryHint').textContent = `${scope} · 最近 ${items.length} 条`;
}

async function submitOverride(ev) {
  ev.preventDefault();
  const fileInput = document.getElementById('imageFile');
  if (!fileInput.files || fileInput.files.length === 0) {
    alert('请先选择图片');
    return;
  }

  const fd = new FormData();
  fd.append('file', fileInput.files[0]);
  fd.append('device_id', document.getElementById('deviceId').value);
  fd.append('duration_minutes', document.getElementById('duration').value);
  fd.append('starts_at', document.getElementById('startsAt').value || '');
  fd.append('note', document.getElementById('note').value || '');

  const headers = authHeaders();
  const resp = await fetch('/api/v1/overrides/upload', {
    method: 'POST',
    headers,
    body: fd,
  });
  const text = await resp.text();
  let data = null;
  try {
    data = JSON.parse(text);
  } catch (_) {
    data = { raw: text };
  }

  if (!resp.ok) {
    const msg = (data && (data.detail || data.error || data.raw)) || `HTTP ${resp.status}`;
    throw new Error(msg);
  }

  document.getElementById('createResult').textContent = formatOverrideCreateResult(data);
  await refreshAll();
}

async function submitDeviceConfig(ev) {
  ev.preventDefault();
  const config = collectDeviceConfigPatch();

  if (Object.keys(config).length === 0) {
    throw new Error('请至少填写一项配置');
  }

  const payload = {
    device_id: document.getElementById('configDeviceId').value || '*',
    note: document.getElementById('configNote').value || '',
    config,
  };

  const data = await fetchJson('/api/v1/device-config', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });

  document.getElementById('configResult').textContent = JSON.stringify(data, null, 2);
  clearConfigPatchInputs();
  await refreshAll();
}

async function refreshAll() {
  await loadHealth();
  await loadDevices();
  await loadOverrides();
  await loadDeviceTokens();
  await loadPublishHistory();
  await loadCurrentPreview();
  await loadDeviceConfigs();
  document.getElementById('lastRefresh').textContent = new Date().toLocaleTimeString();
}

document.getElementById('overrideForm').addEventListener('submit', async (ev) => {
  try {
    await submitOverride(ev);
  } catch (err) {
    document.getElementById('createResult').textContent = `提交失败: ${err.message}`;
  }
});

document.getElementById('deviceConfigForm').addEventListener('submit', async (ev) => {
  try {
    await submitDeviceConfig(ev);
  } catch (err) {
    document.getElementById('configResult').textContent = `发布失败: ${err.message}`;
  }
});

document.getElementById('refreshBtn').addEventListener('click', async () => {
  persistConsoleToken();
  try {
    await refreshAll();
  } catch (err) {
    alert(`刷新失败: ${err.message}`);
  }
});

document.getElementById('previewBtn').addEventListener('click', async () => {
  try {
    await loadCurrentPreview();
  } catch (err) {
    document.getElementById('previewMeta').textContent = `预览失败: ${err.message}`;
  }
});

async function loadPowerSamplesSafe() {
  try {
    await loadPowerSamples();
  } catch (err) {
    setText('powerSummary', `电池曲线加载失败: ${err.message}`);
    setText('powerHint', '');
  }
}

document.getElementById('powerRefreshBtn').addEventListener('click', async () => {
  await loadPowerSamplesSafe();
});

document.getElementById('powerDeviceId').addEventListener('change', async () => {
  await loadPowerSamplesSafe();
});

document.getElementById('powerDays').addEventListener('change', async () => {
  await loadPowerSamplesSafe();
});

document.getElementById('powerLowThreshold').addEventListener('change', async () => {
  await loadPowerSamplesSafe();
});

window.addEventListener('resize', () => {
  if (powerResizeTimer) {
    clearTimeout(powerResizeTimer);
  }
  powerResizeTimer = setTimeout(() => {
    const canvas = document.getElementById('powerCanvas');
    if (!canvas || !powerChartCache) return;
    drawPowerChart(canvas, powerChartCache.items, {
      fromEpoch: powerChartCache.from_epoch,
      toEpoch: powerChartCache.to_epoch,
    });
  }, 120);
});

document.getElementById('deviceId').addEventListener('change', async () => {
  try {
    await loadPublishHistory();
    await loadCurrentPreview();
  } catch (err) {
    document.getElementById('publishHistoryHint').textContent = `加载历史失败: ${err.message}`;
    document.getElementById('previewMeta').textContent = `预览失败: ${err.message}`;
  }
});

document.getElementById('configDeviceId').addEventListener('change', async () => {
  try {
    clearConfigPatchInputs();
    updateConfigHints();
    await loadDeviceConfigs();
  } catch (err) {
    document.getElementById('configHistoryHint').textContent = `加载配置历史失败: ${err.message}`;
  }
});

document.getElementById('fillCurrentDailyBtn').addEventListener('click', () => {
  const url = `${window.location.origin}/public/daily.bmp`;
  document.getElementById('cfgImageUrlTemplate').value = url;
});

document.getElementById('fillPublicDailyBtn').addEventListener('click', () => {
  document.getElementById('cfgImageUrlTemplate').value = PUBLIC_DAILY_EXAMPLE_URL;
});

document.getElementById('token').addEventListener('input', () => {
  persistConsoleToken();
});
document.getElementById('token').addEventListener('change', () => {
  persistConsoleToken();
});
document.getElementById('token').addEventListener('blur', () => {
  persistConsoleToken();
});

loadStoredConsoleToken();
persistConsoleToken();

setInterval(() => {
  refreshAll().catch(() => {});
}, 30000);

refreshAll().catch((err) => {
  document.getElementById('createResult').textContent = `初始化失败: ${err.message}`;
});
