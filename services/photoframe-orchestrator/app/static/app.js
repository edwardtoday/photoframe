function authHeaders() {
  const token = document.getElementById('token').value.trim();
  if (!token) {
    return {};
  }
  return { 'X-PhotoFrame-Token': token };
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

function shorten(text, maxLen = 72) {
  const s = String(text ?? '');
  if (s.length <= maxLen) {
    return s;
  }
  return `${s.slice(0, maxLen - 1)}…`;
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
  const selectedOverrideBefore = overrideDeviceSelect.value;
  const selectedConfigBefore = configDeviceSelect.value;

  body.innerHTML = '';
  overrideDeviceSelect.innerHTML = '<option value="*">全部设备 (*)</option>';
  configDeviceSelect.innerHTML = '<option value="*">全部设备 (*)</option>';

  const devices = data.devices || [];
  document.getElementById('deviceCount').textContent = String(devices.length);
  document.getElementById('serverNow').textContent = fmtEpoch(data.now_epoch);

  for (const d of devices) {
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
      <td>${escapeHtml(cfgVersion)}</td>
      <td>${cfgQuery}</td>
      <td>${renderConfigApplyStatus(d)}</td>
      <td>${escapeHtml(shorten(d.last_error || '', 88))}</td>
    `;
    body.appendChild(tr);

    appendDeviceOption(overrideDeviceSelect, d.device_id);
    appendDeviceOption(configDeviceSelect, d.device_id);
  }

  if ([...overrideDeviceSelect.options].some((o) => o.value === selectedOverrideBefore)) {
    overrideDeviceSelect.value = selectedOverrideBefore;
  }
  if ([...configDeviceSelect.options].some((o) => o.value === selectedConfigBefore)) {
    configDeviceSelect.value = selectedConfigBefore;
  }
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

  document.getElementById('createResult').textContent = JSON.stringify(data, null, 2);
  await refreshAll();
}

async function submitDeviceConfig(ev) {
  ev.preventDefault();
  const raw = document.getElementById('configJson').value.trim();
  let config = {};
  if (raw) {
    try {
      config = JSON.parse(raw);
    } catch (_) {
      throw new Error('配置 JSON 格式错误');
    }
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
  await refreshAll();
}

async function refreshAll() {
  await loadHealth();
  await loadDevices();
  await loadOverrides();
  await loadPublishHistory();
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
  try {
    await refreshAll();
  } catch (err) {
    alert(`刷新失败: ${err.message}`);
  }
});

document.getElementById('deviceId').addEventListener('change', async () => {
  try {
    await loadPublishHistory();
  } catch (err) {
    document.getElementById('publishHistoryHint').textContent = `加载历史失败: ${err.message}`;
  }
});

document.getElementById('configDeviceId').addEventListener('change', async () => {
  try {
    await loadDeviceConfigs();
  } catch (err) {
    document.getElementById('configHistoryHint').textContent = `加载配置历史失败: ${err.message}`;
  }
});

setInterval(() => {
  refreshAll().catch(() => {});
}, 30000);

refreshAll().catch((err) => {
  document.getElementById('createResult').textContent = `初始化失败: ${err.message}`;
});
