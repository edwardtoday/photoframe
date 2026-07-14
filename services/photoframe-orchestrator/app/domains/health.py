from __future__ import annotations

from typing import Any


HEALTH_STATUS_RANK = {
    "unknown": 0,
    "healthy": 1,
    "sleeping": 1,
    "warning": 2,
    "critical": 3,
}


def _as_int(value: Any, default: int = 0) -> int:
  try:
    return int(value)
  except (TypeError, ValueError):
    return default


def _evidence(label: str, value: str, epoch: int | None = None) -> dict[str, Any]:
  item: dict[str, Any] = {"label": label, "value": value}
  if epoch is not None and epoch > 0:
    item["epoch"] = epoch
  return item


def _action(key: str, label: str, intent: str) -> dict[str, str]:
  return {"key": key, "label": label, "intent": intent}


def _duration_text(seconds: int) -> str:
  value = max(0, int(seconds))
  if value >= 2 * 24 * 3600:
    return f"{round(value / 86400)} 天"
  if value >= 2 * 3600:
    return f"{round(value / 3600)} 小时"
  if value >= 2 * 60:
    return f"{round(value / 60)} 分钟"
  return f"{value} 秒"


def _assessment(
    status: str,
    code: str,
    title: str,
    summary: str,
    freshness_seconds: int | None,
    evidence: list[dict[str, Any]],
    actions: list[dict[str, str]],
) -> dict[str, Any]:
  return {
      "status": status,
      "status_rank": HEALTH_STATUS_RANK[status],
      "code": code,
      "title": title,
      "summary": summary,
      "freshness_seconds": freshness_seconds,
      "evidence": evidence,
      "actions": actions,
  }


def assess_device_health(
    device: dict[str, Any] | None,
    now_epoch: int,
    latest_delivery: dict[str, Any] | None = None,
    active_rollout: dict[str, Any] | None = None,
) -> dict[str, Any]:
  if device is None:
    return _assessment(
        "unknown",
        "never_seen",
        "尚未发现设备",
        "服务端还没有收到设备请求。",
        None,
        [],
        [_action("pair_device", "检查配网与设备 Token", "diagnostics")],
    )

  device_id = str(device.get("device_id") or "")
  last_seen = _as_int(device.get("last_seen_epoch") or device.get("last_checkin_epoch"))
  next_wakeup = _as_int(device.get("next_wakeup_epoch"))
  poll_seconds = max(60, _as_int(device.get("poll_interval_seconds"), 3600))
  battery_percent = _as_int(device.get("battery_percent"), -1)
  battery_mv = _as_int(device.get("battery_mv"), -1)
  vbus_good = _as_int(device.get("vbus_good"), -1)
  charging = _as_int(device.get("charging"), -1)
  last_error = str(device.get("last_error") or "").strip()
  ota_error = str(device.get("ota_last_error") or "").strip()
  config_error = str(device.get("config_apply_error") or "").strip()
  failure_count = _as_int(device.get("failure_count"))
  target_config = _as_int(device.get("config_target_version"))
  applied_config = _as_int(device.get("config_applied_version"))

  freshness_seconds = None if last_seen <= 0 else max(0, now_epoch - last_seen)
  common_evidence = [_evidence("设备", device_id)]
  if last_seen > 0:
    common_evidence.append(_evidence("最近活动", f"{_duration_text(freshness_seconds or 0)}前", last_seen))
  if battery_percent >= 0:
    power_text = f"{battery_percent}%"
    if battery_mv >= 0:
      power_text += f" / {battery_mv}mV"
    if vbus_good == 1:
      power_text += " / USB 供电"
    if charging == 1:
      power_text += " / 充电中"
    common_evidence.append(_evidence("电量", power_text))

  if last_seen <= 0:
    return _assessment(
        "unknown",
        "never_seen",
        "设备状态未知",
        "设备记录存在，但没有可信的最近活动时间。",
        None,
        common_evidence,
        [_action("collect_diagnostics", "检查设备连接", "diagnostics")],
    )

  if last_error or ota_error or config_error:
    error_text = last_error or ota_error or config_error
    return _assessment(
        "critical",
        "device_error",
        "设备报告错误",
        error_text,
        freshness_seconds,
        common_evidence + [_evidence("错误", error_text)],
        [
            _action("collect_logs", "请求诊断日志", "diagnostics"),
            _action("open_timeline", "查看事件时间线", "timeline"),
        ],
    )

  overdue_seconds = 0 if next_wakeup <= 0 else max(0, now_epoch - next_wakeup)
  overdue_grace = max(15 * 60, min(6 * 3600, poll_seconds // 2))
  critical_overdue = max(6 * 3600, poll_seconds * 2)
  if overdue_seconds > critical_overdue:
    return _assessment(
        "critical",
        "wake_overdue",
        "设备长时间未按计划唤醒",
        f"已超过预计唤醒时间 {_duration_text(overdue_seconds)}。",
        freshness_seconds,
        common_evidence + [_evidence("预计唤醒", f"逾期 {_duration_text(overdue_seconds)}", next_wakeup)],
        [
            _action("check_power", "检查供电并手动唤醒", "physical_check"),
            _action("open_timeline", "查看最后一次设备周期", "timeline"),
        ],
    )
  if overdue_seconds > overdue_grace:
    return _assessment(
        "warning",
        "wake_overdue",
        "设备唤醒略有逾期",
        f"已超过预计唤醒时间 {_duration_text(overdue_seconds)}。",
        freshness_seconds,
        common_evidence + [_evidence("预计唤醒", f"逾期 {_duration_text(overdue_seconds)}", next_wakeup)],
        [_action("wait_or_sync", "等待或手动同步", "sync")],
    )

  if battery_percent >= 0 and battery_percent <= 10 and vbus_good != 1:
    return _assessment(
        "critical",
        "battery_critical",
        "电量严重不足",
        "设备可能无法完成下一次联网和屏幕刷新。",
        freshness_seconds,
        common_evidence,
        [_action("charge_device", "连接 USB 充电", "physical_check")],
    )

  if active_rollout is not None:
    min_battery = _as_int(active_rollout.get("min_battery_percent"), 50)
    requires_vbus = bool(active_rollout.get("requires_vbus"))
    battery_blocked = battery_percent >= 0 and battery_percent < min_battery
    vbus_blocked = requires_vbus and vbus_good != 1
    if battery_blocked or vbus_blocked:
      reasons: list[str] = []
      if battery_blocked:
        reasons.append(f"电量需达到 {min_battery}%")
      if vbus_blocked:
        reasons.append("需要 USB 供电")
      return _assessment(
          "warning",
          "ota_blocked_low_battery",
          "固件升级正在等待条件",
          "，".join(reasons) + "。",
          freshness_seconds,
          common_evidence + [_evidence("目标固件", str(active_rollout.get("version") or "-"))],
          [_action("prepare_ota", "满足升级条件", "ota")],
      )

  if battery_percent >= 0 and battery_percent <= 20 and vbus_good != 1:
    return _assessment(
        "warning",
        "battery_low",
        "设备电量偏低",
        "建议在下次刷新前充电。",
        freshness_seconds,
        common_evidence,
        [_action("charge_device", "安排充电", "physical_check")],
    )

  if target_config > applied_config:
    return _assessment(
        "warning",
        "config_pending",
        "配置尚未应用",
        f"目标版本 {target_config}，已应用版本 {applied_config}。",
        freshness_seconds,
        common_evidence,
        [_action("open_config", "查看配置下发状态", "configuration")],
    )

  if latest_delivery is not None:
    delivery_status = str(latest_delivery.get("status") or "sent")
    issued_epoch = _as_int(latest_delivery.get("issued_epoch"))
    if delivery_status != "displayed" and issued_epoch > 0 and now_epoch - issued_epoch > max(2 * 3600, poll_seconds):
      return _assessment(
          "warning",
          "display_unconfirmed",
          "最近图片尚未确认显示",
          "服务端已经发送图片，但没有收到屏幕刷新完成回报。",
          freshness_seconds,
          common_evidence + [_evidence("图片状态", delivery_status, issued_epoch)],
          [_action("open_timeline", "查看图片下发过程", "timeline")],
      )

  if failure_count > 0:
    return _assessment(
        "warning",
        "recent_failures",
        "设备最近发生过失败",
        f"连续失败计数为 {failure_count}。",
        freshness_seconds,
        common_evidence + [_evidence("连续失败", str(failure_count))],
        [_action("open_timeline", "查看失败事件", "timeline")],
    )

  if next_wakeup > now_epoch:
    return _assessment(
        "sleeping",
        "sleeping_as_expected",
        "设备正在按计划休眠",
        f"预计 {_duration_text(next_wakeup - now_epoch)}后唤醒。",
        freshness_seconds,
        common_evidence + [_evidence("下次唤醒", f"{_duration_text(next_wakeup - now_epoch)}后", next_wakeup)],
        [_action("change_photo", "换一张照片", "delivery")],
    )

  return _assessment(
      "healthy",
      "healthy_recent_checkin",
      "设备最近活动正常",
      "没有发现需要处理的设备异常。",
      freshness_seconds,
      common_evidence,
      [_action("change_photo", "换一张照片", "delivery")],
  )
