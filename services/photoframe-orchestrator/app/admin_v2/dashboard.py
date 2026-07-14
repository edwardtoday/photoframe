from __future__ import annotations

import sqlite3
from typing import Any
from urllib.parse import urlsplit

from app.admin_v2.timeline import load_device_timeline
from app.domains.events import compact_device_events
from app.domains.health import assess_device_health


def _safe_image_reference(raw_url: Any) -> str:
  value = str(raw_url or "").strip()
  if not value:
    return ""
  parsed = urlsplit(value)
  if parsed.scheme or parsed.netloc:
    return parsed.path
  return value.split("?", 1)[0]


def _latest_delivery(conn: sqlite3.Connection, device_id: str) -> dict[str, Any] | None:
  row = conn.execute(
      """
      SELECT id, device_id, issued_epoch, source, image_url, override_id,
             poll_after_seconds, valid_until_epoch, dither_algorithm,
             status, displayed_epoch, displayed_image_url, displayed_image_sha256
      FROM publish_history
      WHERE device_id = ?
      ORDER BY issued_epoch DESC, id DESC
      LIMIT 1
      """,
      (device_id,),
  ).fetchone()
  if row is None:
    return None
  displayed_epoch = int(row["displayed_epoch"] or 0)
  return {
      "id": int(row["id"]),
      "device_id": str(row["device_id"]),
      "issued_epoch": int(row["issued_epoch"]),
      "source": str(row["source"] or "daily"),
      "image_reference": _safe_image_reference(row["image_url"]),
      "override_id": None if row["override_id"] is None else int(row["override_id"]),
      "poll_after_seconds": int(row["poll_after_seconds"]),
      "valid_until_epoch": int(row["valid_until_epoch"]),
      "dither_algorithm": str(row["dither_algorithm"] or ""),
      "status": str(row["status"] or "sent"),
      "displayed_epoch": None if displayed_epoch <= 0 else displayed_epoch,
      "displayed_image_reference": _safe_image_reference(row["displayed_image_url"]),
      "displayed_image_sha256": str(row["displayed_image_sha256"] or ""),
      "is_confirmed_displayed": displayed_epoch > 0 and str(row["status"] or "") == "displayed",
  }


def _active_rollout(conn: sqlite3.Connection, device_id: str) -> dict[str, Any] | None:
  row = conn.execute(
      """
      SELECT r.id, r.min_battery_percent, r.requires_vbus, r.note, r.created_epoch,
             a.version, a.asset_sha256
      FROM firmware_rollouts r
      JOIN firmware_artifacts a ON a.id = r.artifact_id
      WHERE r.device_id = ? AND r.enabled = 1
      ORDER BY r.created_epoch DESC, r.id DESC
      LIMIT 1
      """,
      (device_id,),
  ).fetchone()
  if row is None:
    return None
  return {
      "id": int(row["id"]),
      "version": str(row["version"]),
      "min_battery_percent": int(row["min_battery_percent"]),
      "requires_vbus": bool(int(row["requires_vbus"])),
      "note": str(row["note"] or ""),
      "created_epoch": int(row["created_epoch"]),
      "asset_sha256": str(row["asset_sha256"]),
  }


def build_admin_dashboard(
    conn: sqlite3.Connection,
    *,
    devices: list[dict[str, Any]],
    requested_device_id: str | None,
    now_epoch: int,
    event_limit: int,
    service: dict[str, Any],
) -> dict[str, Any]:
  available_devices = [
      {
          "device_id": str(item.get("device_id") or ""),
          "last_seen_epoch": item.get("last_seen_epoch"),
          "firmware_version": str(item.get("firmware_version") or ""),
      }
      for item in devices
      if str(item.get("device_id") or "")
  ]
  selected: dict[str, Any] | None = None
  if requested_device_id:
    selected = next((item for item in devices if str(item.get("device_id")) == requested_device_id), None)
  if selected is None and devices:
    selected = max(devices, key=lambda item: int(item.get("last_seen_epoch") or 0))

  if selected is None:
    return {
        "now_epoch": now_epoch,
        "device": None,
        "health": assess_device_health(None, now_epoch),
        "current_delivery": None,
        "active_rollout": None,
        "recent_events": [],
        "available_devices": available_devices,
        "service": service,
    }

  device_id = str(selected["device_id"])
  latest_delivery = _latest_delivery(conn, device_id)
  active_rollout = _active_rollout(conn, device_id)
  timeline = compact_device_events(
      load_device_timeline(conn, device_id, limit=min(200, event_limit * 4)),
      event_limit,
  )
  health = assess_device_health(selected, now_epoch, latest_delivery, active_rollout)
  return {
      "now_epoch": now_epoch,
      "device": selected,
      "health": health,
      "current_delivery": latest_delivery,
      "active_rollout": active_rollout,
      "recent_events": timeline,
      "available_devices": available_devices,
      "service": service,
  }
