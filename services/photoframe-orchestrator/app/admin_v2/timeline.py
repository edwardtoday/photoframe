from __future__ import annotations

import sqlite3
from typing import Any

from app.domains.events import make_device_event, merge_device_events


def _text(value: Any) -> str:
  return str(value or "").strip()


def load_device_timeline(
    conn: sqlite3.Connection,
    device_id: str,
    *,
    limit: int = 50,
    before_epoch: int | None = None,
) -> list[dict[str, Any]]:
  max_rows = max(1, min(200, limit))
  cutoff = int(before_epoch or 0)
  epoch_clause = " AND issued_epoch < ?" if cutoff > 0 else ""
  publish_params: tuple[Any, ...] = (device_id, cutoff, max_rows) if cutoff > 0 else (device_id, max_rows)
  publish_rows = conn.execute(
      f"""
      SELECT id, issued_epoch, source, status, displayed_epoch, dither_algorithm
      FROM publish_history
      WHERE device_id = ?{epoch_clause}
      ORDER BY issued_epoch DESC, id DESC
      LIMIT ?
      """,
      publish_params,
  ).fetchall()

  events: list[dict[str, Any]] = []

  delivery_epoch_clause = " AND requested_epoch < ?" if cutoff > 0 else ""
  delivery_params: tuple[Any, ...] = (device_id, cutoff, max_rows) if cutoff > 0 else (device_id, max_rows)
  delivery_rows = conn.execute(
      f"""
      SELECT id, photo_asset_id, override_id, duration_minutes, note, requested_epoch
      FROM photo_deliveries
      WHERE device_id = ?{delivery_epoch_clause}
      ORDER BY requested_epoch DESC, id DESC
      LIMIT ?
      """,
      delivery_params,
  ).fetchall()
  for row in delivery_rows:
    note = _text(row["note"])
    events.append(
        make_device_event(
            event_id=f"photo-delivery:{int(row['id'])}",
            device_id=device_id,
            epoch=int(row["requested_epoch"]),
            kind="photo_delivery_requested",
            severity="info",
            title="已安排照片显示",
            detail=(note + "；" if note else "") + "将在设备下一次唤醒时生效。",
            source="photo_deliveries",
            metadata={
                "delivery_id": int(row["id"]),
                "photo_asset_id": int(row["photo_asset_id"]),
                "override_id": int(row["override_id"]),
                "duration_minutes": int(row["duration_minutes"]),
            },
        )
    )

  for row in publish_rows:
    status = _text(row["status"]) or "sent"
    source = _text(row["source"]) or "daily"
    issued_epoch = int(row["issued_epoch"])
    algorithm = _text(row["dither_algorithm"]) or "default"
    events.append(
        make_device_event(
            event_id=f"publish:{int(row['id'])}:sent",
            device_id=device_id,
            epoch=issued_epoch,
            kind="photo_sent",
            severity="success" if status == "displayed" else "info",
            title="已下发日常照片" if source == "daily" else "已下发临时照片",
            detail=f"来源 {source}，渲染方案 {algorithm}，当前状态 {status}。",
            source="publish_history",
            metadata={"publish_id": int(row["id"]), "delivery_status": status, "image_source": source},
        )
    )
    displayed_epoch = int(row["displayed_epoch"] or 0)
    if displayed_epoch > 0:
      events.append(
          make_device_event(
              event_id=f"publish:{int(row['id'])}:displayed",
              device_id=device_id,
              epoch=displayed_epoch,
              kind="photo_displayed",
              severity="success",
              title="屏幕刷新完成",
              detail="设备已确认完成图片下载和 E-Ink 刷新。",
              source="publish_history",
              metadata={"publish_id": int(row["id"]), "delivery_status": "displayed"},
          )
      )

  query_cutoff = " AND created_epoch < ?" if cutoff > 0 else ""
  common_params: tuple[Any, ...] = (device_id, cutoff, max_rows) if cutoff > 0 else (device_id, max_rows)
  config_rows = conn.execute(
      f"""
      SELECT id, created_epoch, note
      FROM device_config_plans
      WHERE device_id = ?{query_cutoff}
      ORDER BY created_epoch DESC, id DESC
      LIMIT ?
      """,
      common_params,
  ).fetchall()
  for row in config_rows:
    note = _text(row["note"])
    events.append(
        make_device_event(
            event_id=f"config:{int(row['id'])}",
            device_id=device_id,
            epoch=int(row["created_epoch"]),
            kind="config_published",
            severity="info",
            title=f"已发布设备配置 #{int(row['id'])}",
            detail=note or "等待设备在下一次唤醒时查询并应用。",
            source="device_config_plans",
            metadata={"config_version": int(row["id"])},
        )
    )

  log_rows = conn.execute(
      f"""
      SELECT id, created_epoch, reason, status, completed_epoch, uploaded_line_count
      FROM device_log_upload_requests
      WHERE device_id = ?{query_cutoff}
      ORDER BY created_epoch DESC, id DESC
      LIMIT ?
      """,
      common_params,
  ).fetchall()
  for row in log_rows:
    status = _text(row["status"])
    reason = _text(row["reason"])
    events.append(
        make_device_event(
            event_id=f"log-request:{int(row['id'])}",
            device_id=device_id,
            epoch=int(row["created_epoch"]),
            kind="diagnostics_requested",
            severity="warning" if status in {"expired", "cancelled"} else "info",
            title="已请求设备诊断日志",
            detail=f"{reason or '未填写原因'}；状态 {status}。",
            source="device_log_upload_requests",
            metadata={"request_id": int(row["id"]), "status": status},
        )
    )
    completed_epoch = int(row["completed_epoch"] or 0)
    if completed_epoch > 0:
      events.append(
          make_device_event(
              event_id=f"log-request:{int(row['id'])}:completed",
              device_id=device_id,
              epoch=completed_epoch,
              kind="diagnostics_uploaded",
              severity="success",
              title="设备诊断日志已上传",
              detail=f"收到 {int(row['uploaded_line_count'] or 0)} 行日志。",
              source="device_log_upload_requests",
              metadata={"request_id": int(row["id"]), "status": "completed"},
          )
      )

  rollout_rows = conn.execute(
      f"""
      SELECT r.id, r.created_epoch, r.enabled, r.note, r.min_battery_percent,
             r.requires_vbus, a.version
      FROM firmware_rollouts r
      JOIN firmware_artifacts a ON a.id = r.artifact_id
      WHERE r.device_id = ?{query_cutoff.replace('created_epoch', 'r.created_epoch')}
      ORDER BY r.created_epoch DESC, r.id DESC
      LIMIT ?
      """,
      common_params,
  ).fetchall()
  for row in rollout_rows:
    enabled = bool(int(row["enabled"]))
    detail = f"目标 {row['version']}，电量至少 {int(row['min_battery_percent'])}%"
    if bool(int(row["requires_vbus"])):
      detail += "，需要 USB 供电"
    note = _text(row["note"])
    if note:
      detail += f"；{note}"
    events.append(
        make_device_event(
            event_id=f"rollout:{int(row['id'])}",
            device_id=device_id,
            epoch=int(row["created_epoch"]),
            kind="ota_rollout_created" if enabled else "ota_rollout_disabled",
            severity="warning" if enabled else "info",
            title="已创建固件升级任务" if enabled else "固件升级任务已停用",
            detail=detail + "。",
            source="firmware_rollouts",
            metadata={"rollout_id": int(row["id"]), "version": _text(row["version"]), "enabled": enabled},
        )
    )

  debug_epoch_clause = " AND stage_epoch < ?" if cutoff > 0 else ""
  debug_params: tuple[Any, ...] = (device_id, cutoff, max_rows) if cutoff > 0 else (device_id, max_rows)
  debug_rows = conn.execute(
      f"""
      SELECT id, stage, stage_epoch
      FROM device_debug_stages
      WHERE device_id = ?{debug_epoch_clause}
      ORDER BY stage_epoch DESC, id DESC
      LIMIT ?
      """,
      debug_params,
  ).fetchall()
  for row in debug_rows:
    stage = _text(row["stage"])
    events.append(
        make_device_event(
            event_id=f"debug-stage:{int(row['id'])}",
            device_id=device_id,
            epoch=int(row["stage_epoch"]),
            kind="device_debug_stage",
            severity="info",
            title="设备诊断阶段",
            detail=stage or "未命名阶段",
            source="device_debug_stages",
            metadata={"stage": stage},
        )
    )

  return merge_device_events(events, max_rows)
