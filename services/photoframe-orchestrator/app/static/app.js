function authHeaders() {
  const token = document.getElementById('token').value.trim();
  if (!token) {
    return {};
  }
  return { 'X-PhotoFrame-Token': token };
}

function fmtEpoch(ts) {
  if (!ts) return '-';
  const d = new Date(ts * 1000);
  return d.toLocaleString();
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

async function loadDevices() {
  const data = await fetchJson('/api/v1/devices');
  const body = document.getElementById('devicesBody');
  const deviceSelect = document.getElementById('deviceId');

  body.innerHTML = '';
  deviceSelect.innerHTML = '<option value="*">全部设备 (*)</option>';

  for (const d of data.devices || []) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td>${d.device_id}</td>
      <td>${fmtEpoch(d.last_checkin_epoch)}</td>
      <td>${fmtEpoch(d.next_wakeup_epoch)}</td>
      <td>${d.eta_seconds ?? '-'}</td>
      <td>${d.poll_interval_seconds}</td>
      <td>${d.failure_count}</td>
      <td>${d.image_source}</td>
      <td>${d.last_error || ''}</td>
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

  for (const item of data.overrides || []) {
    const tr = document.createElement('tr');
    const delBtn = `<button data-id="${item.id}" class="deleteBtn">取消</button>`;
    tr.innerHTML = `
      <td>${item.id}</td>
      <td>${item.device_id}</td>
      <td>${item.state}</td>
      <td>${fmtEpoch(item.start_epoch)}</td>
      <td>${fmtEpoch(item.end_epoch)}</td>
      <td>${fmtEpoch(item.expected_effective_epoch)}</td>
      <td>${item.note || ''}</td>
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
  await loadDevices();
  await loadOverrides();
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

refreshAll().catch((err) => {
  document.getElementById('createResult').textContent = `初始化失败: ${err.message}`;
});
