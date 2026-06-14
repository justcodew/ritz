# 四层架构总览

## 设计动机

PyMuPDF（fitz）通过 SWIG 生成 Python↔C 绑定，存在跨语言调用开销大、内存复制频繁、难以并行等问题。ritz 用 Rust 重写绑定层，借鉴 LiteParse 的分层设计，在保持 API 兼容的前提下实现性能跃升。

核心约束：MuPDF 是 C 库，大量使用 `setjmp`/`longjmp` 做异常处理。**longjmp 会跨过 Rust 栈帧，导致 `Drop` trait 不执行，引发资源泄漏和未定义行为（UB）。** 这是整个架构必须解决的头号问题。

## 四层职责

```
┌─────────────────────────────────────────────┐
│           Python 层（用户代码）              │
│   import ritz   # API 兼容 PyMuPDF          │
└─────────────────────────────────────────────┘
                       │ PyO3 绑定
┌─────────────────────────────────────────────┐
│        ritz crate（高层 Rust API + PyO3）    │
│  PyDocument / PyPage / PyPixmap             │
│  - 面向 Python 的对象模型                   │
│  - C buffer → Python dict/str/bytes 转换    │
│  - rayon 并行 / 批量扩展                    │
└─────────────────────────────────────────────┘
                       │
┌─────────────────────────────────────────────┐
│        mupdf crate（安全 RAII 封装）         │
│  Context / Document / Page / Pixmap         │
│  - RAII 管理所有 *mut fz_* 资源             │
│  - 错误码 → Rust Result                     │
│  - 坐标系转换                               │
└─────────────────────────────────────────────┘
                       │
┌─────────────────────────────────────────────┐
│      mupdf-sys crate（FFI + C 错误包装）     │
│  - extern "C" 声明（bindings.rs）           │
│  - error_wrapper.c：fz_try/fz_catch 隔离    │
│  - mupdf_extensions.c：批量扁平化扩展        │
│  - build.rs：编译 MuPDF 子模块 + 扩展        │
└─────────────────────────────────────────────┘
                       │
┌─────────────────────────────────────────────┐
│          MuPDF C 源码（Git 子模块）          │
│  vendor/mupdf/ （1.27.0，源码编译）          │
└─────────────────────────────────────────────┘
```

## 各层详解

### 第 1 层：mupdf-sys（FFI + C 错误包装）

**目录**：`crates/mupdf-sys/`

两个 C 文件构成核心：

| 文件 | 职责 |
|------|------|
| `native/error_wrapper.c` | 每个 MuPDF 调用包在 `fz_try`/`fz_catch` 里，输出 `mupdf_safe_*` 函数 |
| `native/mupdf_extensions.c` | MuPDF 不提供的批量 API（如 `mupdf_ext_extract_pages_text`） |

所有 `mupdf_safe_*` 函数的签名模式：

```c
// 返回 0 = 成功，非 0 = 错误码
// 输出参数通过 **out 传回，调用方负责 mupdf_free
int mupdf_safe_open_document(fz_context *ctx, fz_document **doc, const char *path);
```

**Rust 侧永远只调 `mupdf_safe_*`，永不直接调 MuPDF 原始函数**——这是 longjmp 隔离的保证。详见 [02-longjmp-isolation.md](02-longjmp-isolation.md)。

`build.rs` 编译流程：
1. `make` 编译 MuPDF 子模块 → `libmupdf.a`（~50MB 静态库）
2. `cc` 编译 `error_wrapper.c` + `mupdf_extensions.c` → 链接到 `libmupdf.a`
3. Rust crate 声明 `extern "C"` 调用这些函数

### 第 2 层：mupdf（安全 RAII 封装）

**目录**：`crates/mupdf/src/`

用 Rust RAII 包装裸指针：

```rust
pub struct Document {
    ctx: *mut fz_context,      // 借用 Context（不拥有）
    raw: *mut fz_document,      // 拥有，Drop 时调 fz_drop_document
}

impl Drop for Document {
    fn drop(&mut self) {
        unsafe { mupdf_safe_drop_document(self.ctx, self.raw); }
    }
}
```

这一层提供 Rust 惯用的 `Result<T, MuPdfError>` 返回，上层无需处理错误码。目前 ritz 的 PyO3 层（第 3 层）直接调 mupdf-sys 的 FFI，mupdf crate 主要用于 Rust 集成测试和 criterion 基准。

### 第 3 层：ritz（高层 API + PyO3）

**目录**：`crates/ritz/src/`

Python 面向的对象模型：

| 结构体 | Python 类 | 持有 |
|--------|----------|------|
| `PyDocument` | `ritz.Document` | `*mut fz_context` + `*mut fz_document` |
| `PyPage` | `ritz.Page` | `*mut fz_page` + `Py<PyDocument>`（引用计数防悬挂） |
| `PyPixmap` | `ritz.Pixmap` | `*mut fz_pixmap` + `Py<PyDocument>` |

关键设计：**Page 和 Pixmap 持有 `Py<PyDocument>` 句柄**，确保 Document 不会被 GC 先于 Page 释放（否则 ctx 悬挂 → use-after-free）。详见 [04-memory-safety.md](04-memory-safety.md)。

`lib.rs` 注册模块：

```rust
#[pymodule]
fn _ritz(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDocument>()?;
    m.add_class::<PyPage>()?;
    m.add_class::<PyPixmap>()?;
    m.add_function(wrap_pyfunction!(process_documents, m)?)?;
    Ok(())
}
```

### 第 4 层：Python re-export

**目录**：`python/ritz/__init__.py`

```python
from ._ritz import *  # 导入 PyO3 编译的 cdylib
```

纯薄层，让 `import ritz` 工作。所有实现在 Rust cdylib（`_ritz.so`）。

## 为什么是四层而不是三层？

LiteParse 是三层（FFI → 安全封装 → PyO3）。ritz 多了一层 **C 错误包装**，原因是：

> MuPDF 的 `fz_try`/`fz_catch` 是宏，底层是 `setjmp`/`longjmp`。如果在 Rust 中直接调 MuPDF 函数，异常发生时 longjmp 会跳过 Rust 栈帧，`Drop` 不执行 → 内存泄漏 → UB。

C 错误包装层把每个 MuPDF 调用包在 C 函数里，在 C 侧用 `fz_try`/`fz_catch` 捕获 longjmp，转为错误码返回。**longjmp 永远不穿透 Rust 栈。** 这是 ritz 安全性的基石。

详见下一章 [02-longjmp-isolation.md](02-longjmp-isolation.md)。
