function authHeaders() {
  const token = document.getElementById('token').value.trim();
  if (!token) {
    return {};
  }
  return { 'X-PhotoFrame-Token': token };
}

const previewBlobUrls = { left: null, right: null };
let deviceMap = new Map();
let powerChartCache = null;
let powerResizeTimer = null;
let currentDailyDitherAlgorithm = 'sierra';
let currentPaletteProfile = 'reference';

const TOKEN_STORAGE_KEY = 'photoframe.console.token';
const TOKEN_COOKIE_KEY = 'photoframe_console_token';
const DEVICE_STORAGE_KEY = 'photoframe.console.device_id';
const WORKSPACE_STORAGE_KEY = 'photoframe.console.workspace';
const POWER_DAYS_STORAGE_KEY = 'photoframe.console.power.days';
const POWER_LOW_THRESHOLD_STORAGE_KEY = 'photoframe.console.power.low_threshold';
const DEVICE_COOKIE_KEY = 'photoframe_console_device';
const WORKSPACE_COOKIE_KEY = 'photoframe_console_workspace';
const POWER_DAYS_COOKIE_KEY = 'photoframe_console_power_days';
const POWER_LOW_THRESHOLD_COOKIE_KEY = 'photoframe_console_power_low_threshold';

let storedDeviceId = '';
let powerAutoLoaded = false;
let currentWorkspace = 'overview';

const WORKSPACE_HINTS = {
  overview: '总览：设备在线状态、供电趋势、当前下发预览',
  publish: '发布：创建插播、查看插播列表与图片发布记录',
  config: '配置：设备配对审批、连接设置、参数与 Wi‑Fi 管理',
  history: '历史：发布历史与配置下发历史',
};
const DITHER_ALGORITHM_LABELS = {
  none: '保持原图',
  bayer: 'Bayer 4x4',
  'blue-noise-lab-ciede2000': 'Blue Noise + Lab CIEDE2000',
  'floyd-steinberg': 'Floyd-Steinberg',
  jarvis: 'Jarvis (JJN)',
  stucki: 'Stucki',
  'stucki-serpentine': 'Stucki Serpentine',
  burkes: 'Burkes',
  'sierra-lite': 'Sierra Lite (2-4A)',
  'lab-ciede2000': 'Lab + CIEDE2000',
  'tone-lab-ciede2000': 'Tone + Lab CIEDE2000',
  'paperwhite-lab-ciede2000': 'Paper White + Lab CIEDE2000',
  atkinson: 'Atkinson',
  sierra: 'Sierra',
};
const DAILY_DITHER_ALGORITHMS = ['bayer', 'blue-noise-lab-ciede2000', 'floyd-steinberg', 'jarvis', 'stucki', 'stucki-serpentine', 'burkes', 'sierra-lite', 'lab-ciede2000', 'tone-lab-ciede2000', 'paperwhite-lab-ciede2000', 'atkinson', 'sierra'];

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

function normalizeWorkspace(raw) {
  if (raw === 'overview' || raw === 'publish' || raw === 'config' || raw === 'history') {
    return raw;
  }
  return 'overview';
}

function applyWorkspaceFilter() {
  const cards = document.querySelectorAll('.card[data-workspace]');
  for (const card of cards) {
    const raw = card.getAttribute('data-workspace') || '';
    const spaces = raw.split(',').map((item) => item.trim()).filter((item) => item);
    const show = spaces.includes(currentWorkspace);
    card.classList.toggle('is-hidden', !show);
  }

  const tabs = document.querySelectorAll('#workspaceTabs .workspace-tab');
  for (const tab of tabs) {
    const ws = tab.getAttribute('data-workspace') || '';
    tab.classList.toggle('active', ws === currentWorkspace);
  }

  const grid = document.getElementById('workspaceGrid');
  const primary = document.getElementById('primaryStack');
  const secondary = document.getElementById('secondaryStack');
  const hasVisibleCard = (root) => {
    if (!root) return false;
    return Array.from(root.querySelectorAll('.card[data-workspace]'))
      .some((card) => !card.classList.contains('is-hidden'));
  };
  const hasPrimary = hasVisibleCard(primary);
  const hasSecondary = hasVisibleCard(secondary);
  if (primary) primary.classList.toggle('is-empty', !hasPrimary);
  if (secondary) secondary.classList.toggle('is-empty', !hasSecondary);
  if (grid) {
    grid.classList.toggle('single-column', !hasPrimary || !hasSecondary);
  }

  setText('workspaceHint', WORKSPACE_HINTS[currentWorkspace] || '-');

  if (currentWorkspace === 'overview') {
    requestPowerChartRedrawWhenVisible();
  }
}

function loadStoredWorkspace() {
  let saved = '';
  try {
    saved = window.localStorage.getItem(WORKSPACE_STORAGE_KEY) || '';
  } catch (_) {
    // localStorage 受限时退化到 cookie
  }
  if (!saved) {
    saved = readCookie(WORKSPACE_COOKIE_KEY);
  }
  currentWorkspace = normalizeWorkspace(saved);
  applyWorkspaceFilter();
}

function persistWorkspace() {
  try {
    window.localStorage.setItem(WORKSPACE_STORAGE_KEY, currentWorkspace);
  } catch (_) {
    // ignore
  }
  writeCookie(WORKSPACE_COOKIE_KEY, currentWorkspace, 180 * 24 * 3600);
}

function initWorkspaceTabs() {
  const tabRoot = document.getElementById('workspaceTabs');
  if (!tabRoot) return;
  for (const btn of tabRoot.querySelectorAll('.workspace-tab')) {
    btn.addEventListener('click', () => {
      currentWorkspace = normalizeWorkspace(btn.getAttribute('data-workspace') || 'overview');
      persistWorkspace();
      applyWorkspaceFilter();
    });
  }
  loadStoredWorkspace();
}

function loadStoredPowerPrefs() {
  const deviceSelect = document.getElementById('deviceId');
  const daysInput = document.getElementById('powerDays');
  const thresholdInput = document.getElementById('powerLowThreshold');

  let savedDevice = '';
  let savedDays = '';
  let savedThreshold = '';
  try {
    savedDevice = window.localStorage.getItem(DEVICE_STORAGE_KEY) || '';
    savedDays = window.localStorage.getItem(POWER_DAYS_STORAGE_KEY) || '';
    savedThreshold = window.localStorage.getItem(POWER_LOW_THRESHOLD_STORAGE_KEY) || '';
  } catch (_) {
    // localStorage 受限时退化到 cookie
  }

  if (!savedDevice) {
    savedDevice = readCookie(DEVICE_COOKIE_KEY);
  }
  if (!savedDays) {
    savedDays = readCookie(POWER_DAYS_COOKIE_KEY);
  }
  if (!savedThreshold) {
    savedThreshold = readCookie(POWER_LOW_THRESHOLD_COOKIE_KEY);
  }

  storedDeviceId = (savedDevice || '').trim();
  if (storedDeviceId === '*') {
    storedDeviceId = '';
  }
  if (deviceSelect && storedDeviceId) {
    deviceSelect.value = storedDeviceId;
  }

  if (daysInput) {
    const daysNum = Number(savedDays);
    if (Number.isFinite(daysNum) && daysNum >= 1 && daysNum <= 365) {
      daysInput.value = String(Math.floor(daysNum));
    }
  }

  if (thresholdInput) {
    const thresholdNum = Number(savedThreshold);
    if (Number.isFinite(thresholdNum) && thresholdNum >= 0 && thresholdNum <= 100) {
      thresholdInput.value = String(Math.floor(thresholdNum));
    }
  }
}

function persistPowerPrefs() {
  const deviceId = (document.getElementById('deviceId')?.value || '').trim();
  const daysRaw = Number(document.getElementById('powerDays')?.value || '');
  const days = Number.isFinite(daysRaw) ? Math.max(1, Math.min(365, Math.floor(daysRaw))) : null;
  const thresholdRaw = Number(document.getElementById('powerLowThreshold')?.value || '');
  const threshold = Number.isFinite(thresholdRaw)
    ? Math.max(0, Math.min(100, Math.floor(thresholdRaw)))
    : null;

  try {
    if (deviceId && deviceId !== '*') {
      window.localStorage.setItem(DEVICE_STORAGE_KEY, deviceId);
    } else {
      window.localStorage.removeItem(DEVICE_STORAGE_KEY);
    }
    if (days != null) {
      window.localStorage.setItem(POWER_DAYS_STORAGE_KEY, String(days));
    } else {
      window.localStorage.removeItem(POWER_DAYS_STORAGE_KEY);
    }
    if (threshold != null) {
      window.localStorage.setItem(POWER_LOW_THRESHOLD_STORAGE_KEY, String(threshold));
    } else {
      window.localStorage.removeItem(POWER_LOW_THRESHOLD_STORAGE_KEY);
    }
  } catch (_) {
    // ignore
  }

  if (deviceId && deviceId !== '*') {
    writeCookie(DEVICE_COOKIE_KEY, deviceId, 180 * 24 * 3600);
  } else {
    writeCookie(DEVICE_COOKIE_KEY, '', 0);
  }
  if (days != null) {
    writeCookie(POWER_DAYS_COOKIE_KEY, String(days), 180 * 24 * 3600);
  }
  if (threshold != null) {
    writeCookie(POWER_LOW_THRESHOLD_COOKIE_KEY, String(threshold), 180 * 24 * 3600);
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
    `服务端 Dither: ${ditherAlgorithmLabel(data.dither_algorithm)}`,
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

function ditherAlgorithmLabel(value) {
  return optionLabel(value, DITHER_ALGORITHM_LABELS);
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

  const lastEpoch = Number(device?.last_power_sample_epoch);
  const lastPercent = Number(device?.last_power_battery_percent);
  const lastMv = Number(device?.last_power_battery_mv);

  let fallback = '';
  if (Number.isFinite(lastPercent) && lastPercent >= 0) {
    if (Number.isFinite(lastMv) && lastMv > 0) {
      fallback = `${lastPercent}% / ${lastMv}mV`;
    } else {
      fallback = `${lastPercent}%`;
    }
  } else if (Number.isFinite(lastMv) && lastMv > 0) {
    fallback = `${lastMv}mV`;
  }

  if (fallback) {
    if (Number.isFinite(lastEpoch) && lastEpoch > 0) {
      return `${fallback} (上次 ${fmtEpochCompact(lastEpoch)})`;
    }
    return fallback;
  }

  return '-';
}

function powerSourceText(device) {
  const vbus = Number(device?.vbus_good);
  const charging = Number(device?.charging);

  const vbusText = vbus === 1 ? 'USB' : vbus === 0 ? 'Battery' : '-';
  const chargeText = charging === 1 ? '充电中' : charging === 0 ? '未充电' : '-';

  if (vbusText === '-' && chargeText === '-') {
    const lastVbus = Number(device?.last_power_vbus_good);
    const lastCharging = Number(device?.last_power_charging);
    const lastEpoch = Number(device?.last_power_sample_epoch);

    const lastVbusText = lastVbus === 1 ? 'USB' : lastVbus === 0 ? 'Battery' : '-';
    const lastChargeText = lastCharging === 1 ? '充电中' : lastCharging === 0 ? '未充电' : '-';
    if (lastVbusText === '-' && lastChargeText === '-') {
      return '-';
    }
    const text = `${lastVbusText} / ${lastChargeText}`;
    if (Number.isFinite(lastEpoch) && lastEpoch > 0) {
      return `${text} (上次 ${fmtEpochCompact(lastEpoch)})`;
    }
    return text;
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

function redrawPowerChartIfReady() {
  if (!powerChartCache) return false;
  const canvas = document.getElementById('powerCanvas');
  if (!canvas) return false;
  const rect = canvas.getBoundingClientRect();
  if (!Number.isFinite(rect.width) || !Number.isFinite(rect.height)) return false;
  if (rect.width < 24 || rect.height < 24) return false;
  drawPowerChart(canvas, powerChartCache.items, {
    fromEpoch: powerChartCache.from_epoch,
    toEpoch: powerChartCache.to_epoch,
  });
  return true;
}

function requestPowerChartRedrawWhenVisible(maxFrames = 12) {
  let frameCount = 0;
  const tick = () => {
    if (redrawPowerChartIfReady()) return;
    frameCount += 1;
    if (frameCount < maxFrames) {
      requestAnimationFrame(tick);
    }
  };
  requestAnimationFrame(tick);
}

function powerVbusText(vbus) {
  if (vbus === 1) return 'USB';
  if (vbus === 0) return 'Battery';
  return '-';
}

function powerChargingText(charging) {
  if (charging === 1) return '充电中';
  if (charging === 0) return '未充电';
  return '-';
}

function ensurePowerTooltip(canvas) {
  const host = canvas?.parentElement;
  if (!host) return null;
  let tip = host.querySelector('.power-tooltip');
  if (!tip) {
    tip = document.createElement('div');
    tip.className = 'power-tooltip';
    host.appendChild(tip);
  }
  return tip;
}

function hidePowerTooltip(canvas) {
  const tip = ensurePowerTooltip(canvas);
  if (tip) {
    tip.style.display = 'none';
  }
}

function bindPowerChartHover(canvas) {
  if (!canvas || canvas.__powerHoverBound) return;
  canvas.__powerHoverBound = true;

  const onMove = (event) => {
    const state = canvas.__powerHoverState;
    if (!state || !Array.isArray(state.points) || state.points.length === 0) {
      hidePowerTooltip(canvas);
      return;
    }

    const rect = canvas.getBoundingClientRect();
    const mx = event.clientX - rect.left;
    const my = event.clientY - rect.top;
    const tip = ensurePowerTooltip(canvas);
    if (!tip) return;

    let best = null;
    let bestDist = Number.POSITIVE_INFINITY;
    for (const point of state.points) {
      const candidates = [];
      if (point.percentY != null) {
        candidates.push({ key: 'percent', x: point.x, y: point.percentY });
      }
      if (point.mvY != null) {
        candidates.push({ key: 'mv', x: point.x, y: point.mvY });
      }
      for (const candidate of candidates) {
        const dx = mx - candidate.x;
        const dy = my - candidate.y;
        const dist = dx * dx + dy * dy;
        if (dist < bestDist) {
          bestDist = dist;
          best = point;
        }
      }
    }

    // 命中半径约 16px；避免鼠标在图上任意位置都弹窗。
    if (!best || bestDist > 16 * 16) {
      tip.style.display = 'none';
      return;
    }

    const lines = [
      `<div>${fmtEpoch(best.sampleEpoch)}</div>`,
      `<div>电量: ${best.batteryPercent != null ? `${best.batteryPercent}%` : '-'}</div>`,
      `<div>电压: ${best.batteryMv != null ? `${best.batteryMv}mV` : '-'}</div>`,
      `<div>供电: ${powerVbusText(best.vbus)}</div>`,
      `<div>充电: ${powerChargingText(best.charging)}</div>`,
    ];
    tip.innerHTML = lines.join('');
    tip.style.display = 'block';

    const pad = 12;
    const maxLeft = Math.max(0, rect.width - tip.offsetWidth - 2);
    const maxTop = Math.max(0, rect.height - tip.offsetHeight - 2);
    let left = mx + pad;
    let top = my + pad;
    if (left > maxLeft) left = Math.max(0, mx - tip.offsetWidth - pad);
    if (top > maxTop) top = Math.max(0, my - tip.offsetHeight - pad);
    tip.style.left = `${left}px`;
    tip.style.top = `${top}px`;
  };

  canvas.addEventListener('mousemove', onMove);
  canvas.addEventListener('mouseleave', () => hidePowerTooltip(canvas));
}

function drawPowerChart(canvas, items, opts) {
  const { ctx, width, height } = prepareHiDPICanvas(canvas);
  ctx.clearRect(0, 0, width, height);
  bindPowerChartHover(canvas);

  const pad = { l: 54, r: 68, t: 26, b: 30 };
  const x0 = pad.l;
  const x1 = width - pad.r;
  const y0 = pad.t;
  const y1 = height - pad.b;

  ctx.fillStyle = '#ffffff';
  ctx.fillRect(0, 0, width, height);

  if (!items || items.length === 0) {
    canvas.__powerHoverState = { points: [] };
    hidePowerTooltip(canvas);
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

  const hoverPoints = items.map((it) => {
    const sampleEpoch = Number(it?.sample_epoch);
    const batteryPercentRaw = Number(it?.battery_percent);
    const batteryMvRaw = Number(it?.battery_mv);
    const vbus = normalizeBinaryFlag(it?.vbus_good);
    const charging = normalizeBinaryFlag(it?.charging);

    const batteryPercent = Number.isFinite(batteryPercentRaw) && batteryPercentRaw >= 0
      ? Math.max(0, Math.min(100, Math.round(batteryPercentRaw)))
      : null;
    const batteryMv = Number.isFinite(batteryMvRaw) && batteryMvRaw > 0
      ? Math.round(batteryMvRaw)
      : null;

    return {
      sampleEpoch,
      x: Number.isFinite(sampleEpoch) ? xScale(sampleEpoch) : null,
      percentY: batteryPercent != null ? yPercent(batteryPercent) : null,
      mvY: (batteryMv != null && mvMin != null && mvMax != null) ? yMv(batteryMv) : null,
      batteryPercent,
      batteryMv,
      vbus,
      charging,
    };
  }).filter((it) => it.x != null);
  canvas.__powerHoverState = { points: hoverPoints };

  function drawSeriesDots(getValue, isValueValid, toY, fillStyle, strokeStyle) {
    // “只有一个点”时折线几乎不可见，用散点让“是否上报”一目了然。
    // 性能：默认最多 2 万点；用 fillRect 比 arc 更省。
    const dot = 5;
    const dotHalf = dot / 2;

    ctx.fillStyle = fillStyle;
    for (const it of items) {
      const ts = Number(it?.sample_epoch);
      const v = Number(getValue(it));
      if (!Number.isFinite(ts) || !Number.isFinite(v) || !isValueValid(v)) continue;
      const x = xScale(ts);
      const y = toY(v);
      ctx.fillRect(x - dotHalf, y - dotHalf, dot, dot);
    }

    // 高亮最后一个有效点，便于快速确认“最近一次上报”。
    // 用户反馈：不需要把最后一个点画得更大，因此这里采用“同尺寸 + 描边”。
    const hi = dot;
    const hiHalf = dotHalf;
    for (let i = items.length - 1; i >= 0; --i) {
      const it = items[i];
      const ts = Number(it?.sample_epoch);
      const v = Number(getValue(it));
      if (!Number.isFinite(ts) || !Number.isFinite(v) || !isValueValid(v)) continue;
      const x = xScale(ts);
      const y = toY(v);
      ctx.fillRect(x - hiHalf, y - hiHalf, hi, hi);
      if (strokeStyle) {
        ctx.strokeStyle = strokeStyle;
        ctx.lineWidth = 2;
        ctx.strokeRect(x - hiHalf, y - hiHalf, hi, hi);
      }
      break;
    }
  }

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
  ctx.strokeStyle = 'rgba(53, 88, 229, 0.38)';
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

  drawSeriesDots(
    (it) => Number(it?.battery_percent),
    (p) => p >= 0,
    (p) => yPercent(Math.max(0, Math.min(100, p))),
    'rgba(53, 88, 229, 0.90)',
    'rgba(15, 23, 42, 0.30)'
  );

  // 线条：battery_mv
  if (mvMin != null && mvMax != null) {
    ctx.strokeStyle = 'rgba(212, 133, 28, 0.38)';
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

    drawSeriesDots(
      (it) => Number(it?.battery_mv),
      (mv) => mv > 0,
      (mv) => yMv(mv),
      'rgba(212, 133, 28, 0.85)',
      'rgba(15, 23, 42, 0.30)'
    );
  }
}

async function loadPowerSamples() {
  const deviceId = (document.getElementById('deviceId')?.value || '').trim();
  const daysRaw = Number(document.getElementById('powerDays')?.value || 3);
  const thresholdRaw = Number(document.getElementById('powerLowThreshold')?.value || 10);

  if (!deviceId || deviceId === '*') {
    setText('powerSummary', '请在“创建插播”的目标设备中选择具体设备后再刷新曲线。');
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
    if (!redrawPowerChartIfReady()) {
      // 当前工作区可能隐藏了画布，切回可见区后自动按真实尺寸重绘。
      requestPowerChartRedrawWhenVisible();
    }
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

function selectedDailyDitherAlgorithm() {
  const raw = document.getElementById('dailyDitherAlgorithm')?.value || currentDailyDitherAlgorithm;
  if (DAILY_DITHER_ALGORITHMS.includes(raw)) {
    return raw;
  }
  return currentDailyDitherAlgorithm;
}

function selectedPaletteProfile() {
  const raw = document.getElementById('paletteProfile')?.value || currentPaletteProfile;
  return raw || currentPaletteProfile;
}

function preferredCompareRightAlgorithm(leftAlgorithm) {
  if (leftAlgorithm !== 'lab-ciede2000' && DAILY_DITHER_ALGORITHMS.includes('lab-ciede2000')) {
    return 'lab-ciede2000';
  }
  return DAILY_DITHER_ALGORITHMS.find((item) => item !== leftAlgorithm) || leftAlgorithm;
}

function selectedCompareAlgorithm(elementId, fallback) {
  const raw = document.getElementById(elementId)?.value || fallback;
  if (DAILY_DITHER_ALGORITHMS.includes(raw)) {
    return raw;
  }
  return fallback;
}

function syncCompareSelectors() {
  const leftSelect = document.getElementById('compareLeftAlgorithm');
  const rightSelect = document.getElementById('compareRightAlgorithm');
  if (!leftSelect || !rightSelect) return;
  if (!DAILY_DITHER_ALGORITHMS.includes(leftSelect.value)) {
    leftSelect.value = currentDailyDitherAlgorithm;
  }
  if (!DAILY_DITHER_ALGORITHMS.includes(rightSelect.value) || rightSelect.value === leftSelect.value) {
    rightSelect.value = preferredCompareRightAlgorithm(leftSelect.value);
  }
}

function updateCompareSliderUi() {
  const slider = document.getElementById('compareSlider');
  const overlay = document.getElementById('compareOverlay');
  const divider = document.getElementById('compareDivider');
  if (!slider || !overlay || !divider) return;
  const value = Math.max(0, Math.min(100, Number(slider.value || 50)));
  overlay.style.width = `${value}%`;
  divider.style.left = `${value}%`;
}

function updateDailyDitherHint(savedAlgorithm = currentDailyDitherAlgorithm) {
  const selected = selectedDailyDitherAlgorithm();
  const savedText = ditherAlgorithmLabel(savedAlgorithm || currentDailyDitherAlgorithm);
  const previewText = ditherAlgorithmLabel(selected);
  document.getElementById('dailyDitherHint').textContent = `Daily Dither: 当前保存 ${savedText} · 预览使用 ${previewText} · Palette ${selectedPaletteProfile()}`;
}

async function loadDailyRenderConfig() {
  const data = await fetchJson('/api/v1/daily-render-config');
  currentDailyDitherAlgorithm = data.daily_dither_algorithm || currentDailyDitherAlgorithm;
  currentPaletteProfile = data.palette_profile || currentPaletteProfile;
  const select = document.getElementById('dailyDitherAlgorithm');
  if (select) {
    select.value = currentDailyDitherAlgorithm;
  }
  const paletteSelect = document.getElementById('paletteProfile');
  if (paletteSelect) {
    const profiles = Array.isArray(data.palette_profiles) ? data.palette_profiles : [];
    paletteSelect.innerHTML = profiles.map((item) => `<option value="${escapeHtml(item.key)}">${escapeHtml(item.label)}</option>`).join('')
      || '<option value="reference">Reference Palette</option>';
    paletteSelect.value = currentPaletteProfile;
  }
  const leftSelect = document.getElementById('compareLeftAlgorithm');
  if (leftSelect) {
    leftSelect.value = currentDailyDitherAlgorithm;
  }
  syncCompareSelectors();
  updateDailyDitherHint(currentDailyDitherAlgorithm);
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

function renderFetchStatus(device) {
  const status = Number(device?.last_http_status || 0);
  const ok = device?.fetch_ok === true || Number(device?.fetch_ok) === 1;

  if (ok) {
    const label = status > 0 ? `ok / ${status}` : 'ok';
    return `<span class="tag active">${escapeHtml(label)}</span>`;
  }

  if (status > 0) {
    return `<span class="tag expired">fail / ${escapeHtml(String(status))}</span>`;
  }

  if (device?.last_error) {
    return '<span class="tag expired">fail</span>';
  }

  return '<span class="tag">-</span>';
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

function getReportedWifiProfiles(device) {
  const reported = normalizeReported(device);
  if (!Array.isArray(reported.wifi_profiles)) {
    return [];
  }
  return reported.wifi_profiles
    .map((item) => {
      if (typeof item === 'string') {
        const ssid = item.trim();
        return ssid ? { ssid, password_set: null } : null;
      }
      if (!item || typeof item !== 'object') {
        return null;
      }
      const ssid = typeof item.ssid === 'string' ? item.ssid.trim() : '';
      if (!ssid) {
        return null;
      }
      return {
        ssid,
        password_set: typeof item.password_set === 'boolean' ? item.password_set : null,
      };
    })
    .filter((item) => item);
}

function formatReportedWifiProfiles(device, emptyText = '-') {
  const profiles = getReportedWifiProfiles(device);
  if (profiles.length === 0) {
    return emptyText;
  }
  return profiles
    .map((item) => (item.password_set === false ? `${item.ssid}(open)` : item.ssid))
    .join(', ');
}

function updateDeviceContextBanner() {
  const selectedDevice = document.getElementById('deviceId')?.value || '*';
  const titleEl = document.getElementById('deviceContextTitle');
  const metaEl = document.getElementById('deviceContextMeta');
  if (!titleEl || !metaEl) return;

  if (selectedDevice === '*') {
    titleEl.textContent = '当前设备：全部设备 (*)';
    metaEl.textContent = `将影响全部设备（当前在线 ${deviceMap.size} 台）`;
    return;
  }

  const device = deviceMap.get(selectedDevice);
  if (!device) {
    titleEl.textContent = `当前设备：${selectedDevice}`;
    metaEl.textContent = '设备详情尚未加载';
    return;
  }

  const ip = device.sta_ip || '-';
  const battery = batteryStatusText(device);
  const nextWakeup = fmtEpoch(device.next_wakeup_epoch);
  const wifi = formatReportedWifiProfiles(device, '-');
  titleEl.textContent = `当前设备：${selectedDevice}`;
  metaEl.textContent = `IP ${ip} · 电量 ${battery} · 下次唤醒 ${nextWakeup} · Wi‑Fi ${wifi}`;
}

function fillWifiEditorFromDevice(device) {
  const profiles = getReportedWifiProfiles(device).slice(0, 3);
  for (let i = 1; i <= 3; i++) {
    const profile = profiles[i - 1] || null;
    const ssidInput = document.getElementById(`cfgWifiSsid${i}`);
    const pwdInput = document.getElementById(`cfgWifiPwd${i}`);
    if (!ssidInput || !pwdInput) continue;
    ssidInput.value = profile ? profile.ssid : '';
    pwdInput.value = '';
    if (!profile) {
      pwdInput.placeholder = '留空则保持该 SSID 现有密码';
    } else if (profile.password_set === false) {
      pwdInput.placeholder = '当前为开放网络，可填入新密码';
    } else {
      pwdInput.placeholder = '留空则保持该 SSID 现有密码';
    }
  }
  const replaceCheckbox = document.getElementById('cfgWifiReplace');
  if (replaceCheckbox) {
    replaceCheckbox.checked = false;
  }
}

function updateConfigHints() {
  const selectedDevice = document.getElementById('deviceId').value || '*';
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
    '0': '直接转换',
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

  let wifiText = '-';
  if (selectedDevice === '*') {
    if (deviceMap.size > 1) {
      wifiText = '请先选择具体设备（当前为全部设备）';
    } else if (deviceMap.size === 1) {
      const onlyDevice = Array.from(deviceMap.values())[0];
      wifiText = formatReportedWifiProfiles(onlyDevice, '-');
    }
  } else {
    wifiText = formatReportedWifiProfiles(device, '-');
  }
  setText('cfgHintWifiProfiles', `设备已记住: ${wifiText}`);
}

function clearConfigPatchInputs() {
  const textIds = [
    'cfgTimezone',
    'cfgOrchBaseUrl',
    'cfgImageUrlTemplate',
    'cfgOrchToken',
    'cfgPhotoToken',
    'cfgWifiSsid1',
    'cfgWifiPwd1',
    'cfgWifiSsid2',
    'cfgWifiPwd2',
    'cfgWifiSsid3',
    'cfgWifiPwd3',
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

  const wifiReplace = document.getElementById('cfgWifiReplace');
  if (wifiReplace) {
    wifiReplace.checked = false;
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

  const wifiProfiles = [];
  for (let i = 1; i <= 3; i++) {
    const ssid = (document.getElementById(`cfgWifiSsid${i}`)?.value || '').trim();
    const password = document.getElementById(`cfgWifiPwd${i}`)?.value ?? '';
    if (!ssid && !password) continue;
    if (!ssid) {
      throw new Error(`Wi-Fi SSID ${i} 不能为空`);
    }
    const item = { ssid };
    if (password !== '') {
      item.password = password;
    }
    wifiProfiles.push(item);
  }
  if (document.getElementById('cfgWifiReplace')?.checked) {
    patch.wifi_profiles = wifiProfiles;
  }

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
  setText('appVersion', version);
}

async function loadDevices() {
  const data = await fetchJson('/api/v1/devices');
  const body = document.getElementById('devicesBody');
  const deviceSelect = document.getElementById('deviceId');
  const selectedBefore = deviceSelect.value;

  body.innerHTML = '';
  deviceSelect.innerHTML = '<option value="*">全部设备 (*)</option>';

  const devices = data.devices || [];
  deviceMap = new Map();
  document.getElementById('serverNow').textContent = fmtEpoch(data.now_epoch);

  for (const d of devices) {
    deviceMap.set(d.device_id, d);

    const tr = document.createElement('tr');
    const cfgVersion = `${d.config_target_version || 0}/${d.config_seen_version || 0}/${d.config_applied_version || 0}`;
    const cfgQuery = fmtEpoch(d.config_last_query_epoch);
    const wifiSummary = formatReportedWifiProfiles(d, '-');

    tr.innerHTML = `
      <td><span class="tag">${escapeHtml(d.device_id)}</span></td>
      <td>${fmtEpoch(d.last_checkin_epoch)}</td>
      <td>${fmtEpoch(d.next_wakeup_epoch)}</td>
      <td>${fmtDuration(d.eta_seconds)}</td>
      <td>${fmtDuration(d.poll_interval_seconds)}</td>
      <td>${escapeHtml(d.failure_count)}</td>
      <td>${escapeHtml(d.image_source || 'daily')}</td>
      <td>${renderFetchStatus(d)}</td>
      <td>${escapeHtml(batteryStatusText(d))}</td>
      <td>${escapeHtml(powerSourceText(d))}</td>
      <td title="${escapeHtml(wifiSummary)}">${escapeHtml(shorten(wifiSummary, 48))}</td>
      <td>${escapeHtml(d.sta_ip || '-')}</td>
      <td>${escapeHtml(cfgVersion)}</td>
      <td>${cfgQuery}</td>
      <td>${renderConfigApplyStatus(d)}</td>
      <td>${escapeHtml(shorten(d.last_error || '', 88))}</td>
    `;
    body.appendChild(tr);

    appendDeviceOption(deviceSelect, d.device_id);
  }

  if (selectedBefore &&
      selectedBefore !== '*' &&
      [...deviceSelect.options].some((o) => o.value === selectedBefore)) {
    deviceSelect.value = selectedBefore;
  } else if (storedDeviceId && [...deviceSelect.options].some((o) => o.value === storedDeviceId)) {
    deviceSelect.value = storedDeviceId;
  } else if (deviceSelect.options.length > 1) {
    // 默认选第一台设备，减少“全部设备”误操作与无效空状态。
    deviceSelect.value = deviceSelect.options[1].value;
  } else if ([...deviceSelect.options].some((o) => o.value === selectedBefore)) {
    // 没有具体设备时，再保留“全部设备”。
    deviceSelect.value = selectedBefore;
  }
  persistPowerPrefs();

  fillWifiEditorFromDevice(deviceMap.get(deviceSelect.value || '*'));
  updateConfigHints();
  updateDeviceContextBanner();
  if (!powerAutoLoaded) {
    powerAutoLoaded = true;
    loadPowerSamplesSafe();
  }
}

async function loadOverrides() {
  const data = await fetchJson('/api/v1/overrides');
  const body = document.getElementById('overridesBody');
  body.innerHTML = '';

  const overrides = data.overrides || [];

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
      <td>${escapeHtml(ditherAlgorithmLabel(item.dither_algorithm))}</td>
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
  const ditherText = item.dither_algorithm
    ? ditherAlgorithmLabel(item.dither_algorithm)
    : '-';

  return `
    <article class="release-item">
      <div class="release-head">
        <p class="release-title">${sourceTag} <span class="tag">${escapeHtml(item.device_id)}</span></p>
        <span class="release-date">${fmtEpoch(item.issued_epoch)}</span>
      </div>
      <p class="release-summary">${shortUrl}</p>
      <ul>
        <li>override_id: ${escapeHtml(overrideText)}</li>
        <li>dither: ${escapeHtml(ditherText)}</li>
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
  const leftImg = document.getElementById('compareLeftPreview');
  const rightImg = document.getElementById('compareRightPreview');
  const leftLabel = document.getElementById('compareLeftLabel');
  const rightLabel = document.getElementById('compareRightLabel');

  syncCompareSelectors();
  const leftAlgorithm = selectedCompareAlgorithm('compareLeftAlgorithm', selectedDailyDitherAlgorithm());
  const rightAlgorithm = selectedCompareAlgorithm('compareRightAlgorithm', preferredCompareRightAlgorithm(leftAlgorithm));

  async function fetchPreview(algorithm) {
    const headers = authHeaders();
    const resp = await fetch(
      `/api/v1/preview/current.bmp?device_id=${encodeURIComponent(selectedDevice)}&daily_dither_algorithm=${encodeURIComponent(algorithm)}&palette_profile=${encodeURIComponent(selectedPaletteProfile())}`,
      {
        headers,
      },
    );

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

    return {
      blob: await resp.blob(),
      source: resp.headers.get('X-PhotoFrame-Source') || 'daily',
      target: resp.headers.get('X-PhotoFrame-Device') || selectedDevice,
      dither: resp.headers.get('X-PhotoFrame-Dither') || algorithm,
    };
  }

  const leftPromise = fetchPreview(leftAlgorithm);
  const rightPromise = leftAlgorithm === rightAlgorithm ? leftPromise : fetchPreview(rightAlgorithm);
  const [leftPreview, rightPreview] = await Promise.all([leftPromise, rightPromise]);

  if (previewBlobUrls.left) {
    URL.revokeObjectURL(previewBlobUrls.left);
    previewBlobUrls.left = null;
  }
  if (previewBlobUrls.right && previewBlobUrls.right !== previewBlobUrls.left) {
    URL.revokeObjectURL(previewBlobUrls.right);
    previewBlobUrls.right = null;
  }

  previewBlobUrls.left = URL.createObjectURL(leftPreview.blob);
  previewBlobUrls.right = leftAlgorithm === rightAlgorithm
    ? previewBlobUrls.left
    : URL.createObjectURL(rightPreview.blob);
  leftImg.src = previewBlobUrls.left;
  rightImg.src = previewBlobUrls.right;
  leftLabel.textContent = ditherAlgorithmLabel(leftPreview.dither || leftAlgorithm);
  rightLabel.textContent = ditherAlgorithmLabel(rightPreview.dither || rightAlgorithm);

  meta.textContent = `设备 ${leftPreview.target} · 当前来源 ${leftPreview.source} · Palette ${selectedPaletteProfile()} · 左 ${ditherAlgorithmLabel(leftAlgorithm)} / 右 ${ditherAlgorithmLabel(rightAlgorithm)} · ${fmtEpoch(Math.floor(Date.now() / 1000))}`;
  updateCompareSliderUi();
  updateDailyDitherHint(currentDailyDitherAlgorithm);
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
  const selectedDevice = document.getElementById('deviceId').value || '*';
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
  fd.append('dither_algorithm', document.getElementById('overrideDitherAlgorithm').value || 'none');

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
    device_id: document.getElementById('deviceId').value || '*',
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
  await loadDailyRenderConfig();
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

document.getElementById('dailyDitherAlgorithm').addEventListener('change', async () => {
  document.getElementById('compareLeftAlgorithm').value = selectedDailyDitherAlgorithm();
  syncCompareSelectors();
  updateDailyDitherHint(currentDailyDitherAlgorithm);
  try {
    await loadCurrentPreview();
  } catch (err) {
    document.getElementById('previewMeta').textContent = `预览失败: ${err.message}`;
  }
});

document.getElementById('paletteProfile').addEventListener('change', async () => {
  currentPaletteProfile = selectedPaletteProfile();
  try {
    await loadCurrentPreview();
  } catch (err) {
    document.getElementById('previewMeta').textContent = `预览失败: ${err.message}`;
  }
});

document.getElementById('saveDailyDitherBtn').addEventListener('click', async () => {
  try {
    const data = await fetchJson('/api/v1/daily-render-config', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        daily_dither_algorithm: selectedDailyDitherAlgorithm(),
        palette_profile: selectedPaletteProfile(),
      }),
    });
    currentDailyDitherAlgorithm = data.daily_dither_algorithm || currentDailyDitherAlgorithm;
    currentPaletteProfile = data.palette_profile || currentPaletteProfile;
    document.getElementById('dailyDitherAlgorithm').value = currentDailyDitherAlgorithm;
    document.getElementById('paletteProfile').value = currentPaletteProfile;
    document.getElementById('compareLeftAlgorithm').value = currentDailyDitherAlgorithm;
    syncCompareSelectors();
    updateDailyDitherHint(currentDailyDitherAlgorithm);
    await loadCurrentPreview();
  } catch (err) {
    document.getElementById('dailyDitherHint').textContent = `Daily Dither 保存失败: ${err.message}`;
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
  persistPowerPrefs();
  await loadPowerSamplesSafe();
});

document.getElementById('powerDays').addEventListener('change', async () => {
  persistPowerPrefs();
  await loadPowerSamplesSafe();
});

document.getElementById('powerDays').addEventListener('input', () => {
  // 仅持久化（不触发请求），避免用户输入过程中频繁刷新。
  persistPowerPrefs();
});

document.getElementById('powerLowThreshold').addEventListener('input', () => {
  persistPowerPrefs();
});

document.getElementById('powerLowThreshold').addEventListener('change', async () => {
  persistPowerPrefs();
  await loadPowerSamplesSafe();
});

window.addEventListener('resize', () => {
  if (powerResizeTimer) {
    clearTimeout(powerResizeTimer);
  }
  powerResizeTimer = setTimeout(() => {
    redrawPowerChartIfReady();
  }, 120);
});

document.getElementById('deviceId').addEventListener('change', async () => {
  try {
    persistPowerPrefs();
    updateDeviceContextBanner();
    clearConfigPatchInputs();
    fillWifiEditorFromDevice(deviceMap.get(document.getElementById('deviceId').value || '*'));
    updateConfigHints();
    await loadPublishHistory();
    await loadCurrentPreview();
    await loadDeviceConfigs();
    await loadPowerSamplesSafe();
  } catch (err) {
    document.getElementById('publishHistoryHint').textContent = `加载历史失败: ${err.message}`;
    document.getElementById('previewMeta').textContent = `预览失败: ${err.message}`;
    document.getElementById('configHistoryHint').textContent = `加载配置历史失败: ${err.message}`;
  }
});

for (let i = 1; i <= 3; i++) {
  const ssidInput = document.getElementById(`cfgWifiSsid${i}`);
  const pwdInput = document.getElementById(`cfgWifiPwd${i}`);
  const onWifiEdit = () => {
    const replaceCheckbox = document.getElementById('cfgWifiReplace');
    if (replaceCheckbox) {
      replaceCheckbox.checked = true;
    }
  };
  if (ssidInput) {
    ssidInput.addEventListener('input', onWifiEdit);
  }
  if (pwdInput) {
    pwdInput.addEventListener('input', onWifiEdit);
  }
}

document.getElementById('fillCurrentDailyBtn').addEventListener('click', () => {
  const url = `${window.location.origin}/public/daily.bmp`;
  document.getElementById('cfgImageUrlTemplate').value = url;
});

document.getElementById('fillCurrentDailyJpgBtn').addEventListener('click', () => {
  const url = `${window.location.origin}/public/daily.jpg`;
  document.getElementById('cfgImageUrlTemplate').value = url;
});

document.getElementById('fillFeaturedCurrentBtn').addEventListener('click', () => {
  document.getElementById('cfgImageUrlTemplate').value = 'http://192.168.58.113:8000/featured/current/480x800.jpg';
});

document.getElementById('fillImageByDateBtn').addEventListener('click', () => {
  document.getElementById('cfgImageUrlTemplate').value = 'http://192.168.58.113:8000/image/480x800.jpg?date=%DATE%';
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

document.getElementById('compareLeftAlgorithm').addEventListener('change', async () => {
  syncCompareSelectors();
  try {
    await loadCurrentPreview();
  } catch (err) {
    document.getElementById('previewMeta').textContent = `预览失败: ${err.message}`;
  }
});

document.getElementById('compareRightAlgorithm').addEventListener('change', async () => {
  syncCompareSelectors();
  try {
    await loadCurrentPreview();
  } catch (err) {
    document.getElementById('previewMeta').textContent = `预览失败: ${err.message}`;
  }
});

document.getElementById('compareSlider').addEventListener('input', () => {
  updateCompareSliderUi();
});

loadStoredConsoleToken();
persistConsoleToken();
loadStoredPowerPrefs();
initWorkspaceTabs();

setInterval(() => {
  refreshAll().catch(() => {});
}, 30000);

refreshAll().catch((err) => {
  document.getElementById('createResult').textContent = `初始化失败: ${err.message}`;
});
