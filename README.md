# ritz

> fitz, reforged in Rust.

高性能 PDF 处理库（Rust + MuPDF），输出 API 兼容 PyMuPDF 的 Python 包。

[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)
[![Build](https://img.shields.io/badge/build-cargo%20build-green.svg)](https://github.com/justcodew/ritz)
[![Changelog](https://img.shields.io/badge/changelog-CHANGELOG.md-blue.svg)](CHANGELOG.md)

## 为什么用 ritz

**性能**：在 32 MB / 43 页的真实 PDF 上对比 PyMuPDF（完整数据见 [benchmarks/benchmark_report.md](benchmarks/benchmark_report.md)）：

| 场景 | 加速比 | 说明 |
|------|-------|------|
| 链接读取 | **27x** | C 层扁平化链表，一次 FFI |
| JPEG 图片提取 | **6.2x** | 返回原始 JPEG 字节，跳过 decode+re-encode |
| 文档打开 | **5.6x** | PyO3 零成本抽象 |
| 页面加载 | **1.9x** | 同上 |
| 批量提取 | **2.4x** | vs PyMuPDF per-page |
| 多文档并行 | **2.1x** | rayon vs multiprocessing |
| 文本提取 | **2.1x** | stext 构造是双方共享地板 |
| FlateDecode 图片 | **1.1x** | PNG 编码是双方共享瓶颈 |

> 加速比上界由"瓶颈在语言调度还是 MuPDF 内部计算"决定。纯调度场景（链接 27x）可达数量级；计算密集场景（文本/渲染/Flate 图片）双方共享 MuPDF C 引擎，上限 ~2x。详见报告中的"plan_v1 KPI 兑现账本"。

**一级图片提取**：`get_images(include_data=True)` 一次调用返回 bbox + 原始格式字节（JPEG/PNG/JPX），消除 PyMuPDF 的 `get_images()` + `extract_image(xref)` 两步调用。

**安全**：1000 页 PDF 峰值 RSS 72.9 MB（plan 目标 ≤200 MB）；1000 次循环泄漏 < 5 KB/iter。

**兼容**：API 与 PyMuPDF 核心一致，迁移只需 `import fitz` → `import ritz`。

**安全**：Rust 端 RAII，C 层 `fz_try/fz_catch` 隔离 longjmp，杜绝 UB。

## 安装

### 从 PyPI 安装（推荐）

```bash
pip install ritz-tool
```

> 包名是 `ritz-tool`（PyPI 上 `ritz` 已被占用），但 import 名仍然是 `ritz`：
> ```python
> import ritz
> doc = ritz.open("paper.pdf")
> ```

### 从源码构建

#### 1. 系统依赖

```bash
# macOS
xcode-select --install
brew install make

# Linux (Debian/Ubuntu)
sudo apt install build-essential pkg-config
```

### 2. 添加 MuPDF 子模块

```bash
git submodule add https://github.com/ArtifexSoftware/mupdf.git vendor/mupdf
cd vendor/mupdf && git checkout 1.27.0 && cd ..
git add vendor/mupdf
```

### 3. 构建 Python 包

```bash
# 创建 venv
python -m venv .venv && source .venv/bin/activate
pip install maturin

# 编译并安装（editable）
maturin develop --release

# 验证
python -c "import ritz; print(ritz.__version__)"
```

## 快速开始

```python
import ritz

# 打开文档（与 fitz.open 完全兼容）
doc = ritz.open("paper.pdf")
print(f"页数: {len(doc)}")

# 取一页
page = doc[0]

# 文本提取（10 种模式，与 PyMuPDF 一致）
text = page.get_text("text")           # 纯文本
words = page.get_text("words")         # 词级 bbox
blocks = page.get_text("blocks")       # 块级 bbox
d = page.get_text("dict")              # 嵌套 block→line→span
raw = page.get_text("rawdict")         # dict + 逐字符 bbox
html = page.get_text("html")           # HTML 输出
j = page.get_text("json")              # JSON 序列化
rj = page.get_text("rawjson")          # JSON of rawdict

# 页面属性
print(page.rect, page.rotation, page.mediabox)

# 图片提取（一级 API：bbox + 原始格式字节一次调用）
for img in page.get_images():
    print(img["bbox"], img["width"], img["height"])
for img in page.get_images(include_data=True):
    open(f"out.{img['ext']}", "wb").write(img["image"])  # ext: "jpeg"/"png"/"jpx"

# 链接
for link in page.get_links():
    print(link)

# 渲染
pix = page.get_pixmap(dpi=144)
pix.tobytes("png")                     # bytes
pix.save_png("page.png")
```

## 批量与并行

ritz 提供原生并行扩展，是相对 PyMuPDF 的核心性能卖点：

```python
import ritz

# 单文档批量提取：1 次 FFI 调用拿全部页文本
pages_text = doc.get_text_batch()
# 等价于 [doc[i].get_text("text") for i in range(len(doc))]，但更快

# 多文档并行：rayon 自动调度
paths = ["a.pdf", "b.pdf", "c.pdf", ...]
results = ritz.process_documents(paths)
# results[i] = list[str] (每页文本) 或 None (失败)
```

## API 速查

### Document

| 方法 | 等价 PyMuPDF | 备注 |
|------|-------------|------|
| `ritz.open(path)` | `fitz.open(path)` | |
| `len(doc)` / `doc.page_count` | `len(doc)` / `doc.page_count` | |
| `doc[i]` / `doc.load_page(i)` | `doc[i]` / `doc.load_page(i)` | |
| `doc.metadata` | `doc.metadata` | @property 返回 dict |
| `doc.lookup_metadata(key)` | — | 直接查 MuPDF key（如 `"info:Title"`） |
| `doc.is_encrypted` / `doc.needs_password()` | 同 | |
| `doc.authenticate(pw)` | 同 | |
| `doc.get_text_batch()` | — | **ritz 独有** |
| `doc.save(path)` | `doc.save(path)` | |
| `doc.get_toc()` | `doc.get_toc(simple=True)` | 返回 `list[(level, title, page)]` 或 None |
| `doc.set_toc(toc)` | `doc.set_toc(toc)` | 接受同 get_toc 格式 |
| `doc.resolve_names()` | — | 返回 `dict[str, (page, x, y)]` |
| `doc.new_page(pno=-1, width=595, height=842, rotate=0)` | `doc.new_page(...)` | 插入空白页，返回 Page |
| `doc.delete_page(pno=-1)` | `doc.delete_page(pno)` | -1 = 末页 |
| `doc.delete_pages(from_page=-1, to_page=-1)` | `doc.delete_pages(from, to)` | 闭区间 |
| `doc.move_page(src, dst)` | `doc.move_page(src, dst)` | |
| `doc.copy_page(src, dst)` | `doc.copy_page(src, dst)` | 同文档克隆 |
| `doc.insert_pdf(docsrc, start=0, end=-1, at=-1)` | `doc.insert_pdf(docsrc, ...)` | 内部重开 src 以共享 ctx |

### Page

| 方法 | 等价 PyMuPDF | 备注 |
|------|-------------|------|
| `page.rect` | `page.rect` | |
| `page.mediabox` / `page.cropbox` | 同 | |
| `page.rotation` | `page.rotation` | |
| `page.get_text(mode)` | 同 | mode: text/html/xhtml/xml/json/rawjson/words/blocks/dict/rawdict |
| `page.get_images(include_data=False)` | `page.get_images()` / `page.get_image_info()` | ritz 合并两者；`include_data=True` 时含 `ext` + `image`（原始格式字节，非重编码 PNG） |
| `page.get_links()` | 同 | |
| `page.get_pixmap(matrix/dpi/alpha)` | 同 | |
| `page.png(zoom, alpha)` | — | **ritz 独有**：直接返回 PNG bytes |
| `page.search_for(text)` | `page.search_for(text)` | 返回 quad 列表 |
| `page.get_annotations()` | `annots = page.annots()` | 返回 `list[dict]`，每条含 type/rect/contents/color/quads |
| `page.add_highlight_annot(quads, ...)` | `page.add_highlight_annot(quads)` | 类似：underline/strikeout/text |
| `page.delete_annot(index)` | `annot.delete()` | 按索引删除 |

### Pixmap

| 方法 | 等价 PyMuPDF | 备注 |
|------|-------------|------|
| `pix.width` / `pix.height` / `pix.stride` / `pix.components` | 同 | |
| `pix.samples` | `pix.samples` | bytes |
| `pix.tobytes("png")` | 同 | |
| `pix.save_png(path)` | `pix.save(filename)` | |

### 顶层

| 函数 | 备注 |
|------|------|
| `ritz.process_documents(paths, mode="text")` | **ritz 独有**：rayon 并行；mode 可选 text/html/xhtml/xml/json |

## 与 PyMuPDF 的差异

| 特性 | ritz | PyMuPDF |
|------|------|---------|
| MuPDF 版本 | 1.27.0（git submodule） | 1.27.x wheel |
| 命名空间 | `import ritz` | `import fitz` |
| 文本搜索 | `page.search_for()` | 同 |
| 大纲 / TOC | `doc.get_toc()` / `doc.set_toc()` | 同 |
| PDF 编辑（插入/删除页） | 支持（new_page/delete_page/delete_pages/move_page/copy_page/insert_pdf） | 支持 |
| PDF 写出 | `doc.save()` | `doc.write()` / `doc.save()` |
| 注释（annotations） | 读写（highlight/underline/strikeout/text + delete） | 支持 |
| 命名目标 | `doc.resolve_names()` | — |
| JPEG 编码 | 未实现 | `pix.tobytes("jpeg")` |
| 多文档并行 | **原生 rayon** | 需 multiprocessing |

## 架构

四层设计：

| crate | 职责 |
|-------|------|
| `mupdf-sys` | FFI 绑定 + C 错误包装层（longjmp 隔离）+ 批量扁平化扩展 |
| `mupdf` | 安全 RAII 封装、错误转换、坐标系转换 |
| `ritz` | 高层 Rust API + PyO3 Python 绑定（输出 `_ritz` cdylib） |
| `python/ritz` | 纯 Python re-export 层（`from ._ritz import *`） |

### 关键技术决策

1. **longjmp 隔离**：所有 MuPDF 调用包在 C 层的 `fz_try/fz_catch` 里，longjmp 永不穿透 Rust 栈（否则 Drop 不执行 = UB）。
2. **C 层扁平化**：stext/image/links 的链表/树遍历都在 C 层完成，输出连续二进制 buffer 给 Rust，避免 N 次 FFI 往返和 Python 对象构造开销（链接读取 27x 的根因）。
3. **`Py<PyDocument>` 引用计数**：Page 和 Pixmap 持有 `Py<PyDocument>` 句柄防止 ctx 在 GC 时被先释放。
4. **rayon 并行**：每文档独立 fz_context（无锁函数），`py.detach` 释放 GIL 让 worker 真正并行。
5. **patches/ 工作流**（[plan_v1 §5.5](plan/01-plan-v1.md)）：`build.rs` 在编译前自动应用 `patches/*.patch` 到 MuPDF 子模块，幂等跳过已应用的。详见 [patches/README.md](patches/README.md)。
6. **`PdfValue` 数据模型**（[plan_v1 §5.2](plan/01-plan-v1.md) Rennie）：10 种 PDF 对象类型在 Rust 用 `enum`，已暴露给 Python：`from ritz import PdfValue`，支持 `null/bool/int/float/name/str/array/dict/stream/ref` 全套构造和 `to_python()` 转换。

## 基准测试

```bash
# 微基准（criterion）
cargo bench -p mupdf

# vs PyMuPDF 对比
python benchmarks/bench_vs_pymupdf.py

# 公开 corpus 对比（veraPDF + pdf.js + SafeDocs）
python benchmarks/bench_corpus.py
```

报告：
- [crates/mupdf/benches/REPORT.md](crates/mupdf/benches/REPORT.md) — 微基准
- [benchmarks/benchmark_report.md](benchmarks/benchmark_report.md) — vs PyMuPDF 对比

### Corpus 对比（2907 PDFs，macOS arm64）

在三个公开 PDF 测试集（veraPDF corpus + Mozilla pdf.js test/pdfs + DARPA SafeDocs）上对比 ritz 与 PyMuPDF 文本提取性能（脚本 `benchmarks/bench_corpus.py`，超时 60s）：

| Library | Mean | p50 | p95 | p99 | Pass Rate | License |
|---------|------|-----|-----|-----|-----------|---------|
| ritz | 1.0ms | 0.4ms | 3.3ms | 4.2ms | 100.0% (2907/2907) | AGPL-3.0 |
| PyMuPDF | 0.6ms | 0.3ms | 2.1ms | 3.3ms | 100.0% (2907/2907) | AGPL-3.0 |

> ritz 和 PyMuPDF 共享 MuPDF 1.27 文本提取引擎，单文档文本提取是双方共享瓶颈场景（详见 [benchmarks/benchmark_report.md](benchmarks/benchmark_report.md) 的 plan_v1 KPI 兑现账本），故接近同量级。ritz 的核心优势在 **批量并行**（单文档 `get_text_batch` 1 次 FFI 拿全部页；多文档 `process_documents` 用 rayon 并行释放 GIL），以及 **链接读取 27x**、**JPEG 图片提取 6.2x** 等 C 层扁平化场景。

## 测试

```bash
# Rust 集成测试
cargo test -p mupdf --test integration_test

# PdfValue 单元测试
cargo test -p ritz value

# Python pytest 套件（覆盖所有 API）
pytest python/tests/
```

## 开发路线

- [x] **阶段 1**：构建基础设施（MuPDF 编译 + FFI + 集成测试）
- [x] **阶段 2**：文本提取 + PyO3 最小可用包
- [x] **阶段 3**：完整 API（10 种 get_text 模式 + 图片 + 链接 + 元数据）
- [x] **阶段 4**：批量扩展 + rayon 并行 + criterion 基准 + 测试套件
- [x] **plan_v1 KPI 兑现验证**：图片提取原始字节优化（JPEG 6.2x）、安全验证（1000 页 ≤200MB、泄漏 <5KB/iter）、诚实性能账本
- [x] **plan_v1 缺口补齐**：`get_images` 加 xref（与 PyMuPDF 一致）、`metadata` 改 @property dict、`PdfValue` 暴露给 Python、`process_documents` 多模式（text/html/xhtml/xml/json）、`patches/` 工作流（build.rs 自动应用）、wheel ≤35MB（实测 29MB）
- [x] **Tier 1+2 实现层优化**：GIL 释放（多文档并行渲染 15.4x）、memoize（metadata/rect/raw_doc）、PNG tobytes 去重复拷贝（1.37x）、words/blocks 单趟 growbuf、PyString fast path。详见 [CHANGELOG.md](CHANGELOG.md) 和 [docx/10-tier1-tier2-results.md](docx/10-tier1-tier2-results.md)
- [x] **阶段 5a**：文本搜索 `page.search_for()` + 大纲读取 `doc.get_toc()`
- [x] **阶段 5b**：注释读写（highlight/underline/strikeout/text + delete）
- [x] **阶段 5c**：大纲写入 `doc.set_toc()`
- [x] **阶段 5d**：命名目标 `doc.resolve_names()` + 文档保存 `doc.save()`
- [x] **阶段 5e**：PDF 页面编辑（new_page/delete_page/delete_pages/move_page/copy_page/insert_pdf，PyMuPDF 兼容）

> 每个版本的完整改动记录见 [CHANGELOG.md](CHANGELOG.md)。

## 许可证

AGPL-3.0（MuPDF 为 AGPL，本项目同步）。
