import importlib.util
import io
import sys
import tempfile
import types
import unittest
from pathlib import Path

from PIL import Image


MODULE_PATH = Path(__file__).resolve().parents[1] / "app" / "main.py"


class _DummyResponse:

  def __init__(self, *_args, **_kwargs) -> None:
    pass


class _DummyStaticFiles:

  def __init__(self, *_args, **_kwargs) -> None:
    pass


class _DummyHTTPException(Exception):

  def __init__(self, status_code: int, detail: str) -> None:
    super().__init__(detail)
    self.status_code = status_code
    self.detail = detail


class _DummyUploadFile:

  def __init__(self, filename: str, file: io.BytesIO) -> None:
    self.filename = filename
    self.file = file


class _DummyFastAPI:

  def __init__(self, *_args, **_kwargs) -> None:
    pass

  def mount(self, *_args, **_kwargs) -> None:
    return None

  def _decorator(self, *_args, **_kwargs):
    def wrap(func):
      return func
    return wrap

  get = _decorator
  post = _decorator
  delete = _decorator
  on_event = _decorator


def _dummy_param(default=None, **_kwargs):
  return default


def _install_fastapi_stubs() -> None:
  if "fastapi" in sys.modules:
    return

  fastapi_module = types.ModuleType("fastapi")
  fastapi_module.FastAPI = _DummyFastAPI
  fastapi_module.File = _dummy_param
  fastapi_module.Form = _dummy_param
  fastapi_module.Header = _dummy_param
  fastapi_module.HTTPException = _DummyHTTPException
  fastapi_module.Query = _dummy_param
  fastapi_module.Request = object
  fastapi_module.UploadFile = _DummyUploadFile

  responses_module = types.ModuleType("fastapi.responses")
  responses_module.FileResponse = _DummyResponse
  responses_module.HTMLResponse = _DummyResponse
  responses_module.Response = _DummyResponse

  staticfiles_module = types.ModuleType("fastapi.staticfiles")
  staticfiles_module.StaticFiles = _DummyStaticFiles

  sys.modules["fastapi"] = fastapi_module
  sys.modules["fastapi.responses"] = responses_module
  sys.modules["fastapi.staticfiles"] = staticfiles_module


_install_fastapi_stubs()
SPEC = importlib.util.spec_from_file_location("photoframe_orchestrator_main", MODULE_PATH)
if SPEC is None or SPEC.loader is None:  # pragma: no cover
  raise RuntimeError("cannot load photoframe orchestrator main module")
ORCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ORCH)
PALETTE = set(ORCH.PHOTOFRAME_PALETTE)
EPAPER_PALETTE = {tuple(item["rgb"]) for item in ORCH.EPAPER_PALETTE_PROFILES["reference"]["colors"]}
HTTPException = ORCH.HTTPException
UploadFile = ORCH.UploadFile


def _build_gradient_image(size: tuple[int, int] = (24, 24)) -> Image.Image:
  width, height = size
  pixels: list[tuple[int, int, int]] = []
  for y in range(height):
    for x in range(width):
      r = int(255 * x / max(1, width - 1))
      g = int(255 * y / max(1, height - 1))
      b = int(255 * (x + y) / max(1, width + height - 2))
      pixels.append((r, g, b))
  image = Image.new("RGB", size)
  image.putdata(pixels)
  return image


def _encode_png(image: Image.Image) -> bytes:
  out = io.BytesIO()
  image.save(out, format="PNG")
  return out.getvalue()


def _pixel_set(image: Image.Image) -> set[tuple[int, int, int]]:
  pixels = image.load()
  width, height = image.size
  return {pixels[x, y] for y in range(height) for x in range(width)}


class _DummyUrlopenResponse:

  def __init__(self, payload: bytes, status: int = 200) -> None:
    self._payload = payload
    self.status = status

  def read(self) -> bytes:
    return self._payload

  def __enter__(self):
    return self

  def __exit__(self, exc_type, exc, tb) -> bool:
    return False


class DitherAlgorithmTests(unittest.TestCase):

  def test_admin_routes_require_token_when_configured(self) -> None:
    original_token = ORCH.TOKEN
    ORCH.TOKEN = "admin-secret"
    try:
      with self.assertRaises(HTTPException):
        ORCH.get_daily_render_config()
      with self.assertRaises(HTTPException):
        ORCH.devices()
      with self.assertRaises(HTTPException):
        ORCH.overrides()
    finally:
      ORCH.TOKEN = original_token

  def test_asset_route_requires_token_when_configured(self) -> None:
    original_token = ORCH.TOKEN
    ORCH.TOKEN = "admin-secret"
    with tempfile.TemporaryDirectory() as tmp_dir:
      original_asset_dir = ORCH.ASSET_DIR
      ORCH.ASSET_DIR = Path(tmp_dir)
      sample_path = ORCH.ASSET_DIR / "sample.bmp"
      sample_path.write_bytes(b"BMstub")
      try:
        with self.assertRaises(HTTPException):
          ORCH.asset("sample.bmp")
        ORCH.asset("sample.bmp", token="admin-secret")
      finally:
        ORCH.ASSET_DIR = original_asset_dir
        ORCH.TOKEN = original_token

  def test_preview_route_accepts_admin_token_when_device_map_exists(self) -> None:
    original_token = ORCH.TOKEN
    original_device_map = ORCH._DEVICE_TOKEN_MAP_PARSED
    original_render = ORCH._resolve_current_payload_for_device
    ORCH.TOKEN = "admin-secret"
    ORCH._DEVICE_TOKEN_MAP_PARSED = {"pf-demo": "device-secret"}
    ORCH._resolve_current_payload_for_device = lambda *_args, **_kwargs: (b"BMstub", "daily", "jarvis")
    try:
      response = ORCH.preview_current_bmp(device_id="pf-demo", x_photoframe_token="admin-secret")
      self.assertIsNotNone(response)
    finally:
      ORCH.TOKEN = original_token
      ORCH._DEVICE_TOKEN_MAP_PARSED = original_device_map
      ORCH._resolve_current_payload_for_device = original_render

  def test_healthz_exposes_app_git_sha(self) -> None:
    data = ORCH.healthz()

    self.assertEqual(data["app_version"], ORCH.APP_VERSION)
    self.assertEqual(data["app_git_sha"], ORCH.APP_GIT_SHA)
    self.assertTrue(str(data["app_git_sha"]).strip())

  def test_device_checkin_keeps_reported_firmware_version(self) -> None:
    with tempfile.TemporaryDirectory() as tmp_dir:
      tmp_root = Path(tmp_dir)
      original_data_dir = ORCH.DATA_DIR
      original_asset_dir = ORCH.ASSET_DIR
      original_daily_cache_dir = ORCH.DAILY_CACHE_DIR
      original_db_path = ORCH.DB_PATH
      original_db = ORCH.DB

      ORCH.DATA_DIR = tmp_root
      ORCH.ASSET_DIR = tmp_root / "assets"
      ORCH.DAILY_CACHE_DIR = ORCH.ASSET_DIR / "daily-cache"
      ORCH.DB_PATH = tmp_root / "orchestrator.db"
      ORCH.DB = None
      try:
        ORCH._init_db()
        payload = ORCH.DeviceCheckin(
            device_id="pf-demo",
            checkin_epoch=1774200000,
            next_wakeup_epoch=1774203600,
            sleep_seconds=3600,
            poll_interval_seconds=3600,
            failure_count=0,
            last_http_status=200,
            fetch_ok=True,
            image_changed=False,
            image_source="daily",
            last_error="",
            sta_ip="192.168.1.8",
            battery_mv=4100,
            battery_percent=80,
            charging=0,
            vbus_good=0,
            reported_config={
                "orchestrator_enabled": 1,
                "firmware_version": "0.1.0+abcdef12",
            },
        )
        ORCH.device_checkin(payload)
        data = ORCH.devices()
      finally:
        if ORCH.DB is not None:
          ORCH.DB.close()
          ORCH.DB = None
        ORCH.DATA_DIR = original_data_dir
        ORCH.ASSET_DIR = original_asset_dir
        ORCH.DAILY_CACHE_DIR = original_daily_cache_dir
        ORCH.DB_PATH = original_db_path
        ORCH.DB = original_db

      self.assertEqual(len(data["devices"]), 1)
      self.assertEqual(data["devices"][0]["firmware_version"], "0.1.0+abcdef12")
      self.assertEqual(data["devices"][0]["reported_config"]["firmware_version"], "0.1.0+abcdef12")

  def test_preferred_output_format_prefers_bmp_when_device_supports_both(self) -> None:
    self.assertEqual(ORCH._preferred_output_format("jpeg,bmp"), "bmp")
    self.assertEqual(ORCH._preferred_output_format("bmp,jpeg"), "bmp")
    self.assertEqual(ORCH._preferred_output_format("jpeg"), "jpg")
    self.assertEqual(ORCH._preferred_output_format(None), "bmp")

  def test_unknown_dither_algorithm_is_rejected(self) -> None:
    with self.assertRaises(HTTPException):
      ORCH._normalize_override_dither_algorithm("unknown")

  def test_palette_dither_algorithms_output_only_device_palette(self) -> None:
    image = _build_gradient_image()

    for algorithm in ("bayer", "floyd-steinberg", "jarvis", "stucki", "stucki-serpentine", "burkes", "sierra-lite", "atkinson", "sierra"):
      with self.subTest(algorithm=algorithm):
        rendered = ORCH._apply_override_dither(image, algorithm)
        self.assertTrue(_pixel_set(rendered).issubset(PALETTE))

  def test_blue_noise_lab_ciede2000_output_only_issue_palette(self) -> None:
    image = _build_gradient_image()
    rendered = ORCH._apply_override_dither(image, "blue-noise-lab-ciede2000")
    self.assertTrue(_pixel_set(rendered).issubset(EPAPER_PALETTE))

  def test_lab_ciede2000_output_only_issue_palette(self) -> None:
    image = _build_gradient_image()
    rendered = ORCH._apply_override_dither(image, "lab-ciede2000")
    self.assertTrue(_pixel_set(rendered).issubset(EPAPER_PALETTE))

  def test_lab_ciede2000_uses_green_penalty(self) -> None:
    muddy_green = (156, 176, 60)
    picked = ORCH._nearest_palette_color_lab_ciede2000(muddy_green)
    self.assertNotEqual(picked, (40, 140, 80))

  def test_tone_lab_ciede2000_output_only_issue_palette(self) -> None:
    image = _build_gradient_image()
    rendered = ORCH._apply_override_dither(image, "tone-lab-ciede2000")
    self.assertTrue(_pixel_set(rendered).issubset(EPAPER_PALETTE))

  def test_paperwhite_lab_ciede2000_output_only_issue_palette(self) -> None:
    image = _build_gradient_image()
    rendered = ORCH._apply_override_dither(image, "paperwhite-lab-ciede2000")
    self.assertTrue(_pixel_set(rendered).issubset(EPAPER_PALETTE))

  def test_stucki_serpentine_changes_output(self) -> None:
    image = _build_gradient_image()
    normal = ORCH._apply_override_dither(image, "stucki").tobytes()
    serpentine = ORCH._apply_override_dither(image, "stucki-serpentine").tobytes()
    self.assertNotEqual(normal, serpentine)

  def test_upload_conversion_generates_distinct_assets_for_different_algorithms(self) -> None:
    source_image = _build_gradient_image(size=(64, 64))
    source_png = _encode_png(source_image)

    with tempfile.TemporaryDirectory() as tmp_dir:
      original_asset_dir = ORCH.ASSET_DIR
      ORCH.ASSET_DIR = Path(tmp_dir)
      try:
        none_upload = UploadFile(filename="sample.png", file=io.BytesIO(source_png))
        none_asset, none_sha = ORCH._read_and_convert_bmp(none_upload, "none")

        jarvis_upload = UploadFile(filename="sample.png", file=io.BytesIO(source_png))
        jarvis_asset, jarvis_sha = ORCH._read_and_convert_bmp(jarvis_upload, "jarvis")
      finally:
        ORCH.ASSET_DIR = original_asset_dir

      self.assertNotEqual(none_sha, jarvis_sha)
      self.assertTrue((Path(tmp_dir) / none_asset).exists())
      self.assertTrue((Path(tmp_dir) / jarvis_asset).exists())

      with Image.open(Path(tmp_dir) / jarvis_asset) as rendered:
        palette_pixels = _pixel_set(rendered.convert("RGB"))
      self.assertTrue(palette_pixels.issubset(PALETTE))

  def test_render_daily_payload_accepts_jpeg_upstream_and_returns_palette_bmp(self) -> None:
    source_image = _build_gradient_image(size=(96, 96))
    source_jpeg = io.BytesIO()
    source_image.save(source_jpeg, format="JPEG", quality=92)

    with tempfile.TemporaryDirectory() as tmp_dir:
      original_daily_cache_dir = ORCH.DAILY_CACHE_DIR
      original_urlopen = ORCH.urlopen
      ORCH.DAILY_CACHE_DIR = Path(tmp_dir) / "daily-cache"
      ORCH.DAILY_CACHE_DIR.mkdir(parents=True, exist_ok=True)
      ORCH.urlopen = lambda *_args, **_kwargs: _DummyUrlopenResponse(source_jpeg.getvalue())
      try:
        bmp_bytes = ORCH._render_daily_payload(1773910400, "https://example.com/daily.jpg", "bmp", "jarvis")
        jpg_bytes = ORCH._render_daily_payload(1773910400, "https://example.com/daily.jpg", "jpg", "jarvis")
      finally:
        ORCH.DAILY_CACHE_DIR = original_daily_cache_dir
        ORCH.urlopen = original_urlopen

      self.assertTrue(bmp_bytes.startswith(b"BM"))
      with Image.open(io.BytesIO(bmp_bytes)) as rendered_bmp:
        self.assertEqual(rendered_bmp.size, (480, 800))
        self.assertTrue(_pixel_set(rendered_bmp.convert("RGB")).issubset(PALETTE))
      with Image.open(io.BytesIO(jpg_bytes)) as rendered_jpg:
        self.assertEqual(rendered_jpg.size, (480, 800))

  def test_render_daily_payload_uses_saved_palette_profile_when_omitted(self) -> None:
    source_image = _build_gradient_image(size=(96, 96))
    source_jpeg = io.BytesIO()
    source_image.save(source_jpeg, format="JPEG", quality=92)

    with tempfile.TemporaryDirectory() as tmp_dir:
      tmp_root = Path(tmp_dir)
      original_data_dir = ORCH.DATA_DIR
      original_asset_dir = ORCH.ASSET_DIR
      original_daily_cache_dir = ORCH.DAILY_CACHE_DIR
      original_db_path = ORCH.DB_PATH
      original_db = ORCH.DB
      original_urlopen = ORCH.urlopen

      ORCH.DATA_DIR = tmp_root
      ORCH.ASSET_DIR = tmp_root / "assets"
      ORCH.DAILY_CACHE_DIR = tmp_root / "daily-cache"
      ORCH.DB_PATH = tmp_root / "orchestrator.db"
      ORCH.DB = None
      ORCH.urlopen = lambda *_args, **_kwargs: _DummyUrlopenResponse(source_jpeg.getvalue())
      try:
        ORCH._init_db()
        ORCH._set_palette_profile("measured")
        default_bytes = ORCH._render_daily_payload(1773910400, "https://example.com/daily.jpg", "bmp", "lab-ciede2000")
        measured_bytes = ORCH._render_daily_payload(
            1773910400,
            "https://example.com/daily.jpg",
            "bmp",
            "lab-ciede2000",
            palette_profile="measured",
        )
        reference_bytes = ORCH._render_daily_payload(
            1773910400,
            "https://example.com/daily.jpg",
            "bmp",
            "lab-ciede2000",
            palette_profile="reference",
        )
      finally:
        if ORCH.DB is not None:
          ORCH.DB.close()
          ORCH.DB = None
        ORCH.DATA_DIR = original_data_dir
        ORCH.ASSET_DIR = original_asset_dir
        ORCH.DAILY_CACHE_DIR = original_daily_cache_dir
        ORCH.DB_PATH = original_db_path
        ORCH.DB = original_db
        ORCH.urlopen = original_urlopen

      self.assertEqual(default_bytes, measured_bytes)
      self.assertNotEqual(default_bytes, reference_bytes)
      self.assertTrue((tmp_root / "daily-cache" / "daily-2026-03-19-lab-ciede2000-measured.bmp").exists())

  def test_render_daily_payload_fresh_bypasses_existing_daily_cache(self) -> None:
    first_source = Image.new("RGB", (96, 96), (255, 0, 0))
    second_source = Image.new("RGB", (96, 96), (0, 0, 255))
    first_jpeg = io.BytesIO()
    second_jpeg = io.BytesIO()
    first_source.save(first_jpeg, format="JPEG", quality=92)
    second_source.save(second_jpeg, format="JPEG", quality=92)

    with tempfile.TemporaryDirectory() as tmp_dir:
      original_daily_cache_dir = ORCH.DAILY_CACHE_DIR
      original_urlopen = ORCH.urlopen
      ORCH.DAILY_CACHE_DIR = Path(tmp_dir) / "daily-cache"
      ORCH.DAILY_CACHE_DIR.mkdir(parents=True, exist_ok=True)
      try:
        ORCH.urlopen = lambda *_args, **_kwargs: _DummyUrlopenResponse(first_jpeg.getvalue())
        cached_bytes = ORCH._render_daily_payload(1773910400, "https://example.com/daily.jpg", "bmp", "jarvis")

        ORCH.urlopen = lambda *_args, **_kwargs: _DummyUrlopenResponse(second_jpeg.getvalue())
        cached_again = ORCH._render_daily_payload(1773910400, "https://example.com/daily.jpg", "bmp", "jarvis")
        fresh_bytes = ORCH._render_daily_payload_fresh(1773910400, "https://example.com/daily.jpg", "bmp", "jarvis")
      finally:
        ORCH.DAILY_CACHE_DIR = original_daily_cache_dir
        ORCH.urlopen = original_urlopen

      self.assertEqual(cached_again, cached_bytes)
      self.assertNotEqual(fresh_bytes, cached_bytes)
      with Image.open(io.BytesIO(cached_bytes)) as cached_image:
        self.assertEqual(cached_image.getpixel((0, 0)), (255, 0, 0))
      with Image.open(io.BytesIO(fresh_bytes)) as fresh_image:
        self.assertEqual(fresh_image.getpixel((0, 0)), (0, 0, 255))

  def test_upload_conversion_uses_saved_palette_profile_when_omitted(self) -> None:
    source_image = _build_gradient_image(size=(64, 64))
    source_png = _encode_png(source_image)

    with tempfile.TemporaryDirectory() as tmp_dir:
      tmp_root = Path(tmp_dir)
      original_data_dir = ORCH.DATA_DIR
      original_asset_dir = ORCH.ASSET_DIR
      original_daily_cache_dir = ORCH.DAILY_CACHE_DIR
      original_db_path = ORCH.DB_PATH
      original_db = ORCH.DB

      ORCH.DATA_DIR = tmp_root
      ORCH.ASSET_DIR = tmp_root / "assets"
      ORCH.DAILY_CACHE_DIR = ORCH.ASSET_DIR / "daily-cache"
      ORCH.DB_PATH = tmp_root / "orchestrator.db"
      ORCH.DB = None
      try:
        ORCH._init_db()
        ORCH._set_palette_profile("measured")
        default_asset, default_sha = ORCH._read_and_convert_bmp(
            UploadFile(filename="sample.png", file=io.BytesIO(source_png)),
            "lab-ciede2000",
        )
        measured_asset, measured_sha = ORCH._read_and_convert_bmp(
            UploadFile(filename="sample.png", file=io.BytesIO(source_png)),
            "lab-ciede2000",
            palette_profile="measured",
        )
        reference_asset, reference_sha = ORCH._read_and_convert_bmp(
            UploadFile(filename="sample.png", file=io.BytesIO(source_png)),
            "lab-ciede2000",
            palette_profile="reference",
        )
      finally:
        if ORCH.DB is not None:
          ORCH.DB.close()
          ORCH.DB = None
        ORCH.DATA_DIR = original_data_dir
        ORCH.ASSET_DIR = original_asset_dir
        ORCH.DAILY_CACHE_DIR = original_daily_cache_dir
        ORCH.DB_PATH = original_db_path
        ORCH.DB = original_db

      self.assertEqual(default_sha, measured_sha)
      self.assertEqual(default_asset, measured_asset)
      self.assertNotEqual(default_sha, reference_sha)
      self.assertNotEqual(default_asset, reference_asset)

  def test_daily_dither_setting_roundtrip(self) -> None:
    with tempfile.TemporaryDirectory() as tmp_dir:
      tmp_root = Path(tmp_dir)
      original_data_dir = ORCH.DATA_DIR
      original_asset_dir = ORCH.ASSET_DIR
      original_daily_cache_dir = ORCH.DAILY_CACHE_DIR
      original_db_path = ORCH.DB_PATH
      original_db = ORCH.DB

      ORCH.DATA_DIR = tmp_root
      ORCH.ASSET_DIR = tmp_root / "assets"
      ORCH.DAILY_CACHE_DIR = ORCH.ASSET_DIR / "daily-cache"
      ORCH.DB_PATH = tmp_root / "orchestrator.db"
      ORCH.DB = None
      try:
        ORCH._init_db()
        self.assertEqual(ORCH._get_daily_dither_algorithm(), ORCH._normalize_daily_dither_algorithm(ORCH.DAILY_DITHER_DEFAULT))
        self.assertEqual(ORCH._set_daily_dither_algorithm("jarvis"), "jarvis")
        self.assertEqual(ORCH._get_daily_dither_algorithm(), "jarvis")
        self.assertEqual(ORCH._get_palette_profile(), ORCH._normalize_palette_profile(ORCH.PALETTE_PROFILE_DEFAULT))
        self.assertEqual(ORCH._set_palette_profile("reference"), "reference")
        self.assertEqual(ORCH._get_palette_profile(), "reference")
      finally:
        if ORCH.DB is not None:
          ORCH.DB.close()
          ORCH.DB = None
        ORCH.DATA_DIR = original_data_dir
        ORCH.ASSET_DIR = original_asset_dir
        ORCH.DAILY_CACHE_DIR = original_daily_cache_dir
        ORCH.DB_PATH = original_db_path
        ORCH.DB = original_db


if __name__ == "__main__":  # pragma: no cover
  unittest.main()
