#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

IMAGE_REPO="${IMAGE_REPO:-edwardtoday/photoframe-orchestrator}"
TAG="${1:-}"
if [[ -z "${TAG}" ]]; then
  TAG="$(git -C "${REPO_ROOT}" rev-parse --short HEAD)"
fi

PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
BUILDER_NAME="${BUILDER_NAME:-photoframe-multi}"
CONTEXT_DIR="${REPO_ROOT}/services/photoframe-orchestrator"

if ! docker buildx inspect "${BUILDER_NAME}" >/dev/null 2>&1; then
  # 使用独立 buildx builder，避免污染默认环境。
  docker buildx create --name "${BUILDER_NAME}" --driver docker-container --use >/dev/null
else
  docker buildx use "${BUILDER_NAME}" >/dev/null
fi

docker buildx inspect --bootstrap >/dev/null

echo "[info] build and push: ${IMAGE_REPO}:${TAG} (plus latest)"
docker buildx build \
  --platform "${PLATFORMS}" \
  -t "${IMAGE_REPO}:${TAG}" \
  -t "${IMAGE_REPO}:latest" \
  --push \
  "${CONTEXT_DIR}"

echo "[info] inspect manifest: ${IMAGE_REPO}:${TAG}"
docker buildx imagetools inspect "${IMAGE_REPO}:${TAG}"

echo "[done] published ${IMAGE_REPO}:${TAG} and ${IMAGE_REPO}:latest"
