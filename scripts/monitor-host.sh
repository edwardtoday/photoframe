#!/usr/bin/env bash
set -euo pipefail

PORT="${1:-}"
BAUD="${2:-115200}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VENV_PY="${REPO_ROOT}/.venv-host-tools/bin/python"

if [[ -z "${PORT}" ]]; then
  echo "用法: scripts/monitor-host.sh <serial-port> [baud]" >&2
  exit 1
fi

if [[ ! -x "${VENV_PY}" ]]; then
  echo "[error] 未找到 host tools 虚拟环境: ${VENV_PY}" >&2
  echo "[hint] 先执行 scripts/setup-host-tools.sh" >&2
  exit 2
fi

echo "[info] 监控串口 ${PORT} @ ${BAUD}"
exec "${VENV_PY}" -m serial.tools.miniterm "${PORT}" "${BAUD}" --raw
