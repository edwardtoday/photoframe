#!/usr/bin/env python3
"""下载并整理 ESP32-S3-PhotoPainter 相关资料。"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import sys
from pathlib import Path
from urllib.parse import urlparse
from urllib.request import Request, urlopen

WIKI_URL = "https://www.waveshare.net/wiki/ESP32-S3-PhotoPainter"
OFFICIAL_REPO = "waveshareteam/ESP32-S3-PhotoPainter"
GITHUB_API_ROOT = "https://api.github.com"

# 固定白名单：确保关键资料可重复拉取，避免页面改版导致遗漏。
ASSETS = [
    ("official", "https://files.waveshare.net/wiki/ESP32-S3-PhotoPainter/ESP32-S3-PhotoPainter-Demo.zip"),
    ("official", "https://www.waveshare.net/w/upload/0/05/ESP32-S3-PhotoPainter-Schematic.pdf"),
    ("official", "https://www.waveshare.net/w/upload/a/a8/PhotoPainter_dimension.pdf"),
    ("official", "https://www.waveshare.net/w/upload/5/5b/7.3inch-e-Paper-%28E%29-user-manual.pdf"),
    ("samples", "https://www.waveshare.net/w/upload/3/37/PhotoPainter_B-BMP.zip"),
    ("tools", "https://www.waveshare.net/w/upload/5/5b/ConverTo6c_bmp-7.3.zip"),
    ("datasheets", "https://www.waveshare.net/w/upload/4/48/ES7210_DS.pdf"),
    ("datasheets", "https://www.waveshare.net/w/upload/5/56/ES8311.user.Guide.pdf"),
    ("datasheets", "https://www.waveshare.net/w/upload/6/65/ES8311.DS.pdf"),
    ("datasheets", "https://www.waveshare.net/w/upload/3/33/SHTC3_Datasheet.pdf"),
    ("datasheets", "https://www.waveshare.net/w/upload/c/c0/Pcf85063atl1118-NdPQpTGE-loeW7GbZ7.pdf"),
    ("datasheets", "https://www.waveshare.net/w/upload/5/58/Esp32-s3_datasheet_cn.pdf"),
    ("datasheets", "https://www.waveshare.net/w/upload/b/bd/Esp32-s3_datasheet_en.pdf"),
    ("datasheets", "https://www.waveshare.net/w/upload/8/88/Esp32-s3_technical_reference_manual_cn.pdf"),
    ("datasheets", "https://www.waveshare.net/w/upload/1/11/Esp32-s3_technical_reference_manual_en.pdf"),
    ("espressif", "https://dl.espressif.com/public/flash_download_tool.zip"),
    ("espressif", "https://dl.espressif.com/dl/idf-driver/idf-driver-esp32-usb-jtag-2021-07-15.zip"),
]


def fetch_bytes(url: str, timeout: int) -> bytes:
    req = Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urlopen(req, timeout=timeout) as resp:
        return resp.read()


def fetch_json(url: str, timeout: int) -> object:
    headers = {
        "User-Agent": "photoframe-fetch-waveshare-assets/1.0",
        "Accept": "application/vnd.github+json",
    }
    token = os.getenv("GITHUB_TOKEN", "").strip()
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = Request(url, headers=headers)
    with urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def file_name_from_url(url: str) -> str:
    path = urlparse(url).path
    name = Path(path).name
    if not name:
        raise ValueError(f"无法从 URL 提取文件名: {url}")
    return name


def sha256sum(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def save_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def safe_release_tag(tag: str) -> str:
    return re.sub(r"[^0-9A-Za-z._-]+", "_", tag.strip() or "untagged")


def fetch_github_releases(repo: str, timeout: int) -> list[dict]:
    url = f"{GITHUB_API_ROOT}/repos/{repo}/releases?per_page=100"
    data = fetch_json(url, timeout)
    if not isinstance(data, list):
        raise ValueError(f"unexpected GitHub release payload: {type(data)}")
    return [item for item in data if isinstance(item, dict)]


def main() -> int:
    parser = argparse.ArgumentParser(description="拉取 ESP32-S3-PhotoPainter 资料")
    parser.add_argument("--force", action="store_true", help="覆盖已下载文件")
    parser.add_argument("--timeout", type=int, default=60, help="HTTP 超时时间（秒）")
    parser.add_argument("--github-repo", default=OFFICIAL_REPO, help="GitHub 发布源（owner/repo）")
    parser.add_argument("--skip-github-releases", action="store_true", help="跳过 GitHub releases 下载")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[1]
    waveshare_root = repo_root / "references" / "waveshare"
    wiki_dir = waveshare_root / "wiki"
    dl_root = waveshare_root / "downloads"

    wiki_dir.mkdir(parents=True, exist_ok=True)
    dl_root.mkdir(parents=True, exist_ok=True)

    print(f"[info] 下载 wiki 页面: {WIKI_URL}")
    wiki_html = fetch_bytes(WIKI_URL, args.timeout).decode("utf-8", errors="ignore")
    save_text(wiki_dir / "ESP32-S3-PhotoPainter.html", wiki_html)

    links = sorted(set(re.findall(r"https?://[^\"'<>\s]+", wiki_html)))
    save_text(wiki_dir / "links-from-page.txt", "\n".join(links) + "\n")

    manifest: list[dict[str, str | int]] = []
    github_release_manifest: dict[str, object] = {
        "repo": args.github_repo,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "releases": [],
    }

    for category, url in ASSETS:
        name = file_name_from_url(url)
        out_dir = dl_root / category
        out_dir.mkdir(parents=True, exist_ok=True)
        out_file = out_dir / name

        if out_file.exists() and not args.force:
            status = "cached"
            print(f"[skip] {out_file.relative_to(repo_root)}")
        else:
            print(f"[get ] {url}")
            data = fetch_bytes(url, args.timeout)
            out_file.write_bytes(data)
            status = "downloaded"

        manifest.append(
            {
                "category": category,
                "file": str(out_file.relative_to(repo_root)),
                "url": url,
                "size": out_file.stat().st_size,
                "sha256": sha256sum(out_file),
                "status": status,
            }
        )

    if not args.skip_github_releases:
        release_root = dl_root / "official" / "releases"
        releases = fetch_github_releases(args.github_repo, args.timeout)
        for release in releases:
            tag_name = str(release.get("tag_name") or "untagged")
            safe_tag = safe_release_tag(tag_name)
            assets = release.get("assets")
            if not isinstance(assets, list):
                assets = []

            release_item = {
                "tag_name": tag_name,
                "name": str(release.get("name") or ""),
                "draft": bool(release.get("draft")),
                "prerelease": bool(release.get("prerelease")),
                "published_at": str(release.get("published_at") or ""),
                "assets": [],
            }

            for asset in assets:
                if not isinstance(asset, dict):
                    continue
                asset_name = str(asset.get("name") or "")
                download_url = str(asset.get("browser_download_url") or "")
                if not asset_name or not download_url:
                    continue

                out_dir = release_root / safe_tag
                out_dir.mkdir(parents=True, exist_ok=True)
                out_file = out_dir / asset_name

                if out_file.exists() and not args.force:
                    status = "cached"
                    print(f"[skip] {out_file.relative_to(repo_root)}")
                else:
                    print(f"[get ] {download_url}")
                    data = fetch_bytes(download_url, args.timeout)
                    out_file.write_bytes(data)
                    status = "downloaded"

                sha = sha256sum(out_file)
                rel_item = {
                    "name": asset_name,
                    "file": str(out_file.relative_to(repo_root)),
                    "size": out_file.stat().st_size,
                    "sha256": sha,
                    "url": download_url,
                    "status": status,
                }
                release_item["assets"].append(rel_item)

                manifest.append(
                    {
                        "category": "official-release",
                        "file": str(out_file.relative_to(repo_root)),
                        "url": download_url,
                        "size": out_file.stat().st_size,
                        "sha256": sha,
                        "status": status,
                    }
                )

            github_release_manifest["releases"].append(release_item)

    official_readme_lines = [
        "# Waveshare 官方代码与发布文件",
        "",
        "- 官方仓库 submodule：`ESP32-S3-PhotoPainter/`",
        "- Releases 下载目录：`releases/<tag>/...`",
        "- Release 清单：`releases-manifest.json`",
        "",
        "说明：`releases/` 下的原始二进制默认不纳入 git，可通过脚本重拉。",
    ]
    save_text(dl_root / "official" / "README.md", "\n".join(official_readme_lines) + "\n")

    save_text(
        dl_root / "official" / "releases-manifest.json",
        json.dumps(github_release_manifest, indent=2, ensure_ascii=False) + "\n",
    )

    ts = dt.datetime.now(dt.timezone.utc).isoformat()
    manifest_path = dl_root / "manifest.json"
    manifest_path.write_text(
        json.dumps({"generated_at": ts, "items": manifest}, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )

    lines = [
        "# Waveshare 资料下载清单",
        "",
        f"- 生成时间（UTC）: `{ts}`",
        f"- Wiki 页面: `{WIKI_URL}`",
        "",
        "## 文件列表",
        "",
        "| 分类 | 文件 | 大小(字节) | SHA256 | 来源 |",
        "|---|---|---:|---|---|",
    ]

    for item in manifest:
        lines.append(
            f"| {item['category']} | `{item['file']}` | {item['size']} | `{item['sha256']}` | {item['url']} |"
        )

    lines.extend(
        [
            "",
        "## 说明",
        "",
        "- 下载原文件默认不纳入 git（见 `.gitignore`），用于本地离线查阅。",
        "- `references/waveshare/downloads/official/ESP32-S3-PhotoPainter` 为官方仓库 submodule。",
        "- GitHub Releases 资产下载清单见 `references/waveshare/downloads/official/releases-manifest.json`。",
        "- 若来源更新，可执行 `python3 scripts/fetch_waveshare_assets.py --force` 强制重拉。",
    ]
    )

    (dl_root / "README.md").write_text("\n".join(lines) + "\n", encoding="utf-8")

    print(f"[done] 共处理 {len(manifest)} 个文件，清单已写入 {manifest_path.relative_to(repo_root)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
