# 加速机会审计（2026-06-14）

> 对 ritz 当前实现的全面性能审计，识别 plan_v1 之后的剩余加速空间。
> 本文档作为**历史记录**——记录优化前的实现状态与机会清单。

## 背景

plan_v1 优化的重点是**架构层**：C 层扁平化、batch 一次性 FFI、rayon 并行。当前性能（vs PyMuPDF）：

| 场景 | 加速比 |
|------|-------|
| 链接读取 | 27x |
| JPEG 图片提取 | 6.2x |
| 文档打开 | 5.6x |
| 批量文本 | 2.4x |
| 多文档并行 | 2.1x |
| 文本提取 | 2.1x |

本次审计聚焦**实现层**——架构已经对了，但细节有没有浪费。三条主路径（文本/图片/渲染）+ 横切关注点（FFI/PyO3/内存）逐一审查。

## 审计方法

并行三路探索：
1. **文本 + 批量路径**：`page.rs get_text` → `error_wrapper.c mupdf_safe_*` → MuPDF `fz_new_stext_page_from_page`
2. **图片 + 渲染路径**：`page.rs get_images/get_pixmap` → `error_wrapper.c image_capture_device` → MuPDF draw device
3. **横切关注**：GIL 释放、上下文锁、Py<PyDocument> 引用计数、setjmp 开销、缓存层、字节拷贝

## 当前管线摘要

### 文本提取（`get_text("text")`）

每次调用的 FFI 穿越：**5 次**

```
PyO3 arg parse
  → fz_new_stext_page_from_page      [FFI #1] 建 block→line→char 树
  → fz_print_stext_page_as_text      [FFI #2] 走第二遍树写 fz_buffer
  → fz_buffer_extract (malloc 拷贝)
  → Rust String::from_utf8_lossy().into_owned()    [总是 alloc+copy]
  → PyString 构造                    [第三次拷贝]
  → fz_drop_stext_page               [FFI #4]
  → mupdf_free                       [FFI #5]
```

**树构造了一次，又遍历了一次**；字节拷贝 ~4 次（fz_buffer → extract malloc → String → PyUnicode）。

### 图片提取（`get_images(include_data=True)`）

- 建 display list → 跑自定义 `image_capture_device` → `fill_image` 回调每张图写 bbox+xref+bytes 进 growbuf
- xref 预扫（`pdf_load_image` 每张 XObject 全量加载）→ 然后回调里**再加载一次**
- JPEG/PNG/JPX 走 `fz_compressed_buffer`（直出原始压缩字节），其他格式 PNG 重编码
- growbuf → Rust `slice::from_raw_parts` → `PyBytes::new`（**又一次 memcpy**）

### 渲染（`get_pixmap`）

- 直跑 `fz_run_page + fz_new_draw_device`（**不建 display list**，正确）
- matrix 被折叠成单 scalar `zoom = max(a, d)`——**静默丢弃旋转/平移/斜切**
- samples → `PyBytes::new`（拷贝）
- `tobytes("png")` 多一次 `slice.to_vec()`（**笔误式重复拷贝**）

### 横切关注

- **GIL 在最重的两个调用上不释放**：`get_pixmap`（50–500ms 纯 C）、`get_text_batch` 都全程持 GIL
- **缓存完全不存在**：metadata 每次 9 次 FFI、rect/width/height 三次 setjmp、stext 每次 `get_text` 重建、raw_doc 每次重取
- **Py<PyDocument> 在每次 page 方法调用上 deref**（`.bind().borrow()`）
- **setjmp 每次 FFI ~20–80ns + TLS 清字节**（40+ 个 fz_try 站点，不可避免）
- **bytes 路径双拷贝**：C heap → Rust Vec → PyBytes

## 优化机会（按 ROI 排序）

### Tier 1 — 低成本/立即可做

| # | 优化 | 估计加速 | 风险 |
|---|------|---------|------|
| 1 | `get_pixmap` 包 `py.detach` 释放 GIL | 解锁 Python 多线程并行渲染 | 低 |
| 2 | `get_text_batch` 包 `py.detach` 释放 GIL | 批量场景可被多线程 | 低 |
| 3 | memoize `metadata` dict（OnceCell） | 当前每次访问 9 次 FFI + 9 次 setjmp | 无 |
| 4 | 删掉 `tobytes("png")` 里多余的 `slice.to_vec()` | PNG 路径省一次全图拷贝 | 无 |
| 5 | Page 上缓存 `raw_doc` 指针（load 时一次性取） | 每次调用省一次 `.bind().borrow()` | 无 |
| 6 | memoize `page.rect`（OnceCell） | rect/width/height 三次 → 一次 FFI | 无 |

**预估合计影响**：单线程 5–15%，多线程渲染数量级。改动 < 100 行。

### Tier 2 — 中成本/明显收益

| # | 优化 | 估计加速 | 风险 |
|---|------|---------|------|
| 7 | words/blocks 改单趟 growbuf（现 sizing+writing 两趟） | 该模式 30–40% | 低 |
| 8 | `from_utf8_lossy` → `PyString::new` 直接构造 | text/html/xml 路径 10–20% | 低（先用 `std::str::from_utf8` 校验） |
| 9 | `get_text_batch` 零拷贝（PyBytes 视图替代 per-page String→PyString 双拷贝） | batch 15–30% | API 变化，需加 `raw=False` 参数 |
| 10 | stext 缓存（Page 上 `OnceCell<*mut fz_stext_page>`，Drop 释放） | 重复 `get_text` 调用 2x | 中（跨 mode 失效） |

### Tier 3 — 高成本/需谨慎评估

| # | 优化 | 估计加速 | 风险 |
|---|------|---------|------|
| 11 | XObject 图片直读 `pdf_load_object_stream`，绕过 display list | 图片密集页 3–20x | 中（inline image / 非 PDF 走不通，需 fallback） |
| 12 | 自定义 `fz_device` 写 text，绕过 stext 树 | `get_text("text")` 2–4x | **高**——bidi/RTL/CJK 顺序会出错。plan_v1 早就分析过这是 10x 不可达的根因，不建议碰 |
| 13 | PyBytes buffer protocol / memoryview 零拷贝 | 大 pixmap（4K RGBA ~67MB）省一次完整 memcpy | API 变化，需 Python 端配合 |

## 顺便要修的两个正确性 bug（非性能）

### Bug 1：`get_pixmap` matrix 折叠

`page.rs:316` 只取 `max(a, d)`，**静默丢弃旋转/平移/斜切**。任何带 rotation 的页面、任何带 e/f 平移的 matrix 都渲染错位。应传完整 6 元组给 `fz_concat` / `fz_scale + fz_translate`。

### Bug 2：`Send+Sync` 是 unsound

`document.rs:280`、`page.rs:664`、`pixmap.rs:141` 上 `unsafe impl Send + Sync`，但 MuPDF 默认 context 无锁（`fz_new_context(NULL, NULL, ...)`）。

**目前安全仅因为**热路径都没释放 GIL。一旦做 Tier 1 第 1、2 项（释放 GIL），必须二选一：
- 给 `PyDocument` 加 `Mutex`/`RwLock`
- 文档化"同 doc 跨线程使用是 UB"

`process_documents` 已经是正确范式——每个 rayon task 用独立 `fz_context`，所以并行不受影响。

## 不建议做

**rawdict 的 per-char PyDict churn**（`page.rs:600`）：要破 2–5x 必须改返回结构，破坏 PyMuPDF 兼容。直接在文档里推荐用 `rawjson`（已实现，走 MuPDF 原生 JSON 序列化）。

## 整体判断

当前 ritz 已优化的是**架构层**（C 层扁平化、batch 一次性 FFI、rayon 并行），**实现层还有大量低垂果实**：

- 缓存**完全没有**（metadata、rect、stext、raw_doc 都每次重算）
- GIL 在最重的两个调用上没释放
- bytes 路径有**重复 memcpy**

**保守估计**：Tier 1（6 项）+ Tier 2 的 7、8 两项做完，**单线程能再快 15–25%，多线程渲染能快 5–10x**。这比"破 text 10x 天花板"现实得多。

## 执行计划

本次先做 Tier 1 全部 6 项，跑基准验证，再决定是否进 Tier 2。

| Tier 1 项 | 验证方式 |
|----------|---------|
| GIL 释放（#1, #2） | 并行渲染基准 |
| 缓存类（#3, #5, #6） | 重复调用基准 + pytest |
| 删 to_vec（#4） | PNG tobytes 基准 + 图像 hash 比对 |

完成标准：
- `pytest python/tests/ -q` 全绿
- `benchmarks/bench_vs_pymupdf.py` 在涉及场景上不回归或加速
- `benchmarks/bench_vs_pdf_oxide.py` 同上

## 附录：关键文件

| 文件 | 关注点 |
|------|-------|
| `crates/ritz/src/page.rs` | get_text/get_images/get_pixmap/matrix 折叠 |
| `crates/ritz/src/document.rs` | metadata/get_text_batch/Send+Sync |
| `crates/ritz/src/pixmap.rs` | tobytes 重复 to_vec/Send+Sync |
| `crates/ritz/src/batch.rs` | process_documents GIL 释放（已是正确范式） |
| `crates/mupdf-sys/native/error_wrapper.c` | 所有 mupdf_safe_* 包装、image_capture_device |
| `crates/mupdf-sys/native/mupdf_extensions.c` | mupdf_ext_extract_pages_text（批量流式） |
