#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
用法：
  scripts/build-with-y9000.sh [--target upstream|rs] [--host HOST] [--remote-dir DIR]
                              [--no-pull-output] [--bootstrap-if-missing]
                              [--keep-remote-dir] [--env NAME]
                              [-- 远端构建脚本参数...]

默认：
  --target     rs
  --host       y9000
  --remote-dir 非 linked worktree 默认 /home/qingpei/git/github/photoframe
               linked worktree 默认 /home/qingpei/git/github/.remote-build-worktrees/<worktree-id>

说明：
  - 先把本地工作区 rsync 到 y9000，再在远端执行现有构建脚本，复用 y9000 上的 Docker。
  - 默认把构建产物按“本机烧录所需的最小集合”拉回本地，避免同步整个 build/ 或 target/。
  - 当前支持的 target：
      upstream -> scripts/build-upstream.sh
      rs       -> scripts/build-photoframe-rs.sh
  - 如加 --bootstrap-if-missing，当远端目录不存在时会自动创建父目录并初始化最小 git 仓库。
  - 如加 --keep-remote-dir，脚本退出时不会清理远端目录，便于排障时保留现场。
  - 如需把本地环境变量传到远端构建，使用 --env NAME（可重复）。

示例：
  scripts/build-with-y9000.sh --bootstrap-if-missing
  scripts/build-with-y9000.sh --target upstream --bootstrap-if-missing
  scripts/build-with-y9000.sh --target rs --bootstrap-if-missing
  export PHOTOFRAME_BOOTSTRAP_CONFIG_JSON='{"timezone":"Asia/Shanghai"}'
  scripts/build-with-y9000.sh --target rs --bootstrap-if-missing --env PHOTOFRAME_BOOTSTRAP_CONFIG_JSON
EOF
}

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST="y9000"
TARGET="rs"
REMOTE_PRIMARY_DIR="/home/qingpei/git/github/photoframe"
REMOTE_WORKTREE_BASE="/home/qingpei/git/github/.remote-build-worktrees"
REMOTE_DIR="${REMOTE_PRIMARY_DIR}"
REMOTE_DIR_EXPLICIT=0
LOCAL_IS_LINKED_WORKTREE=0
AUTO_REMOTE_WORKTREE=0
BOOTSTRAP_IF_MISSING=0
KEEP_REMOTE_DIR=0
PULL_OUTPUT=1
SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10)
SYNCED_REMOTE=0
BUILD_ARGS=()
FORWARD_ENV_NAMES=()
REMOTE_CHOWN_IMAGE=""

require_cmd() {
  local cmd="$1"
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    echo "[build-with-y9000] 未找到命令：${cmd}" >&2
    exit 1
  fi
}

hash256_local() {
  local file="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${file}" | awk '{print $1}'
    return 0
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${file}" | awk '{print $1}'
    return 0
  fi
  return 1
}

hash_text_local() {
  local text="$1"
  if command -v shasum >/dev/null 2>&1; then
    printf '%s' "${text}" | shasum -a 256 | awk '{print $1}'
    return 0
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s' "${text}" | sha256sum | awk '{print $1}'
    return 0
  fi
  return 1
}

sanitize_name() {
  local text="$1"
  text="${text//[^A-Za-z0-9._-]/-}"
  while [[ "${text}" == *--* ]]; do
    text="${text//--/-}"
  done
  text="${text#-}"
  text="${text%-}"
  if [[ -z "${text}" ]]; then
    text="worktree"
  fi
  printf '%s' "${text}"
}

remote_hash256() {
  local remote_file="$1"
  ssh "${SSH_OPTS[@]}" "${HOST}" \
    "remote_file='${remote_file}'; (sha256sum \"\$remote_file\" 2>/dev/null || shasum -a 256 \"\$remote_file\" 2>/dev/null) | awk '{print \$1}'"
}

copy_remote_file() {
  local remote_file="$1"
  local local_file="$2"
  mkdir -p "$(dirname "${local_file}")"
  RSYNC_RSH="ssh ${SSH_OPTS[*]}" rsync -a "${HOST}:${remote_file}" "${local_file}"
}

copy_remote_dir() {
  local remote_dir="$1"
  local local_dir="$2"
  mkdir -p "${local_dir}"
  RSYNC_RSH="ssh ${SSH_OPTS[*]}" rsync -a --delete "${HOST}:${remote_dir}/" "${local_dir}/"
}

pull_idf_artifacts() {
  local remote_build_dir="$1"
  local local_build_dir="$2"
  local local_flasher_args="${local_build_dir}/flasher_args.json"
  local app_rel=""

  mkdir -p "${local_build_dir}"
  copy_remote_file "${remote_build_dir}/flasher_args.json" "${local_flasher_args}"

  while IFS= read -r rel; do
    [[ -z "${rel}" ]] && continue
    copy_remote_file "${remote_build_dir}/${rel}" "${local_build_dir}/${rel}"
  done < <(
    python3 - "${local_flasher_args}" <<'PY'
import json
import pathlib
import sys

data = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
flash_files = data.get("flash_files", {})
items = []
if isinstance(flash_files, dict):
    items = [str(value) for value in flash_files.values()]
elif isinstance(flash_files, list):
    items = [str(item[1]) for item in flash_files if isinstance(item, (list, tuple)) and len(item) == 2]
for rel in dict.fromkeys(items):
    print(rel)
PY
  )

  app_rel="$(
    python3 - "${local_flasher_args}" <<'PY'
import json
import pathlib
import sys

data = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
print(str((data.get("app") or {}).get("file") or ""))
PY
  )"

  if [[ -n "${app_rel}" ]]; then
    local local_app="${local_build_dir}/${app_rel}"
    local remote_app="${remote_build_dir}/${app_rel}"
    local remote_sha=""
    local local_sha=""
    remote_sha="$(remote_hash256 "${remote_app}" || true)"
    local_sha="$(hash256_local "${local_app}" || true)"
    if [[ -n "${remote_sha}" && -n "${local_sha}" ]]; then
      echo "[build-with-y9000] remote sha256: ${remote_sha}"
      echo "[build-with-y9000] local  sha256: ${local_sha}"
      if [[ "${remote_sha}" != "${local_sha}" ]]; then
        echo "[build-with-y9000] 拉回产物 hash 不一致：${app_rel}" >&2
        exit 1
      fi
    fi
  fi
}

pull_rs_runtime_artifacts() {
  local remote_release_dir="$1"
  local local_release_dir="$2"
  local remote_flasher_args="$3"
  local flasher_dir_rel=""

  copy_remote_dir "${REMOTE_DIR}/firmware/photoframe-rs/dist" "${ROOT_DIR}/firmware/photoframe-rs/dist"
  copy_remote_file "${remote_release_dir}/photoframe-firmware-device" "${local_release_dir}/photoframe-firmware-device"
  copy_remote_file "${remote_release_dir}/bootloader.bin" "${local_release_dir}/bootloader.bin"
  copy_remote_file "${remote_release_dir}/partition-table.bin" "${local_release_dir}/partition-table.bin"

  flasher_dir_rel="$(dirname "${remote_flasher_args}")"
  pull_idf_artifacts "${REMOTE_DIR}/${flasher_dir_rel}" "${ROOT_DIR}/${flasher_dir_rel}"

  if [[ -f "${ROOT_DIR}/firmware/photoframe-rs/dist/photoframe-rs-app.bin" ]]; then
    local remote_app="${REMOTE_DIR}/firmware/photoframe-rs/dist/photoframe-rs-app.bin"
    local local_app="${ROOT_DIR}/firmware/photoframe-rs/dist/photoframe-rs-app.bin"
    local remote_sha=""
    local local_sha=""
    remote_sha="$(remote_hash256 "${remote_app}" || true)"
    local_sha="$(hash256_local "${local_app}" || true)"
    if [[ -n "${remote_sha}" && -n "${local_sha}" ]]; then
      echo "[build-with-y9000] remote sha256: ${remote_sha}"
      echo "[build-with-y9000] local  sha256: ${local_sha}"
      if [[ "${remote_sha}" != "${local_sha}" ]]; then
        echo "[build-with-y9000] 拉回产物 hash 不一致：firmware/photoframe-rs/dist/photoframe-rs-app.bin" >&2
        exit 1
      fi
    fi
  fi
}

fix_remote_ownership() {
  if [[ -z "${REMOTE_CHOWN_IMAGE}" ]]; then
    return 0
  fi

  ssh "${SSH_OPTS[@]}" "${HOST}" bash -s -- "${REMOTE_DIR}" "${REMOTE_CHOWN_IMAGE}" <<'EOF'
set -euo pipefail

REMOTE_DIR="$1"
IMAGE="$2"

if ! docker image inspect "${IMAGE}" >/dev/null 2>&1; then
  echo "[build-with-y9000] WARNING: cleanup image missing, skip remote chown: ${IMAGE}" >&2
  exit 0
fi

uid="$(id -u)"
gid="$(id -g)"

docker run --rm \
  -v "${REMOTE_DIR}:/work" \
  --entrypoint /bin/sh \
  "${IMAGE}" \
  -c "chown -R ${uid}:${gid} /work"
EOF
}

cleanup_remote() {
  local exit_code="$?"
  if [[ "${SYNCED_REMOTE}" -eq 1 ]]; then
    fix_remote_ownership || true
    if [[ "${KEEP_REMOTE_DIR}" -eq 1 ]]; then
      echo "[build-with-y9000] keep remote dir: ${HOST}:${REMOTE_DIR}"
      echo "[build-with-y9000] manual cleanup: ssh ${HOST} \"cd '${REMOTE_DIR}' && git reset --hard HEAD && git clean -ffdx\""
    else
      echo "[build-with-y9000] cleanup remote worktree: ${HOST}:${REMOTE_DIR}"
      ssh "${SSH_OPTS[@]}" "${HOST}" bash -s -- "${REMOTE_DIR}" <<'EOF' || true
set -euo pipefail

REMOTE_DIR="$1"
cd "${REMOTE_DIR}"
git reset --hard HEAD >/dev/null
git clean -ffdx >/dev/null
EOF
    fi
  fi
  trap - EXIT
  exit "${exit_code}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      TARGET="${2:-}"; shift 2 ;;
    --host)
      HOST="${2:-}"; shift 2 ;;
    --remote-dir)
      REMOTE_DIR_EXPLICIT=1
      REMOTE_DIR="${2:-}"; shift 2 ;;
    --no-pull-output)
      PULL_OUTPUT=0; shift ;;
    --bootstrap-if-missing)
      BOOTSTRAP_IF_MISSING=1; shift ;;
    --keep-remote-dir)
      KEEP_REMOTE_DIR=1; shift ;;
    --env)
      FORWARD_ENV_NAMES+=("${2:-}"); shift 2 ;;
    -h|--help)
      usage; exit 0 ;;
    --)
      shift
      BUILD_ARGS=("$@")
      break ;;
    *)
      echo "未知参数：$1" >&2
      usage
      exit 2 ;;
  esac
done

trap cleanup_remote EXIT

require_cmd ssh
require_cmd rsync
require_cmd python3

if [[ "${KEEP_REMOTE_DIR}" -eq 1 ]]; then
  echo "[build-with-y9000] WARNING: --keep-remote-dir 已开启；脚本退出后不会自动清理远端目录" >&2
fi

if [[ -f "${ROOT_DIR}/.git" ]]; then
  LOCAL_IS_LINKED_WORKTREE=1
fi

if [[ "${LOCAL_IS_LINKED_WORKTREE}" -eq 1 && "${REMOTE_DIR_EXPLICIT}" -eq 0 ]]; then
  worktree_name="$(sanitize_name "$(basename "${ROOT_DIR}")")"
  worktree_hash="$(hash_text_local "${ROOT_DIR}" | cut -c1-12)"
  if [[ -z "${worktree_hash}" ]]; then
    echo "[build-with-y9000] 无法为 worktree 生成稳定标识" >&2
    exit 1
  fi
  REMOTE_DIR="${REMOTE_WORKTREE_BASE}/${worktree_name}-${worktree_hash}"
  AUTO_REMOTE_WORKTREE=1
fi

REMOTE_BUILD_SCRIPT=""
VERIFY_PATH=""
PULL_MODE=""

case "${TARGET}" in
  upstream)
    REMOTE_BUILD_SCRIPT="./scripts/build-upstream.sh"
    VERIFY_PATH="upstream/ESP32-S3-PhotoPainter/01_Example/xiaozhi-esp32/build/flasher_args.json"
    PULL_MODE="idf-upstream"
    REMOTE_CHOWN_IMAGE="${IDF_DOCKER_IMAGE:-espressif/idf:v5.5.1}"
    ;;
  rs)
    REMOTE_BUILD_SCRIPT="./scripts/build-photoframe-rs.sh"
    VERIFY_PATH="firmware/photoframe-rs/dist/photoframe-rs-app.bin"
    PULL_MODE="rs"
    REMOTE_CHOWN_IMAGE="${RUST_IDF_DOCKER_IMAGE:-photoframe-rs-idf:v5.5.1}"
    ;;
  *)
    echo "[build-with-y9000] 不支持的 target：${TARGET}" >&2
    exit 2
    ;;
esac

FORWARD_ENV_ASSIGNMENTS=()
for name in "${FORWARD_ENV_NAMES[@]}"; do
  if [[ ! "${name}" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
    echo "[build-with-y9000] --env 名称非法：${name}" >&2
    exit 2
  fi
  if [[ -z "${!name+x}" ]]; then
    echo "[build-with-y9000] 本地环境变量未设置：${name}" >&2
    exit 2
  fi
  FORWARD_ENV_ASSIGNMENTS+=("${name}=${!name}")
done

echo "[build-with-y9000] precheck remote repo: ${HOST}:${REMOTE_DIR}"
ssh "${SSH_OPTS[@]}" "${HOST}" bash -s -- \
  "${REMOTE_DIR}" "${REMOTE_PRIMARY_DIR}" "${AUTO_REMOTE_WORKTREE}" "${BOOTSTRAP_IF_MISSING}" <<'EOF'
set -euo pipefail

REMOTE_DIR="$1"
REMOTE_PRIMARY_DIR="$2"
AUTO_REMOTE_WORKTREE="$3"
BOOTSTRAP_IF_MISSING="$4"

bootstrap_init_repo() {
  local target="$1"
  local parent_dir
  parent_dir="$(dirname "$target")"
  if [[ "${BOOTSTRAP_IF_MISSING}" != "1" ]]; then
    echo "[remote] 目录不存在：${target}" >&2
    echo "[remote] 可重试：scripts/build-with-y9000.sh --bootstrap-if-missing ..." >&2
    exit 1
  fi
  mkdir -p "${parent_dir}"
  echo "[remote] bootstrap init git repo: ${target}"
  git init "${target}" >/dev/null
  git -C "${target}" config user.name codex-bootstrap
  git -C "${target}" config user.email codex-bootstrap@localhost
  git -C "${target}" commit --allow-empty -m "bootstrap remote build repo" >/dev/null
}

if ! command -v git >/dev/null 2>&1; then
  echo "[remote] 未找到 git" >&2
  exit 1
fi

if [[ "${AUTO_REMOTE_WORKTREE}" == "1" ]]; then
  if [[ ! -d "${REMOTE_PRIMARY_DIR}" ]]; then
    bootstrap_init_repo "${REMOTE_PRIMARY_DIR}"
  fi
  if ! git -C "${REMOTE_PRIMARY_DIR}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "[remote] 主仓库目录不是 git 仓库：${REMOTE_PRIMARY_DIR}" >&2
    exit 1
  fi
  mkdir -p "$(dirname "${REMOTE_DIR}")"
  if [[ ! -d "${REMOTE_DIR}" ]]; then
    git -C "${REMOTE_PRIMARY_DIR}" worktree add --force --detach "${REMOTE_DIR}" HEAD >/dev/null
  fi
elif [[ ! -d "${REMOTE_DIR}" ]]; then
  bootstrap_init_repo "${REMOTE_DIR}"
fi

cd "${REMOTE_DIR}"
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "[remote] 不是 git 仓库：${REMOTE_DIR}" >&2
  exit 1
fi

if [[ -n "$(git status --short --untracked-files=all)" ]]; then
  echo "[remote] 工作区不是 clean，拒绝覆盖同步：" >&2
  git status --short --untracked-files=all | sed -n '1,40p' >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "[remote] 未找到 docker" >&2
  exit 1
fi
EOF

echo "[build-with-y9000] sync local workspace -> ${HOST}:${REMOTE_DIR}"
RSYNC_RSH="ssh ${SSH_OPTS[*]}" rsync -a --delete \
  --exclude '.git' \
  --exclude '.git/' \
  --exclude '.DS_Store' \
  --exclude '__pycache__/' \
  --exclude '.venv-host-tools/' \
  --exclude 'references/waveshare/downloads/' \
  --exclude 'references/waveshare/downloads/**' \
  --exclude 'upstream/ESP32-S3-PhotoPainter/01_Example/xiaozhi-esp32/build/' \
  --exclude 'upstream/ESP32-S3-PhotoPainter/01_Example/xiaozhi-esp32/managed_components/' \
  --exclude 'firmware/photoframe-rs/target/' \
  --exclude 'firmware/photoframe-rs/dist/' \
  --exclude '.worktrees/' \
  "${ROOT_DIR}/" "${HOST}:${REMOTE_DIR}/"
SYNCED_REMOTE=1

echo "[build-with-y9000] remote build: host=${HOST} target=${TARGET}"
ssh "${SSH_OPTS[@]}" "${HOST}" bash -s -- \
  "${REMOTE_DIR}" "${REMOTE_BUILD_SCRIPT}" "${#FORWARD_ENV_ASSIGNMENTS[@]}" \
  "${FORWARD_ENV_ASSIGNMENTS[@]}" "${BUILD_ARGS[@]}" <<'EOF'
set -euo pipefail

REMOTE_DIR="$1"
REMOTE_BUILD_SCRIPT="$2"
ENV_COUNT="$3"
shift 3

declare -a FORWARD_ENV=()
for ((i = 0; i < ENV_COUNT; i++)); do
  FORWARD_ENV+=("$1")
  shift
done

cd "${REMOTE_DIR}"
env "${FORWARD_ENV[@]}" "${REMOTE_BUILD_SCRIPT}" "$@"
EOF

fix_remote_ownership

if [[ "${PULL_OUTPUT}" -eq 0 ]]; then
  echo "[build-with-y9000] skip pull output"
  exit 0
fi

echo "[build-with-y9000] sync remote artifacts -> local workspace"
case "${PULL_MODE}" in
  idf-upstream)
    pull_idf_artifacts \
      "${REMOTE_DIR}/upstream/ESP32-S3-PhotoPainter/01_Example/xiaozhi-esp32/build" \
      "${ROOT_DIR}/upstream/ESP32-S3-PhotoPainter/01_Example/xiaozhi-esp32/build"
    ;;
  rs)
    remote_rust_flasher_args="$(
      ssh "${SSH_OPTS[@]}" "${HOST}" bash -s -- "${REMOTE_DIR}" <<'EOF'
set -euo pipefail

REMOTE_DIR="$1"
cd "${REMOTE_DIR}"

python3 - <<'PY'
import pathlib

root = pathlib.Path("firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release/build")
matches = [path for path in root.glob("**/out/build/flasher_args.json") if path.is_file()]
if matches:
    newest = max(matches, key=lambda path: path.stat().st_mtime)
    print(newest.as_posix())
PY
EOF
    )"
    if [[ -z "${remote_rust_flasher_args}" ]]; then
      echo "[build-with-y9000] 未找到 Rust flasher_args.json" >&2
      exit 1
    fi
    pull_rs_runtime_artifacts \
      "${REMOTE_DIR}/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release" \
      "${ROOT_DIR}/firmware/photoframe-rs/target/xtensa-esp32s3-espidf/release" \
      "${remote_rust_flasher_args}"
    ;;
  *)
    echo "[build-with-y9000] 未知 pull mode：${PULL_MODE}" >&2
    exit 1
    ;;
esac

if [[ ! -e "${ROOT_DIR}/${VERIFY_PATH}" ]]; then
  echo "[build-with-y9000] 未拉回预期产物：${VERIFY_PATH}" >&2
  exit 1
fi

echo "[build-with-y9000] OK: ${ROOT_DIR}/${VERIFY_PATH}"
