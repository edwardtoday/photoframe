import hashlib
import io
import os
import sqlite3
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import Any
from zoneinfo import ZoneInfo

from fastapi import FastAPI, File, Form, Header, HTTPException, Query, Request, UploadFile
from fastapi.responses import FileResponse, HTMLResponse
from fastapi.staticfiles import StaticFiles
from PIL import Image, ImageOps
from pydantic import BaseModel, Field

APP_DIR = Path(__file__).resolve().parent
DATA_DIR = APP_DIR.parent / "data"
ASSET_DIR = DATA_DIR / "assets"
DB_PATH = DATA_DIR / "orchestrator.db"

DEFAULT_DAILY_TEMPLATE = "http://192.168.58.113:8000/image/480x800?date=%DATE%"
DAILY_TEMPLATE = os.getenv("DAILY_IMAGE_URL_TEMPLATE", DEFAULT_DAILY_TEMPLATE)
PUBLIC_BASE_URL = os.getenv("PUBLIC_BASE_URL", "").rstrip("/")
DEFAULT_POLL_SECONDS = max(60, int(os.getenv("DEFAULT_POLL_SECONDS", "3600")))
TOKEN = os.getenv("PHOTOFRAME_TOKEN", "")
TZ_NAME = os.getenv("TZ", "Asia/Shanghai")
LOCAL_TZ = ZoneInfo(TZ_NAME)

app = FastAPI(title="PhotoFrame Orchestrator", version="0.1.0")
app.mount("/static", StaticFiles(directory=APP_DIR / "static"), name="static")

DB_LOCK = threading.Lock()
DB: sqlite3.Connection | None = None


def _open_db() -> sqlite3.Connection:
  conn = sqlite3.connect(DB_PATH, check_same_thread=False)
  conn.row_factory = sqlite3.Row
  return conn


def _ensure_db() -> sqlite3.Connection:
  global DB
  if DB is None:
    DB = _open_db()
  return DB


def _init_db() -> None:
  DATA_DIR.mkdir(parents=True, exist_ok=True)
  ASSET_DIR.mkdir(parents=True, exist_ok=True)
  conn = _ensure_db()
  with DB_LOCK:
    conn.executescript(
        """
        CREATE TABLE IF NOT EXISTS devices (
          device_id TEXT PRIMARY KEY,
          last_checkin_epoch INTEGER NOT NULL DEFAULT 0,
          next_wakeup_epoch INTEGER NOT NULL DEFAULT 0,
          sleep_seconds INTEGER NOT NULL DEFAULT 0,
          poll_interval_seconds INTEGER NOT NULL DEFAULT 3600,
          failure_count INTEGER NOT NULL DEFAULT 0,
          last_http_status INTEGER NOT NULL DEFAULT 0,
          fetch_ok INTEGER NOT NULL DEFAULT 0,
          image_changed INTEGER NOT NULL DEFAULT 0,
          image_source TEXT NOT NULL DEFAULT 'daily',
          last_error TEXT NOT NULL DEFAULT '',
          updated_at INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS overrides (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          device_id TEXT NOT NULL,
          start_epoch INTEGER NOT NULL,
          end_epoch INTEGER NOT NULL,
          asset_name TEXT NOT NULL,
          asset_sha256 TEXT NOT NULL,
          note TEXT NOT NULL DEFAULT '',
          created_epoch INTEGER NOT NULL,
          enabled INTEGER NOT NULL DEFAULT 1
        );

        CREATE INDEX IF NOT EXISTS idx_overrides_window
          ON overrides (start_epoch, end_epoch);
        CREATE INDEX IF NOT EXISTS idx_overrides_device_window
          ON overrides (device_id, start_epoch, end_epoch);
        """
    )
    conn.commit()


def _now_epoch() -> int:
  return int(time.time())


def _clamp(v: int, low: int, high: int) -> int:
  return max(low, min(high, v))


def _require_token(header_token: str | None) -> None:
  if TOKEN and header_token != TOKEN:
    raise HTTPException(status_code=401, detail="invalid token")


def _daily_image_url(now_epoch: int) -> str:
  date_text = datetime.fromtimestamp(now_epoch, LOCAL_TZ).strftime("%Y-%m-%d")
  url = DAILY_TEMPLATE.replace("%DATE%", date_text)
  if "date=" not in url:
    connector = "&" if "?" in url else "?"
    url = f"{url}{connector}date={date_text}"
  return url


def _public_base(request: Request) -> str:
  if PUBLIC_BASE_URL:
    return PUBLIC_BASE_URL
  return f"{request.url.scheme}://{request.url.netloc}"


def _parse_start_epoch(starts_at: str | None) -> int:
  if starts_at is None or starts_at.strip() == "":
    return _now_epoch()
  try:
    dt = datetime.fromisoformat(starts_at)
  except ValueError as exc:
    raise HTTPException(status_code=400, detail="starts_at format invalid") from exc
  if dt.tzinfo is None:
    dt = dt.replace(tzinfo=LOCAL_TZ)
  return int(dt.timestamp())


def _read_and_convert_bmp(upload: UploadFile) -> tuple[str, str]:
  raw = upload.file.read()
  if not raw:
    raise HTTPException(status_code=400, detail="empty upload file")

  try:
    with Image.open(io.BytesIO(raw)) as image:
      rgb = image.convert("RGB")
      # 固件只接收 480x800 BMP，服务端统一做裁剪缩放保证设备可直接显示。
      fitted = ImageOps.fit(rgb, (480, 800), method=Image.Resampling.LANCZOS)
      out = io.BytesIO()
      fitted.save(out, format="BMP")
      bmp_data = out.getvalue()
  except Exception as exc:  # pragma: no cover
    raise HTTPException(status_code=400, detail="cannot decode image") from exc

  sha256 = hashlib.sha256(bmp_data).hexdigest()
  asset_name = f"{sha256}.bmp"
  out_path = ASSET_DIR / asset_name
  if not out_path.exists():
    out_path.write_bytes(bmp_data)
  return asset_name, sha256


def _device_next_wakeup(device_id: str) -> int | None:
  conn = _ensure_db()
  row = conn.execute(
      "SELECT next_wakeup_epoch FROM devices WHERE device_id = ?",
      (device_id,),
  ).fetchone()
  if row is None:
    return None
  return int(row["next_wakeup_epoch"])


def _guess_effective_epoch(device_id: str, start_epoch: int) -> int | None:
  if device_id == "*":
    return None
  next_wakeup = _device_next_wakeup(device_id)
  if next_wakeup is None:
    return start_epoch
  return max(start_epoch, next_wakeup)


class DeviceCheckin(BaseModel):
  device_id: str = Field(min_length=1, max_length=64)
  checkin_epoch: int
  next_wakeup_epoch: int
  sleep_seconds: int = 0
  poll_interval_seconds: int = 3600
  failure_count: int = 0
  last_http_status: int = 0
  fetch_ok: bool = False
  image_changed: bool = False
  image_source: str = "daily"
  last_error: str = ""


@app.on_event("startup")
def _startup() -> None:
  _init_db()


@app.get("/", response_class=HTMLResponse)
def index() -> str:
  return (APP_DIR / "static" / "index.html").read_text(encoding="utf-8")


@app.get("/healthz")
def healthz() -> dict[str, Any]:
  return {"ok": True, "now_epoch": _now_epoch(), "timezone": TZ_NAME}


@app.get("/api/v1/assets/{asset_name}")
def asset(asset_name: str) -> FileResponse:
  safe_name = os.path.basename(asset_name)
  path = ASSET_DIR / safe_name
  if not path.exists():
    raise HTTPException(status_code=404, detail="asset not found")
  return FileResponse(path=path, media_type="image/bmp", filename=safe_name)


@app.get("/api/v1/device/next")
def device_next(
    request: Request,
    device_id: str = Query(..., min_length=1, max_length=64),
    now_epoch: int | None = Query(default=None),
    default_poll_seconds: int = Query(default=DEFAULT_POLL_SECONDS),
    failure_count: int = Query(default=0),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  now_ts = _now_epoch() if now_epoch is None else now_epoch
  poll_sec = _clamp(default_poll_seconds, 60, 86400)
  conn = _ensure_db()

  with DB_LOCK:
    conn.execute(
        """
        INSERT INTO devices (device_id, updated_at, failure_count)
        VALUES (?, ?, ?)
        ON CONFLICT(device_id) DO UPDATE SET
          updated_at = excluded.updated_at,
          failure_count = excluded.failure_count
        """,
        (device_id, now_ts, max(0, failure_count)),
    )

    active = conn.execute(
        """
        SELECT * FROM overrides
        WHERE enabled = 1
          AND start_epoch <= ?
          AND end_epoch > ?
          AND (device_id = ? OR device_id = '*')
        ORDER BY CASE WHEN device_id = ? THEN 0 ELSE 1 END, created_epoch DESC
        LIMIT 1
        """,
        (now_ts, now_ts, device_id, device_id),
    ).fetchone()

    upcoming = conn.execute(
        """
        SELECT start_epoch FROM overrides
        WHERE enabled = 1
          AND start_epoch > ?
          AND (device_id = ? OR device_id = '*')
        ORDER BY start_epoch ASC
        LIMIT 1
        """,
        (now_ts, device_id),
    ).fetchone()
    conn.commit()

  source = "daily"
  valid_until = now_ts + poll_sec
  image_url = _daily_image_url(now_ts)
  active_override_id = None

  if active is not None:
    source = "override"
    active_override_id = int(active["id"])
    valid_until = int(active["end_epoch"])
    image_url = f"{_public_base(request)}/api/v1/assets/{active['asset_name']}"
    remain = max(1, valid_until - now_ts)
    poll_sec = min(poll_sec, _clamp(remain, 60, 86400))

  if upcoming is not None:
    until_next = max(1, int(upcoming["start_epoch"]) - now_ts)
    poll_sec = min(poll_sec, _clamp(until_next, 60, 86400))

  return {
      "device_id": device_id,
      "server_epoch": now_ts,
      "source": source,
      "image_url": image_url,
      "valid_until_epoch": valid_until,
      "poll_after_seconds": poll_sec,
      "default_poll_seconds": _clamp(default_poll_seconds, 60, 86400),
      "active_override_id": active_override_id,
  }


@app.post("/api/v1/device/checkin")
def device_checkin(
    payload: DeviceCheckin,
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  now_ts = _now_epoch()
  conn = _ensure_db()
  with DB_LOCK:
    conn.execute(
        """
        INSERT INTO devices (
          device_id, last_checkin_epoch, next_wakeup_epoch, sleep_seconds,
          poll_interval_seconds, failure_count, last_http_status, fetch_ok,
          image_changed, image_source, last_error, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(device_id) DO UPDATE SET
          last_checkin_epoch = excluded.last_checkin_epoch,
          next_wakeup_epoch = excluded.next_wakeup_epoch,
          sleep_seconds = excluded.sleep_seconds,
          poll_interval_seconds = excluded.poll_interval_seconds,
          failure_count = excluded.failure_count,
          last_http_status = excluded.last_http_status,
          fetch_ok = excluded.fetch_ok,
          image_changed = excluded.image_changed,
          image_source = excluded.image_source,
          last_error = excluded.last_error,
          updated_at = excluded.updated_at
        """,
        (
            payload.device_id,
            int(payload.checkin_epoch),
            int(payload.next_wakeup_epoch),
            max(0, int(payload.sleep_seconds)),
            max(60, int(payload.poll_interval_seconds)),
            max(0, int(payload.failure_count)),
            int(payload.last_http_status),
            1 if payload.fetch_ok else 0,
            1 if payload.image_changed else 0,
            payload.image_source,
            payload.last_error,
            now_ts,
        ),
    )
    conn.commit()

  return {"ok": True}


@app.get("/api/v1/devices")
def devices() -> dict[str, Any]:
  conn = _ensure_db()
  now_ts = _now_epoch()
  rows = conn.execute(
      """
      SELECT * FROM devices
      ORDER BY CASE WHEN next_wakeup_epoch > 0 THEN next_wakeup_epoch ELSE 9223372036854775807 END,
               device_id ASC
      """
  ).fetchall()

  items: list[dict[str, Any]] = []
  for row in rows:
    next_wakeup = int(row["next_wakeup_epoch"])
    eta = max(0, next_wakeup - now_ts) if next_wakeup > 0 else None
    items.append(
        {
            "device_id": row["device_id"],
            "last_checkin_epoch": int(row["last_checkin_epoch"]),
            "next_wakeup_epoch": next_wakeup,
            "eta_seconds": eta,
            "sleep_seconds": int(row["sleep_seconds"]),
            "poll_interval_seconds": int(row["poll_interval_seconds"]),
            "failure_count": int(row["failure_count"]),
            "last_http_status": int(row["last_http_status"]),
            "fetch_ok": bool(row["fetch_ok"]),
            "image_source": row["image_source"],
            "last_error": row["last_error"],
        }
    )

  return {"now_epoch": now_ts, "devices": items}


@app.get("/api/v1/overrides")
def overrides(now_epoch: int | None = Query(default=None)) -> dict[str, Any]:
  now_ts = _now_epoch() if now_epoch is None else now_epoch
  conn = _ensure_db()
  rows = conn.execute(
      """
      SELECT * FROM overrides
      WHERE enabled = 1
      ORDER BY start_epoch DESC, id DESC
      LIMIT 200
      """
  ).fetchall()

  items: list[dict[str, Any]] = []
  for row in rows:
    start = int(row["start_epoch"])
    end = int(row["end_epoch"])
    if now_ts < start:
      state = "upcoming"
    elif now_ts >= end:
      state = "expired"
    else:
      state = "active"

    items.append(
        {
            "id": int(row["id"]),
            "device_id": row["device_id"],
            "start_epoch": start,
            "end_epoch": end,
            "state": state,
            "asset_name": row["asset_name"],
            "asset_sha256": row["asset_sha256"],
            "note": row["note"],
            "created_epoch": int(row["created_epoch"]),
            "expected_effective_epoch": _guess_effective_epoch(row["device_id"], start),
        }
    )

  return {"now_epoch": now_ts, "overrides": items}


@app.post("/api/v1/overrides/upload")
def override_upload(
    request: Request,
    file: UploadFile = File(...),
    duration_minutes: int = Form(...),
    device_id: str = Form(default="*"),
    starts_at: str | None = Form(default=None),
    note: str = Form(default=""),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  if duration_minutes <= 0:
    raise HTTPException(status_code=400, detail="duration_minutes must be > 0")

  target_device = device_id.strip() or "*"
  start_epoch = _parse_start_epoch(starts_at)
  end_epoch = start_epoch + duration_minutes * 60

  asset_name, sha256 = _read_and_convert_bmp(file)
  now_ts = _now_epoch()
  conn = _ensure_db()

  with DB_LOCK:
    cursor = conn.execute(
        """
        INSERT INTO overrides (device_id, start_epoch, end_epoch, asset_name, asset_sha256, note, created_epoch)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        """,
        (target_device, start_epoch, end_epoch, asset_name, sha256, note, now_ts),
    )
    override_id = int(cursor.lastrowid)
    conn.commit()

  expected_effective_epoch = _guess_effective_epoch(target_device, start_epoch)
  image_url = f"{_public_base(request)}/api/v1/assets/{asset_name}"

  return {
      "ok": True,
      "id": override_id,
      "device_id": target_device,
      "start_epoch": start_epoch,
      "end_epoch": end_epoch,
      "duration_minutes": duration_minutes,
      "image_url": image_url,
      "asset_sha256": sha256,
      "expected_effective_epoch": expected_effective_epoch,
  }


@app.delete("/api/v1/overrides/{override_id}")
def override_delete(
    override_id: int,
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  conn = _ensure_db()
  with DB_LOCK:
    cur = conn.execute(
        "UPDATE overrides SET enabled = 0 WHERE id = ?",
        (override_id,),
    )
    conn.commit()
  if cur.rowcount == 0:
    raise HTTPException(status_code=404, detail="override not found")
  return {"ok": True}
