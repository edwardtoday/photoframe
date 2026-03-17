#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_SCRIPT="${REPO_ROOT}/scripts/flash-photoframe-rs.sh"
PYTHON_BIN="${PYTHON_BIN:-$(command -v python3)}"

if [[ -z "${PYTHON_BIN}" || ! -x "${PYTHON_BIN}" ]]; then
  echo "[error] 未找到可执行 python3" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

fail() {
  echo "[fail] $*" >&2
  exit 1
}

assert_contains() {
  local output="$1"
  local expected="$2"
  if [[ "${output}" != *"${expected}"* ]]; then
    fail "输出未包含预期内容: ${expected}"
  fi
}

resolve_path() {
  "${PYTHON_BIN}" -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).resolve())' "$1"
}

prepare_fixture() {
  local root="$1"
  local build_root="${root}/build-root"
  local build_dir="${build_root}/build/mock/out/build"
  local dist_dir="${root}/dist"

  mkdir -p "${build_dir}" "${dist_dir}"

  cat > "${build_dir}/flasher_args.json" <<'EOF'
{
  "extra_esptool_args": {
    "chip": "esp32s3",
    "before": "default_reset",
    "after": "hard_reset"
  },
  "write_flash_args": [
    "--flash_mode",
    "dio"
  ],
  "flash_files": {
    "0x0": "bootloader.bin",
    "0x8000": "partition-table.bin",
    "0xf000": "ota_data_initial.bin",
    "0x20000": "libespidf.bin"
  },
  "app": {
    "offset": "0x20000"
  }
}
EOF

  printf 'boot' > "${build_root}/bootloader.bin"
  printf 'part' > "${build_root}/partition-table.bin"
  printf 'boot' > "${build_dir}/bootloader.bin"
  printf 'part' > "${build_dir}/partition-table.bin"
  printf 'ota' > "${build_dir}/ota_data_initial.bin"
  printf 'old-app' > "${build_dir}/libespidf.bin"
  printf 'elf' > "${build_root}/photoframe-firmware-device"
  printf 'dist-app' > "${dist_dir}/photoframe-rs-app.bin"
  printf 'recovery' > "${dist_dir}/photoframe-rs-recovery-app.bin"
}

run_flash_script() {
  local fixture_root="$1"
  shift

  env \
    PHOTOFRAME_FLASH_VENV_PY_OVERRIDE="${PYTHON_BIN}" \
    PHOTOFRAME_FLASH_BUILD_ROOT_OVERRIDE="${fixture_root}/build-root" \
    PHOTOFRAME_FLASH_ELF_ABS_OVERRIDE="${fixture_root}/build-root/photoframe-firmware-device" \
    PHOTOFRAME_FLASH_DEFAULT_APP_BIN_OVERRIDE="${fixture_root}/dist/photoframe-rs-app.bin" \
    PHOTOFRAME_FLASHER_ARGS_JSON_OVERRIDE="${fixture_root}/build-root/build/mock/out/build/flasher_args.json" \
    bash "${TARGET_SCRIPT}" /dev/cu.test 115200 "$@" 2>&1
}

case_default_uses_dist() {
  local root="${TMP_DIR}/case-default"
  prepare_fixture "${root}"
  touch -t 202603171000 "${root}/build-root/photoframe-firmware-device"
  touch -t 202603171001 "${root}/dist/photoframe-rs-app.bin"
  local resolved_app
  resolved_app="$(resolve_path "${root}/dist/photoframe-rs-app.bin")"

  local output
  output="$(run_flash_script "${root}" --dry-run)"
  assert_contains "${output}" "[info] 默认使用 dist 应用镜像: ${resolved_app}"
  assert_contains "${output}" "0x20000 ${resolved_app}"
}

case_override_app_bin() {
  local root="${TMP_DIR}/case-override"
  prepare_fixture "${root}"
  touch -t 202603171000 "${root}/build-root/photoframe-firmware-device"
  touch -t 202603170959 "${root}/dist/photoframe-rs-app.bin"
  local resolved_recovery
  resolved_recovery="$(resolve_path "${root}/dist/photoframe-rs-recovery-app.bin")"

  local output
  output="$(run_flash_script "${root}" --app-bin "${root}/dist/photoframe-rs-recovery-app.bin" --dry-run)"
  assert_contains "${output}" "[info] 使用覆盖应用镜像: ${resolved_recovery}"
  assert_contains "${output}" "0x20000 ${resolved_recovery}"
}

case_stale_default_app_fails() {
  local root="${TMP_DIR}/case-stale"
  prepare_fixture "${root}"
  touch -t 202603171001 "${root}/build-root/photoframe-firmware-device"
  touch -t 202603171000 "${root}/dist/photoframe-rs-app.bin"

  local output
  set +e
  output="$(run_flash_script "${root}" --dry-run)"
  local rc=$?
  set -e

  if [[ ${rc} -eq 0 ]]; then
    fail "默认镜像陈旧时脚本不应成功"
  fi
  assert_contains "${output}" "默认应用镜像早于当前 ELF 产物"
  assert_contains "${output}" "请先执行 scripts/build-photoframe-rs.sh 再刷机"
}

case_default_uses_dist
case_override_app_bin
case_stale_default_app_fails

echo "[ok] flash-photoframe-rs 默认镜像与陈旧产物保护测试通过"
