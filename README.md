# waifu2x repair

批量导入 PDF / 图片, 用 [unlimited:waifu2x](https://unlimited.waifu2x.net/) 同款 ONNX 在**本地**修复 (图片不会上传). 界面参数与网站一致: Backend / Model / DeNoise / Upscaling / Tile / Shuffle / TTA / Alpha.

PDF:
- 整页嵌入图直接抽像素
- 混排页在导入时选分辨率再光栅化
- 纯矢量页跳过 (无需修复)

失败 (含内存不足) 会重新排队, 并自动收缩并发或 tile.

## 依赖

发行包需在可执行文件同目录放 pdfium 动态库 (`pdfium.dll` / `libpdfium.dylib`). ONNX Runtime 由构建时下载并打进包.

## 构建

```
cargo build -r
```

macOS 用 GitHub Actions (见 `.github/workflows`).
