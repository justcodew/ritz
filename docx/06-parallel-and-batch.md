# rayon 并行与批量提取

> [plan_v1 §5.3](../plan/01-plan-v1.md) 的并行策略。本文解释为什么 fz_context 不能跨线程共享，以及 ritz 如何用 rayon + 独立 ctx 实现真正的多文档并行。

## 问题：MuPDF 不是线程安全的

MuPDF 的 `fz_context` **不是线程安全的**——多个线程同时用同一个 ctx 操作文档会竞争内部状态（store、lock、allocator）。

MuPDF 的并行模型：
- 每个**线程**需要独立的 `fz_context`（通过 `fz_clone_context` 克隆）
- 克隆的 ctx 共享全局锁（`fz_lock`），但仍需谨慎
- 单文档内多页并行**不安全**（`fz_document` 不是线程安全）

## 策略：多文档并行（每文档独立 ctx）

plan_v1 明确选择多文档并行而非单文档多页并行：

> **多文档并行（主推）**：每个线程创建独立的 fz_context 并独立打开文档，通过 rayon 实现多文档并行处理。
> **不承诺单文档多页并行**：因 fz_document 非线程安全，强行并行需每页独立打开文档，内存开销大且收益不稳定。

### 实现：process_documents

```rust
// batch.rs
use rayon::prelude::*;

#[pyfunction]
pub fn process_documents(
    py: Python<'_>,
    paths: Vec<String>,
) -> PyResult<Vec<Option<Vec<String>>>> {
    // py.detach 释放 GIL，让 rayon worker 真正并行
    let results: Vec<DocResult> = py.detach(|| {
        paths.par_iter().map(|p| process_one(p)).collect()
    });

    // 重新获取 GIL，构造 Python 返回值
    Ok(results.into_iter().map(|r| match r {
        DocResult::Ok(texts) => Some(texts),
        DocResult::Err(_) => None,
    }).collect())
}
```

`py.detach`（PyO3 0.29，旧名 `allow_threads`）释放 GIL。没有这行，rayon 的 worker 线程虽然能跑，但都卡在 GIL 上，实际串行。

### 每文档独立 ctx

```rust
fn process_one(path: &str) -> DocResult {
    // 每个文档创建全新 ctx（不是 clone）
    let ctx: *mut fz_context = unsafe { mupdf_safe_new_context() };
    if ctx.is_null() {
        return DocResult::Err;
    }

    let doc: *mut fz_document = ptr::null_mut();
    let rc = unsafe { mupdf_safe_open_document(ctx, &mut doc, path_c.as_ptr()) };
    if rc != 0 {
        unsafe { mupdf_safe_drop_context(ctx) };
        return DocResult::Err;
    }

    // 用这个文档的独立 ctx 提取全部文本
    let texts = extract_all_text(ctx, doc);

    // 清理（逆序：doc → ctx）
    unsafe {
        mupdf_safe_drop_document(ctx, doc);
        mupdf_safe_drop_context(ctx);
    }
    DocResult::Ok(texts)
}
```

**关键**：每个文档用 `mupdf_safe_new_context()` 创建全新 ctx，不是 `fz_clone_context`。全新 ctx 无共享状态，rayon worker 之间零竞争。

### 为何不用 clone_context？

`fz_clone_context` 克隆 ctx 但共享全局锁。MuPDF 文档说：

> A cloned context shares locks with its parent. This means that operations in the cloned context will block on the same locks as the parent.

这意味着 clone 的 ctx 之间仍有锁竞争。用全新 ctx（`fz_new_context` + `fz_register_document_handlers`）彻底避免锁。

### DocResult：跨线程安全

rayon 的 `par_iter` 要求闭包返回 `Send`。`DocResult` 是纯 Rust 枚举：

```rust
enum DocResult {
    Ok(Vec<String>),
    Err,
}
```

`Vec<String>` 是 `Send`，所以 `DocResult` 自动 `Send`。裸指针（`*mut fz_context`）不跨线程——它们在 `process_one` 内创建和销毁，不返回到外层。

## 批量提取：减少 FFI 往返

### 单文档批量：get_text_batch

```python
# 一次调用提取全部页文本
pages_text = doc.get_text_batch()
# 等价于 [doc[i].get_text("text") for i in range(len(doc))]，但更快
```

### C 层实现：流式 growbuf

```c
// mupdf_extensions.c — mupdf_ext_extract_pages_text
int mupdf_ext_extract_pages_text(fz_context *ctx, fz_document *doc,
                                  char **out_text, size_t *out_len,
                                  size_t **out_offsets, int *page_count) {
    growbuf gb = {0};
    int n = fz_count_pages(ctx, doc);

    // 输出 page_count + 1 个 offset（offsets[i] 到 offsets[i+1] 是第 i 页的文本）
    // ...

    for (int i = 0; i < n; i++) {
        size_t start = gb.len;  // 记录本页起始偏移
        fz_page *page = fz_load_page(ctx, doc, i);
        fz_stext_page *st = fz_new_stext_page_from_page(ctx, page, NULL);

        // 用自定义 fz_output write callback 直接流入 growbuf（零中间拷贝）
        fz_output *out = fz_new_output(ctx, 4096, &gb,
                                       ext_write_to_growbuf,  // write 回调
                                       NULL, NULL, NULL, NULL, NULL);
        fz_print_stext_page_as_text(ctx, out, st);
        fz_close_output(ctx, out);
        fz_drop_output(ctx, out);

        // 记录 offset
        offsets[i] = start;

        fz_drop_stext_page(ctx, st);
        fz_drop_page(ctx, page);
    }
    offsets[n] = gb.len;  // 结束偏移

    *out_text = (char*)gb.data;
    *out_len = gb.len;
    return 0;
}
```

关键：用 `fz_new_output` + 自定义 write 回调，让 `fz_print_stext_page_as_text` 直接写入 growbuf。避免 `fz_buffer_extract` 的中间拷贝。

### Rust 侧：按 offset 切片

```rust
// document.rs — get_text_batch
fn get_text_batch(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
    let mut text_ptr: *mut c_char = ptr::null_mut();
    let mut offsets_ptr: *mut usize = ptr::null_mut();
    let mut text_len: usize = 0;
    let mut page_count: c_int = 0;

    unsafe {
        mupdf_ext_extract_pages_text(
            self.ctx, self.raw,
            &mut text_ptr, &mut text_len,
            &mut offsets_ptr, &mut page_count,
        );
    }

    // 一次切片，按 offset 分段构造每页的 str
    let text_bytes = unsafe { slice::from_raw_parts(text_ptr as *const u8, text_len) };
    let offsets = unsafe { slice::from_raw_parts(offsets_ptr, page_count as usize + 1) };

    let mut out = Vec::with_capacity(page_count as usize);
    for i in 0..page_count as usize {
        let start = offsets[i];
        let end = offsets[i + 1];
        let s = std::str::from_utf8(&text_bytes[start..end])
            .unwrap_or("")
            .to_string();
        out.push(PyString::new(py, &s).into_any().unbind());
    }

    unsafe {
        mupdf_free(text_ptr as *mut _);
        mupdf_free(offsets_ptr as *mut _);
    }
    Ok(out)
}
```

### 批量 vs 逐页：什么时候有意义？

| 维度 | get_text_batch | 逐页循环 |
|------|---------------|---------|
| FFI 调用 | 1 次 | N 次（N = 页数） |
| Python 对象构造 | N 个 str | N 个 str + N 个临时 Page |
| stext 构造 | N 次（不可避免） | N 次 |
| 实测加速（vs ritz 逐页） | **1.0x** | 基准 |
| 实测加速（vs PyMuPDF 逐页） | **2.4x** | — |

**关键发现**：batch vs ritz 自己的逐页循环只有 1.0x——因为 stext 构造占总耗时的 99%，省下的 FFI 往返被淹没。batch 的真正价值是 vs PyMuPDF 的 2.4x（PyMuPDF 逐页有更大的 Python 调度开销）。

**plan_v1 预期 batch 带来 10x，实测证明这个预期过于乐观**：当单页操作已被 stext 构造主导时，减少 FFI 往返次数的收益可忽略。详见 [07-performance-analysis.md](07-performance-analysis.md)。

## py.detach 与 GIL

PyO3 默认持有 GIL。rayon worker 在 Rust 侧并行，但如果 GIL 未释放，Python 侧仍是单线程。

```rust
// ✅ 正确：释放 GIL 让 worker 真正并行
let results = py.detach(|| paths.par_iter().map(process_one).collect());

// ❌ 错误：GIL 未释放，rayon worker 卡在 GIL 上
let results = paths.par_iter().map(process_one).collect();
```

`py.detach` 的语义：
1. 释放 GIL
2. 执行闭包（worker 线程真正并行）
3. 重新获取 GIL
4. 返回结果

闭包内不能访问 Python 对象（无 GIL）。所以 `process_one` 是纯 Rust + C 操作，不碰 Python。结果用 `Send` 的 `DocResult` 带回，再在持有 GIL 时构造 Python 对象。

## 性能数据

| 场景 | ritz | PyMuPDF | 加速 |
|------|------|---------|------|
| 8 文档串行 | 81 ms | 114 ms | 1.4x |
| 8 文档并行 | 54 ms | 111 ms | **2.1x** |
| rayon 内部加速 | 1.5x | multiprocessing 1.02x | — |

PyMuPDF 的 multiprocessing.Pool 几乎无加速（1.02x）——fork + pickle 序列化开销抵消了并行收益。ritz 的 rayon 无 IPC 开销，实际并行收益 1.5x（受限于小文档负载不足）。
