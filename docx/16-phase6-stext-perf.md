# Phase 6：文本提取热路径性能优化（stext 缓存 + buffer 借用 + UTF-8 跳过）

## 总览

Corpus benchmark（2907 PDFs）显示 ritz 在小 PDF 上比 PyMuPDF 慢 ~50%（1.0ms vs 0.6ms / doc）。
两个库共享同一 MuPDF 1.27 stext 引擎，差距来自 ritz 多出的 dispatch 层
（PyO3 → Rust → C wrapper → MuPDF）。

本次定位 3 个具体开销点并实现优化：

| # | 优化 | 文件 | 单次调用收益 | 多模式同页收益 |
|---|------|------|------|------|
| 1 | PyPage 缓存 `fz_stext_page` | `crates/ritz/src/page.rs` | 0（首次必建） | **关键**（70x+） |
| 2 | `fz_buffer_storage` 借用替代 `fz_buffer_extract` malloc+memcpy | `crates/mupdf-sys/native/error_wrapper.c` | ~200ns | ~200ns × 模式数 |
| 3 | release build 跳过 UTF-8 校验 | `crates/ritz/src/page.rs::slice_to_pystring` | ~50–100ns | 同上 |

**实测**（sample.pdf，多模式同页 4 次/轮 × 200 轮）：
- ritz（cache）：**0.009 ms/round**
- PyMuPDF（无 cache）：0.675 ms/round
- **加速 71.9x**

（单模式 text 场景 169x，因为 PyMuPDF 也每次重建 stext，cache 让 ritz 几乎只剩 PyString 构造。）

## stext 缓存设计

### 字段
```rust
pub struct PyPage {
    // ...
    rect_cache: OnceLock<(f32, f32, f32, f32)>,    // 旧
    stext_cache: std::sync::Mutex<Option<*mut fz_stext_page>>,  // 新
}
```

为什么用 `Mutex<Option<T>>` 而不是 `OnceLock<T>`？
- `OnceLock::take()` 不存在（一旦 set 无法 unset）
- annot 写操作必须能 invalidate（stext 包含 annot 文本）
- Python 单线程，锁竞争为零，开销 ~10ns

### 辅助方法
```rust
fn cached_stext(&self) -> PyResult<*mut fz_stext_page> {
    let mut guard = self.stext_cache.lock().unwrap();
    if let Some(stpage) = *guard { return Ok(stpage); }
    let mut stpage = ptr::null_mut();
    let rc = unsafe { mupdf_safe_new_stext_page(self.ctx, self.raw, &mut stpage) };
    if rc != 0 || stpage.is_null() { return Err(PyDocument::last_error_pub()); }
    *guard = Some(stpage);
    Ok(stpage)
}

fn invalidate_stext(&self) {
    if let Some(stpage) = self.stext_cache.lock().unwrap().take() {
        unsafe { mupdf_safe_drop_stext_page(self.ctx, stpage); }
    }
}
```

### 调用方改造
- `get_text` / `search_for`：把 `new_stext_page + drop_stext_page` 换成 `cached_stext()?`
- 5 个 annot 写方法（add_highlight/underline/strikeout/text_annot + delete_annot）：开头加 `self.invalidate_stext()`
- `Drop`：先释放 cache（如果已构建），再 drop_page

## buffer 借用（fz_buffer_storage）

旧路径（每次 malloc + memcpy）：
```
MuPDF growbuf → fz_buffer_extract (malloc + memcpy) → PyString::new (拷贝)
```

新路径（0 拷贝到中间层）：
```
MuPDF growbuf → fz_buffer_storage (borrow ptr) → PyString::new (拷贝)
```

### C 层
```c
int mupdf_safe_stext_to_buffer_borrow(
    fz_context *ctx, fz_stext_page *stpage, int mode_id,
    const char **out_data, size_t *out_len, void **out_handle);

void mupdf_safe_release_buffer(fz_context *ctx, void *handle);
```

**生命周期不变量**：`*out_data` 仅在 `mupdf_safe_release_buffer(handle)` 之前有效。
release 后访问 = UAF。

mode_id：0=text, 1=html, 2=xhtml, 3=xml。复用现有 `stext_print_fn` 表。

### fz_buffer_data vs fz_buffer_storage

MuPDF 1.27 的 buffer.h 没有 `fz_buffer_data`，对应函数是 `fz_buffer_storage`。
签名一致：`size_t fz_buffer_storage(ctx, buf, &datap)`。datap 借用 buf 内部 storage。

踩坑：第一次写代码用了 `fz_buffer_data`（来自网上更老/更新版本的 API），编译报
`undeclared function`。grep `vendor/mupdf/include/` 确认 1.27 实际函数名。

### Rust 调用方
```rust
let mut data: *const c_char = ptr::null();
let mut handle: *mut c_void = ptr::null_mut();
let rc = unsafe { mupdf_safe_stext_to_buffer_borrow(
    self.ctx, stpage, mode_id, &mut data, &mut len, &mut handle) };
if rc != 0 { return Err(...); }
let py_str = unsafe { cbuf_borrowed_to_pystring(py, data, len)? };
unsafe { mupdf_safe_release_buffer(self.ctx, handle); }
```

`cbuf_borrowed_to_pystring` 与 `cbuf_to_pystring` 共享 `slice_to_pystring`，区别是不调 `mupdf_free`。

## release build 跳过 UTF-8 校验

```rust
fn slice_to_pystring<'py>(py: Python<'py>, slice: &[u8]) -> PyResult<Bound<'py, PyString>> {
    #[cfg(not(debug_assertions))]
    {
        // SAFETY: MuPDF fz_print_stext_page_* 输出保证 UTF-8
        let s = unsafe { std::str::from_utf8_unchecked(slice) };
        Ok(PyString::new(py, s))
    }
    #[cfg(debug_assertions)]
    {
        // debug 保留校验 + lossy fallback，便于 corrupt PDF 触发 bug 时定位
        match std::str::from_utf8(slice) { /* ... */ }
    }
}
```

**安全论证**：MuPDF stext device 在写入 fz_stext_line/span 时已用 UTF-8 编码（PyMuPDF 等价假设相同）。
若 MuPDF bug 产生非 UTF-8，release 会构造非法 &str（违反 Rust 不变量，PyString::new 可能 panic 或产生乱码字符串）。
debug build 仍校验，便于发现上游 bug。

## 测试覆盖

新增 `TestStextCache` 类（`python/tests/test_ritz.py`）：

1. `test_repeated_get_text_consistent` — 同模式多次结果一致
2. `test_multi_mode_share_stext` — text/words/blocks/dict 共享 cache
3. `test_search_shares_stext_with_get_text` — search 复用 cache
4. `test_html_mode_works_via_cache` — html/xhtml/xml 走 borrow 路径
5. `test_annot_invalidates_cache` — add_highlight 后 cache 失效
6. `test_delete_annot_invalidates_cache` — delete_annot 也 invalidate
7. `test_different_pages_have_independent_cache` — 不同 PyPage 实例独立

总计 82 个 pytest 全过（原 75 + 新 7）。

## 性能数据

### Corpus benchmark（单文档单次调用，cache 不触发）
| Library | Mean | p50 | p95 | p99 | Pass |
|---------|------|-----|-----|-----|------|
| ritz | 1.0ms | 0.5ms | 3.3ms | 4.3ms | 100% |
| ritz_batch | 0.9ms | 0.3ms | 2.7ms | 3.6ms | 100% |
| pymupdf | 0.7ms | 0.3ms | 2.1ms | 3.4ms | 100% |

**结论**：单次调用场景改动 2+3 节省的 ~200ns 被 MuPDF 内部计算 + 统计噪声盖住。
单文档场景的瓶颈在 fz_new_stext_page_from_page（200-500μs MuPDF 内部），ritz 这层无法优化。

### Multimode benchmark（同页 4 次提取 × 200 轮）
| Library | Mean per round | 加速 |
|---------|----------------|------|
| ritz（cache） | **0.009 ms** | **71.9x** vs PyMuPDF |
| pymupdf | 0.675 ms | baseline |

**结论**：真实多模式场景（用户最常见用法），cache 让 ritz 跳过 3/4 的 stext 重建，加速 70x+。
单模式 text 场景更夸张（169x），因为 PyMuPDF 也每次重建。

### Leak test
- 1000 页峰值 RSS：72.9 MB（plan limit 200 MB）
- page-ops leak slope：2.4 KB/iter（limit 100 KB/iter）
- 全 PASS，证明 Drop 正确释放 stext_cache

## 踩坑记录

1. **`Mutex<Option<T>>::take()` 返回 `Option<T>` 不是 `Option<Option<T>>`**
   写成 `if let Some(Some(stpage)) = ...take()` 编译错。改成 `if let Some(stpage)` 即可。
   原因：`Option::take` 已经把内层 Option 折叠掉了。

2. **MuPDF 1.27 没有 `fz_buffer_data`**
   网上搜到的某些版本叫 `fz_buffer_data`，1.27 实际叫 `fz_buffer_storage`，签名一致。
   编译报 `undeclared function`，grep `vendor/mupdf/include/mupdf/fitz/buffer.h` 定位真名。

3. **`OnceLock` 无法 invalidate**
   原计划用 `OnceLock<*mut fz_stext_page>`，但 annot 写操作要能丢缓存。
   OnceLock 没 take()，只能用 `Mutex<Option<T>>`。

4. **stext 包含 annotation 文本**
   `fz_new_stext_page_from_page` 把 page（含 annot 图形）渲染到 stext device。
   如果不 invalidate，add_highlight 后 get_text 看不到新 annot 文本。所有 annot 写方法开头强制 invalidate。

## 缓存与 PyMuPDF 行为差异

PyMuPDF 的 `Page.get_text()` 每次**重建** TextPage（C++ 内部），不缓存。
所以同一 page 多次调 get_text 在 PyMuPDF 是 N 次 stext 重建成本。

ritz Phase 6 引入 cache 后：
- **优势**：同页多次提取（多模式 / search + get_text / 循环统计）显著快
- **行为差异**：annot 写操作后，新 annot 文本不会出现在下一次 get_text 中（因为 cache 被丢弃后重建——实际上会反映，因为我们 invalidate 后下次重建）
- 实际上：**行为一致**。invalidate 后下次 get_text 重建 stext，包含 annot 文本。和 PyMuPDF 重建立场相同。

## 相关文件

| 文件 | 改动 |
|------|------|
| `crates/ritz/src/page.rs` | +stext_cache 字段；+cached_stext/invalidate_stext；改 get_text/search_for；annot 方法 invalidate；Drop 释放；改 stext_to_string；+slice_to_pystring / cbuf_borrowed_to_pystring；改 cbuf_to_pystring 加 cfg |
| `crates/mupdf-sys/native/error_wrapper.c` | +mupdf_safe_stext_to_buffer_borrow；+mupdf_safe_release_buffer（~60 行）|
| `crates/mupdf-sys/native/error_wrapper.h` | +2 条声明 |
| `crates/mupdf-sys/bindings.rs` | +2 个 extern "C" 块 |
| `python/tests/test_ritz.py` | +TestStextCache 类（7 用例）|
| `benchmarks/bench_multimode.py` | 新建：多模式同页对比脚本 |
