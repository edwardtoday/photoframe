const TOKEN_STORAGE_KEY = 'photoframe.console.token';
const DEVICE_STORAGE_KEY = 'photoframe.console.device_id';

const state = {
  dashboard: null,
  previewUrl: '',
  refreshTimer: null,
  workspace: 'today',
  photos: [],
  photoUrls: new Map(),
  selectedPhotoId: null,
  deviceTimeline: [],
  deviceIntents: [],
  lab: null,
  comparisonUrls: new Map(),
};

function byId(id) {
  return document.getElementById(id);
}

function storedValue(key) {
  try {
    return window.localStorage.getItem(key) || '';
  } catch (_) {
    return '';
  }
}

function saveValue(key, value) {
  try {
    if (value) window.localStorage.setItem(key, value);
    else window.localStorage.removeItem(key);
  } catch (_) {
    // 浏览器限制本地存储时，本次会话仍可继续使用。
  }
}

function authHeaders() {
  const token = byId('token').value.trim();
  return token ? { 'X-PhotoFrame-Token': token } : {};
}

function formatDate(epoch) {
  if (!epoch) return '未知';
  return new Date(Number(epoch) * 1000).toLocaleString('zh-CN', {
    month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit',
  });
}

function formatRelative(epoch, nowEpoch) {
  if (!epoch) return '未知';
  const seconds = Number(epoch) - Number(nowEpoch || Date.now() / 1000);
  const abs = Math.abs(seconds);
  let value;
  if (abs < 90) value = `${Math.round(abs)} 秒`;
  else if (abs < 7200) value = `${Math.round(abs / 60)} 分钟`;
  else if (abs < 172800) value = `${Math.round(abs / 3600)} 小时`;
  else value = `${Math.round(abs / 86400)} 天`;
  return seconds >= 0 ? `${value}后` : `${value}前`;
}

function statusText(status) {
  return {
    healthy: '状态正常',
    sleeping: '按计划休眠',
    warning: '需要留意',
    critical: '需要处理',
    unknown: '状态未知',
  }[status] || '状态未知';
}

function showError(message) {
  const banner = byId('errorBanner');
  banner.textContent = message || '';
  banner.classList.toggle('is-visible', Boolean(message));
}

function setWorkspace(name) {
  state.workspace = name;
  for (const workspace of document.querySelectorAll('[data-workspace]')) {
    workspace.hidden = workspace.dataset.workspace !== name;
  }
  for (const button of document.querySelectorAll('[data-workspace-target]')) {
    const active = button.dataset.workspaceTarget === name;
    if (active) button.setAttribute('aria-current', 'page');
    else button.removeAttribute('aria-current');
  }
  if (name === 'photos') loadPhotos();
  if (name === 'device') loadDeviceWorkspace();
  if (name === 'lab') loadLab();
}

function selectedDeviceId() {
  return state.dashboard?.device?.device_id || byId('deviceSelect').value || storedValue(DEVICE_STORAGE_KEY);
}

function renderDeviceOptions(items, selectedDeviceId) {
  const select = byId('deviceSelect');
  const currentOptions = Array.from(select.options).map((option) => option.value).join(',');
  const nextOptions = ['', ...items.map((item) => item.device_id)].join(',');
  if (currentOptions !== nextOptions) {
    select.replaceChildren(new Option('自动选择最近设备', ''));
    for (const item of items) {
      const label = item.firmware_version ? `${item.device_id} · ${item.firmware_version}` : item.device_id;
      select.add(new Option(label, item.device_id));
    }
  }
  select.value = selectedDeviceId || '';
}

function renderTimelineInto(root, events, nowEpoch) {
  root.replaceChildren();
  if (!events.length) {
    const empty = document.createElement('p');
    empty.className = 'empty';
    empty.textContent = '还没有可展示的设备事件。';
    root.append(empty);
    return;
  }
  for (const event of events) {
    const item = document.createElement('article');
    item.className = 'event';
    item.dataset.severity = event.severity || 'info';

    const time = document.createElement('time');
    time.className = 'event-time';
    time.dateTime = new Date(Number(event.epoch) * 1000).toISOString();
    time.textContent = formatRelative(event.epoch, nowEpoch);

    const rail = document.createElement('div');
    rail.className = 'event-rail';

    const body = document.createElement('div');
    body.className = 'event-body';
    const title = document.createElement('strong');
    title.textContent = event.title || event.kind;
    const detail = document.createElement('p');
    detail.textContent = event.detail || '';
    body.append(title, detail);
    item.append(time, rail, body);
    root.append(item);
  }
}

function renderTimeline(events, nowEpoch) {
  renderTimelineInto(byId('timeline'), events, nowEpoch);
}

function renderEvidence(items) {
  const root = byId('evidence');
  root.replaceChildren();
  if (!items.length) {
    const empty = document.createElement('p');
    empty.className = 'empty';
    empty.textContent = '暂无判断依据。';
    root.append(empty);
    return;
  }
  for (const evidence of items) {
    const item = document.createElement('div');
    item.className = 'evidence-item';
    const label = document.createElement('span');
    label.textContent = evidence.label || '-';
    const value = document.createElement('strong');
    value.textContent = evidence.value || '-';
    item.append(label, value);
    root.append(item);
  }
}

function renderDashboard(data) {
  state.dashboard = data;
  const health = data.health || {};
  const device = data.device;
  const delivery = data.current_delivery;
  byId('hero').dataset.status = health.status || 'unknown';
  byId('statusLabel').textContent = statusText(health.status);
  byId('healthTitle').textContent = health.title || '设备状态未知';
  byId('healthSummary').textContent = health.summary || '连接管理端后查看设备状态。';
  byId('deviceFact').textContent = device?.device_id || '尚未发现';
  byId('wakeupFact').textContent = device?.next_wakeup_epoch
    ? `${formatDate(device.next_wakeup_epoch)} · ${formatRelative(device.next_wakeup_epoch, data.now_epoch)}`
    : '尚未上报';
  const battery = Number(device?.battery_percent);
  const powerParts = [];
  if (Number.isFinite(battery) && battery >= 0) powerParts.push(`${battery}%`);
  if (Number(device?.battery_mv) >= 0) powerParts.push(`${device.battery_mv}mV`);
  if (Number(device?.vbus_good) === 1) powerParts.push('USB 供电');
  if (Number(device?.charging) === 1) powerParts.push('充电中');
  byId('powerFact').textContent = powerParts.join(' · ') || '尚未上报';
  byId('photoBadge').textContent = delivery?.is_confirmed_displayed
    ? `最近确认显示 · ${formatDate(delivery.displayed_epoch)}`
    : delivery
      ? `仅已发送，尚未确认显示 · ${formatDate(delivery.issued_epoch)}`
      : '服务器预计下发画面 · 尚无发布记录';
  renderTimeline(data.recent_events || [], data.now_epoch);
  renderEvidence(health.evidence || []);
  const service = data.service || {};
  byId('serviceNote').textContent = `服务 ${service.app_version || '-'} · ${service.app_git_sha || '-'} · ${service.timezone || '-'}`;
  renderDeviceOptions(data.available_devices || [], device?.device_id || '');
}

async function fetchDashboard() {
  const selectedDevice = byId('deviceSelect').value || storedValue(DEVICE_STORAGE_KEY);
  const query = new URLSearchParams({ event_limit: '12' });
  if (selectedDevice) query.set('device_id', selectedDevice);
  const response = await fetch(`/api/v2/admin/dashboard?${query}`, { headers: authHeaders(), cache: 'no-store' });
  if (!response.ok) {
    if (response.status === 401) throw new Error('管理端 Token 无效或尚未填写。');
    throw new Error(`Dashboard 请求失败：HTTP ${response.status}`);
  }
  return response.json();
}

async function fetchJson(url, options = {}) {
  const headers = { ...authHeaders(), ...(options.headers || {}) };
  const response = await fetch(url, { ...options, headers, cache: 'no-store' });
  if (!response.ok) {
    let detail = '';
    try {
      const body = await response.json();
      detail = body.detail || '';
    } catch (_) {
      // 非 JSON 错误响应使用状态码说明。
    }
    throw new Error(detail || `请求失败：HTTP ${response.status}`);
  }
  return response.json();
}

function releasePhotoUrls() {
  for (const url of state.photoUrls.values()) URL.revokeObjectURL(url);
  state.photoUrls.clear();
}

async function attachProtectedPhoto(image, reference, key) {
  if (!reference) return;
  try {
    const response = await fetch(reference, { headers: authHeaders(), cache: 'force-cache' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const blobUrl = URL.createObjectURL(await response.blob());
    state.photoUrls.set(key, blobUrl);
    image.src = blobUrl;
  } catch (_) {
    image.alt = '照片预览加载失败';
  }
}

function photoTitle(photo) {
  return photo.note || photo.original_filename || `照片 #${photo.id}`;
}

function renderPhotos() {
  releasePhotoUrls();
  const root = byId('photoGrid');
  root.replaceChildren();
  if (!state.photos.length) {
    const empty = document.createElement('p');
    empty.className = 'empty';
    empty.textContent = '照片库还是空的。上传第一张照片后，可以随时让它重新显示。';
    root.append(empty);
    return;
  }
  for (const photo of state.photos) {
    const card = document.createElement('article');
    card.className = 'photo-card';
    const media = document.createElement('div');
    media.className = 'photo-card-media';
    const image = document.createElement('img');
    image.alt = photoTitle(photo);
    media.append(image);
    if (photo.favorite) {
      const mark = document.createElement('span');
      mark.className = 'favorite-mark';
      mark.textContent = '已收藏';
      media.append(mark);
    }
    const body = document.createElement('div');
    body.className = 'photo-card-body';
    const title = document.createElement('div');
    title.className = 'photo-card-title';
    title.textContent = photoTitle(photo);
    const meta = document.createElement('div');
    meta.className = 'photo-card-meta';
    meta.textContent = photo.last_delivery
      ? `上次显示请求：${formatDate(photo.last_delivery.requested_epoch)} · ${photo.last_delivery.device_id}`
      : `加入：${formatDate(photo.created_epoch)}`;
    const actions = document.createElement('div');
    actions.className = 'photo-card-actions';
    const deliver = document.createElement('button');
    deliver.type = 'button';
    deliver.className = 'deliver-photo';
    deliver.textContent = '显示这张';
    deliver.addEventListener('click', () => openDeliverDialog(photo.id));
    const favorite = document.createElement('button');
    favorite.type = 'button';
    favorite.textContent = photo.favorite ? '取消收藏' : '收藏';
    favorite.addEventListener('click', () => sendPhotoFeedback(photo.id, photo.favorite ? 'neutral' : 'favorite'));
    actions.append(deliver, favorite);
    const feedback = document.createElement('div');
    feedback.className = 'feedback-row';
    for (const [kind, label] of [['crop_issue', '裁剪有问题'], ['color_issue', '色彩有问题'], ['hide', '不再显示']]) {
      const button = document.createElement('button');
      button.type = 'button';
      button.textContent = label;
      button.addEventListener('click', () => sendPhotoFeedback(photo.id, kind));
      feedback.append(button);
    }
    body.append(title, meta, actions, feedback);
    card.append(media, body);
    root.append(card);
    attachProtectedPhoto(image, photo.render_variant?.image_reference, `photo:${photo.id}`);
  }
}

async function loadPhotos() {
  const root = byId('photoGrid');
  root.replaceChildren();
  const loading = document.createElement('p');
  loading.className = 'empty';
  loading.textContent = '正在加载照片库…';
  root.append(loading);
  try {
    const data = await fetchJson('/api/v2/admin/photos?limit=80');
    state.photos = data.items || [];
    renderPhotos();
  } catch (error) {
    loading.textContent = `照片库加载失败：${error.message}`;
  }
}

async function uploadPhoto(event) {
  event.preventDefault();
  const fileInput = byId('photoUploadFile');
  const file = fileInput.files?.[0];
  if (!file) return;
  const status = byId('photoUploadStatus');
  const button = byId('photoUploadButton');
  status.textContent = '正在生成相框版本…';
  button.disabled = true;
  const form = new FormData();
  form.append('file', file);
  form.append('note', byId('photoUploadNote').value.trim());
  try {
    const response = await fetch('/api/v2/admin/photos/upload', { method: 'POST', headers: authHeaders(), body: form });
    if (!response.ok) {
      const body = await response.json().catch(() => ({}));
      throw new Error(body.detail || `HTTP ${response.status}`);
    }
    const data = await response.json();
    status.textContent = '已加入照片库。';
    fileInput.value = '';
    byId('photoUploadNote').value = '';
    await loadPhotos();
    openDeliverDialog(data.item.id);
  } catch (error) {
    status.textContent = `上传失败：${error.message}`;
  } finally {
    button.disabled = false;
  }
}

function openDeliverDialog(photoId) {
  const photo = state.photos.find((item) => Number(item.id) === Number(photoId));
  if (!photo) return;
  state.selectedPhotoId = photo.id;
  byId('deliverDialogDescription').textContent = `“${photoTitle(photo)}”会在当前设备下一次唤醒时生效。`;
  byId('deliverStatus').textContent = '';
  byId('deliverDialog').showModal();
}

async function deliverPhoto(event) {
  event.preventDefault();
  const deviceId = state.dashboard?.device?.device_id || byId('deviceSelect').value;
  if (!deviceId) {
    byId('deliverStatus').textContent = '请先连接并选择设备。';
    return;
  }
  const button = byId('deliverConfirm');
  button.disabled = true;
  byId('deliverStatus').textContent = '正在安排显示…';
  try {
    const data = await fetchJson(`/api/v2/admin/photos/${state.selectedPhotoId}/deliver`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        device_id: deviceId,
        duration_minutes: Number(byId('deliverDuration').value),
      }),
    });
    byId('deliverStatus').textContent = data.start_policy === 'next_wakeup'
      ? `已安排，将在 ${formatDate(data.expected_effective_epoch)} 左右生效。`
      : '已安排，将尽快生效。';
    window.setTimeout(() => byId('deliverDialog').close(), 1100);
    await loadPhotos();
    await refresh();
  } catch (error) {
    byId('deliverStatus').textContent = `安排失败：${error.message}`;
  } finally {
    button.disabled = false;
  }
}

async function sendPhotoFeedback(photoId, kind) {
  try {
    await fetchJson(`/api/v2/admin/photos/${photoId}/feedback`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ kind, note: '' }),
    });
    await loadPhotos();
  } catch (error) {
    showError(`照片反馈失败：${error.message}`);
  }
}

function addDeviceFact(root, label, value) {
  const item = document.createElement('div');
  item.className = 'device-fact';
  const caption = document.createElement('span');
  caption.textContent = label;
  const content = document.createElement('strong');
  content.textContent = value || '未知';
  item.append(caption, content);
  root.append(item);
}

function renderDeviceWorkspace() {
  const data = state.dashboard;
  const device = data?.device;
  const health = data?.health || {};
  byId('deviceHealthTitle').textContent = health.title || '设备状态未知';
  byId('deviceHealthSummary').textContent = health.summary || '连接管理端后查看设备状态。';
  byId('deviceHealthPill').textContent = statusText(health.status);
  const facts = byId('deviceFacts');
  facts.replaceChildren();
  if (!device) {
    addDeviceFact(facts, '设备', '尚未发现设备');
  } else {
    addDeviceFact(facts, '最近看到', `${formatDate(device.last_seen_epoch)} · ${formatRelative(device.last_seen_epoch, data.now_epoch)}`);
    addDeviceFact(facts, '下一次唤醒', device.next_wakeup_epoch ? `${formatDate(device.next_wakeup_epoch)} · ${formatRelative(device.next_wakeup_epoch, data.now_epoch)}` : '尚未上报');
    addDeviceFact(facts, '固件与分区', `${device.firmware_version || '未知版本'} · ${device.running_partition || '未知分区'}`);
    addDeviceFact(facts, '网络', device.sta_ip ? `${device.sta_ip} · HTTP ${device.last_http_status || '-'}` : `HTTP ${device.last_http_status || '-'}`);
    addDeviceFact(facts, '配置版本', `目标 ${device.config_target_version || 0} · 已见 ${device.config_seen_version || 0} · 已应用 ${device.config_applied_version || 0}`);
    addDeviceFact(facts, 'OTA', device.ota_state ? `${device.ota_state}${device.ota_target_version ? ` · ${device.ota_target_version}` : ''}` : '没有进行中的升级');
  }
  renderFilteredDeviceTimeline();
  const intentRoot = byId('deviceIntentList');
  intentRoot.replaceChildren();
  for (const intent of state.deviceIntents.filter((item) => item.key !== 'custom_interval')) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'intent-button';
    const copy = document.createElement('div');
    const title = document.createElement('strong');
    title.textContent = intent.label;
    const description = document.createElement('span');
    description.textContent = `${intent.description} ${intent.impact}`;
    copy.append(title, description);
    button.append(copy);
    button.addEventListener('click', () => applyDeviceIntent(intent.key, intent));
    intentRoot.append(button);
  }
  if (!state.deviceIntents.length) {
    const empty = document.createElement('p');
    empty.className = 'empty';
    empty.textContent = device ? '暂时没有可用操作。' : '连接并选择设备后显示操作。';
    intentRoot.append(empty);
  }
}

function renderFilteredDeviceTimeline() {
  const filter = byId('deviceTimelineFilter').value;
  const filtered = state.deviceTimeline.filter((event) => !filter || String(event.kind || '').startsWith(filter));
  const events = [];
  for (const event of filtered) {
    const previous = events.at(-1);
    if (event.kind === 'photo_sent' && previous?.kind === 'photo_sent'
      && previous.metadata?.image_source === event.metadata?.image_source) {
      const count = Number(previous.metadata?.group_count || 1) + 1;
      const latestDetail = previous.metadata?.latest_detail || previous.detail;
      previous.metadata = { ...(previous.metadata || {}), group_count: count, latest_detail: latestDetail };
      previous.title = `${String(previous.title).replace(/ × \d+$/, '')} × ${count}`;
      previous.detail = `连续 ${count} 条同类下发记录；最近一条：${latestDetail}`;
      continue;
    }
    events.push({ ...event, metadata: { ...(event.metadata || {}) } });
  }
  renderTimelineInto(byId('deviceTimeline'), events, state.dashboard?.now_epoch);
}

async function loadDeviceWorkspace() {
  renderDeviceWorkspace();
  const deviceId = selectedDeviceId();
  if (!deviceId) return;
  try {
    const [timeline, intents] = await Promise.all([
      fetchJson(`/api/v2/admin/devices/${encodeURIComponent(deviceId)}/timeline?limit=120`),
      fetchJson(`/api/v2/admin/devices/${encodeURIComponent(deviceId)}/intents`),
    ]);
    state.deviceTimeline = timeline.items || [];
    state.deviceIntents = intents.items || [];
    byId('customInterval').value = String(intents.current_interval_minutes || 60);
    renderDeviceWorkspace();
  } catch (error) {
    byId('deviceIntentStatus').textContent = `设备详情加载失败：${error.message}`;
  }
}

async function applyDeviceIntent(intent, definition = null, intervalMinutes = null) {
  const deviceId = selectedDeviceId();
  if (!deviceId) {
    byId('deviceIntentStatus').textContent = '请先连接并选择设备。';
    return;
  }
  const impact = definition?.impact || `刷新间隔将改为 ${intervalMinutes} 分钟。`;
  if (!window.confirm(`${impact}\n\n设备不会立即醒来，设置将在下一次唤醒时生效。是否继续？`)) return;
  byId('deviceIntentStatus').textContent = '正在保存操作…';
  try {
    const data = await fetchJson(`/api/v2/admin/devices/${encodeURIComponent(deviceId)}/intents`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ intent, interval_minutes: intervalMinutes, note: '' }),
    });
    byId('deviceIntentStatus').textContent = `${data.ack} 回执 #${data.id || data.request_id}`;
    await refresh();
    await loadDeviceWorkspace();
    if (state.lab) await loadLab();
  } catch (error) {
    byId('deviceIntentStatus').textContent = `操作失败：${error.message}`;
  }
}

function releaseComparisonUrls() {
  for (const url of state.comparisonUrls.values()) URL.revokeObjectURL(url);
  state.comparisonUrls.clear();
}

function compactRow(titleText, detailText, action = null) {
  const row = document.createElement('div');
  row.className = 'compact-row';
  const copy = document.createElement('div');
  const title = document.createElement('strong');
  title.textContent = titleText;
  const detail = document.createElement('span');
  detail.textContent = detailText;
  copy.append(title, detail);
  row.append(copy);
  if (action) row.append(action);
  return row;
}

function renderLab() {
  const data = state.lab;
  if (!data) return;
  const renderConfig = data.render_config || {};
  byId('renderConfigSummary').textContent = `当前默认：${renderConfig.daily_dither_algorithm || '-'} · ${renderConfig.palette_profile || '-'}。对比工具${renderConfig.photo_render_experiment_enabled ? '已启用' : '未启用，但仍可手动生成预览'}。`;

  const artifactSelect = byId('otaArtifact');
  const selectedArtifact = artifactSelect.value;
  artifactSelect.replaceChildren(new Option('选择固件制品', ''));
  for (const artifact of data.firmware_artifacts || []) {
    artifactSelect.add(new Option(`${artifact.version} · ${Math.round(artifact.size_bytes / 1024)} KB`, String(artifact.id)));
  }
  artifactSelect.value = selectedArtifact;

  const otaList = byId('otaList');
  otaList.replaceChildren();
  for (const rollout of data.firmware_rollouts || []) {
    otaList.append(compactRow(
      `${rollout.version} · ${rollout.enabled ? '进行中' : '已停用'}`,
      `${formatDate(rollout.created_epoch)} · 最低 ${rollout.min_battery_percent}%${rollout.requires_vbus ? ' · 要求 USB' : ''}`,
    ));
  }
  if (!otaList.children.length) otaList.append(compactRow('没有升级任务', '上传固件制品后可在旧控制台管理，或从现有制品创建 rollout。'));

  const diagnosticsList = byId('diagnosticsList');
  diagnosticsList.replaceChildren();
  for (const request of (data.log_requests || []).slice(0, 8)) {
    diagnosticsList.append(compactRow(
      `请求 #${request.request_id} · ${request.status}`,
      `${formatDate(request.created_epoch)} · ${request.reason || '未填写原因'}${request.uploaded_line_count ? ` · ${request.uploaded_line_count} 行` : ''}`,
    ));
  }
  if (!diagnosticsList.children.length) diagnosticsList.append(compactRow('还没有诊断请求', '请求后会在设备下一次唤醒时上传。'));

  const configList = byId('configPlanList');
  configList.replaceChildren();
  for (const plan of (data.config_plans || []).slice(0, 6)) {
    configList.append(compactRow(`配置 #${plan.id}`, `${formatDate(plan.created_epoch)} · ${plan.note || '原始配置发布'}`));
  }
  if (!configList.children.length) configList.append(compactRow('还没有配置发布记录', '日常设置建议优先在“设备”工作区完成。'));

  const tokenList = byId('deviceTokenList');
  tokenList.replaceChildren();
  for (const item of data.device_tokens || []) {
    let action = null;
    if (!item.approved) {
      action = document.createElement('button');
      action.type = 'button';
      action.textContent = '审批';
      action.addEventListener('click', () => approveDeviceToken(item.device_id));
    }
    tokenList.append(compactRow(item.device_id, `${item.approved ? '已审批' : '等待审批'} · 最近 ${formatDate(item.last_seen_epoch)}`, action));
  }
  if (!tokenList.children.length) tokenList.append(compactRow('没有设备 Token 记录', '设备首次使用独立 Token 请求时会出现在这里。'));
}

async function loadLab() {
  const deviceId = selectedDeviceId();
  const query = new URLSearchParams();
  if (deviceId) query.set('device_id', deviceId);
  try {
    state.lab = await fetchJson(`/api/v2/admin/lab?${query}`);
    renderLab();
  } catch (error) {
    byId('renderConfigSummary').textContent = `实验室加载失败：${error.message}`;
  }
}

async function generateComparison() {
  const data = state.lab?.render_config;
  if (!data) return;
  releaseComparisonUrls();
  const root = byId('comparisonGrid');
  root.replaceChildren();
  const presets = data.photo_render_presets || [];
  const sourceKey = `vnext-lab-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
  for (const preset of presets) {
    const card = document.createElement('article');
    card.className = 'comparison-card';
    const body = document.createElement('div');
    body.className = 'comparison-card-body';
    const title = document.createElement('strong');
    title.textContent = preset.label;
    const status = document.createElement('p');
    status.textContent = '正在基于同一张源图生成…';
    body.append(title, status);
    card.append(body);
    root.append(card);
    const query = new URLSearchParams({
      device_id: selectedDeviceId() || '*',
      daily_dither_algorithm: preset.algorithm,
      palette_profile: preset.palette_profile,
      fresh_daily_source: '1',
      fresh_daily_source_key: sourceKey,
    });
    fetch(`/api/v1/preview/current.jpg?${query}`, { headers: authHeaders(), cache: 'no-store' })
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.blob();
      })
      .then((blob) => {
        const url = URL.createObjectURL(blob);
        state.comparisonUrls.set(preset.key, url);
        const image = document.createElement('img');
        image.src = url;
        image.alt = `${preset.label} 预览`;
        const description = document.createElement('p');
        description.textContent = preset.description || `${preset.algorithm} · ${preset.palette_profile}`;
        const choose = document.createElement('button');
        choose.type = 'button';
        choose.textContent = preset.algorithm === data.daily_dither_algorithm && preset.palette_profile === data.palette_profile ? '当前默认' : '选为默认';
        choose.disabled = choose.textContent === '当前默认';
        choose.addEventListener('click', () => chooseRenderDefault(preset));
        status.remove();
        body.append(description, choose);
        card.prepend(image);
      })
      .catch((error) => { status.textContent = `生成失败：${error.message}`; });
  }
}

async function chooseRenderDefault(preset) {
  if (!window.confirm(`将默认渲染改为“${preset.label}”？这会影响后续日常照片，但不会立刻刷新当前画面。`)) return;
  try {
    await fetchJson('/api/v1/daily-render-config', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        daily_dither_algorithm: preset.algorithm,
        palette_profile: preset.palette_profile,
        photo_render_experiment_enabled: Boolean(state.lab?.render_config?.photo_render_experiment_enabled),
      }),
    });
    await loadLab();
    await generateComparison();
  } catch (error) {
    byId('renderConfigSummary').textContent = `保存渲染默认值失败：${error.message}`;
  }
}

async function createOtaRollout(event) {
  event.preventDefault();
  const deviceId = selectedDeviceId();
  const artifactId = Number(byId('otaArtifact').value);
  if (!deviceId || !artifactId) {
    byId('otaStatus').textContent = '请选择设备和固件制品。';
    return;
  }
  const battery = Number(byId('otaBattery').value);
  const requiresVbus = byId('otaRequiresVbus').checked;
  if (!window.confirm(`将为 ${deviceId} 创建 OTA 任务。最低电量 ${battery}%${requiresVbus ? '，且必须连接 USB' : ''}。继续？`)) return;
  try {
    const data = await fetchJson('/api/v1/firmware-rollouts', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ device_id: deviceId, firmware_artifact_id: artifactId, min_battery_percent: battery, requires_vbus: requiresVbus, note: 'vNext 实验室' }),
    });
    byId('otaStatus').textContent = `升级任务已创建，回执 #${data.id}。`;
    await loadLab();
    await refresh();
  } catch (error) {
    byId('otaStatus').textContent = `创建失败：${error.message}`;
  }
}

async function requestDiagnosticsFromLab() {
  const definition = state.deviceIntents.find((item) => item.key === 'diagnostics') || { impact: '设备下次唤醒时上传日志。' };
  await applyDeviceIntent('diagnostics', definition);
  byId('diagnosticsStatus').textContent = byId('deviceIntentStatus').textContent;
  await loadLab();
}

async function publishRawConfig(event) {
  event.preventDefault();
  const deviceId = selectedDeviceId();
  if (!deviceId) return;
  let config;
  try {
    config = JSON.parse(byId('rawConfigJson').value);
  } catch (error) {
    byId('rawConfigStatus').textContent = `JSON 无效：${error.message}`;
    return;
  }
  if (!window.confirm(`将向 ${deviceId} 发布原始配置。未知字段会被忽略，下一次唤醒时生效。继续？`)) return;
  try {
    const data = await fetchJson('/api/v1/device-config', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ device_id: deviceId, config, note: 'vNext 实验室原始配置' }),
    });
    byId('rawConfigStatus').textContent = `配置已发布，回执 #${data.id}。`;
    await loadLab();
    await refresh();
  } catch (error) {
    byId('rawConfigStatus').textContent = `发布失败：${error.message}`;
  }
}

async function approveDeviceToken(deviceId) {
  if (!window.confirm(`审批设备 ${deviceId} 的 Token 请求？`)) return;
  try {
    await fetchJson(`/api/v1/device-tokens/${encodeURIComponent(deviceId)}/approve`, { method: 'POST' });
    await loadLab();
  } catch (error) {
    showError(`Token 审批失败：${error.message}`);
  }
}

async function loadPreview(data) {
  if (state.previewUrl) URL.revokeObjectURL(state.previewUrl);
  state.previewUrl = '';
  const image = byId('previewImage');
  const placeholder = byId('photoPlaceholder');
  image.hidden = true;
  placeholder.hidden = false;
  placeholder.textContent = '正在加载相框画面…';
  const deviceId = data.device?.device_id;
  if (!deviceId) {
    placeholder.textContent = '发现设备后，这里会显示相框画面。';
    return;
  }
  const confirmedReference = data.current_delivery?.is_confirmed_displayed
    ? (data.current_delivery.displayed_image_reference || data.current_delivery.image_reference)
    : '';
  const previewPath = confirmedReference || `/api/v1/preview/current.bmp?device_id=${encodeURIComponent(deviceId)}`;
  try {
    const response = await fetch(previewPath, { headers: authHeaders(), cache: 'no-store' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const blob = await response.blob();
    state.previewUrl = URL.createObjectURL(blob);
    image.src = state.previewUrl;
    image.hidden = false;
    placeholder.hidden = true;
  } catch (error) {
    placeholder.textContent = `画面暂时无法加载：${error.message}`;
  }
}

async function refresh() {
  showError('');
  byId('refreshButton').disabled = true;
  byId('connectButton').disabled = true;
  saveValue(TOKEN_STORAGE_KEY, byId('token').value.trim());
  saveValue(DEVICE_STORAGE_KEY, byId('deviceSelect').value);
  try {
    const data = await fetchDashboard();
    renderDashboard(data);
    await loadPreview(data);
    if (state.workspace === 'device') await loadDeviceWorkspace();
    if (state.workspace === 'lab') await loadLab();
  } catch (error) {
    showError(error.message || String(error));
  } finally {
    byId('refreshButton').disabled = false;
    byId('connectButton').disabled = false;
  }
}

function init() {
  byId('token').value = storedValue(TOKEN_STORAGE_KEY);
  byId('deviceSelect').value = storedValue(DEVICE_STORAGE_KEY);
  byId('authToggle').addEventListener('click', () => {
    const panel = byId('authPanel');
    const shouldOpen = panel.hidden;
    panel.hidden = !shouldOpen;
    byId('authToggle').setAttribute('aria-expanded', String(shouldOpen));
    if (shouldOpen) byId('token').focus();
  });
  byId('authPanel').addEventListener('submit', (event) => {
    event.preventDefault();
    refresh();
  });
  byId('refreshButton').addEventListener('click', refresh);
  byId('deviceSelect').addEventListener('change', refresh);
  byId('quickPhotoButton').addEventListener('click', () => setWorkspace('photos'));
  byId('refreshPhotosButton').addEventListener('click', loadPhotos);
  byId('refreshDeviceButton').addEventListener('click', loadDeviceWorkspace);
  byId('refreshLabButton').addEventListener('click', loadLab);
  byId('deviceTimelineFilter').addEventListener('change', renderFilteredDeviceTimeline);
  byId('customIntervalButton').addEventListener('click', () => {
    const interval = Number(byId('customInterval').value);
    const definition = state.deviceIntents.find((item) => item.key === 'custom_interval');
    applyDeviceIntent('custom_interval', definition, interval);
  });
  byId('generateComparisonButton').addEventListener('click', generateComparison);
  byId('otaForm').addEventListener('submit', createOtaRollout);
  byId('requestDiagnosticsButton').addEventListener('click', requestDiagnosticsFromLab);
  byId('rawConfigForm').addEventListener('submit', publishRawConfig);
  byId('photoUploadForm').addEventListener('submit', uploadPhoto);
  byId('deliverForm').addEventListener('submit', deliverPhoto);
  byId('deliverCancel').addEventListener('click', () => byId('deliverDialog').close());
  for (const button of document.querySelectorAll('[data-workspace-target]')) {
    button.addEventListener('click', () => setWorkspace(button.dataset.workspaceTarget));
  }
  refresh();
  state.refreshTimer = window.setInterval(refresh, 60_000);
}

window.addEventListener('beforeunload', () => {
  if (state.previewUrl) URL.revokeObjectURL(state.previewUrl);
  releasePhotoUrls();
  releaseComparisonUrls();
  if (state.refreshTimer) window.clearInterval(state.refreshTimer);
});

init();
