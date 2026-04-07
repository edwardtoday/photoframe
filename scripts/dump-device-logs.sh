#!/usr/bin/env bash
set -euo pipefail

PORT="${1:-}"
shift || true
BAUD="115200"
TIMEOUT_SECONDS="45"
OUTPUT_PATH=""
RAW_OUTPUT_PATH=""

if [[ -n "${1:-}" && "${1:-}" != --* ]]; then
  BAUD="${1}"
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --timeout-seconds)
      shift
      TIMEOUT_SECONDS="${1:-}"
      ;;
    --output)
      shift
      OUTPUT_PATH="${1:-}"
      ;;
    --raw-output)
      shift
      RAW_OUTPUT_PATH="${1:-}"
      ;;
    *)
      echo "[error] 未知参数: $1" >&2
      echo "用法: scripts/dump-device-logs.sh <serial-port> [baud] [--timeout-seconds <seconds>] [--output <path>] [--raw-output <path>]" >&2
      exit 1
      ;;
  esac
  shift
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VENV_PY="${REPO_ROOT}/.venv-host-tools/bin/python"

if [[ -z "${PORT}" ]]; then
  echo "用法: scripts/dump-device-logs.sh <serial-port> [baud] [--timeout-seconds <seconds>] [--output <path>] [--raw-output <path>]" >&2
  exit 1
fi

if [[ ! -x "${VENV_PY}" ]]; then
  echo "[error] 未找到 host tools 虚拟环境: ${VENV_PY}" >&2
  echo "[hint] 先执行 scripts/setup-host-tools.sh" >&2
  exit 2
fi

export PHOTOFRAME_DUMP_PORT="${PORT}"
export PHOTOFRAME_DUMP_BAUD="${BAUD}"
export PHOTOFRAME_DUMP_TIMEOUT_SECONDS="${TIMEOUT_SECONDS}"
export PHOTOFRAME_DUMP_OUTPUT_PATH="${OUTPUT_PATH}"
export PHOTOFRAME_DUMP_RAW_OUTPUT_PATH="${RAW_OUTPUT_PATH}"

"${VENV_PY}" - <<'PY'
import os
import pathlib
import sys
import time

import serial


BEGIN_PREFIX = "PHOTOFRAME_TF_LOG_DUMP_BEGIN "
LINE_PREFIX = "PHOTOFRAME_TF_LOG_DUMP_LINE "
END_PREFIX = "PHOTOFRAME_TF_LOG_DUMP_END"


def resolve_output(raw: str) -> pathlib.Path | None:
    if not raw:
        return None
    path = pathlib.Path(raw).expanduser()
    if not path.is_absolute():
        path = (pathlib.Path.cwd() / path).resolve()
    else:
        path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


port = os.environ["PHOTOFRAME_DUMP_PORT"]
baud = int(os.environ["PHOTOFRAME_DUMP_BAUD"])
timeout_seconds = float(os.environ["PHOTOFRAME_DUMP_TIMEOUT_SECONDS"])
output_path = resolve_output(os.environ.get("PHOTOFRAME_DUMP_OUTPUT_PATH", ""))
raw_output_path = resolve_output(os.environ.get("PHOTOFRAME_DUMP_RAW_OUTPUT_PATH", ""))

deadline = time.monotonic() + timeout_seconds
raw_lines: list[str] = []
dump_lines: list[str] = []
metadata: str | None = None
began = False

print(
    f"[info] 等待设备通过 {port} @ {baud} 输出 TF 历史日志，超时 {timeout_seconds:.0f}s",
    file=sys.stderr,
)

with serial.Serial(port=port, baudrate=baud, timeout=0.2, write_timeout=1) as ser:
    try:
        ser.dtr = False
        ser.rts = False
    except Exception:
        pass

    while time.monotonic() < deadline:
        raw = ser.readline()
        if not raw:
            continue
        line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
        raw_lines.append(line)

        if line.startswith(BEGIN_PREFIX):
            metadata = line[len(BEGIN_PREFIX):]
            began = True
            dump_lines.clear()
            continue
        if line.startswith(LINE_PREFIX) and began:
            dump_lines.append(line[len(LINE_PREFIX):])
            continue
        if line.startswith(END_PREFIX) and began:
            break

if raw_output_path is not None:
    raw_output_path.write_text("\n".join(raw_lines) + ("\n" if raw_lines else ""), encoding="utf-8")
    print(f"[info] 原始串口转录已保存到 {raw_output_path}", file=sys.stderr)

if not began:
    print("[error] 超时前未收到 PHOTOFRAME_TF_LOG_DUMP_BEGIN", file=sys.stderr)
    raise SystemExit(3)

if metadata is None:
    print("[error] 已进入 dump，但缺少元数据", file=sys.stderr)
    raise SystemExit(4)

if output_path is not None:
    output_path.write_text("\n".join(dump_lines) + ("\n" if dump_lines else ""), encoding="utf-8")
    print(f"[info] 结构化 TF 日志已保存到 {output_path}", file=sys.stderr)

print(f"[meta] {metadata}")
for item in dump_lines:
    print(item)
PY
