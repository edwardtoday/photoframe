#!/usr/bin/env bash
set -euo pipefail

IMAGE="${RUST_IDF_DOCKER_IMAGE:-photoframe-rs-idf:v5.5.1}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORKDIR="${RUST_IDF_WORKDIR:-/work/firmware/photoframe-rs/crates/firmware-device}"
DOCKERFILE="${REPO_ROOT}/firmware/photoframe-rs/docker/Dockerfile"

if [[ ! -f "${DOCKERFILE}" ]]; then
  echo "[error] Dockerfile 不存在: ${DOCKERFILE}" >&2
  exit 1
fi

if [[ $# -eq 0 ]]; then
  cat <<USAGE
用法:
  scripts/rust-idf-docker.sh '<shell command>'

示例:
  scripts/rust-idf-docker.sh 'cargo build --release'
USAGE
  exit 1
fi

if ! docker image inspect "${IMAGE}" >/dev/null 2>&1; then
  echo "[info] 构建 Rust ESP-IDF Docker 镜像: ${IMAGE}"
  docker build -t "${IMAGE}" -f "${DOCKERFILE}" "${REPO_ROOT}"
fi

TTY_ARGS=()
if [[ -t 0 && -t 1 ]]; then
  TTY_ARGS=(-it)
fi

CMD="$*"

mkdir -p "${HOME}/.cargo/registry" "${HOME}/.cargo/git" "${HOME}/.espressif"

docker run --rm "${TTY_ARGS[@]}" \
  -v "${REPO_ROOT}:/work" \
  -v "${HOME}/.cargo/registry:/root/.cargo/registry" \
  -v "${HOME}/.cargo/git:/root/.cargo/git" \
  -v "${HOME}/.espressif:/root/.espressif" \
  -w "${WORKDIR}" \
  "${IMAGE}" \
  ". /opt/esp/idf/export.sh >/dev/null && . /root/export-esp.sh >/dev/null && ${CMD}"
