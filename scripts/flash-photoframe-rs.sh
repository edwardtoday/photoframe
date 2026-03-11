#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VENV_PY="${REPO_ROOT}/.venv-host-tools/bin/python"
BUILD_ROOT="${REPO_ROOT}/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release"

PORT="${1:-}"
shift || true
BAUD="115200"
DRY_RUN=0
APP_BIN_OVERRIDE=""

if [[ -n "${1:-}" && "${1:-}" != --* ]]; then
  BAUD="${1}"
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      DRY_RUN=1
      ;;
    --app-bin)
      shift
      APP_BIN_OVERRIDE="${1:-}"
      if [[ -z "${APP_BIN_OVERRIDE}" ]]; then
        echo "[error] --app-bin 需要文件路径" >&2
        exit 5
      fi
      ;;
    *)
      echo "[error] 未知参数: $1" >&2
      echo "用法: scripts/flash-photoframe-rs.sh <serial-port> [baud] [--app-bin <path>] [--dry-run]" >&2
      exit 1
      ;;
  esac
  shift
done

if [[ -z "${PORT}" ]]; then
  echo "用法: scripts/flash-photoframe-rs.sh <serial-port> [baud] [--app-bin <path>] [--dry-run]" >&2
  exit 1
fi

if [[ ! -x "${VENV_PY}" ]]; then
  echo "[error] 未找到 host tools 虚拟环境: ${VENV_PY}" >&2
  echo "[hint] 先执行 scripts/setup-host-tools.sh" >&2
  exit 2
fi

FLASHER_ARGS="$(find "${BUILD_ROOT}/build" -path '*/out/build/flasher_args.json' | head -n 1)"
if [[ -z "${FLASHER_ARGS}" || ! -f "${FLASHER_ARGS}" ]]; then
  echo "[error] 未找到 flasher_args.json；请先执行 scripts/build-photoframe-rs.sh" >&2
  exit 3
fi

if [[ ! -f "${BUILD_ROOT}/bootloader.bin" || ! -f "${BUILD_ROOT}/partition-table.bin" ]]; then
  echo "[error] 缺少 release 刷写产物；请先执行 scripts/build-photoframe-rs.sh" >&2
  exit 4
fi

export PHOTOFRAME_FLASHER_ARGS_JSON="${FLASHER_ARGS}"
export PHOTOFRAME_FLASH_PORT="${PORT}"
export PHOTOFRAME_FLASH_BAUD="${BAUD}"
export PHOTOFRAME_FLASH_DRY_RUN="${DRY_RUN}"
export PHOTOFRAME_FLASH_APP_BIN="${APP_BIN_OVERRIDE}"

"${VENV_PY}" - <<'PY'
import json
import os
import pathlib
import shlex
import subprocess
import sys

flasher_args = pathlib.Path(os.environ["PHOTOFRAME_FLASHER_ARGS_JSON"]).resolve()
build_dir = flasher_args.parent
data = json.loads(flasher_args.read_text(encoding="utf-8"))
extra = data.get("extra_esptool_args", {})

cmd = [
    sys.executable,
    "-m",
    "esptool",
    "--chip",
    str(extra.get("chip") or "esp32s3"),
    "--port",
    os.environ["PHOTOFRAME_FLASH_PORT"],
    "--baud",
    os.environ["PHOTOFRAME_FLASH_BAUD"],
    "--before",
    str(extra.get("before") or "default_reset"),
    "--after",
    str(extra.get("after") or "hard_reset"),
]
if extra.get("stub", True) is False:
    cmd.append("--no-stub")

cmd.append("write_flash")
for item in data.get("write_flash_args", []):
    cmd.append(str(item))

flash_files = data.get("flash_files", {})
app_override = os.environ.get("PHOTOFRAME_FLASH_APP_BIN", "").strip()
if app_override:
    app_offset = str((data.get("app") or {}).get("offset") or "")
    if not app_offset:
        raise SystemExit("[error] flasher_args.json 缺少 app offset，无法覆盖应用镜像")
    flash_files = dict(flash_files)
    flash_files[app_offset] = str(pathlib.Path(app_override).resolve())
    print(f"[info] 使用覆盖应用镜像: {flash_files[app_offset]}")
for offset, rel in sorted(flash_files.items(), key=lambda item: int(item[0], 0)):
    path = pathlib.Path(rel)
    if not path.is_absolute():
        path = (build_dir / rel).resolve()
    cmd.extend([offset, str(path)])

print("[info] 保留 NVS 的分段刷写命令：")
print(" ".join(shlex.quote(part) for part in cmd))
if os.environ.get("PHOTOFRAME_FLASH_DRY_RUN") == "1":
    print("[dry-run] 跳过实际烧录")
    raise SystemExit(0)

subprocess.run(cmd, check=True)
print("[done] 烧录完成（未覆盖 NVS 分区）")
PY
