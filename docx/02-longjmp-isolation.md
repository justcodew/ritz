# longjmp 隔离：C 错误包装层

> 这是 ritz 安全性的头号设计。不理解这个，就不理解为什么不能直接在 Rust 里调 MuPDF。

## 问题：longjmp 穿透 Rust 栈

MuPDF 用 `fz_try` / `fz_catch` 做异常处理，底层是 C 的 `setjmp` / `longjmp`：

```c
fz_try(ctx) {
    doc = fz_open_document(ctx, path);  // 失败时 longjmp
}
fz_catch(ctx) {
    // 错误处理
}
```

如果 Rust 直接调 `fz_open_document`，异常发生时 `longjmp` 会**跳过 Rust 栈帧**：

```rust
// ❌ 危险！永远不要这样做
fn open(ctx: *mut fz_context, path: &str) -> *mut fz_document {
    let buffer = Box::new([0u8; 1024]);  // 堆分配
    let doc = unsafe { fz_open_document(ctx, path_c.as_ptr()) };
    // 如果上面 longjmp 了，这行永远不执行：
    drop(buffer);  // ← Box 的 Drop 被跳过！内存泄漏！
    doc
}
```

更严重的是：Rust 的 `Drop` trait 不只是释放内存。`MutexGuard`、`RwLockReadGuard`、文件句柄、临时文件……所有 RAII 资源都会泄漏。这是**未定义行为（UB）**，编译器可以假设它不发生。

## 解决方案：C 错误包装层

在 C 和 Rust 之间插入一层，在 C 侧捕获 longjmp，转为错误码：

```
Rust  ──extern "C"──►  error_wrapper.c  ──fz_try/fz_catch──►  MuPDF
                          ↑ longjmp 在这里被捕获
                          ↓ 返回错误码（int）
Rust  ◄──返回值────────  error_wrapper.c
```

### 实现模式

每个 MuPDF 函数都有一个对应的 `mupdf_safe_*` 包装：

```c
// error_wrapper.c
int mupdf_safe_open_document(fz_context *ctx, fz_document **doc, const char *path) {
    if (!ctx || !doc || !path) return -1;
    *doc = NULL;
    mupdf_clear_error();
    fz_try(ctx) {
        *doc = fz_open_document(ctx, path);
    }
    fz_catch(ctx) {
        set_error("open_document: %s", fz_caught_message(ctx));
        return fz_caught(ctx);   // 返回 MuPDF 错误码
    }
    return 0;                     // 0 = 成功
}
```

**Rust 侧只调 `mupdf_safe_*`**：

```rust
// ✅ 安全：longjmp 被 C 层捕获，不会穿透 Rust 栈
let mut doc: *mut fz_document = ptr::null_mut();
let rc = unsafe { mupdf_safe_open_document(ctx, &mut doc, path_c.as_ptr()) };
if rc != 0 {
    return Err(PyDocument::last_error_pub());
}
```

### 错误消息传递

C 层用线程局部存储（thread-local storage）保存错误消息：

```c
// error_wrapper.c
static __thread char error_buf[512];

void set_error(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(error_buf, sizeof(error_buf), fmt, ap);
    va_end(ap);
}

const char *mupdf_last_error(void) {
    return error_buf;
}
```

Rust 侧通过 `mupdf_last_error()` 取回错误消息，构造 Python 异常：

```rust
fn last_error_pub() -> PyErr {
    let msg = unsafe { CStr::from_ptr(mupdf_last_error()) }
        .to_string_lossy()
        .into_owned();
    PyRuntimeError::new_err(msg)
}
```

### fz_always 的作用：资源清理

有些 MuPDF 操作需要无论是否异常都清理中间资源。C 层用 `fz_always` 块：

```c
int mupdf_safe_get_images(...) {
    fz_display_list *list = NULL;
    image_capture_device *cap = NULL;

    fz_try(ctx) {
        list = fz_new_display_list_from_page(ctx, page);
        cap = fz_new_derived_device(ctx, image_capture_device);
        fz_run_display_list(ctx, list, (fz_device*)cap, ...);
    }
    fz_always(ctx) {
        // 无论是否异常，都清理 list 和 cap
        // 关键：在 drop_device 前把 growbuf 所有权转移出来
        if (cap) {
            gb_data = cap->gb.data;  // 抢救数据
            cap->gb.data = NULL;
            fz_drop_device(ctx, (fz_device*)cap);
        }
        if (list) fz_drop_display_list(ctx, list);
    }
    fz_catch(ctx) {
        set_error("get_images: %s", fz_caught_message(ctx));
        return -1;
    }
    // 成功路径：gb_data 已转移，可以返回
    *out = (char*)gb_data;
    return 0;
}
```

`fz_always` 等价于 try-finally，保证资源释放。这与 Rust 的 `Drop` 不同——`fz_always` 在 C 栈内执行，不受 longjmp 影响。

## 为什么不用 Rust 的 catch_unwind？

有人可能想：能不能用 `std::panic::catch_unwind` 捕获？

**不能**。`longjmp` 不是 Rust panic。它是 C 的 `setjmp`/`longjmp`，直接操作栈指针，不走 Rust 的 panic 机制。`catch_unwind` 只能捕获 Rust panic，对 longjmp 无效。

## 为什么不用信号处理或 setjmp 在 Rust 侧？

理论上可以在 Rust 里调 `setjmp` 然后 longjmp 回来。但：
1. `setjmp`/`longjmp` 在 Rust 中是 **unsafe** 且未定义行为（Rust 不保证栈帧布局与 C 兼容）
2. Rust 的 `Drop` 仍不会执行（longjmp 跳过的栈帧里可能有需要 Drop 的值）
3. 与 C 错误包装层相比，没有性能优势

C 错误包装层是 MuPDF Rust 绑定的行业标准做法（mupdf-rs 也用类似方案）。

## 所有 _safe 函数清单

`error_wrapper.h` 声明了 ~30 个 `mupdf_safe_*` 函数，覆盖 ritz 用到的所有 MuPDF 操作：

| 类别 | 函数 |
|------|------|
| 生命周期 | `new_context` / `drop_context` / `open_document` / `drop_document` / `load_page` / `drop_page` |
| 文本 | `new_stext_page` / `stext_to_text` / `stext_to_html` / `stext_to_dict` / `stext_to_words` / `stext_to_blocks` |
| 渲染 | `render_pixmap` / `pixmap_to_png` / `drop_pixmap` |
| 链接 | `load_links`（扁平化输出） |
| 图片 | `get_images`（device callback 扁平化） |
| 元数据 | `lookup_metadata` / `page_rotation` |
| 批量扩展 | `mupdf_ext_extract_pages_text`（见 mupdf_extensions.c） |

**规则：ritz 永远不直接调 MuPDF 原始函数，只调 `mupdf_safe_*` / `mupdf_ext_*`。** 这条规则是 longjmp 隔离的保证。
