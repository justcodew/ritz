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

### Added — 扩展 PyPI 平台覆盖

补全 macOS Intel、Windows x86_64、Linux arm64 manylinux 兼容，实现全平台覆盖：

- **macOS x86_64**：从 macos-14 (Apple Silicon) 交叉编译，修复 build.rs 仅设 ARCHFLAGS 不生效的问题，
  改为设 `CC="clang -arch x86_64"` + `LD="clang -arch x86_64"`。
- **Windows x86_64**：build.rs 加 Windows 分支，CI 中用 MSBuild 编译 MuPDF（`platform/win32/mupdf.sln`），
  build.rs 读取预编译的 `.lib`。link_mupdf 适配 `libthirdparty.lib` 命名。
- **Linux arm64 manylinux**：改用 `manylinux: '2_28'` + `before-script-linux` 在容器内安装 git/make，
  产出 `manylinux_2_28_aarch64` wheel（glibc 2.28+），替代之前的 `linux_aarch64`（需 glibc 2.39+）。

### 覆盖矩阵（更新后）

| 平台 | Python | wheel |
|------|--------|-------|
| macOS arm64 | 3.9-3.13 | `cp39-abi3-macosx_11_0_arm64.whl` |
| macOS x86_64 | 3.9-3.13 | `cp39-abi3-macosx_10_12_x86_64.whl` |
| Linux x86_64 | 3.9-3.13 | `cp39-abi3-manylinux_2_17_x86_64.whl` |
| Linux aarch64 | 3.9-3.13 | `cp39-abi3-manylinux_2_28_aarch64.whl` |
| Windows x86_64 | 3.9-3.13 | `cp39-abi3-win_amd64.whl` |

---

## [0.4.4] - 2026-06-15

### Fixed — Linux arm64 + macOS Intel 卡 publish 的两个问题

v0.4.3 在 ubuntu-24.04-arm（native ARM runner）上仍失败：maturin-action 默认
通过 Docker manylinux 容器构建 wheel，但该容器在 arm64 runner 上 `docker failed
with exit code 1`。同时 macos-13 job 永远 queued，`publish: needs: build` 被卡死
（`continue-on-error: true` + `if: always()` 不能绕过 matrix 里 *未完成* 的 job）。

### 改动

- **Linux arm64 用 `manylinux: off` 走 native build**：绕开 Docker manylinux 容器
  在 arm64 runner 上的失败。代价：wheel tag 从 `manylinux_*_aarch64` 退化为
  `linux_aarch64`，要求用户 glibc >= 2.39（Ubuntu 24.04+）。现代 ARM Linux 系统
  基本都满足。后续可调研修复 Docker 路径，恢复 manylinux_2_28 兼容。
- **matrix 再次移除 macos-13**：Intel macOS runner 持续短缺，queued 状态会卡死
  整个 publish。Intel Mac 用户暂不可用（占比极低，Intel Mac 已停产）。

### 覆盖矩阵（v0.4.4 发布后）

| 平台 | Python | wheel |
|------|--------|-------|
| macOS arm64 | 3.9-3.13 | `cp39-abi3-macosx_11_0_arm64.whl` |
| Linux x86_64 | 3.9-3.13 | `cp39-abi3-manylinux_2_17_x86_64.whl` |
| Linux arm64 | 3.9-3.13 | `cp39-abi3-linux_aarch64.whl`（manylinux=off，需 glibc 2.39+） |
| macOS x86_64 | — | 暂不支持（macos-13 runner 短缺） |

---

## [0.4.3] - 2026-06-15

### Fixed — Linux arm64 wheel 构建失败

v0.4.2 的 Linux arm64 wheel 构建失败：maturin-action 在 ubuntu-latest 上对
`aarch64-unknown-linux-gnu` 做 cross-compile 时，会用 Docker manylinux 容器 + qemu
模拟 ARM，MuPDF C 库在 qemu 下编译失败（`/usr/bin/docker failed with exit code 1`）。
v0.4.2 tag 推送后，Linux arm64 / macOS Intel 两个 job 失败/卡住，PyPI 未发布。

### 改动

- **Linux arm64 改用 native ARM runner**：matrix 项从
  `{ os: ubuntu-latest, cross_deps: 'gcc-aarch64-linux-gnu' }`
  改为 `{ os: ubuntu-24.04-arm, cross_deps: '' }`。2025 年起 GitHub 为公开仓库
  免费提供 ARM runner，host == target 不再需要 cross 工具链，也不再走 Docker/qemu。
- 移除 Build wheel step 的 `CC_aarch64_unknown_linux_gnu` / `LD_*` / `AR_*` env
  （native build 不需要）。"Install cross-compile toolchain" step 保留但
  `cross_deps == ''` 时跳过。
- `crates/mupdf-sys/build.rs` 的 cross 分支保留（host != target 时仍生效），
  native ARM build 时 host == target 自动跳过，等价无影响。

### 覆盖矩阵（v0.4.3 发布后，目标）

| 平台 | Python | wheel |
|------|--------|-------|
| macOS arm64 | 3.9-3.13 | `cp39-abi3-macosx_11_0_arm64.whl` |
| macOS x86_64 | 3.9-3.13 | `cp39-abi3-macosx_10_12_x86_64.whl`（依赖 macos-13 可用） |
| Linux x86_64 | 3.9-3.13 | `cp39-abi3-manylinux_2_17_x86_64.whl` |
| Linux arm64 | 3.9-3.13 | `cp39-abi3-manylinux_2_17_aarch64.whl`（native ARM runner） |

---

## [0.4.2] - 2026-06-15

### Build — 扩展 PyPI wheel 覆盖

v0.4.1 只发布了 macOS arm64 (cp312) + Linux x86_64 (cp39) 两个 wheel，
Python 版本各只有一个，平台覆盖窄。本次大幅扩展：

- **abi3 wheel**（`pyo3/abi3-py39` feature）：单个 wheel 覆盖 Python 3.9-3.13，
  不再每个 Python 版本单独构建。wheel 命名从 `cp312-cp312` 变为 `cp39-abi3`。
- **新增 Intel Mac (x86_64-apple-darwin)**：macos-13 native runner。
- **新增 Linux arm64 (aarch64-unknown-linux-gnu)**：ubuntu-latest cross-compile，
  装 `gcc-aarch64-linux-gnu`，build.rs 把 `CC_aarch64_unknown_linux_gnu` 传给 MuPDF Makefile + cc crate。
- **publish 不阻塞**：build matrix 加 `continue-on-error: true`，publish job 用 `if: always()`。
  macos-13 runner 短缺超时不再阻塞其他平台 wheel 发布。
- `requires-python` 从 `>=3.8` 改为 `>=3.9`（与 abi3-py39 对齐）。
- classifiers 加 Python 3.13，移除 3.8。

### 覆盖矩阵（v0.4.2 发布后）

| 平台 | Python | wheel |
|------|--------|-------|
| macOS arm64 | 3.9-3.13 | `cp39-abi3-macosx_11_0_arm64.whl` |
| macOS x86_64 | 3.9-3.13 | `cp39-abi3-macosx_10_12_x86_64.whl` |
| Linux x86_64 | 3.9-3.13 | `cp39-abi3-manylinux_2_17_x86_64.whl` |
| Linux arm64 | 3.9-3.13 | `cp39-abi3-manylinux_2_17_aarch64.whl` |
| Windows | — | 不支持（MuPDF 走 MSVC，未来单独加） |

### Changed

- `crates/ritz/Cargo.toml`：pyo3 features 加 `abi3-py39`
- `crates/mupdf-sys/build.rs`：`build_mupdf` 检测 host != target 时传交叉 CC/LD/AR；
  `compile_c_wrapper` 给 cc crate 显式传 `target`（native build 等价）
- `.github/workflows/release.yml`：matrix 从 2 项扩到 4 项；加 cross-compile toolchain step；
  build job 加 `continue-on-error`；publish 改 `if: always()` + 加 wheel 存在性检查
- `pyproject.toml`：`requires-python` → `>=3.9`；classifiers 加 3.13、删 3.8

### 风险与回退

- Linux arm64 cross 首次启用，MuPDF Makefile 对 cross CC 的支持可能不完美。
  若失败，build job `continue-on-error` 让其他平台照常发布；后续单独修。
- abi3 模式首次启用，PyO3 有限 API 可能有意外。若本地编译失败，临时退回非 abi3。

---

## [0.4.1] - 2026-06-15

### Fixed

- **release.yml 移除 macos-13 (Intel runner)**：GitHub Actions 的 Intel macOS runner
  经常短缺，导致 `build wheel (x86_64-apple-darwin)` job 永远卡在 queued，
  `publish` job (`needs: build`) 因此从未触发——这是 v0.3.0 / v0.3.1 / v0.4.0
  从未真正发布到 PyPI 的根因。本次仅保留 Linux x86_64 + macOS arm64 两个目标，
  Intel macOS 用户暂时不可用（Intel Mac 已停产，占比极低）。后续如需 x86_64 支持，
  考虑 universal2 wheel 或 macos-14 cross-compile。

### Note

本版本是 v0.4.0 的"再发布"——v0.4.0 tag 已 push 但 publish 被 runner 短缺卡死，
PyPI 上从不存在 0.4.0。版本号 bump 到 0.4.1 以避免与可能的部分上传冲突。
代码层面 v0.4.1 = v0.4.0（无功能改动）。

---

## [0.4.0] - 2026-06-15

Phase 6：文本提取热路径性能优化。针对 corpus benchmark 显示的 ritz vs PyMuPDF
~50% 单文档差距，定位 3 个开销点并修复。

### Performance — 文本提取热路径

- **PyPage `fz_stext_page` 缓存**：`stext_cache: Mutex<Option<*mut fz_stext_page>>`。
  同一 page 多次 get_text / search_for 共享同一 stext，不重建。
  annot 写操作（add_highlight/underline/strikeout/text_annot + delete_annot）调用
  invalidate_stext，确保 stext 包含 annot 文本这一不变量。
- **借用版 stext 输出**：`mupdf_safe_stext_to_buffer_borrow` 用 `fz_buffer_storage`
  借用 buf 内部 storage，省下 `fz_buffer_extract` 的 malloc + memcpy。
- **release build 跳过 UTF-8 校验**：`slice_to_pystring` 用 `from_utf8_unchecked`
  （MuPDF stext 输出保证 UTF-8）。debug build 保留校验 + lossy fallback。

### 实测收益

| 场景 | 优化前 | 优化后 | 加速 |
|------|--------|--------|------|
| 多模式同页（text+words+blocks+dict） | 0.675 ms/轮 | **0.009 ms/轮** | **71.9x** |
| 单模式 text 同页重复 | 0.130 ms/轮 | **0.001 ms/轮** | **169x** |
| corpus 单文档（cache 不触发） | 1.0ms | ~1.0ms（无显著变化，瓶颈在 MuPDF 内部）| — |

详见 [docx/16-phase6-stext-perf.md](docx/16-phase6-stext-perf.md) 和
[benchmarks/bench_multimode.py](benchmarks/bench_multimode.py)。

### Added

- `benchmarks/bench_corpus.py` 新增 `ritz_batch` 模式（用 `get_text_batch` 1 次 FFI 拿全部页文本）
- `benchmarks/bench_multimode.py` 新建：多模式同页对比脚本

### Changed

- `PyPage` 新增 `stext_cache: Mutex<Option<*mut fz_stext_page>>` 字段
- `crates/mupdf-sys/native/error_wrapper.c` 新增 2 个 C 函数（borrow + release_buffer）
- `crates/ritz/src/page.rs::stext_to_string` 切换到 borrow 路径
- 5 个 annot 写方法开头加 `invalidate_stext()`
- `Drop for PyPage` 释放 stext_cache

### Documentation

- 新增 [docx/16-phase6-stext-perf.md](docx/16-phase6-stext-perf.md) — 完整 8 节设计文档
- 新增 [docx/17-optimization-dead-ends.md](docx/17-optimization-dead-ends.md) —
  LTO + 轨道 A 自定义 device 两个失败尝试的复盘，记录假设/PoC/数据/根因
- README.md corpus 对比表说明更新（指明 cache 是真实场景加速点）

### Build

- `Cargo.toml` 新增 `[profile.release]` 段：`lto=true` + `codegen-units=1`。
  实测对 ritz 当前场景无可测收益（瓶颈在 MuPDF C 内部，Rust 编译器优化碰不到），
  保留作为未来 Rust 层逻辑变重时的保险。详见
  [docx/17-optimization-dead-ends.md](docx/17-optimization-dead-ends.md) 死路 1。

### 优化死路记录

本次 v0.4.0 周期尝试了两条额外优化路径，**两条都失败**，已全部回退或保留为无痕配置：

1. **LTO/codegen-units**：Rust 编译器层面优化，对 C-bound 场景无效
   （corpus mean 1.0ms → 1.03ms，噪声内）。配置保留（无害）。
2. **轨道 A 自定义 fz_device**：假设 stext 构造占主导成本（plan_v1 估计 200-500μs），
   实测 stext 构造只占 ~9μs，且 stext device 有 `lasttext` 去重 + pool allocator 高度优化，
   minimal device 在大 PDF 上反而更慢（0.58x）。PoC 代码已删除。

两条死路共同证明：MuPDF 1.27 stext 引擎是双方共享地板，ritz 单文档单次提取场景
已接近优化天花板。详见 [docx/17-optimization-dead-ends.md](docx/17-optimization-dead-ends.md)。

---

## [0.3.1] - 2026-06-15

### Fixed

- **Linux build 修复**：`crates/mupdf-sys/build.rs` 在 `target_os = "linux"` 时给 MuPDF `make` 传 `XCFLAGS=-fPIC`。
  - 根因：MuPDF 静态库被链接到 Python cdylib（共享库 `_ritz.so`），Linux x86_64 要求所有 `.o` 是 PIC，否则报 `relocation R_X86_64_32 cannot be used against local symbol; recompile with -fPIC`。
  - macOS 默认 PIC（Darwin ABI），Windows 用 COFF，都不受影响，所以 Phase 5e 在 macOS 上 75 个 pytest 全绿但 Linux CI build 失败。
  - 修复后首次成功发布 Linux x86_64 manylinux wheel 到 PyPI。

### Changed

- 包发布名从 `ritz` 改为 `ritz-tool`（PyPI 上 `ritz` 已被占用）。import 名不变（仍是 `import ritz`）。

---

## [0.3.0] - 2026-06-14

Phase 5e：PDF 页面编辑（plan_v1 最后一个 KPI）。落地后 plan_v1 全部 KPI 达成。
75 个 pytest 用例全过（含 13 个新增页面编辑用例）。

### Added — Phase 5e 页面编辑 API

- **`doc.new_page(pno=-1, width=595, height=842, rotate=0) -> Page`** —— PyMuPDF 兼容。
  插入空白页，返回新建 Page 对象。C 层 `pdf_add_page` + `pdf_insert_page`，正确处理 refcount
  （详见 [docx/15-phase5e-page-editing.md](docx/15-phase5e-page-editing.md)）。
- **`doc.delete_page(pno=-1)`** —— 删除单页，-1 表示末页。
- **`doc.delete_pages(from_page=-1, to_page=-1)`** —— 删除闭区间 [from, to]，-1 表示边界。
  内部转 MuPDF 半开区间 `[start, end+1)`。
- **`doc.move_page(src, dst)`** —— 移动页位置。用 `pdf_keep_obj` 保命 + `pdf_drop_obj` 释放。
- **`doc.copy_page(src, dst)`** —— 同文档克隆，用 `pdf_graft_page`。
- **`doc.insert_pdf(docsrc, start=0, end=-1, at=-1)`** —— 把 docsrc 的 [start, end) 页插入到当前文档。
  **关键**：内部用 docsrc 的 filename 在当前 doc 的 fz_context 里重开 src，
  避免 MuPDF 跨 ctx `pdf_graft_page` 的 UB。为此 `PyDocument` 新增 `filename: String` 字段。

### Changed

- `PyDocument` 新增 `pub(crate) filename: String` 字段，`new()` 时记录源路径，供 `insert_pdf` 复用。

### Documentation

- 新增 [docx/15-phase5e-page-editing.md](docx/15-phase5e-page-editing.md) —— 6 个 API 实现 + refcount 陷阱 + 跨 ctx insert_pdf 方案 + 4 个踩坑记录

### 踩坑记录

- `fz_try/fz_always/fz_catch` 必须同级（嵌套写法导致第二次 new_page SIGSEGV）
- `move_page` 的 dst 不需要调整（pdf_insert_page 的 at 是相对新页树）
- `pdf_graft_page` 要求 src/dst 共享 ctx（ritz 每个 doc 独立 ctx，必须重开 src）

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
