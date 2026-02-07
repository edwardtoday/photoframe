#!/usr/bin/env bash
set -euo pipefail

# Phase B 验证：在容器中编译 Waveshare 上游固件。
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"${SCRIPT_DIR}/idf-docker.sh" "idf.py set-target esp32s3 && idf.py build"
