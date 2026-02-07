#!/usr/bin/env bash
set -euo pipefail

# 在宿主机创建最小工具环境：用于烧录与串口监控。
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VENV_DIR="${REPO_ROOT}/.venv-host-tools"

python3 -m venv "${VENV_DIR}"
"${VENV_DIR}/bin/python" -m pip install --upgrade pip
"${VENV_DIR}/bin/python" -m pip install esptool pyserial

echo "[done] host 工具已就绪: ${VENV_DIR}"
echo "[hint] 可用 scripts/flash-host.py 与 scripts/monitor-host.sh"
