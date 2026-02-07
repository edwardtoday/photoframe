#!/usr/bin/env bash
set -euo pipefail

# 在 Docker 中执行 ESP-IDF 命令（默认镜像与上游配置一致：5.5.1）。
IMAGE="${IDF_DOCKER_IMAGE:-espressif/idf:v5.5.1}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
PROJECT_DIR="${IDF_PROJECT_DIR:-upstream/ESP32-S3-PhotoPainter/01_Example/xiaozhi-esp32}"
CONTAINER_WORKDIR="/work/${PROJECT_DIR}"

if [[ ! -d "${REPO_ROOT}/${PROJECT_DIR}" ]]; then
  echo "[error] 项目目录不存在: ${REPO_ROOT}/${PROJECT_DIR}" >&2
  exit 1
fi

if [[ $# -eq 0 ]]; then
  cat <<USAGE
用法:
  scripts/idf-docker.sh <idf.py 或 shell 命令>

示例:
  scripts/idf-docker.sh idf.py --version
  scripts/idf-docker.sh idf.py set-target esp32s3
  scripts/idf-docker.sh 'idf.py build'
USAGE
  exit 1
fi

mkdir -p "${HOME}/.espressif" "${HOME}/.cache/pip"

TTY_ARGS=()
if [[ -t 0 && -t 1 ]]; then
  TTY_ARGS=(-it)
fi

DEVICE_ARGS=()
if [[ "$(uname -s)" == "Linux" && -n "${IDF_SERIAL_PORT:-}" ]]; then
  DEVICE_ARGS=(--device "${IDF_SERIAL_PORT}:${IDF_SERIAL_PORT}")
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
  echo "[info] macOS 上 Docker Desktop 不支持稳定 USB 直通；建议容器内编译，宿主机烧录/串口监控。"
fi

docker run --rm "${TTY_ARGS[@]}" \
  -v "${REPO_ROOT}:/work" \
  -v "${HOME}/.espressif:/root/.espressif" \
  -v "${HOME}/.cache/pip:/root/.cache/pip" \
  -w "${CONTAINER_WORKDIR}" \
  "${DEVICE_ARGS[@]}" \
  "${IMAGE}" \
  bash -lc "$*"
