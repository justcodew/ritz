# ritz 接手指南（给新维护者）

> 你刚刚接手了这个项目。本文帮你在 2 小时内从"完全不懂"到"能改第一个 PR"。
>
> 阅读顺序：**先读完本文** → 按 §1 推荐顺序读其他 docx/ → 跑通 §2 构建 → 选一个 §5 的实战任务。

## 0. 30 秒了解 ritz

**ritz = PyMuPDF（fitz）的 Rust 重写**。

- **做什么**：高性能 PDF 处理（提取文本/图片/链接、渲染、批量并行）
- **怎么做**：保留 MuPDF C 引擎，重写 Python 绑定层和数据模型
- **为什么**：PyMuPDF 受 AGPL 限制 + 想要 Rust 的内存安全 + 想要更激进的性能
- **不做什么**：PDF 编辑/创建/表单（plan_v1 Phase 5 才考虑）
- **现状**：0.1.0，单人项目，2026 开发，**继承 MuPDF 实场稳健性 + 在多个场景 2–10x 快于 PyMuPDF**

一句话总结：**MuPDF 是引擎，Rust 是骨架，Python 是脸**。

> **要看每个版本做了什么改动**：读根目录 [CHANGELOG.md](../CHANGELOG.md)。本文档教你怎么"接手"，CHANGELOG 教你"项目到哪了"。

## 1. 理解项目（必读 docx/ 顺序）

按这个顺序读，2 小时入门：

| 顺序 | 文档 | 学到什么 |
|------|------|---------|
| 1 | [01-architecture.md](01-architecture.md) | 四层架构（mupdf-sys/mupdf/ritz/python）为什么这么分 |
| 2 | [02-longjmp-isolation.md](02-longjmp-isolation.md) | **不可破坏的不变量 #1**：MuPDF setjmp/longjmp 不能穿透 Rust 栈 |
| 3 | [03-c-layer-flattening.md](03-c-layer-flattening.md) | 为什么链接读取能 27x：树/链表遍历放 C 层 |
| 4 | [04-memory-safety.md](04-memory-safety.md) | `*mut fz_context` 裸指针如何防悬挂，Drop 顺序 |
| 5 | [07-performance-analysis.md](07-performance-analysis.md) | 哪些场景能 10x、哪些不能（stext 是双方共享地板） |
| 6 | [10-tier1-tier2-results.md](10-tier1-tier2-results.md) | 当前实际性能 + 已做的优化 + 风险地图 |

可选阅读（按需）：
- [05-image-extraction.md](05-image-extraction.md) — 要改图片提取时读
- [06-parallel-and-batch.md](06-parallel-and-batch.md) — 要改并行/批量时读
- [08-comparison-with-pdf-oxide.md](08-comparison-with-pdf-oxide.md) — 要回答"为什么不用 pdf_oxide"时读
- [09-acceleration-opportunities.md](09-acceleration-opportunities.md) — 想继续优化时读

## 2. 把项目跑起来（15 分钟）

### 2.1 环境要求

```bash
# macOS
xcode-select --install
brew install make

# Linux (Debian/Ubuntu)
sudo apt install build-essential pkg-config

# Rust（必须，源码编译 MuPDF 需要）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Python 3.8+
python3 --version
```

### 2.2 首次构建

```bash
cd ritz

# 1. MuPDF 子模块（首次必做）
git submodule update --init --recursive
# 确认 vendor/mupdf 存在且是 1.27.0
cd vendor/mupdf && git checkout 1.27.0 && cd ../..

# 2. Python 虚拟环境
python -m venv .venv
source .venv/bin/activate
pip install maturin pytest

# 3. 编译并安装（editable 模式）
maturin develop --release
# 首次编译 MuPDF 大概 2–5 分钟，后续增量 < 10 秒

# 4. 验证
python -c "import ritz; print(ritz.__version__)"
# 应输出 0.1.0
```

### 2.3 跑测试

```bash
source .venv/bin/activate

# Python 测试套件（37 用例，覆盖所有 API）
pytest python/tests/ -q

# Rust 集成测试
cargo test -p mupdf --test integration_test

# PdfValue 单元测试
cargo test -p ritz value
```

**37 个 pytest 必须全绿。这是合并任何 PR 的硬门。**

### 2.4 跑基准

```bash
# 对比 PyMuPDF（要装：pip install pymupdf）
python benchmarks/bench_vs_pymupdf.py

# 对比 pdf_oxide（要装：pip install pdf_oxide）
python benchmarks/bench_vs_pdf_oxide.py

# Tier 1/2 优化前后对比（自检用）
python benchmarks/bench_tier12_diff.py

# 内存泄漏测试（1000 页 PDF + 1000 次循环）
python benchmarks/leak_test.py
```

## 3. 代码地图

```
ritz/
├── Cargo.toml                   # ⭐ workspace 根（版本号单一来源，[workspace.package].version）
├── README.md                    # 用户文档（API 速查）
├── CHANGELOG.md                 # 版本变更记录
├── docx/                        # 技术决策文档（本文所在）
├── patches/                     # MuPDF 补丁（build.rs 自动应用）
│   └── README.md                # 补丁工作流
├── vendor/mupdf/                # MuPDF 1.27.0 子模块（git submodule）
├── crates/
│   ├── mupdf-sys/               # 第 1 层：FFI + C 扩展
│   │   ├── native/
│   │   │   ├── error_wrapper.c  # ⭐ 所有 mupdf_safe_* 包装（longjmp 隔离）
│   │   │   ├── error_wrapper.h  # C 函数声明（bindgen 输入之一）
│   │   │   ├── mupdf_extensions.c  # mupdf_ext_* 扩展（批量扁平化）
│   │   │   └── mupdf_extensions.h
│   │   ├── wrapper.h            # bindgen 主输入（include 上面两 .h）
│   │   ├── bindings.rs          # ⚠️ 生成产物，C 签名改后要重生成
│   │   ├── build.rs             # 编译 MuPDF + 应用 patches/ + bindgen
│   │   └── src/lib.rs           # 仅 re-export bindings
│   ├── mupdf/                   # 第 2 层：薄安全封装（少量使用）
│   │   ├── src/lib.rs
│   │   ├── benches/             # criterion 微基准
│   │   └── tests/               # Rust 集成测试
│   └── ritz/                    # 第 3 层：PyO3 绑定（主战场）
│       └── src/
│           ├── lib.rs           # 模块注册 + #[pymodule]
│           ├── document.rs      # ⭐ PyDocument（open/metadata/batch）
│           ├── page.rs          # ⭐ PyPage（get_text/get_images/get_pixmap）
│           ├── pixmap.rs        # PyPixmap
│           ├── batch.rs         # process_documents（rayon 并行）
│           └── value.rs         # PdfValue 数据模型
├── python/
│   ├── ritz/
│   │   ├── __init__.py          # from ._ritz import *（纯 re-export）
│   │   └── __main__.py          # python -m ritz 入口
│   └── tests/test_ritz.py       # ⭐ pytest 套件
├── benchmarks/                  # 所有基准脚本（见 §2.4）
└── pyproject.toml               # maturin 配置（module-name = "ritz._ritz"，version = dynamic 从 Cargo 读）
```

### 数据流（一次 `doc[0].get_text("text")` 的旅程）

```
Python: doc[0].get_text("text")
  ↓ PyO3 argument parsing
Rust:   PyPage::get_text(py, "text")                    [page.rs:96]
  ↓     mupdf_safe_new_stext_page(ctx, raw, &mut stpage)
FFI:    error_wrapper.c: fz_try { fz_new_stext_page_from_page() } fz_catch
  ↓
MuPDF C: 解析页面内容流，构造 block→line→char 树
  ↑
FFI:    error_wrapper.c: fz_try { fz_print_stext_page_as_text() } fz_catch
  ↓     growbuf 输出连续 UTF-8 字节
Rust:   cbuf_to_pystring(py, ptr, len)                  [page.rs]
  ↓     PyString::new(py, slice)
Python: 返回 str
```

**FFI 穿越次数**：典型 `get_text("text")` 是 5 次。每次穿越都付 setjmp + TLS 清错开销（~50ns）。**这就是为什么 batch 模式（1 次 FFI 拿全部页）能比 per-page 循环快 2.4x。**

## 4. 不可破坏的不变量

接手后**这五条任何时候都不能违反**，否则会引入 UB 或回归。

### 不变量 #1：longjmp 永不穿透 Rust 栈

MuPDF 用 `setjmp/longjmp` 做错误处理。**longjmp 穿过 Rust 栈会导致 Drop 不执行 = 内存泄漏 / UB**。

**正确做法**：所有 MuPDF 调用包在 C 层的 `fz_try/fz_catch` 里，longjmp 在 C 边界内消化，返回错误码。

```c
// error_wrapper.c — 所有 mupdf_safe_* 的范式
mupdf_clear_error();
fz_try(ctx) {
    /* MuPDF 调用 */
} fz_catch(ctx) {
    set_error("...: %s", fz_caught_message(ctx));
    return -1;
}
return 0;
```

**反面教材**：永远不要在 Rust `unsafe { ... }` 块里直接 `fz_run_page()`。一定要走 `mupdf_safe_*` 包装。

详见 [02-longjmp-isolation.md](02-longjmp-isolation.md)。

### 不变量 #2：Page/Pixmap 持有 `Py<PyDocument>`

`PyPage` 和 `PyPixmap` 都有 `doc_handle: Py<PyDocument>` 字段。**这是 GC 顺序的保险**——保证 Document 不会先于 Page 被 GC（否则 `*mut fz_context` 悬挂）。

**Drop 顺序**：raw_page/raw_pixmap 先 drop（借用 ctx），然后 doc_handle 释放引用（可能触发 Document GC）。

**永远不要**为了让 Page "更轻" 而去掉 doc_handle 字段。

### 不变量 #3：C 层 buffer 协议是契约

`error_wrapper.c` 的所有 `mupdf_safe_*` 函数向 Rust 返回**连续二进制 buffer**，格式在注释里写死（如 `[bbox:16][n:4][utf8]...`）。**Rust 侧按这个协议读取**。

改任何一边的格式都要**同步改另一边**，否则会读到错位数据。当前协议见 error_wrapper.c 函数前的注释块。

### 不变量 #4：GIL 释放 ≠ 自动线程安全

`get_pixmap` 和 `get_text_batch` 用 `py.detach` 释放 GIL。**但 MuPDF 默认 ctx 无锁**，所以：

- ✅ **跨文档并行安全**（每个 doc 独立 ctx）
- ❌ **同一 doc 跨线程并发是 UB**（`Send + Sync` unsafe impl 是为了 PyO3 持有，不是为了真并行）

`process_documents` 的范式是正确的：每个 rayon task 创建自己的 fz_context（[batch.rs:46](../crates/ritz/src/batch.rs)）。

**未来若想加同 doc 并行**：必须给 `PyDocument` 加 `Mutex`，**不能直接放开 GIL**。

### 不变量 #5：`patches/` 工作流不能绕过

MuPDF 子模块是只读的（来自 ArtifexSoftware/mupdf）。**任何对 MuPDF 源码的修改**必须：

1. 在 `vendor/mupdf` 里改
2. `git diff > ../../patches/00NN-name.patch`
3. `git checkout .`（回滚子模块）
4. `build.rs` 会在下次编译时幂等应用

**永远不要直接 commit 修改过的 vendor/mupdf 内容**。详见 [patches/README.md](../patches/README.md)。

## 5. 常见开发任务（带 worked example）

### 任务 A：给 Page 加一个新方法（如 `get_drawings()`）

1. **C 层**（如果需要新 MuPDF 调用）：在 `error_wrapper.c` 加 `mupdf_safe_get_drawings()`，遵循 longjmp 隔离范式。在 `error_wrapper.h` 加声明。

2. **重新生成 bindings**（如果加了新 C 函数）：
   ```bash
   cargo build -p mupdf-sys --features bindgen
   cp target/debug/build/mupdf-sys-*/out/bindings.rs crates/mupdf-sys/bindings.rs
   ```
   ⚠️ **这步容易忘，会导致"找不到符号"或参数类型错位**。见 §6.3。

3. **Rust 层**：在 `crates/ritz/src/page.rs` 加 `#[pyo3(signature = ...)] fn get_drawings(&self, py: Python<'_>, ...) -> PyResult<...>`。复用现有 helper（`cbuf_to_pystring`、PyDict 构造等）。

4. **测试**：在 `python/tests/test_ritz.py` 加 `TestGetDrawings` 类。如果 PyMuPDF 有对应 API，做交叉验证。

5. **基准**：若声称性能，加到 `benchmarks/bench_vs_pymupdf.py`。

### 任务 B：加一个 C 扩展函数（如批量图片提取）

1. **C 层**：在 `mupdf_extensions.c` 加 `mupdf_ext_extract_images_batch()`。这是"批量扁平化"层——一次 FFI 调用拿多页数据，避免 N 次 FFI 往返。在 `mupdf_extensions.h` 加声明。
2. **wrapper.h**：确认 `#include "mupdf_extensions.h"`（已有）。
3. **重生成 bindings**（见任务 A 步骤 2）。
4. **Rust 层**：在合适位置（`document.rs` 或 `batch.rs`）加方法，调用扩展函数。
5. **测试 + 基准**。

### 任务 C：升级 MuPDF 版本（高风险）

1. **先备份 patches/**：`cp -r patches patches.bak`
2. **更新子模块**：`cd vendor/mupdf && git fetch && git checkout 1.27.x && cd ../..`
3. **清空 patches**：`rm patches/*.patch`（保留 README.md）
4. **重新编译**：`cargo build -p mupdf-sys`，修复编译错误
5. **重新生成 patches**：按需把以前的修改重新 diff 出来，重新编号
6. **跑全套测试**：`pytest python/tests/ -q` + Rust 集成测试
7. **跑基准**：确认性能没回归

详见 plan_v1 §5.5"季度同步"。

### 任务 D：调试 FFI 崩溃

**症状**：Python 进程 SIGSEGV / abort，无 traceback。

**排查思路**：
1. **90% 是 longjmp 穿透 Rust 栈**——检查是否有 Rust 代码直接调用 MuPDF 函数（绕过 `mupdf_safe_*`）
2. **5% 是 buffer 协议不匹配**——C 写的格式和 Rust 读的格式不一致。检查 error_wrapper.c 函数注释 vs page.rs 的 `take_*` 函数
3. **5% 是 GC 顺序**——某处去掉了 `Py<PyDocument>` 引用，doc 被 GC 后 page 还在用

**调试工具**：
```bash
# AddressSanitizer（需要重编 MuPDF，慢但能精准定位）
ASAN_OPTIONS=detect_leaks=0 cargo build -p mupdf-sys --release
# （build.rs 需临时加 -fsanitize=address）

# Python 层的 faulthandler
python -X faulthandler -c "import ritz; ..."

# lldb（macOS）/ gdb（Linux）
lldb -- python -c "import ritz; ..."
(lldb) run
(lldb) bt
```

## 6. 已知问题与陷阱（"尸体地图"）

接手前**必读**。这些问题已知、已记录、但**没修**——因为要么是设计权衡、要么是低优先级。

### 6.1 ⚠️ `Send + Sync` 是 unsound（高优先级 latent bug）

`crates/ritz/src/document.rs:280`、`page.rs:664`、`pixmap.rs:141` 的 `unsafe impl Send + Sync` 在"同 doc 跨线程并发"场景下是 UB（MuPDF 默认 ctx 无锁）。

**目前安全仅因为**：除 `process_documents` 外，没有任何 per-document 方法释放 GIL。`process_documents` 用的是独立 ctx（正确范式）。

**会激活 bug 的修改**：在 `get_text` / `get_images` / 任何 per-page 方法上**加 `py.detach`**——立即让两个 Python 线程能并发跑同一 doc。

**正确处理方式**（若真要做）：给 `PyDocument` 加 `Mutex` 或 `RwLock`，或文档化"同 doc 跨线程 UB"（当前选择）。

详见 [docx/10-tier1-tier2-results.md §"Tier 1 #1 风险"](10-tier1-tier2-results.md)。

### 6.2 ⚠️ `get_pixmap` matrix 折叠 bug（中优先级 correctness bug）

`page.rs:316` 把传入的 6 元组 matrix `(a,b,c,d,e,f)` 折叠成单 scalar `zoom = max(a,d)`，**静默丢弃旋转/平移/斜切**。

**影响**：任何带 rotation 的页面、任何带 e/f 平移的 matrix 都渲染错位。

**未修原因**：低频触发的边缘 case；正确实现需要传完整 6 元组到 C 层的 `fz_concat` / `fz_scale + fz_translate`。

**复现**：`doc[0].get_pixmap(matrix=(1, 0, 0, 1, 100, 100))` 应该平移 (100,100)，实际不平移。

详见 [docx/09-acceleration-opportunities.md §"Bug 1"](09-acceleration-opportunities.md)。

### 6.3 bindings.rs 缓存陷阱

`crates/mupdf-sys/bindings.rs` 是 bindgen 生成产物，**但被 commit 进 git**（为了让没装 bindgen 的用户也能编译）。

**陷阱**：你改了 C 签名（在 error_wrapper.h 加参数），bindings.rs 不会自动更新，导致 Rust 侧"expected i32, found &mut *mut i8"之类的诡异编译错误。

**解决**：
```bash
cargo build -p mupdf-sys --features bindgen
cp target/debug/build/mupdf-sys-*/out/bindings.rs crates/mupdf-sys/bindings.rs
```

**预防**：CI 应加一步验证 bindings.rs 是最新（diff 生成产物 vs committed）。

### 6.4 文本提取 10x 不可达（设计天花板）

`get_text("text")` 在 ritz 和 PyMuPDF 上**都走 `fz_new_stext_page_from_page`**（构造 block→line→char 树）。**stext 构造是双方共享的硬地板**，ritz 只能省一次 buffer 拷贝，上限 ~2x。

要破 10x 必须写**自定义 `fz_device` 直出 UTF-8**，跳过 stext 树。**但这会丢失 bidi/排版顺序**——对 CJK/RTL 会出错。

**plan_v1 早就分析过这是高风险**，不做。后续也别花时间在这个方向。详见 [docx/09 §Tier 3 #12](09-acceleration-opportunities.md)。

### 6.5 Tier 2 #7/#8 教训：局部优化 ≠ 端到端优化

audit 预测 Tier 2 #7（words/blocks 单趟）能加速 30–40%。实测只 2–4%。

**原因**：预测针对的是"后处理部分"，但 stext 构造占 90%+，端到端收益被稀释。

**教训**：**任何优化都要先实测再合并**。用 [benchmarks/bench_tier12_diff.py](../benchmarks/bench_tier12_diff.py) 验证。

详见 [docx/10 §"为什么实际收益小"](10-tier1-tier2-results.md)。

### 6.6 缓存没有失效逻辑（未来陷阱）

当前 `metadata_cache`、`rect_cache`、`raw_doc` 都假设底层不可变。**若未来加 PDF 编辑功能（plan_v1 Phase 5）**，所有这些缓存都需要在编辑时失效。

**建议**：加编辑功能前，先 grep 所有 `OnceLock` / `OnceCell`，逐个评估失效策略。

### 6.7 patches/ 应用失败只发警告

`build.rs` 应用 patches/ 失败**不会中断构建**——只发 warning。CI 不会因为补丁冲突卡住，但本地排查时**一定要看 build log 里的 warning**。

## 7. 当前状态与路线图

### 7.1 plan_v1 KPI 状态（截至 2026-06-14）

| KPI | 状态 |
|-----|------|
| 架构（四层） | ✅ 达成 |
| 安全健壮（1000 页 ≤200MB、泄漏 <5KB/iter） | ✅ 达成（[benchmarks/leak_test.py](../benchmarks/leak_test.py)） |
| 文本提取性能 | ✅ 2.1x（plan 没点名 10x，达标） |
| 图片提取（JPEG 原始字节） | ✅ 6.2x |
| 批量并行 | ✅ 2.4x（batch）/ 2.1x（多文档） |
| 链接读取 | ✅ 27x |
| 渲染 | ✅ 与 PyMuPDF 持平（共享 MuPDF 引擎） |
| wheel ≤35MB | ✅ 实测 29MB |
| PdfValue 数据模型暴露给 Python | ✅ |
| patches/ 工作流 | ✅ |
| 文本搜索 / 大纲 / PDF 编辑 / 注释 | ❌ Phase 5 待办 |

### 7.2 优先级建议（给新维护者）

按 ROI 排序：

1. **修 §6.2 matrix bug**（< 半天）— 正确性问题，影响所有 rotation 页面
2. **补文本搜索 `page.search_for()`**（1–2 天）— PyMuPDF 兼容 API，用 `fz_search_stext_page`
3. **补大纲 `doc.get_toc()`**（半天）— 用 `fz_load_outline`
4. **图片直读 `pdf_load_object_stream`**（[docx/09 Tier 3 #11](09-acceleration-opportunities.md)，1 周）— 大赢面 3–20x
5. **PDF 编辑（增删页）**（plan_v1 Phase 5，> 1 个月）— 需要完整规划，包含缓存失效

### 7.3 已拒绝的方向（不要再尝试）

- **自定义 `fz_device` 破 text 10x**（§6.4）— 高风险，破坏 CJK/RTL
- **MIT/Apache 重许可**（除非 MuPDF 换引擎，否则 AGPL 不可破）
- **同 doc 跨线程并发**（§6.1）— MuPDF ctx 无锁，必须加 Mutex，复杂度不值

## 8. 沟通约定与商业考虑

### 8.1 AGPL 商业许可

**ritz 是 AGPL-3.0**（被 MuPDF 强制）。任何商业闭源产品想用 ritz 必须买 [Artifex MuPDF commercial license](https://mupdf.com/licensing/index.html)。

**不要承诺"未来换 MIT 许可"**——除非完全抛弃 MuPDF 引擎（相当于重写整个项目，参考 pdf_oxide 的做法）。

### 8.2 Upstream 贡献

通用性强的 MuPDF 扩展（如 `mupdf_ext_*` 里的批量扁平化）整理后**优先 PR 到 ArtifexSoftware/mupdf**。被采纳后 ritz 的 patches/ 里对应的 patch 就可以删了。

### 8.3 季度同步

每季度跑一次：
```bash
cd vendor/mupdf && git fetch origin && git log HEAD..origin/master --oneline
```
评估是否有值得合并的上游改动。详见 plan_v1 §5.5。

## 9. 实用速查

### 9.1 一行命令清单

```bash
# 构建
source ~/.cargo/env && source .venv/bin/activate && maturin develop --release

# 改版本号（单一来源，其余 4 处自动跟随）
sed -i '' 's/^version = "0.1.0"/version = "0.2.0"/' Cargo.toml

# 测试
pytest python/tests/ -q
cargo test -p mupdf --test integration_test

# 基准
python benchmarks/bench_vs_pymupdf.py
python benchmarks/bench_tier12_diff.py
python benchmarks/leak_test.py

# 重新生成 bindings（C 签名改后必跑）
cargo build -p mupdf-sys --features bindgen
cp target/debug/build/mupdf-sys-*/out/bindings.rs crates/mupdf-sys/bindings.rs

# 应用 patches（验证）
cargo build -p mupdf-sys 2>&1 | grep "applied patch"

# 测试 PDF 样本位置
ls crates/mupdf/tests/samples/        # ritz 自带
ls vendor/mupdf/thirdparty/extract/test/  # MuPDF 自带
```

### 9.2 关键文件 quick-link

| 要改什么 | 看哪里 |
|---------|-------|
| 加 Page 方法 | `crates/ritz/src/page.rs` |
| 加 Document 方法 | `crates/ritz/src/document.rs` |
| 加 Python 顶层函数 | `crates/ritz/src/lib.rs` + `batch.rs` |
| 加 C safe 包装（含 longjmp 隔离） | `crates/mupdf-sys/native/error_wrapper.c` |
| 加 C 批量扩展 | `crates/mupdf-sys/native/mupdf_extensions.c` |
| 改 buffer 协议 | C 函数前注释 + Rust 的 `take_*` 函数（同步改） |
| 加测试 | `python/tests/test_ritz.py` |
| 加基准 | `benchmarks/bench_*.py` |

### 9.3 问问题之前先看

| 问题 | 答案在 |
|------|-------|
| "为什么文本不能 10x?" | [07-performance-analysis.md](07-performance-analysis.md) + §6.4 |
| "为什么 MIT 不能？" | README.md "许可证" + §8.1 |
| "为什么 fitz.open 不能用？" | ritz 是 `import ritz` 不是 `import fitz` |
| "为什么 pyo3 编译报错？" | 90% 是 bindings.rs 缓存（§6.3） |
| "为什么 patches 没应用？" | 看构建 log 的 warning（§6.7） |
| "为什么同 doc 多线程崩？" | Send+Sync unsound（§6.1） |
| "为什么 rotation 渲染错位？" | matrix 折叠 bug（§6.2） |
| "为什么我的优化没效果？" | stext 构造主导（§6.5） |

## 10. 第一周建议任务

按难度递增：

1. **跑通构建 + 测试**（§2）— 30 分钟
2. **读完 docx/01–04 + 07 + 10**（§1）— 2 小时
3. **修 matrix bug**（§6.2）— 半天，正确性修复，单 PR
4. **加 `page.search_for("query")`** — 1 天，复用 `fz_search_stext_page`
5. **加 `doc.get_toc()`** — 半天，复用 `fz_load_outline`

完成 1–5 你就基本掌握项目了。后面可以挑 [docx/09 Tier 3](09-acceleration-opportunities.md) 里的大项做。

---

**最后**：ritz 是单人项目，所有决策记录在 docx/。**改任何非 trivial 的东西之前，先 grep docx/ 看有没有相关讨论**。如果有，更新那篇文档；如果没有，写一篇新的。代码会过时，文档不会。
