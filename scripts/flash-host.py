#!/usr/bin/env python3
"""在宿主机使用 esptool 烧录 IDF 构建产物（适配 macOS 无法 Docker USB 透传）。"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import sys
from pathlib import Path

DEFAULT_PROJECT = Path("upstream/ESP32-S3-PhotoPainter/01_Example/xiaozhi-esp32")
DEFAULT_VENV = Path(".venv-host-tools")


def _load_flasher_args(build_dir: Path) -> dict:
    path = build_dir / "flasher_args.json"
    if not path.exists():
        raise FileNotFoundError(f"未找到 {path}，请先完成 idf.py build")
    return json.loads(path.read_text(encoding="utf-8"))


def _norm_write_flash_args(value: object) -> list[str]:
    if value is None:
        return []
    if isinstance(value, list):
        return [str(v) for v in value]
    if isinstance(value, str):
        return shlex.split(value)
    raise TypeError(f"write_flash_args 类型不支持: {type(value)!r}")


def _norm_flash_files(value: object, build_dir: Path) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    if isinstance(value, dict):
        for offset, rel in value.items():
            pairs.append((str(offset), str((build_dir / rel).resolve())))
    elif isinstance(value, list):
        # 兼容少数格式：[[offset, file], ...]
        for item in value:
            if not isinstance(item, (list, tuple)) or len(item) != 2:
                raise ValueError(f"flash_files 列表格式非法: {item!r}")
            pairs.append((str(item[0]), str((build_dir / str(item[1])).resolve())))
    else:
        raise TypeError(f"flash_files 类型不支持: {type(value)!r}")

    pairs.sort(key=lambda x: int(x[0], 0))
    return pairs


def main() -> int:
    parser = argparse.ArgumentParser(description="使用宿主机 esptool 烧录固件")
    parser.add_argument("--project-dir", default=str(DEFAULT_PROJECT), help="ESP-IDF 工程目录")
    parser.add_argument("--port", required=True, help="串口设备，例如 /dev/cu.usbmodemxxxx")
    parser.add_argument("--baud", default="921600", help="烧录波特率")
    parser.add_argument("--venv", default=str(DEFAULT_VENV), help="host tools 虚拟环境目录")
    parser.add_argument("--chip", default=None, help="强制覆盖 chip（默认读取 flasher_args.json）")
    parser.add_argument("--dry-run", action="store_true", help="仅打印最终 esptool 命令，不实际烧录")
    args = parser.parse_args()

    project_dir = Path(args.project_dir).resolve()
    build_dir = project_dir / "build"
    venv_python = Path(args.venv).resolve() / "bin" / "python"

    if not venv_python.exists():
        print(f"[error] 未找到 host tools: {venv_python}")
        print("[hint] 先执行: scripts/setup-host-tools.sh")
        return 2

    data = _load_flasher_args(build_dir)

    extra = data.get("extra_esptool_args", {})
    chip = args.chip or extra.get("chip") or "esp32s3"
    before = extra.get("before") or "default_reset"
    after = extra.get("after") or "hard_reset"
    use_stub = extra.get("stub", True)

    cmd = [
        str(venv_python),
        "-m",
        "esptool",
        "--chip",
        chip,
        "--port",
        args.port,
        "--baud",
        str(args.baud),
        "--before",
        before,
        "--after",
        after,
    ]
    if not use_stub:
        cmd.append("--no-stub")

    cmd.append("write_flash")
    cmd.extend(_norm_write_flash_args(data.get("write_flash_args")))

    for offset, abs_file in _norm_flash_files(data.get("flash_files"), build_dir):
        cmd.extend([offset, abs_file])

    print("[info] 即将执行烧录命令：")
    print(" ".join(shlex.quote(x) for x in cmd))
    if args.dry_run:
        print("[dry-run] 跳过实际烧录")
        return 0

    subprocess.run(cmd, check=True, cwd=str(project_dir), env=os.environ.copy())

    print("[done] 烧录完成")
    return 0


if __name__ == "__main__":
    sys.exit(main())
