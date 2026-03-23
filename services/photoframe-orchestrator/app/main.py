import hashlib
import hmac
import io
import json
import logging
import os
import random
import re
import sqlite3
import threading
import time
import math
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any
from urllib.parse import quote
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
DAILY_CACHE_DIR = ASSET_DIR / "daily-cache"
DB_PATH = DATA_DIR / "orchestrator.db"

DEFAULT_DAILY_TEMPLATE = "http://192.168.58.113:8000/image/480x800.jpg?date=%DATE%"
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
APP_VERSION = os.getenv("PHOTOFRAME_ORCHESTRATOR_VERSION", "0.2.8")
DEVICE_CONFIG_MAX_HISTORY = 200
POWER_SAMPLE_DEFAULT_DAYS = 30
POWER_SAMPLE_RETENTION_DAYS = 365
POWER_SAMPLE_RETENTION_SECONDS = POWER_SAMPLE_RETENTION_DAYS * 24 * 3600
try:
  DAILY_ASSET_RETENTION_DAYS = max(1, int(os.getenv("DAILY_ASSET_RETENTION_DAYS", "14")))
except Exception:
  DAILY_ASSET_RETENTION_DAYS = 14
DAILY_ASSET_NAME_RE = re.compile(r"^daily-(\d{4}-\d{2}-\d{2})-.*\.(bmp|jpg)$", re.IGNORECASE)
# 设备在未校时/时钟漂移严重时可能上报 1970 或未来时间，服务端需要兜底。
MIN_VALID_DEVICE_EPOCH = int(os.getenv("MIN_VALID_DEVICE_EPOCH", "1609459200"))  # 2021-01-01 UTC
MAX_FUTURE_DEVICE_SKEW_SECONDS = int(os.getenv("MAX_FUTURE_DEVICE_SKEW_SECONDS", str(7 * 24 * 3600)))
MAX_PAST_DEVICE_SKEW_SECONDS = int(os.getenv("MAX_PAST_DEVICE_SKEW_SECONDS", str(365 * 24 * 3600)))
DEVICE_CONFIG_ALLOWED_KEYS = {
    "orchestrator_enabled",
    "orchestrator_base_url",
    "orchestrator_token",
    "image_url_template",
    "photo_token",
    "wifi_profiles",
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
REPORTED_DEVICE_STRING_FIELDS = {
    "firmware_version": 128,
}
OVERRIDE_DITHER_DEFAULT = "none"
DAILY_DITHER_SETTING_KEY = "daily_dither_algorithm"
PALETTE_PROFILE_SETTING_KEY = "palette_profile"
DAILY_DITHER_DEFAULT = os.getenv("DAILY_DITHER_ALGORITHM", "sierra").strip().lower() or "sierra"
PALETTE_PROFILE_DEFAULT = os.getenv("EPAPER_PALETTE_PROFILE", "reference").strip().lower() or "reference"
PHOTOFRAME_PALETTE: tuple[tuple[int, int, int], ...] = (
    (0, 0, 0),
    (255, 255, 255),
    (255, 255, 0),
    (255, 0, 0),
    (0, 0, 255),
    (0, 255, 0),
)
EPAPER_PALETTE_PROFILES: dict[str, dict[str, Any]] = {
    "reference": {
        "label": "Reference Palette",
        "description": "按 issue 10 的 6 色目标色表做匹配",
        "green_penalty": 1.2,
        "colors": (
            {"name": "black", "rgb": (0, 0, 0), "lab": (5.0, 0.0, 0.0)},
            {"name": "white", "rgb": (255, 255, 255), "lab": (95.0, 0.0, 0.0)},
            {"name": "red", "rgb": (200, 40, 40), "lab": (50.0, 65.0, 50.0)},
            {"name": "yellow", "rgb": (230, 200, 60), "lab": (85.0, -5.0, 70.0)},
            {"name": "blue", "rgb": (40, 80, 160), "lab": (40.0, 5.0, -55.0)},
            {"name": "green", "rgb": (40, 140, 80), "lab": (60.0, -45.0, 35.0)},
        ),
    },
    "measured": {
        "label": "Measured Placeholder",
        "description": "实测色表占位；当前为轻微偏移版，待后续用色卡实测数据替换",
        "green_penalty": 1.15,
        "colors": (
            {"name": "black", "rgb": (12, 10, 10), "lab": (7.0, 0.5, 0.5)},
            {"name": "white", "rgb": (245, 239, 228), "lab": (92.0, 0.5, 4.0)},
            {"name": "red", "rgb": (190, 56, 48), "lab": (51.0, 58.0, 42.0)},
            {"name": "yellow", "rgb": (217, 191, 78), "lab": (82.0, -2.0, 63.0)},
            {"name": "blue", "rgb": (55, 92, 152), "lab": (43.0, 1.0, -46.0)},
            {"name": "green", "rgb": (59, 128, 86), "lab": (56.0, -33.0, 21.0)},
        ),
    },
}
EPAPER_TONE_BLACK_POINT = 0.08
EPAPER_TONE_WHITE_POINT = 0.92
EPAPER_TONE_CHROMA_BLEND = 0.08
EPAPER_PAPER_WHITE_RGB = (250, 246, 235)
EPAPER_PAPER_WHITE_HIGHLIGHT_STRENGTH = 0.22
BAYER_4X4: tuple[tuple[int, ...], ...] = (
    (0, 8, 2, 10),
    (12, 4, 14, 6),
    (3, 11, 1, 9),
    (15, 7, 13, 5),
)
BAYER_STRENGTH = 5.0
DITHER_ALGORITHM_SPECS: dict[str, dict[str, Any]] = {
    "none": {
        "label": "保持原图",
        "description": "不做服务端预抖动，保持当前 24-bit 上传链路",
        "kind": "passthrough",
    },
    "bayer": {
        "label": "Bayer 4x4",
        "description": "有序抖动，颗粒规整，生成速度最快",
        "kind": "ordered",
    },
    "blue-noise-lab-ciede2000": {
        "label": "Blue Noise + Lab CIEDE2000",
        "description": "蓝噪声阈值图配合 Lab/ΔE00 选色，颗粒更随机、方向性更弱",
        "kind": "ordered",
        "matcher": "lab-ciede2000",
        "threshold_map": "blue-noise-8x8",
    },
    "floyd-steinberg": {
        "label": "Floyd-Steinberg",
        "description": "经典误差扩散，边缘锐利",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 7 / 16),
            (-1, 1, 3 / 16),
            (0, 1, 5 / 16),
            (1, 1, 1 / 16),
        ),
    },
    "jarvis": {
        "label": "Jarvis (JJN)",
        "description": "扩散范围更大，层次更平滑",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 7 / 48),
            (2, 0, 5 / 48),
            (-2, 1, 3 / 48),
            (-1, 1, 5 / 48),
            (0, 1, 7 / 48),
            (1, 1, 5 / 48),
            (2, 1, 3 / 48),
            (-2, 2, 1 / 48),
            (-1, 2, 3 / 48),
            (0, 2, 5 / 48),
            (1, 2, 3 / 48),
            (2, 2, 1 / 48),
        ),
    },
    "stucki": {
        "label": "Stucki",
        "description": "误差分配更均匀，细节与噪点平衡",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 8 / 42),
            (2, 0, 4 / 42),
            (-2, 1, 2 / 42),
            (-1, 1, 4 / 42),
            (0, 1, 8 / 42),
            (1, 1, 4 / 42),
            (2, 1, 2 / 42),
            (-2, 2, 1 / 42),
            (-1, 2, 2 / 42),
            (0, 2, 4 / 42),
            (1, 2, 2 / 42),
            (2, 2, 1 / 42),
        ),
    },
    "stucki-serpentine": {
        "label": "Stucki Serpentine",
        "description": "Stucki 核配合蛇形扫描，减少方向性条纹",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 8 / 42),
            (2, 0, 4 / 42),
            (-2, 1, 2 / 42),
            (-1, 1, 4 / 42),
            (0, 1, 8 / 42),
            (1, 1, 4 / 42),
            (2, 1, 2 / 42),
            (-2, 2, 1 / 42),
            (-1, 2, 2 / 42),
            (0, 2, 4 / 42),
            (1, 2, 2 / 42),
            (2, 2, 1 / 42),
        ),
        "serpentine": True,
    },
    "burkes": {
        "label": "Burkes",
        "description": "扩散核更紧凑，细节稳定，适合照片与中等算力场景",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 8 / 32),
            (2, 0, 4 / 32),
            (-2, 1, 2 / 32),
            (-1, 1, 4 / 32),
            (0, 1, 8 / 32),
            (1, 1, 4 / 32),
            (2, 1, 2 / 32),
        ),
    },
    "atkinson": {
        "label": "Atkinson",
        "description": "对比更强，颗粒感明显",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 1 / 8),
            (2, 0, 1 / 8),
            (-1, 1, 1 / 8),
            (0, 1, 1 / 8),
            (1, 1, 1 / 8),
            (0, 2, 1 / 8),
        ),
    },
    "sierra": {
        "label": "Sierra",
        "description": "平衡层次与稳定性，适合照片",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 5 / 32),
            (2, 0, 3 / 32),
            (-2, 1, 2 / 32),
            (-1, 1, 4 / 32),
            (0, 1, 5 / 32),
            (1, 1, 4 / 32),
            (2, 1, 2 / 32),
            (-1, 2, 2 / 32),
            (0, 2, 3 / 32),
            (1, 2, 2 / 32),
        ),
    },
    "sierra-lite": {
        "label": "Sierra Lite (2-4A)",
        "description": "低成本扩散，速度快，适合快速预览与较轻颗粒感",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 2 / 4),
            (-1, 1, 1 / 4),
            (0, 1, 1 / 4),
        ),
    },
    "lab-ciede2000": {
        "label": "Lab + CIEDE2000",
        "description": "六色电子纸匹配，基于 Lab/ΔE00 选色并对绿色轻微惩罚",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 8 / 42),
            (2, 0, 4 / 42),
            (-2, 1, 2 / 42),
            (-1, 1, 4 / 42),
            (0, 1, 8 / 42),
            (1, 1, 4 / 42),
            (2, 1, 2 / 42),
            (-2, 2, 1 / 42),
            (-1, 2, 2 / 42),
            (0, 2, 4 / 42),
            (1, 2, 2 / 42),
            (2, 2, 1 / 42),
        ),
        "matcher": "lab-ciede2000",
    },
    "tone-lab-ciede2000": {
        "label": "Tone + Lab CIEDE2000",
        "description": "先做轻量明暗压缩，再按 Lab/ΔE00 选色，减少纸白极限下的层次丢失",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 8 / 42),
            (2, 0, 4 / 42),
            (-2, 1, 2 / 42),
            (-1, 1, 4 / 42),
            (0, 1, 8 / 42),
            (1, 1, 4 / 42),
            (2, 1, 2 / 42),
            (-2, 2, 1 / 42),
            (-1, 2, 2 / 42),
            (0, 2, 4 / 42),
            (1, 2, 2 / 42),
            (2, 2, 1 / 42),
        ),
        "matcher": "lab-ciede2000",
        "preprocess": "tone-compress",
    },
    "paperwhite-lab-ciede2000": {
        "label": "Paper White + Lab CIEDE2000",
        "description": "对高光做纸白补偿后再按 Lab/ΔE00 选色，模拟彩墨屏底色与反射白",
        "kind": "diffusion",
        "kernel": (
            (1, 0, 8 / 42),
            (2, 0, 4 / 42),
            (-2, 1, 2 / 42),
            (-1, 1, 4 / 42),
            (0, 1, 8 / 42),
            (1, 1, 4 / 42),
            (2, 1, 2 / 42),
            (-2, 2, 1 / 42),
            (-1, 2, 2 / 42),
            (0, 2, 4 / 42),
            (1, 2, 2 / 42),
            (2, 2, 1 / 42),
        ),
        "matcher": "lab-ciede2000",
        "preprocess": "paper-white",
    },
}
DAILY_DITHER_ALGORITHM_KEYS: tuple[str, ...] = tuple(
    key for key in DITHER_ALGORITHM_SPECS.keys() if key != OVERRIDE_DITHER_DEFAULT
)

app = FastAPI(title="PhotoFrame Orchestrator", version=APP_VERSION)
app.mount("/static", StaticFiles(directory=APP_DIR / "static"), name="static")

LOGGER = logging.getLogger("uvicorn.error")
DB_LOCK = threading.Lock()
DAILY_CACHE_LOCK = threading.Lock()
DB: sqlite3.Connection | None = None
STALE_CHECKIN_WARNINGS: dict[str, int] = {}


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
  _ensure_table_column(conn, "devices", "sta_ip", "TEXT NOT NULL DEFAULT ''")
  _ensure_table_column(conn, "overrides", "dither_algorithm", f"TEXT NOT NULL DEFAULT '{OVERRIDE_DITHER_DEFAULT}'")
  _ensure_table_column(conn, "publish_history", "dither_algorithm", "TEXT NOT NULL DEFAULT ''")

  conn.execute(
      """
      CREATE TABLE IF NOT EXISTS device_tokens (
        device_id TEXT PRIMARY KEY,
        token_sha256 TEXT NOT NULL,
        approved INTEGER NOT NULL DEFAULT 0,
        first_seen_epoch INTEGER NOT NULL DEFAULT 0,
        last_seen_epoch INTEGER NOT NULL DEFAULT 0,
        approved_epoch INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL DEFAULT 0
      )
      """
  )
  conn.execute(
      """
      CREATE TABLE IF NOT EXISTS service_settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT 0
      )
      """
  )


def _init_db() -> None:
  DATA_DIR.mkdir(parents=True, exist_ok=True)
  ASSET_DIR.mkdir(parents=True, exist_ok=True)
  DAILY_CACHE_DIR.mkdir(parents=True, exist_ok=True)
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
          sta_ip TEXT NOT NULL DEFAULT '',
          battery_mv INTEGER NOT NULL DEFAULT -1,
          battery_percent INTEGER NOT NULL DEFAULT -1,
          charging INTEGER NOT NULL DEFAULT -1,
          vbus_good INTEGER NOT NULL DEFAULT -1,
          reported_config_json TEXT NOT NULL DEFAULT '{}',
          reported_config_epoch INTEGER NOT NULL DEFAULT 0,
          updated_at INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS device_power_samples (
          device_id TEXT NOT NULL,
          sample_epoch INTEGER NOT NULL,
          received_epoch INTEGER NOT NULL,
          battery_mv INTEGER NOT NULL,
          battery_percent INTEGER NOT NULL,
          charging INTEGER NOT NULL,
          vbus_good INTEGER NOT NULL,
          PRIMARY KEY (device_id, sample_epoch)
        );

        CREATE INDEX IF NOT EXISTS idx_power_samples_received_epoch
          ON device_power_samples (received_epoch);

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
    conn.execute(
        """
        INSERT INTO service_settings (key, value, updated_at)
        VALUES (?, ?, ?)
        ON CONFLICT(key) DO NOTHING
        """,
        (DAILY_DITHER_SETTING_KEY, DAILY_DITHER_DEFAULT, _now_epoch()),
    )
    conn.commit()


def _now_epoch() -> int:
  return int(time.time())


def _touch_device_seen(conn: sqlite3.Connection, device_id: str, seen_epoch: int) -> None:
  """记录“服务端最近看到设备”的时间戳。

  说明：
  - /api/v1/device/checkin 与 /api/v1/device/next 会更新 devices.updated_at
  - 但在“仅能访问公网日图 /public/daily.*”的场景下（例如设备 token 失效或只走日图代理），
    仍希望控制台能看到设备活跃，因此 /public/daily.* 也会触发一次轻量的 last_seen 更新。
  """
  target = _normalize_device_id(device_id)
  if target == "*":
    return

  with DB_LOCK:
    conn.execute(
        """
        INSERT INTO devices (device_id, updated_at)
        VALUES (?, ?)
        ON CONFLICT(device_id) DO UPDATE SET
          updated_at = excluded.updated_at
        """,
        (target, int(seen_epoch)),
    )
    conn.commit()


def _record_public_daily_activity(conn: sqlite3.Connection, device_id: str, seen_epoch: int) -> None:
  """记录公网日图访问，并补写一条最近有效电量采样。

  设备若长期只走 `/public/daily.*`，控制台仍应能看到连续的活跃轨迹与电量历史。
  这里复用 `devices` 表中最近一次有效电量值，按本次访问时间写入采样点。
  """
  target = _normalize_device_id(device_id)
  if target == "*":
    return

  with DB_LOCK:
    conn.execute(
        """
        INSERT INTO devices (device_id, updated_at)
        VALUES (?, ?)
        ON CONFLICT(device_id) DO UPDATE SET
          updated_at = excluded.updated_at
        """,
        (target, int(seen_epoch)),
    )
    _upsert_device_power_sample_from_devices(conn, target, int(seen_epoch), int(seen_epoch))
    cutoff = int(seen_epoch) - POWER_SAMPLE_RETENTION_SECONDS
    conn.execute("DELETE FROM device_power_samples WHERE received_epoch < ?", (cutoff,))
    conn.commit()


def _upsert_device_power_sample_from_devices(
    conn: sqlite3.Connection,
    device_id: str,
    sample_epoch: int,
    received_epoch: int,
) -> bool:
  """从 devices 表读取当前有效电源状态并写入 power samples。"""
  target = _normalize_device_id(device_id)
  if target == "*":
    return False

  stored_power = conn.execute(
      "SELECT battery_mv, battery_percent, charging, vbus_good FROM devices WHERE device_id = ?",
      (target,),
  ).fetchone()
  if stored_power is None:
    return False

  stored_battery_mv = int(stored_power["battery_mv"])
  stored_battery_percent = int(stored_power["battery_percent"])
  stored_charging = int(stored_power["charging"])
  stored_vbus_good = int(stored_power["vbus_good"])

  has_power_sample = (
      (stored_battery_mv > 0)
      or (stored_battery_percent >= 0)
      or (stored_charging in (0, 1))
      or (stored_vbus_good in (0, 1))
  )
  if not has_power_sample:
    return False

  conn.execute(
      """
      INSERT INTO device_power_samples (
        device_id, sample_epoch, received_epoch,
        battery_mv, battery_percent, charging, vbus_good
      ) VALUES (?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(device_id, sample_epoch) DO UPDATE SET
        received_epoch = excluded.received_epoch,
        battery_mv = excluded.battery_mv,
        battery_percent = excluded.battery_percent,
        charging = excluded.charging,
        vbus_good = excluded.vbus_good
      """,
      (
          target,
          int(sample_epoch),
          int(received_epoch),
          stored_battery_mv,
          stored_battery_percent,
          stored_charging,
          stored_vbus_good,
      ),
  )
  return True


def _coerce_device_epoch(device_epoch: int | None, server_epoch: int) -> tuple[int, bool]:
  """将设备上报的 epoch 兜底到服务端时间，避免 1970/未来时间污染 UI 与日图选择。"""
  if device_epoch is None:
    return server_epoch, False
  try:
    ts = int(device_epoch)
  except Exception:
    return server_epoch, False
  if ts < MIN_VALID_DEVICE_EPOCH:
    return server_epoch, False
  if ts > server_epoch + MAX_FUTURE_DEVICE_SKEW_SECONDS:
    return server_epoch, False
  if ts < server_epoch - MAX_PAST_DEVICE_SKEW_SECONDS:
    return server_epoch, False
  return ts, True


def _device_epoch_for_view(device_epoch: int | None, now_epoch: int) -> int | None:
  """用于控制台展示：过滤明显不可信的时间戳（如 1970），但不“硬改”为当前时间。"""
  if device_epoch is None:
    return None
  try:
    ts = int(device_epoch)
  except Exception:
    return None
  if ts < MIN_VALID_DEVICE_EPOCH:
    return None
  if ts > now_epoch + MAX_FUTURE_DEVICE_SKEW_SECONDS:
    return None
  return ts


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


def _token_sha256(token: str) -> str:
  return hashlib.sha256(token.encode("utf-8")).hexdigest()


def _token_fingerprint(token: str | None) -> str:
  value = (token or "").strip()
  if not value:
    return "missing"
  digest = _token_sha256(value)
  return digest[:12]


def _register_or_check_device_token(device_id: str, token: str) -> str:
  normalized = _normalize_device_id(device_id)
  if normalized == "*":
    return "invalid"

  token_sha = _token_sha256(token)
  now_ts = _now_epoch()
  conn = _ensure_db()

  with DB_LOCK:
    row = conn.execute(
        "SELECT token_sha256, approved FROM device_tokens WHERE device_id = ?",
        (normalized,),
    ).fetchone()

    if row is None:
      conn.execute(
          """
          INSERT INTO device_tokens (
            device_id, token_sha256, approved,
            first_seen_epoch, last_seen_epoch, approved_epoch, updated_at
          ) VALUES (?, ?, 0, ?, ?, 0, ?)
          """,
          (normalized, token_sha, now_ts, now_ts, now_ts),
      )
      conn.commit()
      return "pending"

    approved = int(row["approved"]) == 1
    stored_sha = str(row["token_sha256"])

    if approved:
      if _secure_equal(token_sha, stored_sha):
        conn.execute(
            "UPDATE device_tokens SET last_seen_epoch = ?, updated_at = ? WHERE device_id = ?",
            (now_ts, now_ts, normalized),
        )
        conn.commit()
        return "ok"
      return "invalid"

    # 未审批状态允许覆盖为最新 token，方便设备侧重置后重新发起配对。
    conn.execute(
        """
        UPDATE device_tokens
        SET token_sha256 = ?, last_seen_epoch = ?, updated_at = ?
        WHERE device_id = ?
        """,
        (token_sha, now_ts, now_ts, normalized),
    )
    conn.commit()
    return "pending"


def _list_device_tokens(only_pending: bool = False) -> list[dict[str, Any]]:
  conn = _ensure_db()
  where = "WHERE approved = 0" if only_pending else ""
  rows = conn.execute(
      f"""
      SELECT device_id, approved, first_seen_epoch, last_seen_epoch, approved_epoch, updated_at
      FROM device_tokens
      {where}
      ORDER BY approved ASC, last_seen_epoch DESC, device_id ASC
      """
  ).fetchall()

  items: list[dict[str, Any]] = []
  for row in rows:
    items.append(
        {
            "device_id": str(row["device_id"]),
            "approved": bool(row["approved"]),
            "first_seen_epoch": int(row["first_seen_epoch"]),
            "last_seen_epoch": int(row["last_seen_epoch"]),
            "approved_epoch": int(row["approved_epoch"]),
            "updated_at": int(row["updated_at"]),
        }
    )
  return items


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
      LOGGER.warning(
          "device auth rejected: device_id=%s reason=device-map-mismatch provided_sha=%s",
          _normalize_device_id(device_id),
          _token_fingerprint(provided),
      )
      raise HTTPException(status_code=401, detail="invalid device token")
    return

  if provided:
    global_token = (TOKEN or "").strip()
    if global_token and _secure_equal(provided, global_token):
      _require_token(header_token)
      return

    status = _register_or_check_device_token(device_id, provided)
    if status == "ok":
      return
    LOGGER.warning(
        "device auth rejected: device_id=%s reason=%s provided_sha=%s",
        _normalize_device_id(device_id),
        status,
        _token_fingerprint(provided),
    )
    if status == "pending":
      raise HTTPException(status_code=401, detail="device token pending approval")
    raise HTTPException(status_code=401, detail="invalid device token")

  # 向后兼容：未配置设备 token 且请求未携带 token 时，沿用全局 token。
  if (TOKEN or "").strip():
    LOGGER.warning(
        "device auth rejected: device_id=%s reason=missing-header-token provided_sha=missing",
        _normalize_device_id(device_id),
    )
  _require_token(header_token)


def _require_public_daily_token(header_token: str | None, query_token: str | None) -> None:
  token = (PUBLIC_DAILY_BMP_TOKEN or "").strip()
  if not token:
    raise HTTPException(status_code=403, detail="public daily disabled: set PUBLIC_DAILY_BMP_TOKEN")

  provided = (header_token or query_token or "").strip()
  if not provided or not hmac.compare_digest(provided, token):
    raise HTTPException(status_code=403, detail="public photo token required")


def _normalize_daily_dither_algorithm(raw: str | None) -> str:
  value = (raw or DAILY_DITHER_DEFAULT).strip().lower()
  if value not in DAILY_DITHER_ALGORITHM_KEYS:
    return DAILY_DITHER_DEFAULT if DAILY_DITHER_DEFAULT in DAILY_DITHER_ALGORITHM_KEYS else DAILY_DITHER_ALGORITHM_KEYS[0]
  return value


def _daily_dither_algorithm_specs() -> list[dict[str, str]]:
  items: list[dict[str, str]] = []
  for key in DAILY_DITHER_ALGORITHM_KEYS:
    spec = DITHER_ALGORITHM_SPECS[key]
    items.append(
        {
            "key": key,
            "label": str(spec["label"]),
            "description": str(spec["description"]),
        }
    )
  return items


def _get_service_setting(conn: sqlite3.Connection, key: str) -> str:
  row = conn.execute("SELECT value FROM service_settings WHERE key = ?", (key,)).fetchone()
  return "" if row is None else str(row["value"] or "")


def _set_service_setting(conn: sqlite3.Connection, key: str, value: str) -> str:
  now_ts = _now_epoch()
  conn.execute(
      """
      INSERT INTO service_settings (key, value, updated_at)
      VALUES (?, ?, ?)
      ON CONFLICT(key) DO UPDATE SET
        value = excluded.value,
        updated_at = excluded.updated_at
      """,
      (key, value, now_ts),
  )
  conn.commit()
  return value


def _get_daily_dither_algorithm() -> str:
  conn = _ensure_db()
  value = _get_service_setting(conn, DAILY_DITHER_SETTING_KEY)
  picked = _normalize_daily_dither_algorithm(value)
  if picked != value:
    with DB_LOCK:
      _set_service_setting(conn, DAILY_DITHER_SETTING_KEY, picked)
  return picked


def _set_daily_dither_algorithm(value: str) -> str:
  picked = _normalize_daily_dither_algorithm(value)
  conn = _ensure_db()
  with DB_LOCK:
    return _set_service_setting(conn, DAILY_DITHER_SETTING_KEY, picked)


def _normalize_palette_profile(raw: str | None) -> str:
  value = (raw or PALETTE_PROFILE_DEFAULT).strip().lower()
  if value in EPAPER_PALETTE_PROFILES:
    return value
  return PALETTE_PROFILE_DEFAULT


def _resolve_palette_profile(raw: str | None) -> str:
  if raw is None:
    return _get_palette_profile()
  return _normalize_palette_profile(raw)


def _get_palette_profile() -> str:
  conn = _ensure_db()
  try:
    value = _get_service_setting(conn, PALETTE_PROFILE_SETTING_KEY)
  except sqlite3.OperationalError:
    return PALETTE_PROFILE_DEFAULT
  picked = _normalize_palette_profile(value)
  if picked != value:
    with DB_LOCK:
        _set_service_setting(conn, PALETTE_PROFILE_SETTING_KEY, picked)
  return picked


def _set_palette_profile(value: str) -> str:
  picked = _normalize_palette_profile(value)
  conn = _ensure_db()
  with DB_LOCK:
    return _set_service_setting(conn, PALETTE_PROFILE_SETTING_KEY, picked)


def _palette_profile_spec(override_profile: str | None = None) -> dict[str, Any]:
  return EPAPER_PALETTE_PROFILES[_resolve_palette_profile(override_profile)]


def _current_epaper_palette_profile(override_profile: str | None = None) -> dict[str, Any]:
  return _palette_profile_spec(override_profile)


def _daily_image_url(now_epoch: int) -> str:
  date_text = datetime.fromtimestamp(now_epoch, LOCAL_TZ).strftime("%Y-%m-%d")
  url = DAILY_TEMPLATE.replace("%DATE%", date_text)
  if "date=" not in url:
    connector = "&" if "?" in url else "?"
    url = f"{url}{connector}date={date_text}"
  return url


def _fetch_daily_source_bytes(url: str) -> bytes:
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


def _sanitize_wifi_profiles(raw: Any) -> list[dict[str, Any]]:
  if not isinstance(raw, list):
    raise HTTPException(status_code=400, detail="wifi_profiles must be array")

  items: list[dict[str, Any]] = []
  seen: set[str] = set()

  for entry in raw:
    ssid = ""
    password: str | None = None
    password_set: bool | None = None

    if isinstance(entry, str):
      ssid = entry.strip()
    elif isinstance(entry, dict):
      ssid_raw = entry.get("ssid")
      if isinstance(ssid_raw, str):
        ssid = ssid_raw.strip()
      pw_raw = entry.get("password")
      if isinstance(pw_raw, str):
        password = pw_raw
      pw_set_raw = entry.get("password_set")
      if isinstance(pw_set_raw, bool):
        password_set = pw_set_raw

    if not ssid:
      continue

    ssid = ssid[:64]
    if ssid in seen:
      continue

    item: dict[str, Any] = {"ssid": ssid}
    if password is not None:
      item["password"] = password[:256]
    if password_set is not None:
      item["password_set"] = bool(password_set)

    items.append(item)
    seen.add(ssid)
    if len(items) >= 3:
      break

  return items


def _sanitize_device_config(raw: dict[str, Any]) -> dict[str, Any]:
  if not isinstance(raw, dict):
    raise HTTPException(status_code=400, detail="config must be object")

  sanitized: dict[str, Any] = {}
  for key, value in raw.items():
    if key not in DEVICE_CONFIG_ALLOWED_KEYS:
      continue

    if key == "wifi_profiles":
      sanitized[key] = _sanitize_wifi_profiles(value)
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

  for key, max_len in REPORTED_DEVICE_STRING_FIELDS.items():
    value = raw.get(key)
    if not isinstance(value, str):
      continue
    text_val = value.strip()
    if text_val == "":
      continue
    sanitized[key] = text_val[:max_len]
  return sanitized


def _mask_secret(value: str) -> str:
  if not value:
    return ""
  if len(value) <= 4:
    return "*" * len(value)
  return f"{value[:2]}***{value[-2:]}"

def _hide_secret(value: str) -> str:
  if not value:
    return ""
  return "<hidden>"


def _redact_reported_config_for_view(config: dict[str, Any]) -> dict[str, Any]:
  # Token 等敏感字段仅用于“已设置”提示，返回前统一脱敏。
  redacted = dict(config)
  for key in DEVICE_CONFIG_SECRET_KEYS:
    val = redacted.get(key)
    if isinstance(val, str):
      redacted[key] = _mask_secret(val)

  wifi_profiles = redacted.get("wifi_profiles")
  if isinstance(wifi_profiles, list):
    safe_profiles: list[dict[str, Any]] = []
    for item in wifi_profiles:
      if not isinstance(item, dict):
        continue
      out = dict(item)
      if isinstance(out.get("password"), str):
        out["password"] = _hide_secret(out.get("password") or "")
      safe_profiles.append(out)
    redacted["wifi_profiles"] = safe_profiles
  return redacted


def _redact_device_config_for_view(config: dict[str, Any]) -> dict[str, Any]:
  # 配置发布历史与控制台回显中，避免直接暴露敏感字段（尤其是 Wi-Fi 密码）。
  redacted = dict(config)

  for key in DEVICE_CONFIG_SECRET_KEYS:
    val = redacted.get(key)
    if isinstance(val, str):
      redacted[key] = _mask_secret(val)

  wifi_profiles = redacted.get("wifi_profiles")
  if isinstance(wifi_profiles, list):
    safe_profiles: list[dict[str, Any]] = []
    for item in wifi_profiles:
      if not isinstance(item, dict):
        continue
      out = dict(item)
      if isinstance(out.get("password"), str):
        out["password"] = _hide_secret(out.get("password") or "")
      safe_profiles.append(out)
    redacted["wifi_profiles"] = safe_profiles

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
    *,
    output_format: str = "bmp",
    daily_dither_algorithm: str | None = None,
    palette_profile: str | None = None,
) -> tuple[bytes, str, str]:
  active = _active_override_for_device(conn, now_ts, target_device)
  if active is not None:
    asset_name = str(active["asset_name"])
    if output_format == "jpg":
      asset_sha256 = str(active["asset_sha256"] or "").strip()
      candidate = f"{asset_sha256}.jpg"
      if asset_sha256 and _asset_path_candidates(candidate)[1].exists():
        asset_name = candidate
    path = _locate_asset_path(asset_name)
    return path.read_bytes(), "override", _normalize_override_dither_algorithm(active["dither_algorithm"])

  upstream_url = _daily_image_url(now_ts)
  picked = _normalize_daily_dither_algorithm(daily_dither_algorithm or _get_daily_dither_algorithm())
  payload = _render_daily_payload(now_ts, upstream_url, output_format, picked, palette_profile=palette_profile)
  return payload, "daily", picked


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


def _normalize_override_dither_algorithm(raw: str | None) -> str:
  value = (raw or OVERRIDE_DITHER_DEFAULT).strip().lower()
  if value == "":
    return OVERRIDE_DITHER_DEFAULT
  if value not in DITHER_ALGORITHM_SPECS:
    raise HTTPException(status_code=400, detail=f"unsupported dither_algorithm: {value}")
  return value


def _preferred_output_format(accept_formats: str | None) -> str:
  accept = {
      item.strip().lower()
      for item in (accept_formats or "").split(",")
      if item.strip() != ""
  }
  if "bmp" in accept or len(accept) == 0:
    return "bmp"
  if "jpeg" in accept or "jpg" in accept:
    return "jpg"
  return "bmp"


def _asset_path_candidates(asset_name: str) -> tuple[Path, ...]:
  safe_name = os.path.basename(asset_name)
  return (
      DAILY_CACHE_DIR / safe_name,
      ASSET_DIR / safe_name,
  )


def _locate_asset_path(asset_name: str) -> Path:
  for candidate in _asset_path_candidates(asset_name):
    if candidate.exists():
      return candidate
  raise HTTPException(status_code=404, detail="asset not found")


def _cleanup_daily_assets(now_ts: int) -> None:
  keep_from = datetime.fromtimestamp(now_ts, LOCAL_TZ).date() - timedelta(days=DAILY_ASSET_RETENTION_DAYS - 1)
  removed = 0

  for root in (DAILY_CACHE_DIR, ASSET_DIR):
    if not root.exists():
      continue
    for path in root.iterdir():
      if not path.is_file():
        continue
      match = DAILY_ASSET_NAME_RE.match(path.name)
      if match is None:
        continue
      try:
        asset_date = datetime.strptime(match.group(1), "%Y-%m-%d").date()
      except ValueError:
        continue
      if asset_date >= keep_from:
        continue
      try:
        path.unlink()
        removed += 1
      except FileNotFoundError:
        continue

  if removed > 0:
    LOGGER.info(
        "daily asset cleanup: removed=%s keep_from=%s retention_days=%s",
        removed,
        keep_from.isoformat(),
        DAILY_ASSET_RETENTION_DAYS,
    )


def _asset_query(device_id: str | None = None, token: str | None = None) -> str:
  parts: list[str] = []
  normalized_device = _normalize_device_id(device_id) if device_id is not None else "*"
  if normalized_device != "*":
    parts.append(f"device_id={quote(normalized_device, safe='')}")

  token_text = (token or "").strip()
  if token_text:
    parts.append(f"token={quote(token_text, safe='')}")

  if not parts:
    return ""
  return f"?{'&'.join(parts)}"


def _require_asset_access(
    device_id: str | None,
    header_token: str | None,
    query_token: str | None,
) -> None:
  provided_header = (header_token or "").strip()
  provided_query = (query_token or "").strip()
  provided_any = provided_header or provided_query
  admin_token = (TOKEN or "").strip()
  if admin_token and _secure_equal(provided_any, admin_token):
    return

  normalized_device = _normalize_device_id(device_id)
  if not admin_token and normalized_device == "*" and not provided_any:
    return
  if normalized_device != "*" and provided_any:
    _require_device_token(normalized_device, provided_any)
    return

  if admin_token:
    raise HTTPException(status_code=401, detail="invalid token")
  raise HTTPException(status_code=401, detail="asset token required")


def _require_admin_or_device_token(device_id: str, header_token: str | None) -> None:
  provided = (header_token or "").strip()
  admin_token = (TOKEN or "").strip()
  if admin_token and _secure_equal(provided, admin_token):
    return
  _require_device_token(device_id, header_token)


def _fit_daily_source_image(source_bytes: bytes) -> Image.Image:
  try:
    with Image.open(io.BytesIO(source_bytes)) as image:
      rgb = image.convert("RGB")
      return ImageOps.fit(rgb, (480, 800), method=Image.Resampling.LANCZOS)
  except Exception as exc:
    raise HTTPException(status_code=502, detail="daily upstream cannot decode image") from exc


def _daily_asset_names(now_ts: int, dither_algorithm: str) -> tuple[str, str]:
  date_text = datetime.fromtimestamp(now_ts, LOCAL_TZ).strftime("%Y-%m-%d")
  suffix = dither_algorithm.replace("/", "-").replace(" ", "-")
  return (
      f"daily-{date_text}-{suffix}.bmp",
      f"daily-{date_text}-{suffix}.jpg",
  )


def _ensure_daily_assets(
    now_ts: int,
    url: str,
    dither_algorithm: str,
    *,
    palette_profile: str | None = None,
) -> tuple[str, str]:
  profile_key = _resolve_palette_profile(palette_profile)
  bmp_name, jpg_name = _daily_asset_names(now_ts, f"{dither_algorithm}-{profile_key}")
  bmp_path = DAILY_CACHE_DIR / bmp_name
  jpg_path = DAILY_CACHE_DIR / jpg_name
  if bmp_path.exists() and jpg_path.exists():
    return bmp_name, jpg_name

  with DAILY_CACHE_LOCK:
    if bmp_path.exists() and jpg_path.exists():
      return bmp_name, jpg_name

    source_bytes = _fetch_daily_source_bytes(url)
    fitted = _fit_daily_source_image(source_bytes)
    bmp_data, jpg_data = _render_override_assets(fitted, dither_algorithm, palette_profile=profile_key)
    DAILY_CACHE_DIR.mkdir(parents=True, exist_ok=True)
    bmp_path.write_bytes(bmp_data)
    jpg_path.write_bytes(jpg_data)
    _cleanup_daily_assets(now_ts)
    LOGGER.info(
        "daily asset refreshed: bmp=%s jpg=%s dither=%s source=%s",
        bmp_name,
        jpg_name,
        f"{dither_algorithm}/{profile_key}",
        url,
    )
  return bmp_name, jpg_name


def _render_daily_payload(
    now_ts: int,
    url: str,
    output_format: str,
    dither_algorithm: str,
    *,
    palette_profile: str | None = None,
) -> bytes:
  bmp_name, jpg_name = _ensure_daily_assets(now_ts, url, dither_algorithm, palette_profile=palette_profile)
  target_name = bmp_name if output_format == "bmp" else jpg_name
  return _locate_asset_path(target_name).read_bytes()


def _clamp_channel(value: float) -> int:
  if value <= 0:
    return 0
  if value >= 255:
    return 255
  return int(round(value))


def _nearest_palette_color(rgb: tuple[float, float, float]) -> tuple[int, int, int]:
  r, g, b = rgb
  best = PHOTOFRAME_PALETTE[0]
  best_distance = math.inf
  for candidate in PHOTOFRAME_PALETTE:
    cr, cg, cb = candidate
    dr = r - cr
    dg = g - cg
    db = b - cb
    distance = dr * dr + dg * dg + db * db
    if distance < best_distance:
      best = candidate
      best_distance = distance
  return best


def _srgb_to_linear(channel: float) -> float:
  value = channel / 255.0
  if value <= 0.04045:
    return value / 12.92
  return ((value + 0.055) / 1.055) ** 2.4


def _rgb_to_lab(rgb: tuple[float, float, float]) -> tuple[float, float, float]:
  r_lin = _srgb_to_linear(rgb[0])
  g_lin = _srgb_to_linear(rgb[1])
  b_lin = _srgb_to_linear(rgb[2])

  x = r_lin * 0.4124564 + g_lin * 0.3575761 + b_lin * 0.1804375
  y = r_lin * 0.2126729 + g_lin * 0.7151522 + b_lin * 0.0721750
  z = r_lin * 0.0193339 + g_lin * 0.1191920 + b_lin * 0.9503041

  xr = x / 0.95047
  yr = y / 1.00000
  zr = z / 1.08883

  def f(value: float) -> float:
    delta = 6 / 29
    if value > delta ** 3:
      return value ** (1 / 3)
    return value / (3 * delta * delta) + 4 / 29

  fx = f(xr)
  fy = f(yr)
  fz = f(zr)
  return (116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz))


def _ciede2000(lab1: tuple[float, float, float], lab2: tuple[float, float, float]) -> float:
  l1, a1, b1 = lab1
  l2, a2, b2 = lab2
  avg_lp = (l1 + l2) / 2.0
  c1 = math.sqrt(a1 * a1 + b1 * b1)
  c2 = math.sqrt(a2 * a2 + b2 * b2)
  avg_c = (c1 + c2) / 2.0
  g = 0.5 * (1.0 - math.sqrt((avg_c ** 7) / ((avg_c ** 7) + (25.0 ** 7)))) if avg_c else 0.0
  a1p = (1.0 + g) * a1
  a2p = (1.0 + g) * a2
  c1p = math.sqrt(a1p * a1p + b1 * b1)
  c2p = math.sqrt(a2p * a2p + b2 * b2)
  avg_cp = (c1p + c2p) / 2.0

  def hp(ap: float, bp: float) -> float:
    if ap == 0.0 and bp == 0.0:
      return 0.0
    angle = math.degrees(math.atan2(bp, ap))
    return angle + 360.0 if angle < 0.0 else angle

  h1p = hp(a1p, b1)
  h2p = hp(a2p, b2)
  delta_lp = l2 - l1
  delta_cp = c2p - c1p

  if c1p == 0.0 or c2p == 0.0:
    delta_hp = 0.0
  else:
    diff = h2p - h1p
    if abs(diff) <= 180.0:
      delta_hp = diff
    elif diff > 180.0:
      delta_hp = diff - 360.0
    else:
      delta_hp = diff + 360.0
  delta_hp_term = 2.0 * math.sqrt(c1p * c2p) * math.sin(math.radians(delta_hp / 2.0))

  if c1p == 0.0 or c2p == 0.0:
    avg_hp = h1p + h2p
  elif abs(h1p - h2p) <= 180.0:
    avg_hp = (h1p + h2p) / 2.0
  elif (h1p + h2p) < 360.0:
    avg_hp = (h1p + h2p + 360.0) / 2.0
  else:
    avg_hp = (h1p + h2p - 360.0) / 2.0

  t = (
      1.0
      - 0.17 * math.cos(math.radians(avg_hp - 30.0))
      + 0.24 * math.cos(math.radians(2.0 * avg_hp))
      + 0.32 * math.cos(math.radians(3.0 * avg_hp + 6.0))
      - 0.20 * math.cos(math.radians(4.0 * avg_hp - 63.0))
  )
  delta_theta = 30.0 * math.exp(-(((avg_hp - 275.0) / 25.0) ** 2))
  rc = 2.0 * math.sqrt((avg_cp ** 7) / ((avg_cp ** 7) + (25.0 ** 7))) if avg_cp else 0.0
  sl = 1.0 + (0.015 * ((avg_lp - 50.0) ** 2)) / math.sqrt(20.0 + ((avg_lp - 50.0) ** 2))
  sc = 1.0 + 0.045 * avg_cp
  sh = 1.0 + 0.015 * avg_cp * t
  rt = -math.sin(math.radians(2.0 * delta_theta)) * rc

  return math.sqrt(
      (delta_lp / sl) ** 2
      + (delta_cp / sc) ** 2
      + (delta_hp_term / sh) ** 2
      + rt * (delta_cp / sc) * (delta_hp_term / sh)
  )


def _nearest_palette_color_lab_ciede2000(
    rgb: tuple[float, float, float],
    palette_profile: str | None = None,
    *,
    profile: dict[str, Any] | None = None,
) -> tuple[int, int, int]:
  profile_spec = _palette_profile_spec(palette_profile) if profile is None else profile
  source_lab = _rgb_to_lab(rgb)
  palette = profile_spec["colors"]
  green_penalty = float(profile_spec["green_penalty"])
  best = palette[0]
  best_distance = math.inf
  for candidate in palette:
    distance = _ciede2000(source_lab, candidate["lab"])
    if candidate["name"] == "green":
      distance *= green_penalty
    if distance < best_distance:
      best = candidate
      best_distance = distance
  return best["rgb"]


def _new_error_row(width: int) -> list[float]:
  return [0.0] * (width * 3)


def _generate_blue_noise_order(size: int) -> tuple[tuple[int, ...], ...]:
  rng = random.Random(0xE1A6)
  all_positions = [(x, y) for y in range(size) for x in range(size)]
  chosen: list[tuple[int, int]] = [rng.choice(all_positions)]
  remaining = {pos for pos in all_positions if pos != chosen[0]}

  def toroidal_distance_sq(p1: tuple[int, int], p2: tuple[int, int]) -> int:
    dx = abs(p1[0] - p2[0])
    dy = abs(p1[1] - p2[1])
    dx = min(dx, size - dx)
    dy = min(dy, size - dy)
    return dx * dx + dy * dy

  while remaining:
    candidate_count = min(48, len(remaining))
    candidates = rng.sample(list(remaining), candidate_count)
    best = candidates[0]
    best_score = -1
    for candidate in candidates:
      score = min(toroidal_distance_sq(candidate, point) for point in chosen)
      if score > best_score:
        best = candidate
        best_score = score
    chosen.append(best)
    remaining.remove(best)

  mask = [[0 for _ in range(size)] for _ in range(size)]
  for index, (x, y) in enumerate(chosen):
    mask[y][x] = index
  return tuple(tuple(row) for row in mask)


BLUE_NOISE_8X8 = _generate_blue_noise_order(8)


def _pixel_list(image: Image.Image) -> list[tuple[int, int, int]]:
  width, height = image.size
  pixels = image.load()
  return [pixels[x, y] for y in range(height) for x in range(width)]


def _apply_bayer_dither(
    image: Image.Image,
    matcher: callable = _nearest_palette_color,
    threshold_map: tuple[tuple[int, ...], ...] = BAYER_4X4,
    strength: float = BAYER_STRENGTH,
) -> Image.Image:
  width, height = image.size
  source_pixels = _pixel_list(image)
  output: list[tuple[int, int, int]] = []
  map_height = len(threshold_map)
  map_width = len(threshold_map[0])
  mid = (map_width * map_height) / 2

  for y in range(height):
    row_offset = y * width
    for x in range(width):
      r0, g0, b0 = source_pixels[row_offset + x]
      threshold = threshold_map[y % map_height][x % map_width] - mid
      delta = threshold * strength
      adjusted = (
          _clamp_channel(r0 + delta),
          _clamp_channel(g0 + delta),
          _clamp_channel(b0 + delta),
      )
      output.append(matcher(adjusted))

  rendered = Image.new("RGB", image.size)
  rendered.putdata(output)
  return rendered


def _apply_tone_compression(image: Image.Image) -> Image.Image:
  width, height = image.size
  source_pixels = _pixel_list(image)
  output: list[tuple[int, int, int]] = []

  for r0, g0, b0 in source_pixels:
    luma = (0.2126 * r0 + 0.7152 * g0 + 0.0722 * b0) / 255.0
    target_luma = EPAPER_TONE_BLACK_POINT + (EPAPER_TONE_WHITE_POINT - EPAPER_TONE_BLACK_POINT) * luma
    target_luma = max(0.0, min(1.0, target_luma))
    gray = target_luma * 255.0
    if luma > 1e-6:
      gain = target_luma / luma
      scaled = (r0 * gain, g0 * gain, b0 * gain)
    else:
      scaled = (0.0, 0.0, 0.0)
    output.append((
        _clamp_channel(scaled[0] * (1.0 - EPAPER_TONE_CHROMA_BLEND) + gray * EPAPER_TONE_CHROMA_BLEND),
        _clamp_channel(scaled[1] * (1.0 - EPAPER_TONE_CHROMA_BLEND) + gray * EPAPER_TONE_CHROMA_BLEND),
        _clamp_channel(scaled[2] * (1.0 - EPAPER_TONE_CHROMA_BLEND) + gray * EPAPER_TONE_CHROMA_BLEND),
    ))

  rendered = Image.new("RGB", (width, height))
  rendered.putdata(output)
  return rendered


def _apply_paper_white_compensation(image: Image.Image) -> Image.Image:
  width, height = image.size
  source_pixels = _pixel_list(image)
  output: list[tuple[int, int, int]] = []
  paper_r, paper_g, paper_b = EPAPER_PAPER_WHITE_RGB

  for r0, g0, b0 in source_pixels:
    luma = (0.2126 * r0 + 0.7152 * g0 + 0.0722 * b0) / 255.0
    highlight_weight = (max(0.0, luma) ** 1.8) * EPAPER_PAPER_WHITE_HIGHLIGHT_STRENGTH
    output.append((
        _clamp_channel(r0 * (1.0 - highlight_weight) + paper_r * highlight_weight),
        _clamp_channel(g0 * (1.0 - highlight_weight) + paper_g * highlight_weight),
        _clamp_channel(b0 * (1.0 - highlight_weight) + paper_b * highlight_weight),
    ))

  rendered = Image.new("RGB", (width, height))
  rendered.putdata(output)
  return rendered


def _apply_error_diffusion(
    image: Image.Image,
    kernel: tuple[tuple[int, int, float], ...],
    matcher: callable = _nearest_palette_color,
    serpentine: bool = False,
) -> Image.Image:
  width, height = image.size
  source_pixels = _pixel_list(image)
  max_dy = max(dy for _, dy, _ in kernel)
  error_rows = [_new_error_row(width) for _ in range(max_dy + 1)]
  output: list[tuple[int, int, int]] = [PHOTOFRAME_PALETTE[1]] * (width * height)

  # 误差扩散需要逐像素维护未来 1-2 行的 RGB 残差，这里用紧凑 float buffer 避免引入 numpy。
  for y in range(height):
    row_offset = y * width
    current_errors = error_rows[0]
    reverse = serpentine and (y % 2 == 1)
    x_iter = range(width - 1, -1, -1) if reverse else range(width)
    for x in x_iter:
      source_r, source_g, source_b = source_pixels[row_offset + x]
      base = x * 3
      r = _clamp_channel(source_r + current_errors[base])
      g = _clamp_channel(source_g + current_errors[base + 1])
      b = _clamp_channel(source_b + current_errors[base + 2])
      quantized = matcher((r, g, b))
      output[row_offset + x] = quantized

      err_r = r - quantized[0]
      err_g = g - quantized[1]
      err_b = b - quantized[2]
      if err_r == 0 and err_g == 0 and err_b == 0:
        continue

      for dx, dy, weight in kernel:
        nx = x + (-dx if reverse else dx)
        if nx < 0 or nx >= width or y + dy >= height:
          continue
        target_row = error_rows[dy]
        target_base = nx * 3
        target_row[target_base] += err_r * weight
        target_row[target_base + 1] += err_g * weight
        target_row[target_base + 2] += err_b * weight

    error_rows.pop(0)
    error_rows.append(_new_error_row(width))

  rendered = Image.new("RGB", image.size)
  rendered.putdata(output)
  return rendered


def _apply_override_dither(
    image: Image.Image,
    dither_algorithm: str,
    *,
    palette_profile: str | None = None,
) -> Image.Image:
  normalized = _normalize_override_dither_algorithm(dither_algorithm)
  spec = DITHER_ALGORITHM_SPECS[normalized]
  kind = str(spec["kind"])
  preprocess = str(spec.get("preprocess") or "")
  if preprocess == "tone-compress":
    image = _apply_tone_compression(image)
  elif preprocess == "paper-white":
    image = _apply_paper_white_compensation(image)

  matcher_name = str(spec.get("matcher") or "rgb")
  palette_profile_spec = _palette_profile_spec(palette_profile) if matcher_name == "lab-ciede2000" else None

  if kind == "passthrough":
    return image.copy()
  if kind == "ordered":
    matcher = (
        (lambda rgb: _nearest_palette_color_lab_ciede2000(rgb, profile=palette_profile_spec))
        if matcher_name == "lab-ciede2000"
        else _nearest_palette_color
    )
    threshold_map_name = str(spec.get("threshold_map") or "bayer-4x4")
    threshold_map = BLUE_NOISE_8X8 if threshold_map_name == "blue-noise-8x8" else BAYER_4X4
    return _apply_bayer_dither(image, matcher=matcher, threshold_map=threshold_map)
  if kind == "diffusion":
    kernel = spec.get("kernel")
    if not isinstance(kernel, tuple):
      raise RuntimeError(f"kernel missing for dither algorithm: {normalized}")
    matcher = (
        (lambda rgb: _nearest_palette_color_lab_ciede2000(rgb, profile=palette_profile_spec))
        if matcher_name == "lab-ciede2000"
        else _nearest_palette_color
    )
    return _apply_error_diffusion(image, kernel, matcher=matcher, serpentine=bool(spec.get("serpentine")))
  raise RuntimeError(f"unsupported dither algorithm kind: {kind}")


def _render_override_assets(
    image: Image.Image,
    dither_algorithm: str,
    *,
    palette_profile: str | None = None,
) -> tuple[bytes, bytes]:
  rendered = _apply_override_dither(image, dither_algorithm, palette_profile=palette_profile)

  out_bmp = io.BytesIO()
  rendered.save(out_bmp, format="BMP")
  bmp_data = out_bmp.getvalue()

  jpeg_quality = 85
  try:
    jpeg_quality = int(os.getenv("PHOTOFRAME_ASSET_JPEG_QUALITY", "85"))
  except ValueError:
    jpeg_quality = 85
  jpeg_quality = max(40, min(95, jpeg_quality))

  out_jpg = io.BytesIO()
  rendered.save(out_jpg, format="JPEG", quality=jpeg_quality, optimize=True, progressive=False)
  return bmp_data, out_jpg.getvalue()


def _read_and_convert_bmp(
    upload: UploadFile,
    dither_algorithm: str,
    *,
    palette_profile: str | None = None,
) -> tuple[str, str]:
  raw = upload.file.read()
  if not raw:
    raise HTTPException(status_code=400, detail="empty upload file")

  try:
    with Image.open(io.BytesIO(raw)) as image:
      rgb = image.convert("RGB")
      # 固件侧以 480x800 为基准渲染（另一方向由固件旋转）。服务端统一做裁剪缩放保证设备可直接显示。
      fitted = ImageOps.fit(rgb, (480, 800), method=Image.Resampling.LANCZOS)
      bmp_data, jpg_data = _render_override_assets(
          fitted,
          dither_algorithm,
          palette_profile=_resolve_palette_profile(palette_profile),
      )
  except Exception as exc:  # pragma: no cover
    raise HTTPException(status_code=400, detail="cannot decode image") from exc

  sha256 = hashlib.sha256(bmp_data).hexdigest()
  asset_name = f"{sha256}.bmp"
  out_path = ASSET_DIR / asset_name
  if not out_path.exists():
    out_path.write_bytes(bmp_data)
  jpg_name = f"{sha256}.jpg"
  jpg_path = ASSET_DIR / jpg_name
  if not jpg_path.exists():
    jpg_path.write_bytes(jpg_data)
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


def _maybe_warn_missing_recent_checkin(
    conn: sqlite3.Connection,
    device_id: str,
    server_now: int,
) -> None:
  row = conn.execute(
      """
      SELECT last_checkin_epoch, poll_interval_seconds, last_http_status, fetch_ok, last_error
      FROM devices
      WHERE device_id = ?
      """,
      (device_id,),
  ).fetchone()
  if row is None:
    return

  last_checkin_epoch = int(row["last_checkin_epoch"])
  poll_interval_seconds = max(60, int(row["poll_interval_seconds"]))
  stale_seconds = server_now - last_checkin_epoch
  stale_threshold = poll_interval_seconds * 2
  if last_checkin_epoch <= 0 or stale_seconds < stale_threshold:
    return

  last_warn_epoch = int(STALE_CHECKIN_WARNINGS.get(device_id, 0))
  if last_warn_epoch > 0 and (server_now - last_warn_epoch) < stale_threshold:
    return

  STALE_CHECKIN_WARNINGS[device_id] = server_now
  LOGGER.warning(
      "device next without recent checkin: device_id=%s stale_seconds=%s "
      "last_checkin_epoch=%s poll_interval_seconds=%s last_http_status=%s fetch_ok=%s "
      "last_error=%r",
      device_id,
      stale_seconds,
      last_checkin_epoch,
      poll_interval_seconds,
      int(row["last_http_status"]),
      int(row["fetch_ok"]),
      str(row["last_error"]),
  )


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
  sta_ip: str = Field(default="", max_length=64)
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


class DailyRenderConfigPayload(BaseModel):
  daily_dither_algorithm: str = Field(min_length=1, max_length=64)
  palette_profile: str | None = Field(default=None, min_length=1, max_length=64)


@app.on_event("startup")
def _startup() -> None:
  _init_db()


@app.get("/", response_class=HTMLResponse)
def index() -> str:
  return (APP_DIR / "static" / "index.html").read_text(encoding="utf-8")


@app.get("/healthz")
def healthz() -> dict[str, Any]:
  return {
      "ok": True,
      "now_epoch": _now_epoch(),
      "timezone": TZ_NAME,
      "app_version": APP_VERSION,
      "daily_dither_algorithm": _get_daily_dither_algorithm(),
  }


@app.get("/api/v1/daily-render-config")
def get_daily_render_config(
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)
  return {
      "daily_dither_algorithm": _get_daily_dither_algorithm(),
      "palette_profile": _get_palette_profile(),
      "palette_profiles": [
          {
              "key": key,
              "label": spec["label"],
              "description": spec["description"],
          }
          for key, spec in EPAPER_PALETTE_PROFILES.items()
      ],
      "algorithms": _daily_dither_algorithm_specs(),
  }


@app.post("/api/v1/daily-render-config")
def set_daily_render_config(
    payload: DailyRenderConfigPayload,
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)
  picked = _set_daily_dither_algorithm(payload.daily_dither_algorithm)
  palette_profile = _get_palette_profile() if payload.palette_profile is None else _set_palette_profile(payload.palette_profile)
  return {
      "ok": True,
      "daily_dither_algorithm": picked,
      "palette_profile": palette_profile,
      "palette_profiles": [
          {
              "key": key,
              "label": spec["label"],
              "description": spec["description"],
          }
          for key, spec in EPAPER_PALETTE_PROFILES.items()
      ],
      "algorithms": _daily_dither_algorithm_specs(),
  }


@app.get("/api/v1/device/debug-stage")
def device_debug_stage(
    device_id: str = Query(..., min_length=1, max_length=64),
    stage: str = Query(..., min_length=1, max_length=64),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_device_token(device_id, x_photoframe_token)
  now_ts = _now_epoch()
  LOGGER.warning(
      "device debug stage: device_id=%s stage=%s server_epoch=%s",
      _normalize_device_id(device_id),
      stage,
      now_ts,
  )
  return {"ok": True, "device_id": _normalize_device_id(device_id), "stage": stage, "server_epoch": now_ts}


@app.get("/api/v1/assets/{asset_name}")
def asset(
    asset_name: str,
    device_id: str | None = Query(default=None, min_length=1, max_length=64),
    token: str | None = Query(default=None),
    x_photoframe_token: str | None = Header(default=None),
) -> FileResponse:
  _require_asset_access(device_id, x_photoframe_token, token)
  safe_name = os.path.basename(asset_name)
  path = _locate_asset_path(safe_name)
  ext = path.suffix.lower()
  if ext == ".bmp":
    media_type = "image/bmp"
  elif ext in (".jpg", ".jpeg"):
    media_type = "image/jpeg"
  elif ext == ".png":
    media_type = "image/png"
  else:
    media_type = "application/octet-stream"
  # 资产文件名默认使用内容哈希，属于不可变资源，可大胆缓存。
  return FileResponse(
      path=path,
      media_type=media_type,
      filename=safe_name,
      headers={"Cache-Control": "public, max-age=31536000, immutable"},
  )


@app.get("/public/daily.bmp")
def public_daily_bmp(
    request: Request,
    token: str | None = Query(default=None),
    device_id: str = Query(default="*", min_length=1, max_length=64),
    x_photo_token: str | None = Header(default=None),
) -> Response:
  _require_public_daily_token(x_photo_token, token)
  now_ts = _now_epoch()
  target_device = _normalize_device_id(device_id)
  conn = _ensure_db()

  _record_public_daily_activity(conn, target_device, now_ts)
  payload, source, dither_algorithm = _resolve_current_payload_for_device(
      conn,
      now_ts,
      target_device,
      output_format="bmp",
  )
  etag_value = hashlib.sha256(payload).hexdigest()
  etag = f"\"{etag_value}\""
  headers = {
      "Cache-Control": "private, max-age=60",
      "ETag": etag,
      "X-PhotoFrame-Source": source,
      "X-PhotoFrame-Device": target_device,
  }
  if dither_algorithm:
    headers["X-PhotoFrame-Dither"] = dither_algorithm

  inm = (request.headers.get("if-none-match") or "").strip()
  if inm:
    candidates = [part.strip() for part in inm.split(",") if part.strip()]
    if etag in candidates:
      return Response(
          status_code=304,
          content=b"",
          headers=headers,
      )

  return Response(
      content=payload,
      media_type="image/bmp",
      headers=headers,
  )


@app.get("/public/daily.jpg")
def public_daily_jpg(
    request: Request,
    token: str | None = Query(default=None),
    device_id: str = Query(default="*", min_length=1, max_length=64),
    x_photo_token: str | None = Header(default=None),
) -> Response:
  _require_public_daily_token(x_photo_token, token)
  now_ts = _now_epoch()
  target_device = _normalize_device_id(device_id)
  conn = _ensure_db()

  _record_public_daily_activity(conn, target_device, now_ts)
  jpg_bytes, source, dither_algorithm = _resolve_current_payload_for_device(
      conn,
      now_ts,
      target_device,
      output_format="jpg",
  )

  etag_value = hashlib.sha256(jpg_bytes).hexdigest()
  etag = f"\"{etag_value}\""
  headers = {
      "Cache-Control": "private, max-age=60",
      "ETag": etag,
      "X-PhotoFrame-Source": source,
      "X-PhotoFrame-Device": target_device,
  }
  if dither_algorithm:
    headers["X-PhotoFrame-Dither"] = dither_algorithm
  inm = (request.headers.get("if-none-match") or "").strip()
  if inm:
    candidates = [part.strip() for part in inm.split(",") if part.strip()]
    if etag in candidates:
      return Response(
          status_code=304,
          content=b"",
          headers=headers,
      )

  return Response(
      content=jpg_bytes,
      media_type="image/jpeg",
      headers=headers,
  )


@app.get("/api/v1/preview/current.bmp")
def preview_current_bmp(
    device_id: str = Query(default="*", min_length=1, max_length=64),
    now_epoch: int | None = Query(default=None),
    daily_dither_algorithm: str | None = Query(default=None, max_length=64),
    palette_profile: str | None = Query(default=None, max_length=64),
    x_photoframe_token: str | None = Header(default=None),
) -> Response:
  now_ts = _now_epoch() if now_epoch is None else now_epoch
  target_device = _normalize_device_id(device_id)
  _require_admin_or_device_token(target_device, x_photoframe_token)
  conn = _ensure_db()

  payload, source, dither_algorithm = _resolve_current_payload_for_device(
      conn,
      now_ts,
      target_device,
      output_format="bmp",
      daily_dither_algorithm=daily_dither_algorithm,
      palette_profile=palette_profile,
  )
  headers = {
      "Cache-Control": "no-store",
      "X-PhotoFrame-Source": source,
      "X-PhotoFrame-Device": target_device,
  }
  if dither_algorithm:
    headers["X-PhotoFrame-Dither"] = dither_algorithm
  return Response(
      content=payload,
      media_type="image/bmp",
      headers=headers,
  )


@app.get("/api/v1/preview/current.jpg")
def preview_current_jpg(
    device_id: str = Query(default="*", min_length=1, max_length=64),
    now_epoch: int | None = Query(default=None),
    daily_dither_algorithm: str | None = Query(default=None, max_length=64),
    palette_profile: str | None = Query(default=None, max_length=64),
    x_photoframe_token: str | None = Header(default=None),
) -> Response:
  now_ts = _now_epoch() if now_epoch is None else now_epoch
  target_device = _normalize_device_id(device_id)
  _require_admin_or_device_token(target_device, x_photoframe_token)
  conn = _ensure_db()

  jpg_bytes, source, dither_algorithm = _resolve_current_payload_for_device(
      conn,
      now_ts,
      target_device,
      output_format="jpg",
      daily_dither_algorithm=daily_dither_algorithm,
      palette_profile=palette_profile,
  )

  headers = {
      "Cache-Control": "no-store",
      "X-PhotoFrame-Source": source,
      "X-PhotoFrame-Device": target_device,
  }
  if dither_algorithm:
    headers["X-PhotoFrame-Dither"] = dither_algorithm

  return Response(
      content=jpg_bytes,
      media_type="image/jpeg",
      headers=headers,
  )


@app.get("/api/v1/device/next")
def device_next(
    request: Request,
    device_id: str = Query(..., min_length=1, max_length=64),
    now_epoch: int | None = Query(default=None),
    default_poll_seconds: int = Query(default=DEFAULT_POLL_SECONDS),
    failure_count: int = Query(default=0),
    accept_formats: str | None = Query(default=None, max_length=64),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_device_token(device_id, x_photoframe_token)

  server_now = _now_epoch()
  requested_now = server_now if now_epoch is None else int(now_epoch)
  now_ts, device_clock_ok = _coerce_device_epoch(requested_now, server_now)
  poll_sec = _clamp(default_poll_seconds, 60, 86400)
  preferred_output_format = _preferred_output_format(accept_formats)
  prefer_bmp = preferred_output_format == "bmp"
  prefer_jpeg = preferred_output_format == "jpg"
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
        (device_id, server_now, max(0, failure_count)),
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
    _maybe_warn_missing_recent_checkin(conn, device_id, server_now)
    conn.commit()

  source = "daily"
  valid_until = now_ts + poll_sec
  active_dither_algorithm = _get_daily_dither_algorithm()
  active_palette_profile = _get_palette_profile()
  daily_bmp_name, daily_jpg_name = _ensure_daily_assets(
      now_ts,
      _daily_image_url(now_ts),
      active_dither_algorithm,
      palette_profile=active_palette_profile,
  )
  # 省电优先：daily 先在服务端固化成静态 asset，设备直接拉静态文件，避免每次唤醒都命中动态日图端点。
  if prefer_bmp:
    image_url = f"{_public_base(request)}/api/v1/assets/{daily_bmp_name}{_asset_query(device_id=device_id)}"
  elif prefer_jpeg:
    image_url = f"{_public_base(request)}/api/v1/assets/{daily_jpg_name}{_asset_query(device_id=device_id)}"
  else:
    image_url = f"{_public_base(request)}/api/v1/assets/{daily_bmp_name}{_asset_query(device_id=device_id)}"
  active_override_id = None

  if active is not None:
    source = "override"
    active_override_id = int(active["id"])
    active_dither_algorithm = _normalize_override_dither_algorithm(active["dither_algorithm"])
    valid_until = int(active["end_epoch"])
    asset_name = str(active["asset_name"])
    asset_sha256 = str(active["asset_sha256"])
    chosen = asset_name
    if prefer_jpeg:
      candidate = f"{asset_sha256}.jpg"
      if _asset_path_candidates(candidate)[1].exists():
        chosen = candidate
    image_url = f"{_public_base(request)}/api/v1/assets/{chosen}{_asset_query(device_id=device_id)}"
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
          poll_after_seconds, valid_until_epoch, created_at, dither_algorithm
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            device_id,
            now_ts,
            source,
            image_url,
            active_override_id,
            int(poll_sec),
            int(valid_until),
            server_now,
            active_dither_algorithm,
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
      "server_epoch": server_now,
      "device_epoch": requested_now,
      "device_clock_ok": device_clock_ok,
      "effective_epoch": now_ts,
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
  checkin_epoch, device_clock_ok = _coerce_device_epoch(int(payload.checkin_epoch), now_ts)
  sleep_seconds = max(0, int(payload.sleep_seconds))
  next_wakeup_epoch = int(payload.next_wakeup_epoch)
  # 若设备时钟不可信，则用服务端时间 + sleep_seconds 生成可用的 next_wakeup，避免 UI 显示 1970。
  if (
      (not device_clock_ok)
      or (next_wakeup_epoch <= 0)
      or (next_wakeup_epoch < checkin_epoch)
      or (next_wakeup_epoch > checkin_epoch + max(60, sleep_seconds) + 7 * 24 * 3600)
  ):
    next_wakeup_epoch = checkin_epoch + sleep_seconds
  reported_config = _sanitize_reported_device_config(payload.reported_config)
  reported_config_json = json.dumps(reported_config, ensure_ascii=False)
  sta_ip = (payload.sta_ip or "").strip()[:64]
  battery_mv = int(payload.battery_mv)
  battery_percent = int(payload.battery_percent)
  charging = int(payload.charging)
  vbus_good = int(payload.vbus_good)

  conn = _ensure_db()
  with DB_LOCK:
    conn.execute(
        """
        INSERT INTO devices (
          device_id, last_checkin_epoch, next_wakeup_epoch, sleep_seconds,
          poll_interval_seconds, failure_count, last_http_status, fetch_ok,
          image_changed, image_source, last_error, sta_ip, battery_mv, battery_percent,
          charging, vbus_good, reported_config_json, reported_config_epoch, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
          -- 避免“本次读数缺失(-1/空)”覆盖掉上一轮的有效值，导致控制台突然变成未知。
          sta_ip = CASE WHEN excluded.sta_ip <> '' THEN excluded.sta_ip ELSE sta_ip END,
          battery_mv = CASE WHEN excluded.battery_mv > 0 THEN excluded.battery_mv ELSE battery_mv END,
          battery_percent = CASE WHEN excluded.battery_percent >= 0 THEN excluded.battery_percent ELSE battery_percent END,
          charging = CASE WHEN excluded.charging IN (0, 1) THEN excluded.charging ELSE charging END,
          vbus_good = CASE WHEN excluded.vbus_good IN (0, 1) THEN excluded.vbus_good ELSE vbus_good END,
          reported_config_json = excluded.reported_config_json,
          reported_config_epoch = excluded.reported_config_epoch,
          updated_at = excluded.updated_at
        """,
        (
            payload.device_id,
            checkin_epoch,
            next_wakeup_epoch,
            sleep_seconds,
            max(60, int(payload.poll_interval_seconds)),
            max(0, int(payload.failure_count)),
            int(payload.last_http_status),
            1 if payload.fetch_ok else 0,
            1 if payload.image_changed else 0,
            payload.image_source,
            payload.last_error,
            sta_ip,
            battery_mv,
            battery_percent,
            charging,
            vbus_good,
            reported_config_json,
            checkin_epoch,
            now_ts,
        ),
    )
    # 记录电池采样历史，用于控制台曲线与续航估算。
    #
    # 注意：设备侧在 PMIC/I2C 抽风时可能上报 -1，但服务端 devices 表会“保留上一轮有效值”。
    # 因此这里统一从 devices 表读取“最终有效值”再写入采样，避免曲线断点。
    _upsert_device_power_sample_from_devices(conn, payload.device_id, checkin_epoch, now_ts)

    cutoff = now_ts - POWER_SAMPLE_RETENTION_SECONDS
    conn.execute("DELETE FROM device_power_samples WHERE received_epoch < ?", (cutoff,))
    conn.commit()

  STALE_CHECKIN_WARNINGS.pop(payload.device_id, None)
  LOGGER.info(
      "device checkin accepted: device_id=%s checkin_epoch=%s fetch_ok=%s "
      "last_http_status=%s battery_percent=%s battery_mv=%s charging=%s vbus_good=%s sta_ip=%s",
      payload.device_id,
      checkin_epoch,
      1 if payload.fetch_ok else 0,
      int(payload.last_http_status),
      battery_percent,
      battery_mv,
      charging,
      vbus_good,
      sta_ip,
  )
  return {"ok": True}


@app.get("/api/v1/device/config")
def device_config_get(
    device_id: str = Query(..., min_length=1, max_length=64),
    now_epoch: int | None = Query(default=None),
    current_version: int = Query(default=0),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_device_token(device_id, x_photoframe_token)

  server_now = _now_epoch()
  requested_now = server_now if now_epoch is None else int(now_epoch)
  effective_now, device_clock_ok = _coerce_device_epoch(requested_now, server_now)
  target_device = _normalize_device_id(device_id)
  conn = _ensure_db()

  with DB_LOCK:
    plan = _load_latest_device_config_plan(conn, target_device)
    seen_version = int(current_version)
    target_version = 0 if plan is None else int(plan["id"])

    conn.execute(
        """
        INSERT INTO device_config_status (
          device_id, last_query_epoch, last_seen_version, target_version,
          last_apply_epoch, applied_version, apply_ok, apply_error,
          updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(device_id) DO UPDATE SET
          last_query_epoch = excluded.last_query_epoch,
          last_seen_version = excluded.last_seen_version,
          target_version = excluded.target_version,
          -- 设备在查询时携带 current_version，意味着它“当前运行的配置版本”已生效。
          -- 若 applied_version 尚未更新（例如设备侧回报失败/旧固件不支持 applied 回调），这里做一次隐式对齐。
          last_apply_epoch = CASE
            WHEN applied_version < excluded.last_seen_version THEN excluded.last_query_epoch
            ELSE last_apply_epoch
          END,
          applied_version = CASE
            WHEN applied_version < excluded.last_seen_version THEN excluded.last_seen_version
            ELSE applied_version
          END,
          apply_ok = CASE
            WHEN applied_version < excluded.last_seen_version THEN 1
            ELSE apply_ok
          END,
          apply_error = CASE
            WHEN applied_version < excluded.last_seen_version THEN ''
            ELSE apply_error
          END,
          updated_at = excluded.updated_at
        """,
        (
            target_device,
            server_now,
            seen_version,
            target_version,
            server_now if seen_version > 0 else 0,
            max(0, seen_version),
            1 if seen_version > 0 else 0,
            "",
            server_now,
        ),
    )
    conn.commit()

  config: dict[str, Any] = {}
  note = ""
  if plan is not None:
    config = _decode_config_json(str(plan["config_json"]))
    note = str(plan["note"])

  return {
      "device_id": target_device,
      "server_epoch": server_now,
      "device_epoch": requested_now,
      "device_clock_ok": device_clock_ok,
      "effective_epoch": effective_now,
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
  applied_epoch, _ = _coerce_device_epoch(
      None if payload.applied_epoch is None else int(payload.applied_epoch),
      now_ts,
  )
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
      "config": _redact_device_config_for_view(config),
  }


@app.get("/api/v1/device-tokens")
def device_tokens(
    pending_only: int = Query(default=1),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  now_ts = _now_epoch()
  only_pending = bool(pending_only)
  items = _list_device_tokens(only_pending=only_pending)
  return {"now_epoch": now_ts, "count": len(items), "pending_only": only_pending, "items": items}


@app.post("/api/v1/device-tokens/{device_id}/approve")
def approve_device_token(
    device_id: str,
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  target = _normalize_device_id(device_id)
  if target == "*":
    raise HTTPException(status_code=400, detail="device_id invalid")

  now_ts = _now_epoch()
  conn = _ensure_db()
  with DB_LOCK:
    row = conn.execute(
        "SELECT device_id FROM device_tokens WHERE device_id = ?",
        (target,),
    ).fetchone()
    if row is None:
      raise HTTPException(status_code=404, detail="device token request not found")

    conn.execute(
        """
        UPDATE device_tokens
        SET approved = 1,
            approved_epoch = CASE WHEN approved_epoch > 0 THEN approved_epoch ELSE ? END,
            updated_at = ?
        WHERE device_id = ?
        """,
        (now_ts, now_ts, target),
    )
    conn.commit()

  return {"ok": True, "device_id": target, "approved_epoch": now_ts}


@app.delete("/api/v1/device-tokens/{device_id}")
def delete_device_token(
    device_id: str,
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  target = _normalize_device_id(device_id)
  if target == "*":
    raise HTTPException(status_code=400, detail="device_id invalid")

  conn = _ensure_db()
  with DB_LOCK:
    cur = conn.execute("DELETE FROM device_tokens WHERE device_id = ?", (target,))
    conn.commit()

  if cur.rowcount == 0:
    raise HTTPException(status_code=404, detail="device token not found")
  return {"ok": True, "device_id": target}


@app.get("/api/v1/devices")
def devices(
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)
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

  # 取每台设备最近一次电量采样（即使当前 checkin 读数缺失，也能在控制台做“上次值”兜底展示）。
  last_power_rows = conn.execute(
      """
      SELECT s.device_id, s.sample_epoch, s.battery_mv, s.battery_percent, s.charging, s.vbus_good
      FROM device_power_samples s
      JOIN (
        SELECT device_id, MAX(sample_epoch) AS max_epoch
        FROM device_power_samples
        GROUP BY device_id
      ) t
        ON s.device_id = t.device_id
       AND s.sample_epoch = t.max_epoch
      """
  ).fetchall()
  last_power_map: dict[str, dict[str, int]] = {}
  for row in last_power_rows:
    last_power_map[str(row["device_id"])] = {
        "sample_epoch": int(row["sample_epoch"]),
        "battery_mv": int(row["battery_mv"]),
        "battery_percent": int(row["battery_percent"]),
        "charging": int(row["charging"]),
        "vbus_good": int(row["vbus_good"]),
    }

  items: list[dict[str, Any]] = []
  for row in rows:
    device_id = str(row["device_id"])
    # last_checkin 用服务端“最近看到设备”的时间（updated_at，可能来自 /device/next 或 /device/checkin），
    # 避免设备未校时/时钟漂移时控制台显示 1970 或明显滞后的时间。
    last_seen_epoch = int(row["updated_at"])
    last_seen = _device_epoch_for_view(last_seen_epoch, now_ts)
    last_device_checkin = _device_epoch_for_view(int(row["last_checkin_epoch"]), now_ts)
    next_wakeup = _device_epoch_for_view(int(row["next_wakeup_epoch"]), now_ts)
    eta = max(0, int(next_wakeup) - now_ts) if next_wakeup is not None else None
    status = status_map.get(device_id)
    latest_plan = _load_latest_device_config_plan(conn, device_id)
    target_version = 0 if latest_plan is None else int(latest_plan["id"])

    reported_config = _decode_config_json(str(row["reported_config_json"]))
    firmware_version = str(reported_config.get("firmware_version") or "").strip()
    last_power = last_power_map.get(device_id)
    items.append(
        {
            "device_id": device_id,
            "firmware_version": firmware_version,
            "last_checkin_epoch": last_seen,
            "last_seen_epoch": last_seen,
            "last_device_checkin_epoch": last_device_checkin,
            "next_wakeup_epoch": next_wakeup,
            "eta_seconds": eta,
            "sleep_seconds": int(row["sleep_seconds"]),
            "poll_interval_seconds": int(row["poll_interval_seconds"]),
            "failure_count": int(row["failure_count"]),
            "last_http_status": int(row["last_http_status"]),
            "fetch_ok": bool(row["fetch_ok"]),
            "image_source": row["image_source"],
            "last_error": row["last_error"],
            "sta_ip": str(row["sta_ip"] or ""),
            "battery_mv": int(row["battery_mv"]),
            "battery_percent": int(row["battery_percent"]),
            "charging": int(row["charging"]),
            "vbus_good": int(row["vbus_good"]),
            "last_power_sample_epoch": (
                None if last_power is None else _device_epoch_for_view(last_power["sample_epoch"], now_ts)
            ),
            "last_power_battery_mv": -1 if last_power is None else int(last_power["battery_mv"]),
            "last_power_battery_percent": -1 if last_power is None else int(last_power["battery_percent"]),
            "last_power_charging": -1 if last_power is None else int(last_power["charging"]),
            "last_power_vbus_good": -1 if last_power is None else int(last_power["vbus_good"]),
            "reported_config_epoch": _device_epoch_for_view(int(row["reported_config_epoch"]), now_ts),
            "reported_config": _redact_reported_config_for_view(reported_config),
            "config_target_version": target_version,
            "config_seen_version": 0 if status is None else int(status["last_seen_version"]),
            "config_last_query_epoch": (
                None
                if status is None
                else _device_epoch_for_view(int(status["last_query_epoch"]), now_ts)
            ),
            "config_applied_version": 0 if status is None else int(status["applied_version"]),
            "config_last_apply_epoch": (
                None
                if status is None
                else _device_epoch_for_view(int(status["last_apply_epoch"]), now_ts)
            ),
            "config_apply_ok": False if status is None else bool(status["apply_ok"]),
            "config_apply_error": "" if status is None else str(status["apply_error"]),
        }
    )

  return {"now_epoch": now_ts, "devices": items}


@app.get("/api/v1/power-samples")
def power_samples(
    device_id: str = Query(..., min_length=1, max_length=64),
    from_epoch: int | None = Query(default=None),
    to_epoch: int | None = Query(default=None),
    limit: int = Query(default=5000),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)

  conn = _ensure_db()
  now_ts = _now_epoch()
  target = _normalize_device_id(device_id)
  if target == "*":
    raise HTTPException(status_code=400, detail="device_id invalid")

  start_ts = now_ts - POWER_SAMPLE_DEFAULT_DAYS * 24 * 3600 if from_epoch is None else int(from_epoch)
  end_ts = now_ts if to_epoch is None else int(to_epoch)
  if end_ts < start_ts:
    raise HTTPException(status_code=400, detail="time range invalid")

  max_rows = _clamp(limit, 1, 20000)
  rows = conn.execute(
      """
      SELECT sample_epoch, battery_mv, battery_percent, charging, vbus_good
      FROM device_power_samples
      WHERE device_id = ?
        AND sample_epoch >= ?
        AND sample_epoch <= ?
      ORDER BY sample_epoch ASC
      LIMIT ?
      """,
      (target, start_ts, end_ts, max_rows),
  ).fetchall()

  items: list[dict[str, int]] = []
  for row in rows:
    items.append(
        {
            "sample_epoch": int(row["sample_epoch"]),
            "battery_mv": int(row["battery_mv"]),
            "battery_percent": int(row["battery_percent"]),
            "charging": int(row["charging"]),
            "vbus_good": int(row["vbus_good"]),
        }
    )

  return {
      "now_epoch": now_ts,
      "device_id": target,
      "from_epoch": start_ts,
      "to_epoch": end_ts,
      "count": len(items),
      "items": items,
  }


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
    raw_config = _decode_config_json(str(row['config_json']))
    items.append(
        {
            'id': int(row['id']),
            'device_id': row['device_id'],
            'created_epoch': int(row['created_epoch']),
            'note': row['note'],
            'config': _redact_device_config_for_view(raw_config),
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
             poll_after_seconds, valid_until_epoch, dither_algorithm
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
            "dither_algorithm": row["dither_algorithm"],
        }
    )

  return {
      "now_epoch": now_ts,
      "count": len(items),
      "items": items,
  }


@app.get("/api/v1/overrides")
def overrides(
    now_epoch: int | None = Query(default=None),
    x_photoframe_token: str | None = Header(default=None),
) -> dict[str, Any]:
  _require_token(x_photoframe_token)
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
            "dither_algorithm": _normalize_override_dither_algorithm(row["dither_algorithm"]),
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
    dither_algorithm: str = Form(default=OVERRIDE_DITHER_DEFAULT),
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

  normalized_dither_algorithm = _normalize_override_dither_algorithm(dither_algorithm)
  asset_name, sha256 = _read_and_convert_bmp(
      file,
      normalized_dither_algorithm,
      palette_profile=_get_palette_profile(),
  )
  now_ts = _now_epoch()
  conn = _ensure_db()

  with DB_LOCK:
    cursor = conn.execute(
        """
        INSERT INTO overrides (
          device_id, start_epoch, end_epoch, asset_name, asset_sha256,
          note, created_epoch, dither_algorithm
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            target_device,
            start_epoch,
            end_epoch,
            asset_name,
            sha256,
            note,
            now_ts,
            normalized_dither_algorithm,
        ),
    )
    override_id = int(cursor.lastrowid)
    conn.commit()

  expected_effective_epoch = _guess_effective_epoch(target_device, start_epoch)
  will_expire_before_effective = (
      expected_effective_epoch is not None and expected_effective_epoch >= end_epoch
  )
  image_url = f"{_public_base(request)}/api/v1/assets/{asset_name}{_asset_query(device_id=target_device)}"

  return {
      "ok": True,
      "id": override_id,
      "device_id": target_device,
      "start_epoch": start_epoch,
      "end_epoch": end_epoch,
      "duration_minutes": duration_minutes,
      "dither_algorithm": normalized_dither_algorithm,
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
