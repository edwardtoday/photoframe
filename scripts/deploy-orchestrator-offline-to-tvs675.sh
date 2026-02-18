#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
用法：scripts/deploy-orchestrator-offline-to-tvs675.sh [TAG]

离线投送 photoframe-orchestrator 到 QNAP/NAS，并通过 docker compose 重建容器。
特点：不执行 pull；使用 `docker compose up -d --pull never --force-recreate` 绕过拉取部署。

默认行为：
1) 本机构建 linux/amd64 镜像，并导出为 tar（docker load 格式）
2) scp tar 到 NAS
3) NAS 上 docker load
4) 在 REMOTE_DIR 下执行 docker compose up -d（不 pull，强制重建）

可选参数（环境变量）：
  HOST=tvs675
  IMAGE_REPO=edwardtoday/photoframe-orchestrator
  PLATFORM=linux/amd64
  BUILDER_NAME=photoframe-offline
  REMOTE_DIR=/share/ZFS19_DATA/Container/docker/photoframe-orchestrator
  REMOTE_DOCKER=...   # 可显式指定 QNAP Container Station 的 docker 路径
  SSH_IDENTITY_FILE=... # 额外指定 ssh 私钥（会自动加 -o IdentitiesOnly=yes -i）
  SSH_EXTRA_OPTS=...    # 追加 ssh 参数（按空格分割），用于 StrictHostKeyChecking 等
  SCP_EXTRA_OPTS=...    # 追加 scp 参数（按空格分割）

可选开关：
  DRY_RUN=1           # 只打印将执行的命令
  KEEP_LOCAL_TAR=1    # 保留本地 tar
  KEEP_REMOTE_TAR=1   # 保留 NAS 上的 tar
  NO_DB_BACKUP=1      # 不备份 sqlite db
EOF
}

log() { echo "[$(date +%H:%M:%S)] $*"; }

run() {
  if [[ "${DRY_RUN:-0}" == "1" ]]; then
    log "[dry-run] $*"
    return 0
  fi
  "$@"
}

run_remote_sh() {
  local host="$1"
  local script="$2"
  if [[ "${DRY_RUN:-0}" == "1" ]]; then
    log "[dry-run] ssh ${SSH_ARGS[*]} ${host} <<'SH'"
    printf '%s\n' "${script}"
    log "[dry-run] SH"
    return 0
  fi
  ssh "${SSH_ARGS[@]}" "${host}" "${script}"
}

detect_remote_docker() {
  local host="$1"
  if [[ -n "${REMOTE_DOCKER:-}" ]]; then
    echo "${REMOTE_DOCKER}"
    return 0
  fi

  if [[ "${DRY_RUN:-0}" == "1" ]]; then
    # dry-run 下不强制依赖远端可达；此值仅用于打印将执行的 remote 脚本。
    echo "docker"
    return 0
  fi

  # QNAP 的 docker 常由 Container Station 提供，PATH 不一定包含它。
  ssh "${SSH_ARGS[@]}" "${host}" '(
    if command -v docker >/dev/null 2>&1; then
      command -v docker
      exit 0
    fi
    for p in \
      /share/*/.qpkg/container-station/bin/docker \
      /share/*/*/.qpkg/container-station/bin/docker \
      /share/CACHEDEV1_DATA/.qpkg/container-station/bin/docker \
      /share/CACHEDEV2_DATA/.qpkg/container-station/bin/docker
    do
      if [ -x "$p" ]; then
        echo "$p"
        exit 0
      fi
    done
    exit 1
  )'
}

TAG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      TAG="${1:-}"
      shift || true
      break
      ;;
    -*)
      echo "[error] unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
    *)
      if [[ -n "${TAG}" ]]; then
        echo "[error] too many args" >&2
        usage >&2
        exit 1
      fi
      TAG="$1"
      shift
      ;;
  esac
done
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

HOST="${HOST:-tvs675}"
IMAGE_REPO="${IMAGE_REPO:-edwardtoday/photoframe-orchestrator}"
PLATFORM="${PLATFORM:-linux/amd64}"
BUILDER_NAME="${BUILDER_NAME:-photoframe-offline}"
REMOTE_DIR="${REMOTE_DIR:-/share/ZFS19_DATA/Container/docker/photoframe-orchestrator}"

SSH_ARGS=()
SCP_ARGS=()
if [[ -n "${SSH_IDENTITY_FILE:-}" ]]; then
  SSH_ARGS+=(-o IdentitiesOnly=yes -i "${SSH_IDENTITY_FILE}")
  SCP_ARGS+=(-o IdentitiesOnly=yes -i "${SSH_IDENTITY_FILE}")
fi
if [[ -n "${SSH_EXTRA_OPTS:-}" ]]; then
  # Intentionally split on whitespace so users can pass multiple "-o ..." options.
  # shellcheck disable=SC2206
  extra=( ${SSH_EXTRA_OPTS} )
  SSH_ARGS+=("${extra[@]}")
fi
if [[ -n "${SCP_EXTRA_OPTS:-}" ]]; then
  # shellcheck disable=SC2206
  extra=( ${SCP_EXTRA_OPTS} )
  SCP_ARGS+=("${extra[@]}")
fi

if [[ -z "${TAG}" ]]; then
  TAG="$(git -C "${REPO_ROOT}" rev-parse --short HEAD)"
fi

platform_slug="${PLATFORM//\//_}"
LOCAL_TAR="/tmp/photoframe-orchestrator_${TAG}_${platform_slug}.tar"
REMOTE_TAR="${REMOTE_DIR}/photoframe-orchestrator_${TAG}_${platform_slug}.tar"

log "tag=${TAG}"
log "host=${HOST}"
log "platform=${PLATFORM}"
log "image=${IMAGE_REPO}:${TAG} (plus latest)"
log "local_tar=${LOCAL_TAR}"
log "remote_tar=${REMOTE_TAR}"

log "ensure buildx builder: ${BUILDER_NAME}"
if [[ "${DRY_RUN:-0}" == "1" ]]; then
  log "[dry-run] docker buildx inspect/create/use/bootstrap ..."
else
  if ! docker buildx inspect "${BUILDER_NAME}" >/dev/null 2>&1; then
    docker buildx create --name "${BUILDER_NAME}" --driver docker-container --use >/dev/null
  else
    docker buildx use "${BUILDER_NAME}" >/dev/null
  fi
  docker buildx inspect --bootstrap >/dev/null
fi

log "build (offline export): ${LOCAL_TAR}"
run rm -f "${LOCAL_TAR}"
run docker buildx build \
  --platform "${PLATFORM}" \
  -t "${IMAGE_REPO}:${TAG}" \
  -t "${IMAGE_REPO}:latest" \
  --output "type=docker,dest=${LOCAL_TAR}" \
  "${REPO_ROOT}/services/photoframe-orchestrator"

log "scp to NAS: ${HOST}:${REMOTE_TAR}"
run scp "${SCP_ARGS[@]}" -q "${LOCAL_TAR}" "${HOST}:${REMOTE_TAR}"

log "detect remote docker path"
REMOTE_DOCKER_RESOLVED="$(detect_remote_docker "${HOST}" || true)"
if [[ -z "${REMOTE_DOCKER_RESOLVED}" ]]; then
  echo "[error] failed to detect remote docker path; set REMOTE_DOCKER=... and retry" >&2
  exit 1
fi
log "remote_docker=${REMOTE_DOCKER_RESOLVED}"

log "remote deploy (load + compose up)"
run_remote_sh "${HOST}" "$(
  cat <<SH
set -e

DOCKER="${REMOTE_DOCKER_RESOLVED}"
REMOTE_DIR="${REMOTE_DIR}"
REMOTE_TAR="${REMOTE_TAR}"

if [ -x "\${DOCKER}" ]; then
  :
elif command -v "\${DOCKER}" >/dev/null 2>&1; then
  :
else
  echo "[error] docker not found or not executable: \${DOCKER}" >&2
  exit 1
fi
if [ ! -d "\${REMOTE_DIR}" ]; then
  echo "[error] remote dir not found: \${REMOTE_DIR}" >&2
  exit 1
fi
if [ ! -f "\${REMOTE_TAR}" ]; then
  echo "[error] remote tar not found: \${REMOTE_TAR}" >&2
  exit 1
fi

cd "\${REMOTE_DIR}"

if [ "\${NO_DB_BACKUP:-0}" != "1" ] && [ -f "./data/orchestrator.db" ]; then
  ts="\$(date +%Y%m%d-%H%M%S)"
  cp "./data/orchestrator.db" "./data/orchestrator.db.bak.\${ts}"
  echo "[info] db backup created: ./data/orchestrator.db.bak.\${ts}"
fi

"\${DOCKER}" load -i "\${REMOTE_TAR}"

# 关键：不 pull，强制重建让容器切到新 image id（即使 tag 仍是 latest）。
"\${DOCKER}" compose up -d --pull never --force-recreate

"\${DOCKER}" ps --filter name=photoframe-orchestrator --format "table {{.Names}}\\t{{.Image}}\\t{{.Status}}"

if [ "\${KEEP_REMOTE_TAR:-0}" != "1" ]; then
  rm -f "\${REMOTE_TAR}"
fi
SH
)"

if [[ "${KEEP_LOCAL_TAR:-0}" != "1" ]]; then
  log "cleanup local tar"
  run rm -f "${LOCAL_TAR}"
fi

log "done"
