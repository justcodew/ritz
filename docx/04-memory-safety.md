# 内存安全与生命周期

> ritz 大量使用裸指针（`*mut fz_context` / `*mut fz_document` / `*mut fz_page`）。本章解释为什么这些裸指针不会悬挂，以及 Drop 顺序如何保证安全。

## 头号风险：ctx 悬挂

MuPDF 的所有对象都依赖 `fz_context`（上下文）。如果 ctx 被释放了，但 Page/Pixmap 还持有指向 ctx 分配的内存的指针，就是 use-after-free。

```
Document (持有 ctx + doc)
  └─ Page (持有 ctx + page)     ← page 是 ctx 分配的
       └─ Pixmap (持有 ctx + pix)  ← pix 是 ctx 分配的
```

**危险场景**：Python GC 先回收 Document，再回收 Page。Page 的 Drop 调 `fz_drop_page(ctx, page)`，但 ctx 已经被释放了 → 段错误。

## 解决方案：Py<PyDocument> 引用计数

Page 和 Pixmap 持有 `Py<PyDocument>` 句柄——这是 PyO3 的 Python 对象引用计数。只要 Page 还活着，Document 就不会被 GC。

```rust
// page.rs
pub struct PyPage {
    ctx: *mut fz_context,
    raw: *mut fz_page,
    doc_handle: Py<PyDocument>,  // ← 防止 Document 被 GC
}
```

`Py<PyDocument>` 等价于 Python 的强引用：
- `Py::clone_ref(py)` 增加引用计数
- `Py::drop` 减少引用计数
- 引用计数归零时，Python GC 回收 Document

**效果**：Document 的生命周期 ≥ 所有 Page/Pixmap 的生命周期。ctx 永远不会在 Page/Pixmap 还活着时被释放。

### Drop 顺序

Rust 保证 struct 字段按**声明顺序逆序** drop。`PyPage` 的声明顺序：

```rust
pub struct PyPage {
    ctx: *mut fz_context,      // 1. 先声明
    raw: *mut fz_page,          // 2.
    doc_handle: Py<PyDocument>, // 3. 最后声明 → 最先 drop
}
```

实际 drop 顺序（逆序）：
1. `doc_handle` drop → 减少引用计数（但只要 Python 侧还有引用，Document 不会死）
2. `ctx` drop → 裸指针，无 Drop 实现（什么都不会发生）
3. `raw` drop → 裸指针，无 Drop 实现（什么都不会发生）

**等等，那 `fz_drop_page` 什么时候调？** 裸指针没有 Drop，所以 ritz 用 `Drop` trait 手动调：

```rust
impl Drop for PyPage {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { mupdf_safe_drop_page(self.ctx, self.raw); }
            self.raw = ptr::null_mut();
        }
        // doc_handle 在 struct 字段 drop 时自动减引用
    }
}
```

所以实际 Drop 顺序是：
1. `PyPage::drop` 执行 → 调 `fz_drop_page(ctx, raw)`（ctx 此时一定还活着，因为 doc_handle 还没 drop）
2. struct 字段逆序 drop → `doc_handle` 减引用计数

**关键**：`fz_drop_page` 在 `doc_handle` 减引用**之前**执行。所以 page 释放时 ctx 一定有效。

## 第二个风险：Python GC 顺序不可控

Python 的 GC 不保证回收顺序。用户代码：

```python
doc = ritz.open("x.pdf")
page = doc[0]
del doc      # 不一定立即回收 Document
del page     # 不一定立即回收 Page
```

`Py<PyDocument>` 引用计数解决了这个问题：
- `del doc` 只删除 Python 变量 `doc` 的引用
- `page.doc_handle` 仍持有 `Py<PyDocument>` → Document 引用计数 > 0
- Document 在 page 被 GC **之后**才会被 GC

```python
doc = ritz.open("x.pdf")
page = doc[0]
del doc        # Document refcount = 1（page 持有）
# Document 还活着 ✅
del page       # Page drop → fz_drop_page(ctx, raw)
               # 然后 doc_handle drop → Document refcount = 0 → GC
               # Document drop → fz_drop_document → fz_drop_context
```

## 第三个风险：Rust unsafe 边界

所有裸指针操作都在 `unsafe` 块里。ritz 的原则：

```rust
// ✅ 安全：C 层已隔离 longjmp，裸指针操作不会 panic
let rc = unsafe { mupdf_safe_load_page(ctx, doc.raw, 0, &mut page.raw) };

// ✅ 安全：slice 是只读，长度由 C 层保证
let bytes = unsafe { slice::from_raw_parts(ptr as *const u8, len) };

// ✅ 安全：Py<PyDocument> 保证 ctx 在 Page 生命周期内有效
unsafe { mupdf_safe_drop_page(self.ctx, self.raw); }
```

**不安全的前提**是前面两层保证：
1. longjmp 隔离（C 层）→ C 调用不会跳过 Rust Drop
2. `Py<PyDocument>` 引用计数 → ctx 不会悬挂

## 内存释放：谁分配谁释放

| 分配方 | 释放方 | 函数 |
|--------|--------|------|
| C 层 `fz_realloc`（growbuf） | Rust `mupdf_free` | 链接/图片/dict 扁平 buffer |
| MuPDF `fz_new_*` | MuPDF `fz_drop_*` | document / page / pixmap / stext |
| Rust `Box`/`Vec` | Rust 自动 Drop | Rust 侧临时数据 |

C 层 growbuf 分配的 buffer 返回给 Rust 后，Rust 必须调 `mupdf_free`：

```rust
// page.rs — get_links
let result = parse_links(py, ptr, len);
unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };  // 释放 C 层 growbuf
result
```

如果忘了这行，就是内存泄漏。1000 次循环泄漏测试验证了 ritz 没有这个问题（见下方验证）。

## 验证：1000 页内存 + 1000 次循环泄漏

`benchmarks/leak_test.py` 做了两类验证（plan_v1 §4.2 要求）：

### A. 常驻内存（1000 页 ≤ 200 MB）

```python
doc = ritz.open("/tmp/ritz_big_1000.pdf")  # 1000 页
for i in range(1000):
    doc[i].get_text("text")   # 全页文本提取
    doc[i].get_text("dict")   # 全页 dict 提取
# 峰值 RSS: 72.9 MB（plan 目标 ≤ 200 MB）✅
```

### B. 泄漏压力测试

```python
# 1000 次 open/close
for i in range(1000):
    d = ritz.open(sample)
    _ = len(d)
    d = None
# RSS 斜率: 0.1 KB/iter（目标 < 100 KB/iter）✅

# 1000 次 page 重负载
for i in range(1000):
    p = doc[0]
    p.get_text("dict")
    p.get_pixmap()
    p.get_images(include_data=True)
# RSS 斜率: 4.6 KB/iter（目标 < 100 KB/iter）✅
```

open/close 几乎零泄漏（0.1 KB/iter 在测量噪音内）。page-ops 的 4.6 KB/iter 是 Python 对象缓存（GC 后稳定，非 C 层泄漏）。

## Send + Sync 问题

`fz_context` 不是 `Send`/`Sync`——MuPDF 不保证线程安全。但 ritz 的 `PyDocument` 需要在 rayon 并行时跨线程。

解决方案：**不为 `PyDocument` 实现 `Send`/`Sync`**。并行时每文档创建独立 ctx，不共享。详见 [06-parallel-and-batch.md](06-parallel-and-batch.md)。
