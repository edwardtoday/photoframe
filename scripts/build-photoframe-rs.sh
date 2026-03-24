#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DIST_DIR="${REPO_ROOT}/firmware/photoframe-rs/dist"
ELF_REL="/work/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release/photoframe-firmware-device"
ELF_ABS="${REPO_ROOT}/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release/photoframe-firmware-device"
FULL_BIN_ABS="${DIST_DIR}/photoframe-rs-fullchip.bin"
APP_BIN_ABS="${DIST_DIR}/photoframe-rs-app.bin"
PARTITIONS_CSV="/work/firmware/photoframe-rs/partitions.csv"

mkdir -p "${DIST_DIR}"
rm -f "${DIST_DIR}/photoframe-rs.bin"

BUILD_CMD='cargo build --release'
BUILD_ENV_PREFIX=()
if [[ -n "${PHOTOFRAME_BOOTSTRAP_CONFIG_JSON:-}" ]]; then
  BUILD_ENV_PREFIX+=("PHOTOFRAME_BOOTSTRAP_CONFIG_JSON=${PHOTOFRAME_BOOTSTRAP_CONFIG_JSON}")
  echo "[info] 将 PHOTOFRAME_BOOTSTRAP_CONFIG_JSON 注入 Docker 构建"
fi
if [[ -n "${PHOTOFRAME_DEBUG_STAGE_BEACON:-}" ]]; then
  BUILD_ENV_PREFIX+=("PHOTOFRAME_DEBUG_STAGE_BEACON=${PHOTOFRAME_DEBUG_STAGE_BEACON}")
  echo "[info] 启用 PHOTOFRAME_DEBUG_STAGE_BEACON"
fi
if [[ ${#BUILD_ENV_PREFIX[@]} -gt 0 ]]; then
  BUILD_CMD='env'
  for item in "${BUILD_ENV_PREFIX[@]}"; do
    printf -v BUILD_CMD '%s %q' "${BUILD_CMD}" "${item}"
  done
  BUILD_CMD="${BUILD_CMD} cargo build --release"
fi

"${SCRIPT_DIR}/rust-idf-docker.sh" "${BUILD_CMD}"

if [[ ! -f "${ELF_ABS}" ]]; then
  echo "[error] 未找到 ELF 产物: ${ELF_ABS}" >&2
  exit 1
fi

echo "[info] ELF 已生成: ${ELF_ABS}"

"${SCRIPT_DIR}/rust-idf-docker.sh" "espflash save-image --chip esp32s3 --flash-mode dio --flash-size 16mb --flash-freq 80mhz ${ELF_REL} /work/firmware/photoframe-rs/dist/photoframe-rs-app.bin"

BOOTLOADER_REL="$("${SCRIPT_DIR}/rust-idf-docker.sh" "find /work/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release/build -path '*/out/build/bootloader/bootloader.bin' | head -n 1" | tail -n 1)"
if [[ -n "${BOOTLOADER_REL}" ]]; then
  "${SCRIPT_DIR}/rust-idf-docker.sh" "espflash save-image --chip esp32s3 --merge --flash-mode dio --flash-size 16mb --flash-freq 80mhz --bootloader ${BOOTLOADER_REL} --partition-table ${PARTITIONS_CSV} --target-app-partition factory ${ELF_REL} /work/firmware/photoframe-rs/dist/photoframe-rs-fullchip.bin"
  echo "[info] 整片镜像已生成: ${FULL_BIN_ABS}"
  echo "[warn] 整片镜像会覆盖 NVS；仅限空片首刷，不要用于现场升级"
else
  echo "[warn] 未找到 bootloader.bin，只生成应用镜像: ${APP_BIN_ABS}"
fi
