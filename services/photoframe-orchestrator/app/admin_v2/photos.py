from __future__ import annotations

import json
import sqlite3
from typing import Any


PHOTO_FEEDBACK_KINDS = {"favorite", "neutral", "hide", "crop_issue", "color_issue"}


def backfill_photo_assets_from_overrides(conn: sqlite3.Connection, now_epoch: int) -> int:
  rows = conn.execute(
      """
      SELECT asset_name, asset_sha256, dither_algorithm, MIN(created_epoch) AS first_created
      FROM overrides
      WHERE asset_name <> '' AND asset_sha256 <> ''
      GROUP BY asset_name, asset_sha256, dither_algorithm
      ORDER BY first_created ASC
      """
  ).fetchall()
  inserted = 0
  for row in rows:
    asset_sha256 = str(row["asset_sha256"])
    existing = conn.execute(
        "SELECT id FROM photo_assets WHERE source_sha256 = ?",
        (asset_sha256,),
    ).fetchone()
    if existing is None:
      cursor = conn.execute(
          """
          INSERT INTO photo_assets (
            source_sha256, original_asset_name, original_filename, mime_type,
            source_type, note, favorite, hidden, feedback_json, created_epoch, updated_epoch
          ) VALUES (?, '', '', '', 'legacy_override', '', 0, 0, '{}', ?, ?)
          """,
          (asset_sha256, int(row["first_created"] or now_epoch), now_epoch),
      )
      photo_asset_id = int(cursor.lastrowid)
      inserted += 1
    else:
      photo_asset_id = int(existing["id"])
    conn.execute(
        """
        INSERT INTO photo_render_variants (
          photo_asset_id, asset_name, asset_sha256, dither_algorithm,
          palette_profile, width, height, created_epoch
        ) VALUES (?, ?, ?, ?, 'reference', 480, 800, ?)
        ON CONFLICT(photo_asset_id, asset_sha256) DO NOTHING
        """,
        (
            photo_asset_id,
            str(row["asset_name"]),
            asset_sha256,
            str(row["dither_algorithm"] or "none"),
            int(row["first_created"] or now_epoch),
        ),
    )
  return inserted


def create_or_update_photo_asset(
    conn: sqlite3.Connection,
    *,
    source_sha256: str,
    original_asset_name: str,
    original_filename: str,
    mime_type: str,
    note: str,
    asset_name: str,
    asset_sha256: str,
    dither_algorithm: str,
    palette_profile: str,
    now_epoch: int,
) -> dict[str, Any]:
  row = conn.execute(
      "SELECT id FROM photo_assets WHERE source_sha256 = ?",
      (source_sha256,),
  ).fetchone()
  if row is None:
    cursor = conn.execute(
        """
        INSERT INTO photo_assets (
          source_sha256, original_asset_name, original_filename, mime_type,
          source_type, note, favorite, hidden, feedback_json, created_epoch, updated_epoch
        ) VALUES (?, ?, ?, ?, 'upload', ?, 0, 0, '{}', ?, ?)
        """,
        (
            source_sha256,
            original_asset_name,
            original_filename,
            mime_type,
            note,
            now_epoch,
            now_epoch,
        ),
    )
    photo_asset_id = int(cursor.lastrowid)
  else:
    photo_asset_id = int(row["id"])
    conn.execute(
        """
        UPDATE photo_assets
        SET original_asset_name = CASE WHEN original_asset_name = '' THEN ? ELSE original_asset_name END,
            original_filename = CASE WHEN original_filename = '' THEN ? ELSE original_filename END,
            mime_type = CASE WHEN mime_type = '' THEN ? ELSE mime_type END,
            note = CASE WHEN ? <> '' THEN ? ELSE note END,
            updated_epoch = ?
        WHERE id = ?
        """,
        (original_asset_name, original_filename, mime_type, note, note, now_epoch, photo_asset_id),
    )

  variant = conn.execute(
      """
      SELECT id FROM photo_render_variants
      WHERE photo_asset_id = ? AND asset_sha256 = ?
      """,
      (photo_asset_id, asset_sha256),
  ).fetchone()
  if variant is None:
    cursor = conn.execute(
        """
        INSERT INTO photo_render_variants (
          photo_asset_id, asset_name, asset_sha256, dither_algorithm,
          palette_profile, width, height, created_epoch
        ) VALUES (?, ?, ?, ?, ?, 480, 800, ?)
        """,
        (
            photo_asset_id,
            asset_name,
            asset_sha256,
            dither_algorithm,
            palette_profile,
            now_epoch,
        ),
    )
    variant_id = int(cursor.lastrowid)
  else:
    variant_id = int(variant["id"])

  return {"photo_asset_id": photo_asset_id, "render_variant_id": variant_id}


def create_override_record(
    conn: sqlite3.Connection,
    *,
    device_id: str,
    start_epoch: int,
    end_epoch: int,
    asset_name: str,
    asset_sha256: str,
    note: str,
    created_epoch: int,
    dither_algorithm: str,
) -> int:
  cursor = conn.execute(
      """
      INSERT INTO overrides (
        device_id, start_epoch, end_epoch, asset_name, asset_sha256,
        note, created_epoch, dither_algorithm
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
      """,
      (
          device_id,
          start_epoch,
          end_epoch,
          asset_name,
          asset_sha256,
          note,
          created_epoch,
          dither_algorithm,
      ),
  )
  return int(cursor.lastrowid)


def record_photo_delivery(
    conn: sqlite3.Connection,
    *,
    photo_asset_id: int,
    render_variant_id: int,
    override_id: int,
    device_id: str,
    duration_minutes: int,
    note: str,
    requested_epoch: int,
) -> int:
  cursor = conn.execute(
      """
      INSERT INTO photo_deliveries (
        photo_asset_id, render_variant_id, override_id, device_id,
        duration_minutes, note, requested_epoch
      ) VALUES (?, ?, ?, ?, ?, ?, ?)
      """,
      (
          photo_asset_id,
          render_variant_id,
          override_id,
          device_id,
          duration_minutes,
          note,
          requested_epoch,
      ),
  )
  return int(cursor.lastrowid)


def get_photo_asset(conn: sqlite3.Connection, photo_asset_id: int) -> dict[str, Any] | None:
  row = conn.execute(
      """
      SELECT p.*, v.id AS variant_id, v.asset_name, v.asset_sha256,
             v.dither_algorithm, v.palette_profile
      FROM photo_assets p
      LEFT JOIN photo_render_variants v ON v.id = (
        SELECT id FROM photo_render_variants
        WHERE photo_asset_id = p.id
        ORDER BY created_epoch DESC, id DESC
        LIMIT 1
      )
      WHERE p.id = ?
      """,
      (photo_asset_id,),
  ).fetchone()
  return None if row is None else _photo_row(row)


def list_photo_assets(
    conn: sqlite3.Connection,
    *,
    limit: int,
    include_hidden: bool = False,
) -> list[dict[str, Any]]:
  where = "" if include_hidden else "WHERE p.hidden = 0"
  rows = conn.execute(
      f"""
      SELECT p.*, v.id AS variant_id, v.asset_name, v.asset_sha256,
             v.dither_algorithm, v.palette_profile,
             d.id AS delivery_id, d.device_id AS last_device_id,
             d.override_id AS last_override_id, d.requested_epoch AS last_delivered_epoch
      FROM photo_assets p
      LEFT JOIN photo_render_variants v ON v.id = (
        SELECT id FROM photo_render_variants
        WHERE photo_asset_id = p.id
        ORDER BY created_epoch DESC, id DESC
        LIMIT 1
      )
      LEFT JOIN photo_deliveries d ON d.id = (
        SELECT id FROM photo_deliveries
        WHERE photo_asset_id = p.id
        ORDER BY requested_epoch DESC, id DESC
        LIMIT 1
      )
      {where}
      ORDER BY p.favorite DESC, COALESCE(d.requested_epoch, p.created_epoch) DESC, p.id DESC
      LIMIT ?
      """,
      (max(1, min(200, limit)),),
  ).fetchall()
  return [_photo_row(row) for row in rows]


def update_photo_feedback(
    conn: sqlite3.Connection,
    *,
    photo_asset_id: int,
    kind: str,
    note: str,
    now_epoch: int,
) -> dict[str, Any] | None:
  if kind not in PHOTO_FEEDBACK_KINDS:
    raise ValueError(f"unsupported feedback kind: {kind}")
  row = conn.execute(
      "SELECT feedback_json FROM photo_assets WHERE id = ?",
      (photo_asset_id,),
  ).fetchone()
  if row is None:
    return None
  try:
    feedback = json.loads(str(row["feedback_json"] or "{}"))
  except json.JSONDecodeError:
    feedback = {}
  feedback[kind] = {"note": note, "epoch": now_epoch}
  favorite = 1 if kind == "favorite" else 0 if kind == "neutral" else None
  hidden = 1 if kind == "hide" else None
  conn.execute(
      """
      UPDATE photo_assets
      SET feedback_json = ?,
          favorite = COALESCE(?, favorite),
          hidden = COALESCE(?, hidden),
          updated_epoch = ?
      WHERE id = ?
      """,
      (json.dumps(feedback, ensure_ascii=False), favorite, hidden, now_epoch, photo_asset_id),
  )
  return get_photo_asset(conn, photo_asset_id)


def _photo_row(row: sqlite3.Row) -> dict[str, Any]:
  feedback_raw = str(row["feedback_json"] or "{}")
  try:
    feedback = json.loads(feedback_raw)
  except json.JSONDecodeError:
    feedback = {}
  keys = set(row.keys())
  return {
      "id": int(row["id"]),
      "source_sha256": str(row["source_sha256"]),
      "original_filename": str(row["original_filename"] or ""),
      "source_type": str(row["source_type"]),
      "note": str(row["note"] or ""),
      "favorite": bool(int(row["favorite"])),
      "hidden": bool(int(row["hidden"])),
      "feedback": feedback,
      "created_epoch": int(row["created_epoch"]),
      "updated_epoch": int(row["updated_epoch"]),
      "render_variant": None if row["variant_id"] is None else {
          "id": int(row["variant_id"]),
          "asset_name": str(row["asset_name"]),
          "asset_sha256": str(row["asset_sha256"]),
          "dither_algorithm": str(row["dither_algorithm"]),
          "palette_profile": str(row["palette_profile"]),
          "image_reference": f"/api/v1/assets/{row['asset_name']}",
      },
      "last_delivery": None if "delivery_id" not in keys or row["delivery_id"] is None else {
          "id": int(row["delivery_id"]),
          "device_id": str(row["last_device_id"]),
          "override_id": int(row["last_override_id"]),
          "requested_epoch": int(row["last_delivered_epoch"]),
      },
  }
