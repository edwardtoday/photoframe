# Waveshare 官方代码与发布文件

- 官方仓库 submodule：`ESP32-S3-PhotoPainter/`
- Releases 下载目录：`releases/<tag>/...`
- Release 清单：`releases-manifest.json`
- 一键同步脚本：`scripts/sync-waveshare-official.sh`

## 一键同步

```bash
# 默认：更新 submodule + 重拉 releases
scripts/sync-waveshare-official.sh

# 全量强刷（含固定白名单资料）
scripts/sync-waveshare-official.sh --full-refresh
```

说明：`releases/` 下的原始二进制默认不纳入 git，可通过脚本重拉。
