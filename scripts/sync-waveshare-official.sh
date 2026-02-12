#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<EOF
用法：scripts/sync-waveshare-official.sh [--full-refresh] [--timeout 秒]

默认行为：
1) 更新官方 submodule 到远端最新提交
2) 清空 releases 缓存并重拉 GitHub Releases 资产
3) 刷新下载清单（manifest / README）

可选参数：
  --full-refresh   强制重拉全部资料（含 Wiki 固定白名单资产）
  --timeout N      设置下载超时秒数（默认 120）
  -h, --help       显示帮助
EOF
}

FULL_REFRESH=0
TIMEOUT=120

while [[ $# -gt 0 ]]; do
  case "$1" in
    --full-refresh)
      FULL_REFRESH=1
      shift
      ;;
    --timeout)
      if [[ $# -lt 2 ]]; then
        echo "[error] --timeout 需要一个整数参数" >&2
        exit 1
      fi
      TIMEOUT="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "[error] 未知参数: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SUBMODULE_PATH="references/waveshare/downloads/official/ESP32-S3-PhotoPainter"
RELEASES_DIR="references/waveshare/downloads/official/releases"
RELEASES_MANIFEST="references/waveshare/downloads/official/releases-manifest.json"

echo "[info] 更新官方 submodule: ${SUBMODULE_PATH}"
git -C "${REPO_ROOT}" submodule sync -- "${SUBMODULE_PATH}"
git -C "${REPO_ROOT}" submodule update --init --remote "${SUBMODULE_PATH}"

FETCH_ARGS=(--timeout "${TIMEOUT}")
if [[ "${FULL_REFRESH}" == "1" ]]; then
  echo "[info] 启用全量刷新：将强制重拉全部资料"
  FETCH_ARGS+=(--force)
else
  # 默认只重拉 releases：删除 releases 缓存后调用下载脚本，其他固定资产将命中本地缓存。
  echo "[info] 清理 releases 缓存以触发重拉"
  rm -rf "${REPO_ROOT}/${RELEASES_DIR}"
  rm -f "${REPO_ROOT}/${RELEASES_MANIFEST}"
fi

echo "[info] 刷新 Waveshare 资料清单与 releases"
python3 "${REPO_ROOT}/scripts/fetch_waveshare_assets.py" "${FETCH_ARGS[@]}"

if [[ ! -d "${REPO_ROOT}/${SUBMODULE_PATH}" ]]; then
  echo "[error] submodule 路径不存在: ${SUBMODULE_PATH}" >&2
  exit 1
fi
if [[ ! -f "${REPO_ROOT}/${RELEASES_MANIFEST}" ]]; then
  echo "[error] releases 清单缺失: ${RELEASES_MANIFEST}" >&2
  exit 1
fi

echo "[done] 官方 submodule 与 releases 已同步完成"

