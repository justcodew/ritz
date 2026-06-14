# ritz vs pdf_oxide：架构与性能对比

> 测试日期：2026-06-14
> 测试平台：macOS Darwin 24.5.0 (Apple Silicon ARM64)
> 测试文件：arxiv_2603.09677_translated.pdf (32.79 MB, 43 页)、sample.pdf (1 页)、text_graphic_image.pdf (1 张 JPEG)
> 版本：ritz 0.1.0、pdf_oxide 0.3.64 (PyPI)、PyMuPDF 1.24.x

## pdf_oxide 是什么

[pdf_oxide](https://github.com/yfedoseev/pdf_oxide) 是另一个"超越 PyMuPDF"的 PDF 库：
- **Rust 核心**，纯 Rust PDF 解析器（**不依赖 MuPDF**）
- **MIT/Apache-2.0 双许可**——避开 MuPDF 的 AGPL 限制
- **8 种语言绑定**（Python/Go/JS/TS/C#/.NET/Java/Kotlin/WASM）+ CLI + MCP server
- README 宣称 **0.8 ms mean per document**、**5× faster than PyMuPDF**、**100% pass rate on 3,830 PDFs**

## 架构对比：哲学差异

| 维度 | ritz | pdf_oxide |
|------|------|-----------|
| PDF 引擎 | **MuPDF C**（git submodule，源码编译） | **自研 Rust PDF parser** |
| FFI 模型 | PyO3 + C 错误包装层（longjmp 隔离） | PyO3 直接绑 Rust |
| 文本提取路径 | 走 MuPDF `stext` 树（block→line→char） | 自研 parser，绕过 stext |
| 渲染管线 | MuPDF `fz_run_page` + draw device | 自研光栅化器 |
| 许可证 | **AGPL-3.0**（被 MuPDF 强制） | **MIT/Apache-2.0** |
| 语言绑定 | Python | Python + Go + JS + C# + Java + WASM |
| 历史成熟度 | 0.1.0，2026 单人项目 | 0.3.64，多版本迭代 |
| PDF 测试覆盖 | 继承 MuPDF 数十年实场验证 | 3,830 PDFs（veraPDF + pdf.js + SafeDocs）|

**哲学差异**：
- ritz = "**PyMuPDF reforged in Rust**"——保留 MuPDF 引擎，重写绑定层和数据模型，最大化兼容性和稳健性
- pdf_oxide = "**ground-up pure-Rust PDF parser**"——抛弃 MuPDF，从零写解析器，最大化许可和性能自由度

## 性能实测：pdf_oxide 的"5x faster" 宣称站得住吗？

**结论：站不住。** 5x 仅在小文档打开 + 简单单页文本场景成立（懒加载优势）。在真实多页文档场景，ritz 全面胜出。

### 测试结果总表

| # | 场景 | pdf_oxide | ritz | PyMuPDF | ritz vs pdf_oxide | ritz vs PyMuPDF |
|---|------|----------|------|---------|-------------------|------------------|
| 1 | 文档打开 sample (1 页) | **0.014 ms** | 0.291 ms | 0.203 ms | 0.05x ❌ | 0.7x |
| 2 | 文档打开 arxiv (43 页) | 6.44 ms | **0.33 ms** | 3.22 ms | **19.5x** ✅ | **9.8x** ✅ |
| 3 | 单页文本 sample | 0.072 ms | **0.055 ms** | 0.129 ms | 1.3x ✅ | 2.3x ✅ |
| 4 | arxiv 43 页全文本 | 54.6 ms | **19.1 ms** | 44.8 ms | **2.9x** ✅ | **2.3x** ✅ |
| 5 | 单图提取 (1 JPEG) | **0.008 ms** | 0.020 ms | 0.049 ms | 0.4x ❌ | 2.5x ✅ |
| 6 | 单页渲染 1x sample | 2.162 ms | **0.516 ms** | 0.519 ms | **4.2x** ✅ | 1.0x |
| 7 | 单页渲染 2x sample | 2.268 ms | 1.798 ms | **1.366 ms** | 1.3x ✅ | 0.8x |
| 8 | arxiv 43 页全渲染 1x | 950.9 ms | **101.1 ms** | 107.1 ms | **9.4x** ✅ | **1.1x** ✅ |
| 9 | 8 文档串行文本 | 1034 ms | **419 ms** | 612 ms | **2.5x** ✅ | **1.5x** ✅ |
| 10 | 8 文档并行文本 | — (无原生并行) | **122 ms** | 234 ms (MP) | — | **1.9x** ✅ |
| 11 | 链接读取 sample | — (无原生 API) | **0.001 ms** | 0.006 ms | — | **6x** ✅ |

> 倍率 = 对手时间 / ritz 时间。倍率 > 1 表示 ritz 更快。✅ = ritz 领先，❌ = 对手领先。

### 关键发现

#### 1. pdf_oxide 的"5x" 来自**小文档打开**的懒加载

pdf_oxide 的 `PdfDocument::open()` 几乎不做工作（0.014 ms）——只读 PDF header 和 xref 偏移。ritz 的 `open` 会真正初始化 fz_context + 解析文档结构（0.291 ms）。

但这个优势在**真实文档**上反转：打开 32MB 的 arxiv PDF 时，pdf_oxide **慢 19.5 倍**（6.44 ms vs 0.33 ms）。原因猜测：pdf_oxide 的 Rust parser 在大文档上的 lazy loading 触发了多次重新解析，而 ritz 借助 MuPDF 一次性 mmap。

#### 2. 多页文本提取：ritz **2.9 倍**快于 pdf_oxide

| 工具 | arxiv 43 页全文本 |
|------|------------------|
| **ritz** | **19.1 ms** |
| PyMuPDF | 44.8 ms |
| pdf_oxide | 54.6 ms |

pdf_oxide README 宣称的"5x faster than PyMuPDF"在我们的实测里**不成立**——它在 43 页文档上反而比 PyMuPDF 慢 22%。ritz 借助 MuPDF 的成熟 stext 引擎，是这个场景的最快实现。

**原因分析**：pdf_oxide 的纯 Rust parser 在大文档上没有 MuPDF stext 的优化（缓存、流式解析、特定 C 路径）。"5x faster" 数据集可能是大量**小 PDF**（1-2 页）的均值，而小 PDF 上 ritz/PyMuPDF 都被 Python 调度开销主导，pdf_oxide 的轻量 parser 占优。

#### 3. 渲染：ritz **9.4 倍**快于 pdf_oxide

| 工具 | arxiv 43 页全渲染 1x |
|------|---------------------|
| **ritz** | **101.1 ms** |
| PyMuPDF | 107.1 ms |
| pdf_oxide | 950.9 ms |

pdf_oxide 的渲染管线远未成熟——950 ms vs MuPDF 系的 ~100 ms 是数量级差距。这是 pdf_oxide 的**架构代价**：抛弃 MuPDF 意味着要自己实现完整光栅化（字体引擎、矢量绘制、图像采样、blend 模式）。MuPDF 在这些场景已经优化了 20 年。

#### 4. 并行：ritz 是**唯一原生多文档并行**的库

| 工具 | 8 arxiv 并行 |
|------|-------------|
| **ritz rayon** | **122 ms** |
| PyMuPDF multiprocessing.Pool | 234 ms |
| pdf_oxide | 1034 ms（无原生并行 API） |

pdf_oxide 没有原生的 rayon 等价物。用户要并行只能用 Python `multiprocessing`，序列化开销巨大。

#### 5. ritz 输给 pdf_oxide 的两个场景

- **轻量文档打开**（0.014 ms vs 0.291 ms）：pdf_oxide 极致 lazy，ritz 会初始化完整 fz_context。这是架构选择——ritz 选择"open 后一切就绪"，pdf_oxide 选择"open 后按需解析"。
- **单图提取**（0.008 ms vs 0.020 ms）：pdf_oxide 的 parser 直接从 PDF 对象流读字节，没有 device callback 启动开销。ritz 走 display list 是为了同时拿到 bbox，代价是 ~2x 慢。但在**多图场景**这个开销摊销掉了。

## 功能对比

| 功能 | ritz | pdf_oxide |
|------|------|-----------|
| 文本提取（10 种模式）| ✅ text/html/xhtml/xml/json/rawjson/words/blocks/dict/rawdict | ✅ text/markdown/html（无 dict/rawdict 等结构化） |
| 图片提取（bbox + bytes）| ✅ 一次调用拿 xref + bbox + 原始字节 | ✅ `extract_image_bytes`（无 bbox 整合） |
| 链接提取 | ✅ | ✅ |
| 页面渲染 | ✅（基于 MuPDF） | ⚠️（有但慢 9x） |
| 表单填写/导出 | ❌ | ✅ |
| 注释 | ❌ | ✅ |
| PDF 创建 | ❌ | ✅ |
| PDF 编辑（增删页/合并/拆分）| ❌ | ✅ |
| 表格提取 | ❌ | ✅ |
| Markdown 转换 | ❌ | ✅ |
| 文本搜索 | ❌ | ✅ |
| 大纲/书签 | ❌ | ✅ |
| 多文档并行 | ✅（rayon 原生） | ❌（需用户 multiprocessing） |
| 批量提取 | ✅（C 层流式） | ❌ |
| 数据模型（PdfValue）| ✅（暴露给 Python） | ❌（不暴露内部模型） |

**ritz 的覆盖窄**：聚焦"读取 + 提取 + 渲染"三个核心场景，PDF 编辑/创建/表单都不支持（plan_v1 Phase 5 计划）。

**pdf_oxide 的覆盖广**：是完整的 PDF 工具链（读取 + 编辑 + 创建 + OCR + 转换）。**功能数量远超 ritz**，是当前 Python 生态功能最全的 MIT 许可 PDF 库之一。

## 文本提取质量

pdf_oxide README 宣称 "99.5% text parity vs PyMuPDF and pypdfium2"，但实测 arxiv 上的小差异：
- pdf_oxide: 38 chars
- ritz: 40 chars（PyMuPDF 系）
- PyMuPDF: 37 chars

差异主要在空白处理（trailing newline、段落分隔符）。对文本搜索/RAG 影响不大，但**精确字符级一致性**只有 MuPDF 系（ritz/PyMuPDF）能保证。

### CJK / RTL / bidi

- **ritz**：继承 MuPDF 完整 bidi/RTL/CJK 支持（HarfBuzz 集成、ICU 排序）。20 年实场验证。
- **pdf_oxide**：v0.3.54 (2026) 才加入 Hebrew/RTL 检测，仍在迭代中。

### 测试集广度

- **ritz**（继承 MuPDF）：MuPDF 在生产环境被数十亿级 PDF 处理过（浏览器、移动设备、文档管理系统），覆盖极端边界情况。
- **pdf_oxide**：3,830 PDFs（veraPDF 合规集 + pdf.js 测试集 + SafeDocs 边缘集）。这是**精心挑选的标准测试集**，不能完全代表真实世界的"野生 PDF"分布（损坏文件、扫描件、特殊字体、罕见编码）。100% pass rate 是真实数据，但样本有偏。

## 许可证与商业可用性

| | ritz | pdf_oxide |
|---|------|-----------|
| 许可证 | AGPL-3.0（被 MuPDF 强制） | MIT/Apache-2.0 |
| 商业闭源使用 | 需购买 [Artifex MuPDF commercial license](https://mupdf.com/licensing/index.html) | 自由使用 |
| 修改 MuPDF 源码 | 允许（[plan_v1 §5.5](../plan/01-plan-v1.md) patches/ 工作流） | N/A |

**这是 ritz 最大的劣势**。任何商业闭源产品想用 ritz 必须买 MuPDF 商业许可。pdf_oxide 没有这个问题。

## 谁该用哪个？

| 你的场景 | 推荐 |
|---------|------|
| 高性能文本/图片提取，要 bbox | **ritz** |
| 多文档并行批处理 | **ritz** |
| 页面渲染（缩略图、PDF→PNG） | **ritz** 或 PyMuPDF |
| 商业闭源项目 | **pdf_oxide**（MIT） |
| PDF 编辑/创建/表单 | **pdf_oxide** |
| 多语言绑定（Go/JS/C#） | **pdf_oxide** |
| Markdown 转换 / RAG | **pdf_oxide**（内置） |
| CLI 工具 | **pdf_oxide**（22 commands） |
| 极致轻量打开 + 小 PDF 解析 | **pdf_oxide** |
| CJK / RTL / bidi 文本保真 | **ritz**（继承 MuPDF） |
| 处理损坏/野生 PDF | **ritz**（继承 MuPDF 实场验证） |

## 总结

**pdf_oxide 的 5x 宣传是营销话术**：仅在小文档打开场景成立，在真实多页文档上 ritz 反而快 2.9x，在渲染上 ritz 快 9.4x。pdf_oxide 的真正价值不在性能，而在：
1. MIT 许可（商业可用）
2. 多语言绑定
3. 完整的 PDF 工具链（编辑/创建/表单/markdown）

**ritz 的真正价值**：
1. MuPDF 引擎的稳健性（数十亿 PDF 实场验证）
2. 渲染和并行场景的硬实力（ritz 是唯一原生 rayon 并行的）
3. PDF 编辑前的"读取/提取/渲染"全场景高性能

两者**不是直接竞品**——ritz 是 PyMuPDF 的 Rust 重写，pdf_oxide 是 pdfplumber/pypdf 的 Rust 重写。它们瞄准的市场不同：
- **PyMuPDF 用户**（追求稳健 + 性能）→ **ritz**
- **pdfplumber/pypdf 用户**（追求简单 + MIT）→ **pdf_oxide**

## 附录：复现方法

```bash
cd /path/to/ritz
pip install pdf_oxide  # 0.3.64
source .venv/bin/activate
python -c "
import time, ritz, fitz
from pdf_oxide import PdfDocument
# 见 benchmarks/ 三方对比脚本
"
```

测试脚本：[benchmarks/bench_vs_pdf_oxide.py](../benchmarks/bench_vs_pdf_oxide.py)
