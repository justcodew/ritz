# ritz

> fitz, reforged in Rust.

高性能 PDF 处理库（Rust + MuPDF），输出 API 兼容 PyMuPDF 的 Python 包。

[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)
[![Build](https://img.shields.io/badge/build-cargo%20build-green.svg)](https://github.com/justcodew/ritz)

## 为什么用 ritz

**性能**：在 32 MB / 43 页的真实 PDF 上对比 PyMuPDF（数据见 [benchmarks/benchmark_report.md](benchmarks/benchmark_report.md)）：

| 场景 | 加速比 |
|------|-------|
| 链接读取 | **27x** |
| 文档打开 | **6.6x** |
| 页面加载 | **2.0x** |
| 文本提取 | **2.1x** |
| 多文档并行 | **2.06x**（vs 串行） |

**兼容**：API 与 PyMuPDF 核心一致，迁移只需 `import fitz` → `import ritz`。

**安全**：Rust 端 RAII，C 层 `fz_try/fz_catch` 隔离 longjmp，杜绝 UB。

## 安装

### 1. 系统依赖

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

# 图片提取
for img in page.get_images():
    print(img["bbox"], img["width"], img["height"])
for img in page.get_images(include_data=True):
    open("out.png", "wb").write(img["image"])

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
| `doc.metadata(key)` | `doc.metadata.get(key)` | |
| `doc.is_encrypted` / `doc.needs_password()` | 同 | |
| `doc.authenticate(pw)` | 同 | |
| `doc.get_text_batch()` | — | **ritz 独有** |

### Page

| 方法 | 等价 PyMuPDF | 备注 |
|------|-------------|------|
| `page.rect` | `page.rect` | |
| `page.mediabox` / `page.cropbox` | 同 | |
| `page.rotation` | `page.rotation` | |
| `page.get_text(mode)` | 同 | mode: text/html/xhtml/xml/json/rawjson/words/blocks/dict/rawdict |
| `page.get_images(include_data=False)` | `page.get_images()` / `page.get_image_info()` | ritz 合并两者 |
| `page.get_links()` | 同 | |
| `page.get_pixmap(matrix/dpi/alpha)` | 同 | |
| `page.png(zoom, alpha)` | — | **ritz 独有**：直接返回 PNG bytes |

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
| `ritz.process_documents(paths, mode="text")` | **ritz 独有**：rayon 并行 |

## 与 PyMuPDF 的差异

| 特性 | ritz | PyMuPDF |
|------|------|---------|
| MuPDF 版本 | 1.27.0（git submodule） | 1.27.x wheel |
| 命名空间 | `import ritz` | `import fitz` |
| 文本搜索 | 未实现 | `page.search_for()` |
| 大纲 / TOC | 未实现 | `doc.get_toc()` |
| PDF 编辑（插入/删除页） | 未实现 | 支持 |
| PDF 写出 | 未实现 | `doc.write()` / `doc.save()` |
| 注释（annotations） | 未实现 | 支持 |
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

## 基准测试

```bash
# 微基准（criterion）
cargo bench -p mupdf

# vs PyMuPDF 对比
python benchmarks/bench_vs_pymupdf.py
```

报告：
- [crates/mupdf/benches/REPORT.md](crates/mupdf/benches/REPORT.md) — 微基准
- [benchmarks/benchmark_report.md](benchmarks/benchmark_report.md) — vs PyMuPDF 对比

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
- [ ] **阶段 5**（计划中）：文本搜索、大纲、PDF 编辑、JPEG、注释

## 许可证

AGPL-3.0（MuPDF 为 AGPL，本项目同步）。
