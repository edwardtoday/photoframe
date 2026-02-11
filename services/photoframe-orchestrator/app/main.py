import hashlib
import hmac
import io
import json
import os
import sqlite3
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request as UrlRequest, urlopen
from zoneinfo import ZoneInfo

from fastapi import FastAPI, File, Form, Header, HTTPException, Query, Request, UploadFile
from fastapi.responses import FileResponse, HTMLResponse, Response
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
PUBLIC_DAILY_BMP_TOKEN = os.getenv("PUBLIC_DAILY_BMP_TOKEN", "")
DEVICE_TOKEN_MAP_JSON = os.getenv("DEVICE_TOKEN_MAP_JSON", "")
DEVICE_TOKEN_MAP = os.getenv("DEVICE_TOKEN_MAP", "")
try:
  DAILY_FETCH_TIMEOUT_SECONDS = max(1.0, float(os.getenv("DAILY_FETCH_TIMEOUT_SECONDS", "10")))
except Exception:
  DAILY_FETCH_TIMEOUT_SECONDS = 10.0
TZ_NAME = os.getenv("TZ", "Asia/Shanghai")
LOCAL_TZ = ZoneInfo(TZ_NAME)
APP_VERSION = os.getenv("PHOTOFRAME_ORCHESTRATOR_VERSION", "0.2.4")
DEVICE_CONFIG_MAX_HISTORY = 200
DEVICE_CONFIG_ALLOWED_KEYS = {
    "orchestrator_enabled",
    "orchestrator_base_url",
    "orchestrator_token",
    "image_url_template",
    "photo_token",
    "interval_minutes",
    "retry_base_minutes",
    "retry_max_minutes",
    "max_failure_before_long_sleep",
    "display_rotation",
    "color_process_mode",
    "dither_mode",
    "six_color_tolerance",
    "timezone",
}
DEVICE_CONFIG_SECRET_KEYS = {"orchestrator_token", "photo_token"}

app = FastAPI(title="PhotoFrame Orchestrator", version=APP_VERSION)
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


def _ensure_table_column(conn: sqlite3.Connection, table: str, column: str, ddl: str) -> None:
  existing = {str(row["name"]) for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}
  if column in existing:
    return
  conn.execute(f"ALTER TABLE {table} ADD COLUMN {column} {ddl}")


def _apply_schema_migrations(conn: sqlite3.Connection) -> None:
  _ensure_table_column(conn, "devices", "reported_config_json", "TEXT NOT NULL DEFAULT '{}'")
  _ensure_table_column(conn, "devices", "reported_config_epoch", "INTEGER NOT NULL DEFAULT 0")
  _ensure_table_column(conn, "devices", "battery_mv", "INTEGER NOT NULL DEFAULT -1")
  _ensure_table_column(conn, "devices", "battery_percent", "INTEGER NOT NULL DEFAULT -1")
  _ensure_table_column(conn, "devices", "charging", "INTEGER NOT NULL DEFAULT -1")
  _ensure_table_column(conn, "devices", "vbus_good", "INTEGER NOT NULL DEFAULT -1")


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
          battery_mv INTEGER NOT NULL DEFAULT -1,
          battery_percent INTEGER NOT NULL DEFAULT -1,
          charging INTEGER NOT NULL DEFAULT -1,
          vbus_good INTEGER NOT NULL DEFAULT -1,
          reported_config_json TEXT NOT NULL DEFAULT '{}',
          reported_config_epoch INTEGER NOT NULL DEFAULT 0,
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

        CREATE TABLE IF NOT EXISTS publish_history (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          device_id TEXT NOT NULL,
          issued_epoch INTEGER NOT NULL,
          source TEXT NOT NULL,
          image_url TEXT NOT NULL,
          override_id INTEGER,
          poll_after_seconds INTEGER NOT NULL,
          valid_until_epoch INTEGER NOT NULL,
          created_at INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_publish_history_device_epoch
          ON publish_history (device_id, issued_epoch DESC);

        CREATE TABLE IF NOT EXISTS device_config_plans (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          device_id TEXT NOT NULL,
          config_json TEXT NOT NULL,
          note TEXT NOT NULL DEFAULT '',
          created_epoch INTEGER NOT NULL,
          created_at INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_device_config_plans_device_id
          ON device_config_plans (device_id, id DESC);

        CREATE TABLE IF NOT EXISTS device_config_status (
          device_id TEXT PRIMARY KEY,
          last_query_epoch INTEGER NOT NULL DEFAULT 0,
          last_seen_version INTEGER NOT NULL DEFAULT 0,
          target_version INTEGER NOT NULL DEFAULT 0,
          last_apply_epoch INTEGER NOT NULL DEFAULT 0,
          applied_version INTEGER NOT NULL DEFAULT 0,
          apply_ok INTEGER NOT NULL DEFAULT 0,
          apply_error TEXT NOT NULL DEFAULT '',
          updated_at INTEGER NOT NULL DEFAULT 0
        );
        """
    )
    _apply_schema_migrations(conn)
    conn.commit()


def _now_epoch() -> int:
  return int(time.time())


def _clamp(v: int, low: int, high: int) -> int:
  return max(low, min(high, v))


def _parse_device_token_map() -> dict[str, str]:
  tokens: dict[str, str] = {}

  def normalize_key(raw: str) -> str:
    stripped = raw.strip()
    return stripped if stripped else "*"

  raw_json = (DEVICE_TOKEN_MAP_JSON or "").strip()
  if raw_json:
    try:
      loaded = json.loads(raw_json)
      if isinstance(loaded, dict):
        for key, value in loaded.items():
          if not isinstance(key, str) or not isinstance(value, str):
            continue
          device_id = normalize_key(key)
          token = value.strip()
          if token:
            tokens[device_id] = token
    except Exception:
      # 解析失败时忽略 JSON 来源，继续尝试兼容的 CSV 写法。
      pass

  raw_csv = (DEVICE_TOKEN_MAP or "").strip()
  if raw_csv:
    for pair in raw_csv.split(','):
      if '=' not in pair:
        continue
      key, value = pair.split('=', 1)
      device_id = normalize_key(key)
      token = value.strip()
      if token:
        tokens[device_id] = token

  return tokens


_DEVICE_TOKEN_MAP_PARSED = _parse_device_token_map()


def _secure_equal(provided: str | None, expected: str | None) -> bool:
  if not provided or not expected:
    return False
  return hmac.compare_digest(provided, expected)


def _require_token(header_token: str | None) -> None:
  token = (TOKEN or "").strip()
  if not token:
    return
  provided = (header_token or "").strip()
  if not _secure_equal(provided, token):
    raise HTTPException(status_code=401, detail="invalid token")


def _resolve_device_expected_token(device_id: str) -> str:
  device_key = _normalize_device_id(device_id)
  expected = _DEVICE_TOKEN_MAP_PARSED.get(device_key)
  if expected:
    return expected
  wildcard = _DEVICE_TOKEN_MAP_PARSED.get("*")
  if wildcard:
    return wildcard
  return ""


def _require_device_token(device_id: str, header_token: str | None) -> None:
  provided = (header_token or "").strip()
  expected = _resolve_device_expected_token(device_id)
  if expected:
    if not _secure_equal(provided, expected):
      raise HTTPException(status_code=401, detail="invalid device token")
    return

  # 向后兼容：未配置 device token map 时，沿用原有全局 token。
  _require_token(header_token)


def _require_public_daily_token(header_token: str | None, query_token: str | None) -> None:
  token = (PUBLIC_DAILY_BMP_TOKEN or "").strip()
  if not token:
    raise HTTPException(status_code=403, detail="public daily disabled: set PUBLIC_DAILY_BMP_TOKEN")

  provided = (header_token or query_token or "").strip()
  if not provided or not hmac.compare_digest(provided, token):
    raise HTTPException(status_code=403, detail="public photo token required")


def _daily_image_url(now_epoch: int) -> str:
  date_text = datetime.fromtimestamp(now_epoch, LOCAL_TZ).strftime("%Y-%m-%d")
  url = DAILY_TEMPLATE.replace("%DATE%", date_text)
  if "date=" not in url:
    connector = "&" if "?" in url else "?"
    url = f"{url}{connector}date={date_text}"
  return url


def _fetch_daily_bmp_bytes(url: str) -> bytes:
  req = UrlRequest(url, headers={"User-Agent": f"photoframe-orchestrator/{APP_VERSION}"})
  try:
    with urlopen(req, timeout=DAILY_FETCH_TIMEOUT_SECONDS) as resp:
      status = int(getattr(resp, "status", 200))
      if status != 200:
        raise HTTPException(status_code=502, detail=f"daily upstream status={status}")
      payload = resp.read()
  except HTTPError as exc:
    raise HTTPException(status_code=502, detail=f"daily upstream status={exc.code}") from exc
  except URLError as exc:
    raise HTTPException(status_code=502, detail=f"daily upstream unavailable: {exc}") from exc
  except TimeoutError as exc:
    raise HTTPException(status_code=502, detail="daily upstream timeout") from exc

  if not payload:
    raise HTTPException(status_code=502, detail="daily upstream empty")
  if not payload.startswith(b"BM"):
    raise HTTPException(status_code=502, detail="daily upstream not bmp")
  return payload


def _normalize_device_id(value: str | None) -> str:
  if value is None:
    return "*"
  stripped = value.strip()
  return stripped or "*"


def _decode_config_json(raw: str) -> dict[str, Any]:
  try:
    data = json.loads(raw)
  except Exception:
    return {}
  if isinstance(data, dict):
    return data
  return {}


def _sanitize_device_config(raw: dict[str, Any]) -> dict[str, Any]:
  if not isinstance(raw, dict):
    raise HTTPException(status_code=400, detail="config must be object")

  sanitized: dict[str, Any] = {}
  for key, value in raw.items():
    if key not in DEVICE_CONFIG_ALLOWED_KEYS:
      continue

    if key in {"orchestrator_enabled", "interval_minutes", "retry_base_minutes", "retry_max_minutes",
               "max_failure_before_long_sleep", "display_rotation", "color_process_mode", "dither_mode",
               "six_color_tolerance"}:
      if not isinstance(value, (int, float)):
        raise HTTPException(status_code=400, detail=f"{key} must be number")
      iv = int(value)
      if key == "orchestrator_enabled":
        sanitized[key] = 1 if iv else 0
      elif key == "interval_minutes":
        sanitized[key] = _clamp(iv, 1, 24 * 60)
      elif key == "retry_base_minutes":
        sanitized[key] = _clamp(iv, 1, 24 * 60)
      elif key == "retry_max_minutes":
        sanitized[key] = _clamp(iv, 1, 7 * 24 * 60)
      elif key == "max_failure_before_long_sleep":
        sanitized[key] = _clamp(iv, 1, 1000)
      elif key == "display_rotation":
        sanitized[key] = 0 if iv == 0 else 2
      elif key == "color_process_mode":
        sanitized[key] = _clamp(iv, 0, 2)
      elif key == "dither_mode":
        sanitized[key] = _clamp(iv, 0, 1)
      elif key == "six_color_tolerance":
        sanitized[key] = _clamp(iv, 0, 64)
      continue

    if key in {"orchestrator_base_url", "orchestrator_token", "image_url_template", "photo_token", "timezone"}:
      if not isinstance(value, str):
        raise HTTPException(status_code=400, detail=f"{key} must be string")
      text_val = value.strip()
      max_len = 1024 if key in {"orchestrator_base_url", "image_url_template"} else 256
      if key == "timezone":
        max_len = 64
      sanitized[key] = text_val[:max_len]

  return sanitized

def _sanitize_reported_device_config(raw: Any) -> dict[str, Any]:
  # 设备上报值也走同一套白名单/范围约束，避免脏数据污染控制台提示。
  if not isinstance(raw, dict):
    return {}

  sanitized: dict[str, Any] = {}
  for key in DEVICE_CONFIG_ALLOWED_KEYS:
    if key not in raw:
      continue
    try:
      partial = _sanitize_device_config({key: raw[key]})
    except HTTPException:
      continue
    sanitized.update(partial)
  return sanitized


def _mask_secret(value: str) -> str:
  if not value:
    return ""
  if len(value) <= 4:
    return "*" * len(value)
  return f"{value[:2]}***{value[-2:]}"


def _redact_reported_config_for_view(config: dict[str, Any]) -> dict[str, Any]:
  # Token 等敏感字段仅用于“已设置”提示，返回前统一脱敏。
  redacted = dict(config)
  for key in DEVICE_CONFIG_SECRET_KEYS:
    val = redacted.get(key)
    if isinstance(val, str):
      redacted[key] = _mask_secret(val)
  return redacted



def _load_latest_device_config_plan(conn: sqlite3.Connection, device_id: str) -> sqlite3.Row | None:
  normalized = _normalize_device_id(device_id)
  if normalized == "*":
    return conn.execute(
        """
        SELECT * FROM device_config_plans
        WHERE device_id = '*'
        ORDER BY id DESC
        LIMIT 1
        """
    ).fetchone()

  return conn.execute(
      """
      SELECT * FROM device_config_plans
      WHERE device_id = ? OR device_id = '*'
      ORDER BY CASE WHEN device_id = ? THEN 0 ELSE 1 END, id DESC
      LIMIT 1
      """,
      (normalized, normalized),
  ).fetchone()


def _active_override_for_device(conn: sqlite3.Connection, now_ts: int, device_id: str) -> sqlite3.Row | None:
  normalized = _normalize_device_id(device_id)
  if normalized == "*":
    return conn.execute(
        """
        SELECT * FROM overrides
        WHERE enabled = 1
          AND start_epoch <= ?
          AND end_epoch > ?
          AND device_id = '*'
        ORDER BY created_epoch DESC
        LIMIT 1
        """,
        (now_ts, now_ts),
    ).fetchone()

  return conn.execute(
      """
      SELECT * FROM overrides
      WHERE enabled = 1
        AND start_epoch <= ?
        AND end_epoch > ?
        AND (device_id = ? OR device_id = '*')
      ORDER BY CASE WHEN device_id = ? THEN 0 ELSE 1 END, created_epoch DESC
      LIMIT 1
      """,
      (now_ts, now_ts, normalized, normalized),
  ).fetchone()


def _resolve_current_payload_for_device(
    conn: sqlite3.Connection,
    now_ts: int,
    target_device: str,
) -> tuple[bytes, str]:
  active = _active_override_for_device(conn, now_ts, target_device)
  if active is not None:
    path = ASSET_DIR / str(active["asset_name"])
    if not path.exists():
      raise HTTPException(status_code=502, detail="override asset missing")
    return path.read_bytes(), "override"

  upstream_url = _daily_image_url(now_ts)
  payload = _fetch_daily_bmp_bytes(upstream_url)
  return payload, "daily"


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
  battery_mv: int = -1
  battery_percent: int = -1
  charging: int = -1
  vbus_good: int = -1
  reported_config: dict[str, Any] = Field(default_factory=dict)


class DeviceConfigPublish(BaseModel):
  device_id: str = Field(default="*", min_length=1, max_length=64)
  config: dict[str, Any] = Field(default_factory=dict)
  note: str = Field(default="", max_length=256)


class DeviceConfigApplied(BaseModel):
  device_id: str = Field(min_length=1, max_length=64)
  config_version: int = Field(ge=0)
  applied: bool = True
  error: str = Field(default="", max_length=512)
  applied_epoch: int | None = None


@app.on_event("startup")
def _startup() -> None:
  _init_db()


@app.get("/", response_class=HTMLResponse)
def index() -> str:
  return (APP_DIR / "static" / "index.html").read_text(encoding="utf-8")


@app.get("/healthz")
def healthz() -> dict[str, Any]:
  return {"ok": True, "now_epoch": _now_epoch(), "timezone": TZ_NAME, "app_version": APP_VERSION}


@app.get("/api/v1/assets/{asset_name}")
def asset(asset_name: str) -> FileResponse:
  safe_name = os.path.basename(asset_name)
  path = ASSET_DIR / safe_name
  if not path.exists():
    raise HTTPException(status_code=404, detail="asset not found")
  return FileResponse(path=path, media_type="image/bmp", filename=safe_name)


@app.get("/public/daily.bmp")
def public_daily_bmp(
    token: str | None = Query(default=None),
    device_id: str = Query(default="*", min_length=1, max_length=64),
    x_photo_token: str | None = Header(default=None),
) -> Response:
  _require_public_daily_token(x_photo_token, token)
  now_ts = _now_epoch()
  target_device = _normalize_device_id(device_id)
  conn = _ensure_db()

  payload, source = _resolve_current_payload_for_device(conn, now_ts, target_device)

  return Response(
      content=payload,
      media_type="image/bmp",
      headers={
          "Cache-Control": "private, max-age=60",
          "X-PhotoFrame-Source": source,
          "X-PhotoFrame-Device": target_device,
      },
  )


@app.get("/api/v1/preview/current.bmp")
def preview_current_bmp(
    device_id: str = Query(default="*", min_length=1, max_length=64),
    now_epoch: int | None = Query(default=None),
    x_photoframe_token: str | None = Header(default=None),
) -> Response:
  _require_token(x_photoframe_token)
  now_ts = _now_epoch() if now_epoch is None else now_epoch
  target_device = _normalize_device_id(device_id)
  conn = _ensure_db()

  payload, source = _resolve_current_payload_for_device(conn, now_ts, target_device)
  return Response(
      content=payload,
      media_type="image/bmp",
      headers={
          "Cache-Control": "no-store",
          "X-PhotoFrame-Source": source,
          "X-PhotoFrame-Device": target_device,
      },
  )


@app.get("/api/v1/device/next")
def device_next(
    request: Request,
    device_id: str = Query(..., min_length=1, max_length=64),
    now_epoch: int | None = Query(default=None),
    default_poll_seconds: int = Query(default=DEFAULT_POLL_SECONDS),
    failure_count: int = Query(default=0),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_device_token(device_id, x_photoframe_token)

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

  with DB_LOCK:
    conn.execute(
        """
        INSERT INTO publish_history (
          device_id, issued_epoch, source, image_url, override_id,
          poll_after_seconds, valid_until_epoch, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            device_id,
            now_ts,
            source,
            image_url,
            active_override_id,
            int(poll_sec),
            int(valid_until),
            now_ts,
        ),
    )
    # 控制历史表体积，保留最近 5000 条即可满足家庭场景追溯需求。
    conn.execute(
        """
        DELETE FROM publish_history
        WHERE id IN (
          SELECT id FROM publish_history
          ORDER BY id DESC
          LIMIT -1 OFFSET 5000
        )
        """
    )
    conn.commit()

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
  _require_device_token(payload.device_id, x_photoframe_token)

  now_ts = _now_epoch()
  reported_config = _sanitize_reported_device_config(payload.reported_config)
  reported_config_json = json.dumps(reported_config, ensure_ascii=False)

  conn = _ensure_db()
  with DB_LOCK:
    conn.execute(
        """
        INSERT INTO devices (
          device_id, last_checkin_epoch, next_wakeup_epoch, sleep_seconds,
          poll_interval_seconds, failure_count, last_http_status, fetch_ok,
          image_changed, image_source, last_error, battery_mv, battery_percent,
          charging, vbus_good, reported_config_json, reported_config_epoch, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
          battery_mv = excluded.battery_mv,
          battery_percent = excluded.battery_percent,
          charging = excluded.charging,
          vbus_good = excluded.vbus_good,
          reported_config_json = excluded.reported_config_json,
          reported_config_epoch = excluded.reported_config_epoch,
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
            int(payload.battery_mv),
            int(payload.battery_percent),
            int(payload.charging),
            int(payload.vbus_good),
            reported_config_json,
            int(payload.checkin_epoch),
            now_ts,
        ),
    )
    conn.commit()

  return {"ok": True}


@app.get("/api/v1/device/config")
def device_config_get(
    device_id: str = Query(..., min_length=1, max_length=64),
    now_epoch: int | None = Query(default=None),
    current_version: int = Query(default=0),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_device_token(device_id, x_photoframe_token)

  now_ts = _now_epoch() if now_epoch is None else now_epoch
  target_device = _normalize_device_id(device_id)
  conn = _ensure_db()

  with DB_LOCK:
    plan = _load_latest_device_config_plan(conn, target_device)
    seen_version = int(current_version)
    target_version = 0 if plan is None else int(plan["id"])

    conn.execute(
        """
        INSERT INTO device_config_status (device_id, last_query_epoch, last_seen_version, target_version, updated_at)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(device_id) DO UPDATE SET
          last_query_epoch = excluded.last_query_epoch,
          last_seen_version = excluded.last_seen_version,
          target_version = excluded.target_version,
          updated_at = excluded.updated_at
        """,
        (target_device, now_ts, seen_version, target_version, now_ts),
    )
    conn.commit()

  config: dict[str, Any] = {}
  note = ""
  if plan is not None:
    config = _decode_config_json(str(plan["config_json"]))
    note = str(plan["note"])

  return {
      "device_id": target_device,
      "server_epoch": now_ts,
      "config_version": target_version,
      "config": config,
      "note": note,
  }


@app.post("/api/v1/device/config/applied")
def device_config_applied(
    payload: DeviceConfigApplied,
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_device_token(payload.device_id, x_photoframe_token)

  now_ts = _now_epoch()
  applied_epoch = now_ts if payload.applied_epoch is None else int(payload.applied_epoch)
  target_device = _normalize_device_id(payload.device_id)
  conn = _ensure_db()

  with DB_LOCK:
    conn.execute(
        """
        INSERT INTO device_config_status (
          device_id, last_apply_epoch, applied_version, apply_ok, apply_error,
          updated_at
        ) VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(device_id) DO UPDATE SET
          last_apply_epoch = excluded.last_apply_epoch,
          applied_version = excluded.applied_version,
          apply_ok = excluded.apply_ok,
          apply_error = excluded.apply_error,
          updated_at = excluded.updated_at
        """,
        (
            target_device,
            applied_epoch,
            int(payload.config_version),
            1 if payload.applied else 0,
            (payload.error or "")[:512],
            now_ts,
        ),
    )
    conn.commit()

  return {"ok": True}


@app.post("/api/v1/device-config")
def device_config_publish(
    payload: DeviceConfigPublish,
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  device_id = _normalize_device_id(payload.device_id)
  config = _sanitize_device_config(payload.config)
  now_ts = _now_epoch()
  conn = _ensure_db()

  with DB_LOCK:
    cursor = conn.execute(
        """
        INSERT INTO device_config_plans (device_id, config_json, note, created_epoch, created_at)
        VALUES (?, ?, ?, ?, ?)
        """,
        (device_id, json.dumps(config, ensure_ascii=False), payload.note or "", now_ts, now_ts),
    )
    plan_id = int(cursor.lastrowid)

    # 控制历史条数
    conn.execute(
        """
        DELETE FROM device_config_plans
        WHERE id IN (
          SELECT id FROM device_config_plans
          WHERE device_id = ?
          ORDER BY id DESC
          LIMIT -1 OFFSET ?
        )
        """,
        (device_id, DEVICE_CONFIG_MAX_HISTORY),
    )

    conn.commit()

  return {
      "ok": True,
      "id": plan_id,
      "device_id": device_id,
      "created_epoch": now_ts,
      "config": config,
  }


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

  status_rows = conn.execute("SELECT * FROM device_config_status").fetchall()
  status_map = {str(row["device_id"]): row for row in status_rows}

  items: list[dict[str, Any]] = []
  for row in rows:
    device_id = str(row["device_id"])
    next_wakeup = int(row["next_wakeup_epoch"])
    eta = max(0, next_wakeup - now_ts) if next_wakeup > 0 else None
    status = status_map.get(device_id)
    latest_plan = _load_latest_device_config_plan(conn, device_id)
    target_version = 0 if latest_plan is None else int(latest_plan["id"])

    reported_config = _decode_config_json(str(row["reported_config_json"]))
    items.append(
        {
            "device_id": device_id,
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
            "battery_mv": int(row["battery_mv"]),
            "battery_percent": int(row["battery_percent"]),
            "charging": int(row["charging"]),
            "vbus_good": int(row["vbus_good"]),
            "reported_config_epoch": int(row["reported_config_epoch"]),
            "reported_config": _redact_reported_config_for_view(reported_config),
            "config_target_version": target_version,
            "config_seen_version": 0 if status is None else int(status["last_seen_version"]),
            "config_last_query_epoch": 0 if status is None else int(status["last_query_epoch"]),
            "config_applied_version": 0 if status is None else int(status["applied_version"]),
            "config_last_apply_epoch": 0 if status is None else int(status["last_apply_epoch"]),
            "config_apply_ok": False if status is None else bool(status["apply_ok"]),
            "config_apply_error": "" if status is None else str(status["apply_error"]),
        }
    )

  return {"now_epoch": now_ts, "devices": items}


@app.get("/api/v1/device-configs")
def device_configs(
    device_id: str | None = Query(default=None),
    limit: int = Query(default=50),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  conn = _ensure_db()
  now_ts = _now_epoch()
  max_rows = _clamp(limit, 1, 200)

  where = ''
  params: list[Any] = []
  if device_id and device_id.strip() and device_id.strip() != '*':
    where = 'WHERE device_id = ?'
    params.append(device_id.strip())

  rows = conn.execute(
      f"""
      SELECT id, device_id, config_json, note, created_epoch
      FROM device_config_plans
      {where}
      ORDER BY id DESC
      LIMIT ?
      """,
      (*params, max_rows),
  ).fetchall()

  items: list[dict[str, Any]] = []
  for row in rows:
    items.append(
        {
            'id': int(row['id']),
            'device_id': row['device_id'],
            'created_epoch': int(row['created_epoch']),
            'note': row['note'],
            'config': _decode_config_json(str(row['config_json'])),
        }
    )

  return {'now_epoch': now_ts, 'count': len(items), 'items': items}


@app.get("/api/v1/publish-history")
def publish_history(
    device_id: str | None = Query(default=None),
    limit: int = Query(default=200),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  max_rows = _clamp(limit, 1, 1000)
  conn = _ensure_db()
  now_ts = _now_epoch()

  where = ""
  params: list[Any] = []
  if device_id and device_id.strip() and device_id.strip() != "*":
    where = "WHERE device_id = ?"
    params.append(device_id.strip())

  rows = conn.execute(
      f"""
      SELECT id, device_id, issued_epoch, source, image_url, override_id,
             poll_after_seconds, valid_until_epoch
      FROM publish_history
      {where}
      ORDER BY issued_epoch DESC, id DESC
      LIMIT ?
      """,
      (*params, max_rows),
  ).fetchall()

  items: list[dict[str, Any]] = []
  for row in rows:
    items.append(
        {
            "id": int(row["id"]),
            "device_id": row["device_id"],
            "issued_epoch": int(row["issued_epoch"]),
            "source": row["source"],
            "image_url": row["image_url"],
            "override_id": None if row["override_id"] is None else int(row["override_id"]),
            "poll_after_seconds": int(row["poll_after_seconds"]),
            "valid_until_epoch": int(row["valid_until_epoch"]),
        }
    )

  return {
      "now_epoch": now_ts,
      "count": len(items),
      "items": items,
  }


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

  target_device = _normalize_device_id(device_id)
  explicit_start = starts_at is not None and starts_at.strip() != ""
  start_epoch = _parse_start_epoch(starts_at)
  start_policy = "explicit" if explicit_start else "immediate"

  if not explicit_start and target_device != "*":
    next_wakeup = _device_next_wakeup(target_device)
    if next_wakeup is not None and next_wakeup > start_epoch:
      # 默认按“设备下一次可拉取时刻”开始计时，避免窗口在设备睡眠期内被消耗完。
      start_epoch = next_wakeup
      start_policy = "next_wakeup"

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
  will_expire_before_effective = (
      expected_effective_epoch is not None and expected_effective_epoch >= end_epoch
  )
  image_url = f"{_public_base(request)}/api/v1/assets/{asset_name}"

  return {
      "ok": True,
      "id": override_id,
      "device_id": target_device,
      "start_epoch": start_epoch,
      "end_epoch": end_epoch,
      "duration_minutes": duration_minutes,
      "start_policy": start_policy,
      "will_expire_before_effective": will_expire_before_effective,
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
