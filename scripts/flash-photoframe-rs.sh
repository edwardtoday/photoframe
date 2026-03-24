#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VENV_PY="${PHOTOFRAME_FLASH_VENV_PY_OVERRIDE:-${REPO_ROOT}/.venv-host-tools/bin/python}"
BUILD_ROOT="${PHOTOFRAME_FLASH_BUILD_ROOT_OVERRIDE:-${REPO_ROOT}/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release}"
ELF_ABS="${PHOTOFRAME_FLASH_ELF_ABS_OVERRIDE:-${BUILD_ROOT}/photoframe-firmware-device}"
DEFAULT_APP_BIN="${PHOTOFRAME_FLASH_DEFAULT_APP_BIN_OVERRIDE:-${REPO_ROOT}/firmware/photoframe-rs/dist/photoframe-rs-app.bin}"

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

FLASHER_ARGS="${PHOTOFRAME_FLASHER_ARGS_JSON_OVERRIDE:-}"
if [[ -z "${FLASHER_ARGS}" ]]; then
  FLASHER_ARGS="$(
    python3 - "${BUILD_ROOT}" <<'PY'
import pathlib
import sys

root = pathlib.Path(sys.argv[1]) / "build"
matches = [path for path in root.glob("**/out/build/flasher_args.json") if path.is_file()]
if matches:
    newest = max(matches, key=lambda path: path.stat().st_mtime)
    print(str(newest))
PY
  )"
fi
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
export PHOTOFRAME_FLASH_DEFAULT_APP_BIN="${DEFAULT_APP_BIN}"
export PHOTOFRAME_FLASH_ELF="${ELF_ABS}"

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


def resolve_path(raw: str) -> pathlib.Path:
    path = pathlib.Path(raw).expanduser()
    if not path.is_absolute():
        path = (pathlib.Path.cwd() / path).resolve()
    else:
        path = path.resolve()
    return path

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

flash_files = dict(data.get("flash_files", {}))
app_offset = str((data.get("app") or {}).get("offset") or "")
if not app_offset:
    raise SystemExit("[error] flasher_args.json 缺少 app offset，无法确定应用分区")

app_override = os.environ.get("PHOTOFRAME_FLASH_APP_BIN", "").strip()
if app_override:
    app_image = resolve_path(app_override)
    if not app_image.is_file():
        raise SystemExit(f"[error] 覆盖应用镜像不存在: {app_image}")
    print(f"[info] 使用覆盖应用镜像: {app_image}")
else:
    app_image = resolve_path(os.environ["PHOTOFRAME_FLASH_DEFAULT_APP_BIN"])
    if not app_image.is_file():
        raise SystemExit(
            "[error] 默认应用镜像不存在；请先执行 scripts/build-photoframe-rs.sh "
            f"生成 {app_image}"
        )
    elf_path = resolve_path(os.environ["PHOTOFRAME_FLASH_ELF"])
    if elf_path.is_file() and app_image.stat().st_mtime + 1 < elf_path.stat().st_mtime:
        raise SystemExit(
            "[error] 默认应用镜像早于当前 ELF 产物，可能是陈旧镜像；"
            "请先执行 scripts/build-photoframe-rs.sh 再刷机\n"
            f"  dist app: {app_image}\n"
            f"  current elf: {elf_path}"
        )
    print(f"[info] 默认使用 dist 应用镜像: {app_image}")

flash_files[app_offset] = str(app_image)
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
