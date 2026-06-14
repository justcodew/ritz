# Changelog

本项目所有显著改动都记录在此文件中。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

**版本号位置**：版本号是**单一来源**——只在 `Cargo.toml`（workspace 根）的 `[workspace.package]` 里。
- 3 个子 crate 用 `version.workspace = true` 继承
- `pyproject.toml` 用 `dynamic = ["version"]`，maturin 从 Cargo 读取
- 改版本只需改一处：`Cargo.toml` 第 10 行附近的 `version = "0.1.0"`

**条目分类**：
- `Added` — 新功能 / 新 API
- `Changed` — 不兼容的 API 变化或重要内部重构
- `Deprecated` — 即将移除的 API
- `Removed` — 已移除的 API
- `Fixed` — Bug 修复
- `Performance` — 性能优化（带实测数据）
- `Security` — 安全相关
- `Documentation` — 仅文档改动

**深入背景**：每条重要改动会引用 `docx/` 里的对应文档。CHANGELOG 是"做了什么"，docx/ 是"为什么这么做"。

---

## [Unreleased]

下一个版本计划（详见 [docx/00-onboarding-guide.md §7.2](docx/00-onboarding-guide.md)）：

- `Fixed`: 修 `get_pixmap` matrix 折叠 bug（静默丢弃 rotation/translate）
- `Added`: `doc.resolve_names()` —— 命名目标解析
- `Added`: Phase 5b —— `page.get_annots()` + `add_highlight/underline/strikeout/text` 注释读写
- `Added`: Phase 5c —— `doc.set_toc()` 大纲写入

发版时机：上述至少一项落地后发 `0.3.0`（包含新 API，minor bump）。

---

## [0.2.0] - 2026-06-14

Phase 5a：读路径扩展（文本搜索 + 大纲读取）。底层 C 包装 + Rust 暴露 + PyMuPDF 兼容。
49 个 pytest 用例全过（含 12 个新增 + 与 PyMuPDF 对照）。

### Added — Phase 5a 读路径 API

- **`page.search_for(needle, quads=False)`** —— PyMuPDF 兼容文本搜索。
  - 大小写不敏感，复用 stext 基础设施
  - 默认返回命中位置的包围矩形 `list[(x0,y0,x1,y1)]`
  - `quads=True` 返回 4 顶点 `list[(ul_x,ul_y,ur_x,ur_y,lr_x,lr_y,ll_x,ll_y)]`
  - 一次命中跨多行会产生多个 quad/rect，与 PyMuPDF 行为一致
  - C 层用 `fz_search_stext_page_cb` 回调 API（避免两趟 count+fill）
- **`doc.get_toc()`** —— PyMuPDF `get_toc(simple=True)` 兼容大纲读取。
  - 返回 `list[[level, title, page]]`：level 1-based、page 1-based（外链=-1）
  - 文档无大纲时返回 `None`（与 PyMuPDF 一致）
  - C 层递归前序遍历 `fz_load_outline` 树 + growbuf 扁平化
  - 一次 FFI 拿全部 entry，避免 N 次往返

### Changed

- **版本号整合为单一来源**：原散落在 4 个文件（pyproject.toml + 3 个 crate Cargo.toml）的 `version = "0.1.0"`
  现在统一到 `Cargo.toml` 的 `[workspace.package]`。子 crate 用 `version.workspace = true` 继承，
  pyproject.toml 用 `dynamic = ["version"]` 让 maturin 从 Cargo 读取。**发版改一处即可**。
  详见下方"维护约定"小节

### Documentation

- **新增 `plan/` 目录**：把项目最原始的三篇构想文档收进仓库——
  `plan/01-plan-v1.md`（愿景与 4 项 KPI）、`plan/02-plan-v2.md`（工程方案）、
  `plan/03-rennie-data-model.md`（PdfValue enum 灵感来源）。配 `plan/README.md` 索引。
  新维护者从此可追溯"为什么做、做什么、灵感从哪来"，
  与 `docx/`（技术决策）和 `CHANGELOG`（版本账本）构成三层文档体系
- **`plan_v1 §X.Y` 引用全部转 markdown 链接**：23 处纯文本引用统一改成可点击链接，
  分布在 README/CHANGELOG/patches/docx/benchmarks/plan 等 12 个文件

---

## [0.1.0] - 2026-06-14

首个完整版本。包含 plan_v1 全部目标 + Tier 1/2 实现层优化。

### Added — 核心 API（plan_v1 阶段 1–4）

- **Document API**：`ritz.open(path)`、`len(doc)`、`doc.page_count`、`doc[i]` / `doc.load_page(i)`、
  `doc.is_encrypted`、`doc.needs_password()`、`doc.authenticate(pw)`
- **Page API**：`page.rect`、`page.mediabox`、`page.cropbox`、`page.rotation`、`page.width`、`page.height`
- **10 种文本提取模式**（与 PyMuPDF 对齐）：`text` / `html` / `xhtml` / `xml` / `json` / `rawjson` /
  `words` / `blocks` / `dict` / `rawdict`
- **一级图片提取**：`page.get_images(include_data=False)` 一次调用返回
  `{"xref", "bbox", "width", "height", "bpc", "colorspace", "imagemask"}`；`include_data=True` 时额外含
  `ext`（`"jpeg"`/`"png"`/`"jpx"`）和 `image`（原始压缩字节，非重编码 PNG）
- **链接读取**：`page.get_links()` 返回 PyMuPDF 兼容格式 `{kind, from, to, uri?, is_external}`
- **页面渲染**：`page.get_pixmap(matrix=None, dpi=None, alpha=False)`；`page.png(zoom, alpha)` 直接返回 PNG bytes
- **Pixmap API**：`pix.width`、`pix.height`、`pix.stride`、`pix.components`、`pix.samples`、
  `pix.tobytes("png")`、`pix.save_png(path)`
- **元数据**：`doc.metadata` @property 返回完整 dict（`format/title/author/.../encryption`）；
  `doc.lookup_metadata(key)` 单字段查询

### Added — 批量与并行扩展（[plan_v1 §3.4](plan/01-plan-v1.md)）

- **`doc.get_text_batch()`** —— 1 次 FFI 调用拿全部页文本（vs per-page 循环 N 次 FFI）。
  内部走 `mupdf_ext_extract_pages_text` 流式 growbuf。详见 [docx/06-parallel-and-batch.md](docx/06-parallel-and-batch.md)
- **`ritz.process_documents(paths, mode="text")`** —— rayon 多文档并行。
  每文档独立 fz_context（无锁函数），`py.detach` 释放 GIL 让 worker 真正并行。
  `mode` 可选 `text`（快速批处理路径）/ `html` / `xhtml` / `xml` / `json`。
  失败文档返回 `None`，成功文档返回 `list[str]`

### Added — 数据模型（[plan_v1 §5.2](plan/01-plan-v1.md) Rennie）

- **`PdfValue`** —— 10 种 PDF 对象类型的 Rust `enum`（Null/Bool/Int/Float/Name/Str/Array/Dict/Stream/Ref）。
  通过 `from ritz import PdfValue` 暴露给 Python，全套构造器 + `to_python()` / `get()` 转换。
  详见 [docx/01-architecture.md §"关键技术决策"](docx/01-architecture.md)

### Added — 构建基础设施（[plan_v1 §5.5](plan/01-plan-v1.md)）

- **MuPDF 1.27.0 git submodule**（`vendor/mupdf/`）—— 源码编译，不依赖系统 MuPDF
- **patches/ 工作流** —— `build.rs` 在编译前自动按字母顺序应用 `patches/*.patch` 到子模块，幂等跳过已应用的。
  详见 [patches/README.md](patches/README.md)
- **bindgen 自动生成** —— `crates/mupdf-sys/wrapper.h` 是 bindgen 主输入，
  `crates/mupdf-sys/bindings.rs` 是 commit 进 git 的生成产物（为了让没装 bindgen 的用户也能编译）

### Added — 测试与基准

- **37 个 pytest 用例** —— 覆盖所有公开 API（`python/tests/test_ritz.py`）
- **Rust 集成测试** —— `cargo test -p mupdf --test integration_test`
- **基准套件**：
  - `benchmarks/bench_vs_pymupdf.py` —— 13 场景对比 PyMuPDF
  - `benchmarks/bench_vs_pdf_oxide.py` —— 12 场景对比 pdf_oxide + PyMuPDF
  - `benchmarks/bench_tier12_diff.py` —— Tier 1/2 优化前后对比
  - `benchmarks/leak_test.py` —— 1000 页 PDF 内存 + 1000 次循环泄漏
  - `benchmarks/make_big_pdf.py` —— 一次性 fixture 生成器

### Added — 技术文档（docx/）

11 篇技术决策文档，详见 [docx/README.md](docx/README.md)。亮点：
- [docx/02-longjmp-isolation.md](docx/02-longjmp-isolation.md) —— 不可破坏的不变量 #1
- [docx/03-c-layer-flattening.md](docx/03-c-layer-flattening.md) —— 链接读取 27x 的根因
- [docx/00-onboarding-guide.md](docx/00-onboarding-guide.md) —— 新维护者必读

### Performance — plan_v1 KPI 兑现

vs PyMuPDF（详见 [benchmarks/benchmark_report.md](benchmarks/benchmark_report.md)）：

| 场景 | 加速比 | 上界原因 |
|------|-------|---------|
| 链接读取 | **27x** | C 层扁平化链表，1 次 FFI |
| JPEG 图片提取 | **6.2x** | 返回原始 JPEG 字节，跳过 decode+re-encode |
| 文档打开 | **5.6x** | PyO3 零成本抽象 |
| 页面加载 | **1.9x** | 同上 |
| 批量文本提取 | **2.4x** | 1 次 FFI vs N 次 |
| 多文档并行 | **2.1x** | rayon vs multiprocessing |
| 文本提取 | **2.1x** | stext 构造是双方共享地板，**10x 不可达**（详见 [docx/07-performance-analysis.md](docx/07-performance-analysis.md)） |
| FlateDecode 图片 | **1.1x** | PNG 编码是双方共享瓶颈 |

### Performance — Tier 1 优化（实现层低垂果实）

详见 [docx/10-tier1-tier2-results.md](docx/10-tier1-tier2-results.md)。

- **`get_pixmap` 释放 GIL** —— 多文档并行渲染 **15.4x**（GIL 持有时不可能）。
  风险：同 doc 跨线程并发是 UB（MuPDF ctx 无锁），已文档化
- **`get_text_batch` 释放 GIL** —— 让 Python 主线程在批量提取时能跑其他线程
- **memoize `metadata`** —— `OnceLock<Py<PyAny>>` 缓存，9 次 FFI → 0，sub-µs 命中
- **删 `tobytes(png)` 的 `slice.to_vec()`** —— PNG 路径 **1.37x**（省一次完整图字节 memcpy）
- **缓存 `raw_doc` 指针到 PyPage** —— rotation/get_images 省一次 `bind().borrow()`
- **memoize `page.rect`** —— `OnceCell<(f32,f32,f32,f32)>`，rect/width/height 三次 FFI → 一次

### Performance — Tier 2 优化

- **#7 words/blocks 单趟 growbuf** —— 把 C 层 sizing+writing 两趟树遍历改成 growbuf 单趟。
  用 offset（而非 pointer）记录 header 位置，realloc 不失效。实测 arxiv 单页 words **-2.4%**、blocks **-1.6%**。
  audit 预测 30–40%，实际只 2–4%——因为 stext 构造占 90%+，端到端收益被稀释（详见 [docx/10 §"为什么实际收益小"](docx/10-tier1-tier2-results.md)）
- **#8 `cbuf_to_pystring` fast path** —— 替换 `from_utf8_lossy + into_owned + into_pyobject` 三次拷贝
  为 `std::str::from_utf8` + 单次 `PyString::new`。实测不可测（<0.5%，被 stext 构造淹没），
  保留是为代码清晰度 + 未来 stext 路径若被优化后发挥作用

### Security — 安全验证（[plan_v1 §4.2](plan/01-plan-v1.md)）

- **1000 页 PDF 常驻内存 ≤200MB** —— 实测 72.9MB ✅
- **1000 次 open/close + page 迭代循环泄漏 <5KB/iter** —— 实测达标 ✅
- **longjmp 隔离** —— 所有 MuPDF 调用包在 C 层 `fz_try/fz_catch`，longjmp 永不穿透 Rust 栈
- **C 层扁平化** —— stext/image/links 链表/树遍历都在 C 完成，输出连续 buffer 给 Rust，避免 N 次 FFI 往返

### Changed — 内部架构

- **PyO3 0.29 API** —— `Bound<T>` 引用、`py.detach`（原 `allow_threads`）、`cast`（原 `downcast`）
- **`Py<PyDocument>` 引用计数** —— Page 和 Pixmap 持有 doc handle，防止 ctx 在 GC 时被先释放
- **`Arc<[u8]>` / `Arc<str>`** —— PdfValue 的 stream/str 字段用 Arc 满足 Send+Sync 要求

### Known Issues — 已知问题（详见 [docx/00-onboarding-guide.md §6](docx/00-onboarding-guide.md)）

- ⚠️ **`Send + Sync` 是 unsound** —— MuPDF 默认 ctx 无锁，同 doc 跨线程并发会 UB。
  目前安全仅因为除 `process_documents` 外没有任何 per-doc 方法释放 GIL。
  未来若给 `get_text` 等加 `py.detach` 必须同步给 `PyDocument` 加 `Mutex`
- ⚠️ **`get_pixmap` matrix 折叠 bug** —— `page.rs:316` 把 `(a,b,c,d,e,f)` 折叠成 `max(a,d)`，
  静默丢弃 rotation/translate/skew。**0.2.0 修**
- **patches 应用失败只发 warning** —— `build.rs` 不中断构建，CI 不会卡，但本地排查要看 log
- **bindings.rs 是 commit 进 git 的生成产物** —— C 签名改后必须手动 `cargo build -p mupdf-sys --features bindgen` 重新生成

### Removed — 显式不实现的方向

- **MIT/Apache 重许可** —— 不做。MuPDF 是 AGPL，ritz 同步。商业闭源需购买 [Artifex MuPDF commercial license](https://mupdf.com/licensing/index.html)
- **自定义 `fz_device` 破 text 10x** —— 不做。会破坏 CJK/RTL/bidi 顺序，高风险。详见 [docx/09 §Tier 3 #12](docx/09-acceleration-opportunities.md)
- **同 doc 跨线程并发** —— 不做（除非加 Mutex，复杂度不值）

### Documentation — 用户文档

- **README.md** —— 用户文档，API 速查 + 安装 + 快速开始
- **AGPL-3.0 LICENSE**

---

## 维护约定

### 发版流程（0.2.0 开始执行）

1. **改版本号**：改 `Cargo.toml`（workspace 根）`[workspace.package]` 的 `version` 一处即可，其余 4 处通过继承/dynamic 自动跟随
2. **整理 CHANGELOG**：把 `[Unreleased]` 的内容剪到 `[X.Y.Z] - YYYY-MM-DD`
3. **跑全套验证**：
   ```bash
   source ~/.cargo/env && source .venv/bin/activate
   maturin develop --release
   pytest python/tests/ -q
   python benchmarks/bench_vs_pymupdf.py
   python benchmarks/leak_test.py
   ```
4. **打 git tag**：`git tag v0.X.Y && git push origin v0.X.Y`
5. **构建 wheel**：`maturin build --release`（产物在 `target/wheel/`）
6. **（可选）发 PyPI**：`maturin publish`

### 版本号约定

- **0.X.0** (minor) —— 新 API（新方法、新 mode、新 class）
- **0.0.X** (patch) —— Bug 修复、性能优化、文档改动（无 API 变化）
- **1.0.0** —— plan_v1 Phase 5（编辑/搜索/大纲）全部落地 + 6 个月生产验证后

### 写 CHANGELOG 的规则

- **每个 PR / commit 至少加一行** 到 `[Unreleased]` 对应分类下
- **链接到 docx/** —— 如果改动涉及设计决策，引用对应 docx 文件
- **写"做了什么"，不写"为什么"** —— "为什么"在 docx/，commit message，PR description 里
- **不写内部重构** —— 除非重构改变了公开 API 或显著性能。代码风格调整、rename、private 函数重构**不写**
- **性能改动带数据** —— "加 X 倍" 而非 "更快"

### 格式样板

```markdown
## [Unreleased]

### Added
- `page.new_method(arg)` —— 一句话说明。详见 [docx/XX-...md](docx/XX-...md)

### Fixed
- `page.get_pixmap` 现在正确处理 matrix 的 rotation/translate 部分。
  此前 `(a,b,c,d,e,f)` 被折叠为 `max(a,d)`，丢失 5 个参数

### Performance
- `page.get_text("text")` 在 arxiv 单页上 -2.4%（mean, 50 iter）。
  改成 growbuf 单趟。详见 [docx/10-...](docx/10-...)
```
