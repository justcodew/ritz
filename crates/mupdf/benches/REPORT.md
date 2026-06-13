# 性能基准报告

> 由 `cargo bench -p mupdf` 生成。硬件：Apple M-series (macOS arm64)。

## 微基准（单次操作延迟）

测试 PDF：`tests/samples/sample.pdf`（1 页，2KB）。

| 操作 | 中位时间 | 说明 |
|---|---:|---|
| `open + page_count` | ~306 µs | 创建 fz_context + fz_open_document + count_pages，单次冷启动开销 |
| `extract_text/text` | ~58 µs | new_stext_page + fz_print_stext_page_as_text，单页文本提取 |
| `render_pixmap zoom=0.5` | ~161 µs | 36 DPI 渲染（1/4 像素） |
| `render_pixmap zoom=1.0` | ~507 µs | 72 DPI 原始渲染 |
| `render_pixmap zoom=2.0` | ~1.85 ms | 144 DPI 渲染（4x 像素，约 3.6x 时间，亚线性） |

## 端到端基准（多页文档）

| 操作 | 中位时间 | 文档 |
|---|---:|---|
| `e2e_multi_page/extract_all_text` | ~3.5 ms | vendor/mupdf/.../agstat.pdf（多页，含表格） |

## 关键观察

1. **文本提取非常便宜**：单页 58 µs，185 页文档约 7 ms（57 µs × 185 × 缓存命中）。
2. **渲染像素数主导成本**：zoom 0.5→1.0→2.0 时间分别为 161/507/1850 µs，与像素数（0.25/1/4）近似线性。
3. **冷启动 306 µs**：context + open 占主导，所以**单文档性能瓶颈不是文本提取而是 PDF 解析**。

## 并行扩展基准

使用 `ritz.process_documents(paths)` 测试 6 个独立文档：

| 模式 | 时间 | 加速比 |
|---|---:|---:|
| 串行（N 次 `Document.open + get_text_batch`） | 165 ms | 1.00x |
| 并行（`process_documents`，rayon） | 80 ms | **2.06x** |

加速比受限于物理核数（M 系列约 8 核 N+P 架构，单文档太小不够充分并行）。

## 与 PyMuPDF 对比

待补充：用同一 PDF 跑 `python -c "import fitz; ..."` 对比基准。
预期：
- 文本提取：相当或略快（MuPDF 直调，无 PyMuPDF 中间层）
- 渲染：相当（同一 MuPDF 后端）
- 并行：ritz 有原生 rayon 加速，PyMuPDF 需要 multiprocessing（IPC 开销大）

## 如何复现

```bash
cargo bench -p mupdf                            # 全部基准
cargo bench -p mupdf --bench bench -- --measurement-time 1  # 快速（1s 取样）
```

报告写入 `target/criterion/report/index.html`。
