# 相框资料整理说明

## 已整理内容

- Waveshare Wiki 快照：`references/waveshare/wiki/ESP32-S3-PhotoPainter.html`
- Wiki 外链索引：`references/waveshare/wiki/links-from-page.txt`
- 下载清单：`references/waveshare/downloads/README.md`
- 下载元数据（含 SHA256）：`references/waveshare/downloads/manifest.json`

## 目录分类

- `references/waveshare/downloads/official/`：官方 Demo、原理图、尺寸图、墨水屏用户手册
- `references/waveshare/downloads/samples/`：示例 BMP 资源
- `references/waveshare/downloads/tools/`：图片转换工具
- `references/waveshare/downloads/datasheets/`：芯片/器件数据手册
- `references/waveshare/downloads/espressif/`：ESP32 烧录相关工具

## 更新方式

```bash
python3 scripts/fetch_waveshare_assets.py --force
```

