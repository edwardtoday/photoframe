#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DIST_DIR="${REPO_ROOT}/firmware/photoframe-rs/dist"
ELF_REL="/work/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release/photoframe-firmware-device"
ELF_ABS="${REPO_ROOT}/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release/photoframe-firmware-device"
BIN_ABS="${DIST_DIR}/photoframe-rs.bin"
APP_BIN_ABS="${DIST_DIR}/photoframe-rs-app.bin"
PARTITIONS_CSV="/work/firmware/photoframe-fw/partitions.csv"

mkdir -p "${DIST_DIR}"

"${SCRIPT_DIR}/rust-idf-docker.sh" 'cargo build --release'

if [[ ! -f "${ELF_ABS}" ]]; then
  echo "[error] 未找到 ELF 产物: ${ELF_ABS}" >&2
  exit 1
fi

echo "[info] ELF 已生成: ${ELF_ABS}"

"${SCRIPT_DIR}/rust-idf-docker.sh" "espflash save-image --chip esp32s3 --flash-mode dio --flash-size 16mb --flash-freq 80mhz ${ELF_REL} /work/firmware/photoframe-rs/dist/photoframe-rs-app.bin"

BOOTLOADER_REL="$("${SCRIPT_DIR}/rust-idf-docker.sh" "find /work/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release/build -path '*/out/build/bootloader/bootloader.bin' | head -n 1" | tail -n 1)"
if [[ -n "${BOOTLOADER_REL}" ]]; then
  "${SCRIPT_DIR}/rust-idf-docker.sh" "espflash save-image --chip esp32s3 --merge --flash-mode dio --flash-size 16mb --flash-freq 80mhz --bootloader ${BOOTLOADER_REL} --partition-table ${PARTITIONS_CSV} --target-app-partition factory ${ELF_REL} /work/firmware/photoframe-rs/dist/photoframe-rs.bin"
  echo "[info] 合并镜像已生成: ${BIN_ABS}"
else
  echo "[warn] 未找到 bootloader.bin，只生成应用镜像: ${APP_BIN_ABS}"
fi
