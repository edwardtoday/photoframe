#!/usr/bin/env bash
set -euo pipefail

ONCE=0
if [[ "${1:-}" == "--once" ]]; then
  ONCE=1
  shift
fi

PORT="${1:-}"
BAUD="${2:-115200}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VENV_PY="${REPO_ROOT}/.venv-host-tools/bin/python"

if [[ -z "${PORT}" ]]; then
  echo "用法: scripts/monitor-host.sh [--once] <serial-port> [baud]" >&2
  exit 1
fi

if [[ ! -x "${VENV_PY}" ]]; then
  echo "[error] 未找到 host tools 虚拟环境: ${VENV_PY}" >&2
  echo "[hint] 先执行 scripts/setup-host-tools.sh" >&2
  exit 2
fi

echo "[info] 监控串口 ${PORT} @ ${BAUD}"
echo "[info] Ctrl+C 退出；串口断开后会自动重连"

while true; do
  set +e
  "${VENV_PY}" -m serial.tools.miniterm "${PORT}" "${BAUD}" --raw
  rc=$?
  set -e

  if [[ ${ONCE} -eq 1 ]]; then
    exit ${rc}
  fi

  if [[ ${rc} -eq 0 ]]; then
    echo "[info] 监控已退出（rc=0），1 秒后重连；按 Ctrl+C 可结束。"
  else
    echo "[warn] 串口已断开（rc=${rc}），1 秒后自动重连..."
  fi
  sleep 1
done
