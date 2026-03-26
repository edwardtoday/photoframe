#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VENV_PY="${PHOTOFRAME_FLASH_VENV_PY_OVERRIDE:-${REPO_ROOT}/.venv-host-tools/bin/python}"
BUILD_ROOT="${PHOTOFRAME_FLASH_BUILD_ROOT_OVERRIDE:-${REPO_ROOT}/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release}"
ELF_ABS="${PHOTOFRAME_FLASH_ELF_ABS_OVERRIDE:-${BUILD_ROOT}/photoframe-firmware-device}"
DEFAULT_APP_BIN="${PHOTOFRAME_FLASH_DEFAULT_APP_BIN_OVERRIDE:-${REPO_ROOT}/firmware/photoframe-rs/dist/photoframe-rs-app.bin}"
PARTITIONS_CSV="${PHOTOFRAME_FLASH_PARTITIONS_CSV_OVERRIDE:-${REPO_ROOT}/firmware/photoframe-rs/partitions.csv}"
BOOTLOADER_BIN="${PHOTOFRAME_FLASH_BOOTLOADER_BIN_OVERRIDE:-${BUILD_ROOT}/bootloader.bin}"
PARTITION_TABLE_BIN="${PHOTOFRAME_FLASH_PARTITION_TABLE_BIN_OVERRIDE:-${BUILD_ROOT}/partition-table.bin}"
OTADATA_BIN="${PHOTOFRAME_FLASH_OTADATA_BIN_OVERRIDE:-${BUILD_ROOT}/ota_data_initial.bin}"

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

if [[ ! -f "${BOOTLOADER_BIN}" || ! -f "${PARTITION_TABLE_BIN}" || ! -f "${OTADATA_BIN}" ]]; then
  echo "[error] 缺少 release 刷写产物；请先执行 scripts/build-photoframe-rs.sh" >&2
  exit 4
fi

export PHOTOFRAME_FLASH_PORT="${PORT}"
export PHOTOFRAME_FLASH_BAUD="${BAUD}"
export PHOTOFRAME_FLASH_DRY_RUN="${DRY_RUN}"
export PHOTOFRAME_FLASH_APP_BIN="${APP_BIN_OVERRIDE}"
export PHOTOFRAME_FLASH_DEFAULT_APP_BIN="${DEFAULT_APP_BIN}"
export PHOTOFRAME_FLASH_ELF="${ELF_ABS}"
export PHOTOFRAME_FLASH_PARTITIONS_CSV="${PARTITIONS_CSV}"
export PHOTOFRAME_FLASH_BOOTLOADER_BIN="${BOOTLOADER_BIN}"
export PHOTOFRAME_FLASH_PARTITION_TABLE_BIN="${PARTITION_TABLE_BIN}"
export PHOTOFRAME_FLASH_OTADATA_BIN="${OTADATA_BIN}"

"${VENV_PY}" - <<'PY'
import os
import pathlib
import shlex
import subprocess
import sys


def resolve_path(raw: str) -> pathlib.Path:
    path = pathlib.Path(raw).expanduser()
    if not path.is_absolute():
        path = (pathlib.Path.cwd() / path).resolve()
    else:
        path = path.resolve()
    return path


def parse_partitions_csv(path: pathlib.Path) -> tuple[str, str]:
    app_offset = None
    otadata_offset = None
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = [part.strip() for part in line.split(",")]
        if len(parts) < 5:
            continue
        _name, ptype, subtype, offset, _size = parts[:5]
        if ptype == "data" and subtype == "ota" and not otadata_offset:
            otadata_offset = offset
        if ptype == "app" and not app_offset:
            app_offset = offset
    if not app_offset or not otadata_offset:
        raise SystemExit(
            "[error] 无法从 partitions.csv 解析 app/otadata offset；"
            f"请检查 {path}"
        )
    return app_offset, otadata_offset


partitions_csv = resolve_path(os.environ["PHOTOFRAME_FLASH_PARTITIONS_CSV"])
bootloader_bin = resolve_path(os.environ["PHOTOFRAME_FLASH_BOOTLOADER_BIN"])
partition_table_bin = resolve_path(os.environ["PHOTOFRAME_FLASH_PARTITION_TABLE_BIN"])
otadata_bin = resolve_path(os.environ["PHOTOFRAME_FLASH_OTADATA_BIN"])
app_offset, otadata_offset = parse_partitions_csv(partitions_csv)

cmd = [
    sys.executable,
    "-m",
    "esptool",
    "--chip",
    "esp32s3",
    "--port",
    os.environ["PHOTOFRAME_FLASH_PORT"],
    "--baud",
    os.environ["PHOTOFRAME_FLASH_BAUD"],
    "--before",
    "default_reset",
    "--after",
    "hard_reset",
    "write_flash",
    "--flash_mode",
    "dio",
    "--flash_size",
    "16MB",
    "--flash_freq",
    "80m",
]

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

cmd.extend(
    [
        "0x0",
        str(bootloader_bin),
        "0x8000",
        str(partition_table_bin),
        otadata_offset,
        str(otadata_bin),
        app_offset,
        str(app_image),
    ]
)

print("[info] 保留 NVS 的分段刷写命令：")
print(" ".join(shlex.quote(part) for part in cmd))
if os.environ.get("PHOTOFRAME_FLASH_DRY_RUN") == "1":
    print("[dry-run] 跳过实际烧录")
    raise SystemExit(0)

subprocess.run(cmd, check=True)
print("[done] 烧录完成（未覆盖 NVS 分区）")
PY
