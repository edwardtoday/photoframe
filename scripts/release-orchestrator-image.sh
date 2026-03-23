#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

IMAGE_REPO="${IMAGE_REPO:-edwardtoday/photoframe-orchestrator}"
TAG="${1:-}"
APP_VERSION="${PHOTOFRAME_ORCHESTRATOR_VERSION:-0.2.8}"
if [[ -z "${TAG}" ]]; then
  TAG="$(git -C "${REPO_ROOT}" rev-parse --short HEAD)"
fi
APP_GIT_SHA="$(git -C "${REPO_ROOT}" rev-parse --short=8 HEAD)"
if ! git -C "${REPO_ROOT}" diff --quiet --exit-code; then
  APP_GIT_SHA="${APP_GIT_SHA}-dirty"
fi

PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
BUILDER_NAME="${BUILDER_NAME:-photoframe-multi}"
CONTEXT_DIR="${REPO_ROOT}/services/photoframe-orchestrator"
ENABLE_REBASE_FALLBACK="${ENABLE_REBASE_FALLBACK:-1}"

if ! docker buildx inspect "${BUILDER_NAME}" >/dev/null 2>&1; then
  # 使用独立 buildx builder，避免污染默认环境。
  docker buildx create --name "${BUILDER_NAME}" --driver docker-container --use >/dev/null
else
  docker buildx use "${BUILDER_NAME}" >/dev/null
fi

docker buildx inspect --bootstrap >/dev/null

run_main_build() {
  docker buildx build \
    --platform "${PLATFORMS}" \
    -t "${IMAGE_REPO}:${TAG}" \
    -t "${IMAGE_REPO}:latest" \
    --build-arg "PHOTOFRAME_ORCHESTRATOR_VERSION=${APP_VERSION}" \
    --build-arg "PHOTOFRAME_ORCHESTRATOR_GIT_SHA=${APP_GIT_SHA}" \
    --push \
    "${CONTEXT_DIR}"
}

run_rebase_build() {
  local tmp_dockerfile
  tmp_dockerfile="$(mktemp -t photoframe-orchestrator-rebase.XXXXXX.Dockerfile)"
  trap 'rm -f "${tmp_dockerfile}"' RETURN

  cat >"${tmp_dockerfile}" <<DOCKERFILE
FROM ${IMAGE_REPO}:latest
ARG PHOTOFRAME_ORCHESTRATOR_VERSION=0.2.8
ARG PHOTOFRAME_ORCHESTRATOR_GIT_SHA=nogit
ENV PHOTOFRAME_ORCHESTRATOR_VERSION=\${PHOTOFRAME_ORCHESTRATOR_VERSION} \\
    PHOTOFRAME_ORCHESTRATOR_GIT_SHA=\${PHOTOFRAME_ORCHESTRATOR_GIT_SHA}
COPY services/photoframe-orchestrator/app /app/app
COPY services/photoframe-orchestrator/data /app/data
DOCKERFILE

  # 失败兜底：复用 latest 依赖层，仅替换应用代码和默认数据。
  docker buildx build \
    --platform "${PLATFORMS}" \
    -f "${tmp_dockerfile}" \
    -t "${IMAGE_REPO}:${TAG}" \
    -t "${IMAGE_REPO}:latest" \
    --build-arg "PHOTOFRAME_ORCHESTRATOR_VERSION=${APP_VERSION}" \
    --build-arg "PHOTOFRAME_ORCHESTRATOR_GIT_SHA=${APP_GIT_SHA}" \
    --push \
    "${REPO_ROOT}"
}

echo "[info] build and push: ${IMAGE_REPO}:${TAG} (plus latest)"
echo "[info] app version: ${APP_VERSION}"
echo "[info] app git sha: ${APP_GIT_SHA}"
if run_main_build; then
  echo "[info] primary build path succeeded"
else
  if [[ "${ENABLE_REBASE_FALLBACK}" != "1" ]]; then
    echo "[error] primary build failed and fallback disabled" >&2
    exit 1
  fi

  echo "[warn] primary build failed, fallback to rebase-on-latest path"
  run_rebase_build
fi

echo "[info] inspect manifest: ${IMAGE_REPO}:${TAG}"
docker buildx imagetools inspect "${IMAGE_REPO}:${TAG}"

echo "[done] published ${IMAGE_REPO}:${TAG} and ${IMAGE_REPO}:latest"
