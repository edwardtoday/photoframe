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

async function loadDevices() {
  const data = await fetchJson('/api/v1/devices');
  const body = document.getElementById('devicesBody');
  const deviceSelect = document.getElementById('deviceId');

  body.innerHTML = '';
  deviceSelect.innerHTML = '<option value="*">全部设备 (*)</option>';

  const devices = data.devices || [];
  document.getElementById('deviceCount').textContent = String(devices.length);
  document.getElementById('serverNow').textContent = fmtEpoch(data.now_epoch);

  for (const d of devices) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td><span class="tag">${escapeHtml(d.device_id)}</span></td>
      <td>${fmtEpoch(d.last_checkin_epoch)}</td>
      <td>${fmtEpoch(d.next_wakeup_epoch)}</td>
      <td>${fmtDuration(d.eta_seconds)}</td>
      <td>${fmtDuration(d.poll_interval_seconds)}</td>
      <td>${escapeHtml(d.failure_count)}</td>
      <td>${escapeHtml(d.image_source || 'daily')}</td>
      <td>${escapeHtml(d.last_error || '')}</td>
    `;
    body.appendChild(tr);

    const op = document.createElement('option');
    op.value = d.device_id;
    op.textContent = d.device_id;
    deviceSelect.appendChild(op);
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

function renderReleaseItem(item) {
  const highlights = Array.isArray(item.highlights)
    ? item.highlights.map((h) => `<li>${escapeHtml(h)}</li>`).join('')
    : '';
  const commitText = item.commit ? ` · commit ${escapeHtml(item.commit)}` : '';
  return `
    <article class="release-item">
      <div class="release-head">
        <p class="release-title">v${escapeHtml(item.version)} · ${escapeHtml(item.title || '更新')}</p>
        <span class="release-date">${escapeHtml(item.released_on || '-')}</span>
      </div>
      <p class="release-summary">${escapeHtml(item.summary || '')}${commitText}</p>
      ${highlights ? `<ul>${highlights}</ul>` : ''}
    </article>
  `;
}

async function loadReleases() {
  const data = await fetchJson('/api/v1/releases');
  const releases = data.releases || [];

  const currentVersion = data.current_version || '-';
  document.getElementById('appVersion').textContent = currentVersion;
  document.getElementById('appVersionStat').textContent = currentVersion;

  const body = document.getElementById('releasesBody');
  if (releases.length === 0) {
    body.innerHTML = '<p class="muted">暂无发布记录</p>';
  } else {
    body.innerHTML = releases.map(renderReleaseItem).join('');
  }

  document.getElementById('releaseHint').textContent = `共 ${releases.length} 条发布记录`;
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

async function refreshAll() {
  await Promise.all([loadDevices(), loadOverrides(), loadReleases()]);
  document.getElementById('lastRefresh').textContent = new Date().toLocaleTimeString();
}

document.getElementById('overrideForm').addEventListener('submit', async (ev) => {
  try {
    await submitOverride(ev);
  } catch (err) {
    document.getElementById('createResult').textContent = `提交失败: ${err.message}`;
  }
});

document.getElementById('refreshBtn').addEventListener('click', async () => {
  try {
    await refreshAll();
  } catch (err) {
    alert(`刷新失败: ${err.message}`);
  }
});

setInterval(() => {
  refreshAll().catch(() => {});
}, 30000);

refreshAll().catch((err) => {
  document.getElementById('createResult').textContent = `初始化失败: ${err.message}`;
});
