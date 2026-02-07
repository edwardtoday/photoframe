#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IDF_PROJECT_DIR="firmware/photoframe-fw" "${SCRIPT_DIR}/idf-docker.sh" "idf.py set-target esp32s3 && idf.py build"
