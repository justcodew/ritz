# 实施计划 v2：高性能 PDF 处理库（Rust + MuPDF）

> 基于 [plan_v1.md](./01-plan-v1.md)，结合 LiteParse / gomupdf / PyMuPDF 源码深挖结果细化。
> 本文档为可执行工程方案，含 crate 结构、关键代码骨架、工程量估算、分阶段交付。

---

## 0. 范围确认与工程量实话

### 已确认的技术决策

| 决策项 | 选择 | 影响 |
|---|---|---|
| MuPDF 引入 | **Git 子模块编译源码** | 完全可控、可魔改；CI 编译复杂、需处理第三方依赖 |
| API 兼容范围 | **完整集**（text/words/blocks/dict/rawdict/json/html/xml/xhtml） | `get_text("dict")` 嵌套结构（block→line→span→char）工作量大 |
| 批量扩展 | **MVP 即含** | 新增 `mupdf_extensions.c` 实现多页批量提取 |

### 工程量实话（单人全职）

plan_v1 的"14 天"在上述完整范围下不现实。基于参考项目代码量推算：

| 参考依据 | 规模 |
|---|---|
| LiteParse（PDFium 后端，4 层架构） | 约 1.5 万行 Rust |
| gomupdf C 错误包装层 | `bindings.c` 单文件 2384 行 |
| 本项目 MVP 估算 | **8000–12000 行**（Rust + C + Python） |

**建议：分 4 个阶段，每阶段产出可用版本，总计 5–7 周。** 14 天可达成阶段一+二（最小可用包），完整范围需顺延。

---

## 1. 仓库结构

```
pdf-rs/                              # 仓库根
├── .gitmodules                      # MuPDF 子模块
├── Cargo.toml                       # workspace 根
├── crates/
│   ├── mupdf-sys/                   # 第 1 层：FFI 绑定
│   │   ├── Cargo.toml
│   │   ├── build.rs                 # 编译 MuPDF + error_wrapper.c + bindgen
│   │   ├── wrapper.h                # bindgen 入口（include fitz.h/pdf.h）
│   │   ├── src/
│   │   │   └── lib.rs               # include! 生成的 bindings.rs
│   │   └── native/                  # 手写 C 层
│   │       ├── error_wrapper.c      # fz_try/fz_catch 包装（核心）
│   │       ├── error_wrapper.h
│   │       ├── mupdf_extensions.c   # 批量扩展函数（mupdf_ext_ 前缀）
│   │       └── mupdf_extensions.h
│   ├── mupdf/                       # 第 2 层：安全封装
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs               # ffi! 宏、prelude
│   │       ├── context.rs           # fz_context RAII + Clone
│   │       ├── document.rs          # Document<'ctx>
│   │       ├── page.rs              # Page<'doc,'ctx>
│   │       ├── stext.rs             # STextPage、文本遍历
│   │       ├── pixmap.rs            # Pixmap、像素访问
│   │       ├── image.rs             # 嵌入图片提取
│   │       ├── link.rs              # 链接遍历
│   │       ├── geometry.rs          # Rect/Matrix/Point/Quad（#[repr(C)]）
│   │       └── error.rs             # MuPdfError + from_last_error()
│   ├── ritz/                        # 第 3+4 层：高层 API + PyO3
│   │   ├── Cargo.toml               # crate-type = ["cdylib"], lib.name = "_ritz"
│   │   ├── build.rs                 # rpath 设置（@loader_path / $ORIGIN）
│   │   └── src/
│   │       ├── lib.rs               # #[pymodule] ritz
│   │       ├── model.rs             # Rennie enum 数据模型（PdfValue 等）
│   │       ├── text.rs              # get_text 各模式实现
│   │       ├── py_document.rs       # #[pyclass] Document
│   │       ├── py_page.rs           # #[pyclass] Page
│   │       ├── py_pixmap.rs         # #[pyclass] Pixmap
│   │       └── py_types.rs          # Rect/Matrix/Point Python 类型
│   └── ritz-python/                 # Python 包（纯 Python 胶水）
│       ├── pyproject.toml
│       └── ritz/
│           └── __init__.py          # 从 _ritz 重导出 + 兼容层
├── patches/                         # MuPDF 补丁（git format-patch）
│   └── 0001-no-default-deps.patch
├── tests/                           # 集成测试 + 测试 PDF
│   ├── rust/                        # Rust 单元/集成测试
│   ├── conformance/                 # PyMuPDF 用例移植
│   └── bench/                       # 性能基准（criterion）
└── docs/
```

### workspace Cargo.toml

```toml
[workspace]
resolver = "2"
members = ["crates/mupdf-sys", "crates/mupdf", "crates/ritz"]

[workspace.package]
edition = "2021"
license = "AGPL-3.0"
```

---

## 2. 第 1 层：`mupdf-sys`（FFI 绑定 + C 错误包装）

### 2.1 核心问题：longjmp 隔离

MuPDF 用 `fz_try`/`fz_catch`（底层 `setjmp`/`longjmp`）做异常。**longjmp 穿过 Rust 栈帧会导致 Drop 不执行、引发 UB 和内存泄漏。** 所有暴露给 Rust 的 MuPDF 函数必须先用 C 包一层，在 C 层捕获异常转为错误码。

gomupdf 的 `bindings.c`（2384 行）已验证此模式可行，本项目直接借鉴。

### 2.2 `error_wrapper.c` 关键骨架

```c
#include <mupdf/fitz.h>
#include <mupdf/pdf.h>
#include <string.h>

/* 线程局部错误缓冲区：__thread 保证多线程下各自独立 */
static __thread char last_error[512] = {0};

const char *mupdf_last_error(void) { return last_error; }
void mupdf_clear_error(void) { last_error[0] = '\0'; }

/* 静默警告，避免污染 stderr */
static void warning_cb(void *user, const char *msg) {}

/* ---- 上下文 ---- */
fz_context *mupdf_safe_new_context(void) {
    mupdf_clear_error();
    fz_context *ctx = fz_new_context(NULL, NULL, FZ_STORE_DEFAULT);
    if (!ctx) { strncpy(last_error, "alloc context failed", sizeof last_error); return NULL; }
    fz_register_document_handlers(ctx);
    fz_set_warning_callback(ctx, warning_cb, NULL);
    return ctx;
}

void mupdf_safe_drop_context(fz_context *ctx) {
    if (ctx) fz_drop_context(ctx);
}

/* 为多文档并行：克隆上下文（独立锁状态） */
fz_context *mupdf_safe_clone_context(fz_context *ctx) {
    mupdf_clear_error();
    fz_context *clone = NULL;
    fz_try(ctx) { clone = fz_clone_context(ctx); }
    fz_catch(ctx) {
        snprintf(last_error, sizeof last_error, "clone ctx: %s", fz_caught_message(ctx));
        clone = NULL;
    }
    if (clone) fz_set_warning_callback(clone, warning_cb, NULL);
    return clone;
}

/* ---- 文档：返回 NULL 表示失败，查 last_error ---- */
fz_document *mupdf_safe_open_document(fz_context *ctx, const char *filename) {
    fz_document *doc = NULL;
    mupdf_clear_error();
    fz_try(ctx) { doc = fz_open_document(ctx, filename); }
    fz_catch(ctx) {
        snprintf(last_error, sizeof last_error, "open: %s", fz_caught_message(ctx));
        doc = NULL;
    }
    return doc;
}

/* 从内存字节打开（支持 stream=bytes 用法） */
int mupdf_safe_open_document_with_stream(
    fz_context *ctx, const unsigned char *data, size_t len,
    const char *mimetype, fz_document **out) {
    if (!ctx || !data || !out) return -1;
    *out = NULL;
    mupdf_clear_error();
    fz_stream *stm = NULL;
    fz_try(ctx) {
        stm = fz_open_memory(ctx, data, len);
        *out = fz_open_document_with_stream(ctx, mimetype, stm);
    }
    fz_always(ctx) { fz_drop_stream(ctx, stm); }   /* doc 持有引用，这里 drop stream */
    fz_catch(ctx) {
        snprintf(last_error, sizeof last_error, "open stream: %s", fz_caught_message(ctx));
        return -1;
    }
    return 0;
}

/* ---- 页数（返回 -1 表示错误） ---- */
int mupdf_safe_count_pages(fz_context *ctx, fz_document *doc) {
    int n = -1;
    mupdf_clear_error();
    fz_try(ctx) { n = fz_count_pages(ctx, doc); }
    fz_catch(ctx) { n = -1; }
    return n;
}

/* ---- 加载页：返回 NULL + last_error ---- */
fz_page *mupdf_safe_load_page(fz_context *ctx, fz_document *doc, int number) {
    fz_page *page = NULL;
    mupdf_clear_error();
    fz_try(ctx) { page = fz_load_page(ctx, doc, number); }
    fz_catch(ctx) {
        snprintf(last_error, sizeof last_error, "load page %d: %s", number, fz_caught_message(ctx));
        page = NULL;
    }
    return page;
}

/* ---- 页面尺寸：按值返回 fz_rect（失败返回空 rect） ---- */
fz_rect mupdf_safe_bound_page(fz_context *ctx, fz_page *page) {
    fz_rect r = {0,0,0,0};
    fz_try(ctx) { r = fz_bound_page(ctx, page); }
    fz_catch(ctx) {}
    return r;
}

/* ---- 结构化文本：输出到 out 指针 ---- */
int mupdf_safe_new_stext_page(fz_context *ctx, fz_page *page, fz_stext_page **out) {
    *out = NULL;
    mupdf_clear_error();
    fz_try(ctx) { *out = fz_new_stext_page_from_page_with_cookie(ctx, page, NULL, NULL); }
    fz_catch(ctx) {
        snprintf(last_error, sizeof last_error, "stext: %s", fz_caught_message(ctx));
        return -1;
    }
    return 0;
}

/* ---- 像素图渲染 ---- */
int mupdf_safe_render_pixmap(
    fz_context *ctx, fz_page *page,
    float scale_x, float scale_y, int alpha,
    fz_pixmap **out) {
    *out = NULL;
    mupdf_clear_error();
    fz_matrix ctm = {0}; fz_pixmap *pix = NULL; fz_device *dev = NULL;
    fz_try(ctx) {
        fz_rect bounds = fz_bound_page(ctx, page);
        ctm = fz_scale(scale_x, scale_y);
        fz_irect ibox = fz_round_rect(fz_transform_rect(bounds, ctm));
        pix = fz_new_pixmap(ctx, fz_device_rgb(ctx), ibox.x1-ibox.x0, ibox.y1-ibox.y0, NULL, alpha);
        fz_clear_pixmap_with_value(ctx, pix, 0xFF);
        dev = fz_new_draw_device(ctx, ctm, pix);
        fz_run_page(ctx, page, dev, ctm, NULL);
        fz_close_device(ctx, dev);
    }
    fz_always(ctx) { fz_drop_device(ctx, dev); }
    fz_catch(ctx) {
        if (pix) { fz_drop_pixmap(ctx, pix); pix = NULL; }
        snprintf(last_error, sizeof last_error, "render: %s", fz_caught_message(ctx));
        return -1;
    }
    *out = pix;
    return 0;
}

/* ---- 资源释放（无异常，静默忽略错误） ---- */
void mupdf_safe_drop_document(fz_context *ctx, fz_document *doc) { if (doc) fz_drop_document(ctx, doc); }
void mupdf_safe_drop_page(fz_context *ctx, fz_page *page) { if (page) fz_drop_page(ctx, page); }
void mupdf_safe_drop_stext_page(fz_context *ctx, fz_stext_page *p) { if (p) fz_drop_stext_page(ctx, p); }
void mupdf_safe_drop_pixmap(fz_context *ctx, fz_pixmap *p) { if (p) fz_drop_pixmap(ctx, p); }
```

**关键模式（来自 gomupdf 验证）：**
1. 函数前调用 `mupdf_clear_error()`
2. `fz_try` 内做实际工作
3. `fz_always`（可选）清理中间资源，**无论是否异常都执行**
4. `fz_catch` 内：`snprintf` 写错误消息，返回 NULL/-1/空结构
5. 返回值约定：指针型→NULL、int 型→-1、结构体型→零值

### 2.3 `mupdf_extensions.c`（批量扩展）

MVP 批量提取：一次 C 调用遍历多页 stext，输出扁平化数组，减少 Rust↔C 往返。

```c
/* 扁平化文本块：避免 Rust 侧逐块 FFI 调用 */
typedef struct {
    int block_type;     /* 0=text, 1=image */
    float bbox[4];
    int line_count;
    int char_count;     /* 文本块总字符数（UTF-8 字节，含结尾 0） */
    /* 后跟 line_offsets[line_count] 和 utf8_text[char_count] 连续布局 */
} mupdf_flat_block;

/* 一次提取 [start, end) 页的扁平文本块数组 */
int mupdf_ext_extract_pages_text(
    fz_context *ctx, fz_document *doc,
    int start_page, int end_page,
    mupdf_flat_block **out_blocks, int *out_block_count,
    char **out_text_pool, size_t *out_text_len);
```

实现遍历 `fz_stext_page` 的 block→line→char 链表（gomupdf `bindings.c:405-439` 有遍历参考），一次性 `malloc` 连续内存，Rust 侧拷贝进 `Vec` 后 `free`。

### 2.4 `build.rs` 关键逻辑

```rust
fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mupdf_dir = format!("{}/../../vendor/mupdf", manifest_dir);

    // 1. 应用补丁（可选）
    apply_patches(&mupdf_dir);

    // 2. 编译 MuPDF 静态库（第三方依赖 freetype/harfbuzz 等由 MuPDF Makefile 处理）
    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "4".into());
    run(&format!("make -C {} -j{} build=release HAVE_GLUT=no HAVE_X11=no HAVE_CURL=no", mupdf_dir, jobs));

    // 3. 链接 build/libs/libmupdf.a 及其第三方静态库
    println!("cargo:rustc-link-search=native={}/build/release", mupdf_dir);
    println!("cargo:rustc-link-lib=static=mupdf");
    println!("cargo:rustc-link-lib=static=mupdf-third");  // 第三方聚合库
    // 系统库（Linux/macOS）
    if target_os == "linux" { /* -lpthread -ldl -lm */ }
    if target_os == "macos" { /* -liconv -framework Cocoa（可选） */ }

    // 4. 编译手写 C 层
    cc::Build::new()
        .file("native/error_wrapper.c")
        .file("native/mupdf_extensions.c")
        .include(format!("{}/include", mupdf_dir))
        .compile("mupdf_wrapper");

    // 5. bindgen 生成绑定（含 fallback 预生成版本）
    #[cfg(feature = "bindgen")]
    generate_bindings(&mupdf_dir);
    #[cfg(not(feature = "bindgen"))]
    copy_pregenerated();
}
```

**第三方依赖处理**：MuPDF 自带 `thirdparty/`（freetype、harfbuzz、libjpeg、openjpeg、zlib、gumbo 等），`make` 会一并编译进 `libmupdf-third.a`。需通过 `Makerules` 或 `build=release` 控制平台特定选项。

### 2.5 Rust 侧 `lib.rs`

```rust
#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
```

`bindings.rs` 包含 `#[repr(C)]` 的 `fz_context`/`fz_document`/`fz_page`/`fz_stext_page`/`fz_rect`/`fz_matrix`/`fz_pixmap` 等不透明/值类型，以及所有 `mupdf_safe_*` 和 `mupdf_ext_*` 函数声明。

**注意**：`fz_stext_page` 内部是链表（block→line→char），Rust 侧**不直接遍历这些链表**——遍历逻辑放在 C 层（`error_wrapper.c` 或 `extensions.c`），Rust 只拿扁平化结果。这是 longjmp 安全的关键。

---

## 3. 第 2 层：`mupdf`（安全封装）

### 3.1 设计原则（借鉴 LiteParse）

- **生命周期标记句柄**：`Document<'ctx>` 借用 `Context`，编译期保证不"释放后使用"
- **RAII**：所有句柄实现 `Drop`，调用对应 `mupdf_safe_drop_*`
- **错误转换**：`MuPdfError::from_last_error()` 读取 C 层 `last_error`
- **坐标系转换**：集中在 `Page` 方法里做 `y_new = page_height - y_old`
- **不暴露任何 unsafe 接口**

### 3.2 `context.rs`

```rust
use mupdf_sys::*;

pub struct Context {
    raw: *mut fz_context,
}

impl Context {
    pub fn new() -> Result<Self, MuPdfError> {
        let raw = unsafe { mupdf_safe_new_context() };
        if raw.is_null() { return Err(MuPdfError::from_last_error()); }
        Ok(Context { raw })
    }
    /// 用于多文档并行：每个线程持有一个克隆
    pub fn clone_for_thread(&self) -> Result<Context, MuPdfError> {
        let raw = unsafe { mupdf_safe_clone_context(self.raw) };
        if raw.is_null() { return Err(MuPdfError::from_last_error()); }
        Ok(Context { raw })
    }
    pub(crate) fn as_ptr(&self) -> *mut fz_context { self.raw }
}

impl Drop for Context {
    fn drop(&mut self) { unsafe { mupdf_safe_drop_context(self.raw); } }
}
unsafe impl Send for Context {}   // 单个 context 非线程安全，但通过 Clone-per-thread 模式保证
unsafe impl !Sync for Context {}
```

### 3.3 `document.rs`（生命周期借用模式）

```rust
pub struct Document<'ctx> {
    raw: *mut fz_document,
    _ctx: PhantomData<&'ctx Context>,
}

impl<'ctx> Document<'ctx> {
    pub fn open(ctx: &'ctx Context, path: &str) -> Result<Self, MuPdfError> {
        let c_path = std::ffi::CString::new(path).unwrap();
        let raw = unsafe { mupdf_safe_open_document(ctx.as_ptr(), c_path.as_ptr()) };
        if raw.is_null() { return Err(MuPdfError::from_last_error()); }
        Ok(Document { raw, _ctx: PhantomData })
    }
    pub fn open_from_bytes(ctx: &'ctx Context, data: &[u8], mime: &str) -> Result<Self, MuPdfError> { /* ... */ }
    pub fn page_count(&self) -> Result<usize, MuPdfError> {
        let n = unsafe { mupdf_safe_count_pages(self.ctx_ptr(), self.raw) };
        if n < 0 { return Err(MuPdfError::from_last_error()); }
        Ok(n as usize)
    }
    pub fn page(&self, idx: usize) -> Result<Page<'_, 'ctx>, MuPdfError> { /* ... */ }
    pub fn metadata(&self, key: &str) -> Option<String> { /* fz_lookup_metadata */ }
    fn ctx_ptr(&self) -> *mut fz_context { /* 从 PhantomData 反推或存一份 */ }
}

impl Drop for Document<'_> {
    fn drop(&mut self) { unsafe { mupdf_safe_drop_document(self.ctx_ptr(), self.raw); } }
}
```

### 3.4 `error.rs`

```rust
#[derive(Debug)]
pub enum MuPdfError {
    OpenDocument(String),
    LoadPage(String),
    Render(String),
    Other(String),
}

impl MuPdfError {
    pub fn from_last_error() -> Self {
        let msg = unsafe {
            let p = mupdf_last_error();
            if p.is_null() { return MuPdfError::Other("unknown".into()); }
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        };
        MuPdfError::Other(msg)
    }
}
```

### 3.5 `geometry.rs`（`#[repr(C)]` 值类型）

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect { pub x0: f32, pub y0: f32, pub x1: f32, pub y1: f32 }

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Matrix { pub a: f32, pub b: f32, pub c: f32, pub d: f32, pub e: f32, pub f: f32 }

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Point { pub x: f32, pub y: f32 }
```

这些与 MuPDF 的 `fz_rect`/`fz_matrix`/`fz_point` 二进制兼容，可直接传 FFI。

---

## 4. 第 3+4 层：`ritz`（高层 API + PyO3）

### 4.1 Rennie 数据模型（`model.rs`）

核心：用 enum 替代指针链，紧凑内存布局。

```rust
/// PyMuPDF get_text("dict") 的 Rust 原生表示
pub struct TextPageData {
    pub width: f32,
    pub height: f32,
    pub blocks: Vec<TextBlock>,
}

pub enum TextBlock {
    Text {
        number: usize,
        bbox: Rect,
        lines: Vec<TextLine>,
    },
    Image {
        number: usize,
        bbox: Rect,
        width: u32,
        height: u32,
        ext: String,
        transform: Matrix,
        image: Vec<u8>,     // include_data=True 时填充
    },
}

pub struct TextLine {
    pub wmode: i32,
    pub dir: (f32, f32),
    pub bbox: Rect,
    pub spans: Vec<TextSpan>,
}

pub struct TextSpan {
    pub size: f32,
    pub flags: i32,
    pub font: String,
    pub color: i32,         // sRGB int
    pub origin: (f32, f32),
    pub bbox: Rect,
    pub text: String,
}

/// PDF 对象模型（供未来 pdf 对象操作）
pub enum PdfValue {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    String(String),
    Name(String),
    Array(Vec<PdfValue>),
    Dict(std::collections::HashMap<String, PdfValue>),
    Stream { dict: Box<PdfValue>, data: Vec<u8> },
}
```

`From<mupdf 的 C 句柄>` trait 实现一次性拷贝转换。**Rennie 的论点：拷贝比指针链更快**，因为栈上连续数据缓存命中率高。

### 4.2 PyO3 类（`py_document.rs`）

```rust
use pyo3::prelude::*;

#[pyclass(name = "Document")]
pub struct PyDocument {
    ctx: std::panic::AssertUnwindSafe<mupdf::Context>,
    // Document 借用 ctx，用生命周期擦除（'static 容器）或 Rc
    doc: Option<mupdf::Document<'static>>,   // 实际需用自引用或 Rc<Context>
}

#[pymethods]
impl PyDocument {
    #[new]
    #[pyo3(signature = (filename=None, stream=None, filetype=None))]
    fn new(filename: Option<&str>, stream: Option<&[u8]>, filetype: Option<&str>) -> PyResult<Self> {
        let ctx = mupdf::Context::new().map_err(pyerr)?;
        let doc = match (filename, stream) {
            (Some(path), _) => mupdf::Document::open(&ctx, path),
            (None, Some(data)) => mupdf::Document::open_from_bytes(&ctx, data, filetype.unwrap_or("pdf")),
            _ => return Err(PyTypeError::new_err("need filename or stream")),
        }.map_err(pyerr)?;
        Ok(PyDocument { ctx: AssertUnwindSafe(ctx), doc: Some(/* transmute 到 'static 或重设计 */) })
    }

    #[getter]
    fn page_count(&self) -> PyResult<usize> { self.doc.as_ref().unwrap().page_count().map_err(pyerr) }

    #[getter]
    fn metadata(&self) -> PyResult<PyObject> { /* 返回 dict */ }

    fn load_page(&self, idx: usize) -> PyResult<PyPage> { /* ... */ }
    fn __getitem__(&self, idx: &PyAny) -> PyResult<PyObject> { /* 支持 int 和 slice */ }

    fn close(&mut self) { self.doc = None; }
}

impl Drop for PyDocument {
    fn drop(&mut self) { self.doc = None; /* ctx 在 struct 销毁时自动 drop */ }
}
```

**生命周期擦除**：`Document<'ctx>` 借用 `Context` 在 PyO3 里麻烦。两种方案：
- **方案 A（推荐）**：把 `Context` 装进 `Rc<RefCell<Context>>`，`Document` 持有 `Rc` 克隆，避免生命周期参数
- **方案 B**：自引用结构，需 `ouroboros` crate

### 4.3 `get_text` 各模式（`text.rs`）

| mode | Rust 实现 | 返回 Python 类型 |
|---|---|---|
| `"text"` | 遍历 stext 拼字符串 | `str` |
| `"blocks"` | 扁平化 block 列表 | `list[tuple(x0,y0,x1,y1,text,no,type)]` |
| `"words"` | stext 按空格切词 | `list[tuple(x0,y0,x1,y1,word,blk,line,word_no)]` |
| `"dict"` | 构造 `TextPageData` 再转 Python dict | `dict{width,height,blocks:[...]}` |
| `"rawdict"` | dict + 每字符 `chars` 字段 | 同 dict，span 含 chars |
| `"json"`/`"rawjson"` | dict 结构序列化 | `str`（JSON） |
| `"html"`/`"xhtml"`/`"xml"` | 调 `fz_print_stext_page_as_*` | `str` |

`"dict"` 的嵌套结构是工作量重点（block→line→span→char）。Rust 侧先建 `TextPageData`，再用 PyO3 转 `PyDict`/`PyList`。

### 4.4 图片一级提取（增强）

PyMuPDF 的 `get_images(full)` 返回元组列表，**不含 bbox/data**。本项目增强：

```python
page.get_images(full=False, include_data=False)
# 每项返回 dict（而非元组），含 bbox 和可选 data
{"xref": 123, "smask": 0, "width": 800, "height": 600,
 "bbox": [x0,y0,x1,y1], "ext": "jpeg", "data": b"..."}
```

实现：遍历 stext 的 image block（`block->type == FZ_STEXT_BLOCK_IMAGE`），取 `block->u.i.image`，调 `fz_get_pixmap_from_image` 或直接读 `fz_image` 的 `w/h/n/bpc`，坐标从 block bbox 取。

### 4.5 批量提取（`mupdf_ext_*` 接入）

```rust
#[pyfunction]
#[pyo3(signature = (doc, pages))]
fn get_text_batch(py: Python, doc: &PyDocument, pages: Vec<usize>) -> PyResult<PyObject> {
    let _ = py.detach(|| {  // 释放 GIL
        // 调 mupdf_ext_extract_pages_text 一次拿多页扁平数据
    });
    // 转成 {page_no: TextPageData} 的 Python dict
}
```

### 4.6 Python 包 `ritz/__init__.py`

```python
from ._ritz import *      # 从 cdylib 导入
from ._ritz import Document, Page, Pixmap, Rect, Matrix, Point

# 兼容性别名
open = Document

# 未实现 API：抛 NotImplementedError
def _not_implemented(name):
    def f(*a, **kw): raise NotImplementedError(f"{name} not yet supported")
    return f

# 显式列出未实现的高频 API，避免静默失败
add_redact_annot = _not_implemented("add_redact_annot")
# ...
```

---

## 5. 分阶段交付

### 阶段一：构建基础设施打通（1 周）

**目标**：MuPDF 子模块能编译，Rust 能 `open("x.pdf").page_count()`。

- [ ] 初始化 git 仓库，加 MuPDF 子模块
- [ ] 写 `build.rs` 让 `make` 编译 MuPDF，链接 `libmupdf.a`
- [ ] `error_wrapper.c` 实现：context / open / count_pages / load_page / bound_page / drop_* 
- [ ] bindgen 生成 `bindings.rs`
- [ ] `mupdf-sys` crate 编译通过
- [ ] `mupdf` crate：Context + Document + 基础方法
- [ ] Rust 集成测试：打开测试 PDF，断言 page_count 正确

**交付物**：`cargo test` 能打开 PDF 取页数。

### 阶段二：文本提取 + PyO3 最小可用（1.5 周）

**目标**：`pip install` 后能 `ritz.open("x.pdf")[0].get_text("text")`。

- [ ] `error_wrapper.c` 增加：stext 相关、渲染 pixmap
- [ ] `mupdf` crate：Page、STextPage、Pixmap、Rect/Matrix/Point
- [ ] `ritz` crate：PyO3 绑定 Document/Page/Pixmap
- [ ] 实现 `get_text("text")` 和 `get_text("blocks")`
- [ ] 实现 `get_pixmap()` + `Pixmap.tobytes("png")`
- [ ] `ritz/__init__.py` 重导出
- [ ] `pyproject.toml` + maturin 构建配置

**交付物**：可 `pip install`，核心文本提取 + 渲染可用。

### 阶段三：完整 API + Rennie 数据模型（1.5 周）

**目标**：`get_text` 全模式、`get_images` 含 bbox、`get_links`。

- [ ] `get_text("dict")` / `"rawdict"`：实现 TextPageData + 嵌套转换
- [ ] `get_text("words")` / `"json"` / `"rawjson"` / `"html"` / `"xhtml"` / `"xml"`
- [ ] 图片一级提取：`get_images(include_data=True)` 含 bbox + data
- [ ] `get_links()` 返回链接字典列表
- [ ] metadata、rotation、cropbox、mediabox 属性
- [ ] Rennie `PdfValue` enum（为后续对象操作铺路）

**交付物**：与 PyMuPDF 核心文本/图片/链接 API 兼容。

### 阶段四：批量扩展 + 并行 + 打磨（1 周）

**目标**：性能优势兑现。

- [ ] `mupdf_extensions.c`：`mupdf_ext_extract_pages_text` 批量扁平化
- [ ] `get_text_batch()` Python API
- [ ] rayon 多文档并行：`process_documents(paths: List[str])` 每文档独立 cloned context
- [ ] criterion 性能基准：对比 PyMuPDF，产出对比报告
- [ ] PyMuPDF 测试用例移植：目标通过率 ≥ 90%
- [ ] 文档：README + Jupyter 演示

**交付物**：性能基准报告 + 完整测试套件。

---

## 6. 关键技术风险与应对

| 风险 | 等级 | 应对 |
|---|---|---|
| MuPDF 子模块跨平台编译失败（Windows 尤甚） | 高 | 阶段一优先验证 Linux/macOS；Windows 可降级为预编译动态库方案（plan_v1 的备选） |
| `fz_stext_page` 链表遍历触发 longjmp | 高 | 遍历逻辑全部放 C 层；Rust 只接收扁平化结果。任何 Rust 侧的 stext 访问都视为危险 |
| 生命周期 `'ctx` 在 PyO3 难处理 | 中 | 用 `Rc<Context>` 方案，避免生命周期参数泄漏到 pyclass |
| MuPDF 版本升级破坏补丁 | 中 | 补丁最小化；`mupdf_ext_*` 放独立文件不改 MuPDF 源码；季度同步 |
| `get_text("dict")` 性能不如 PyMuPDF | 中 | 用 Rennie 扁平 Vec + 一次性 PyDict 构建，避免逐字段 FFI；criterion 验证 |
| bindgen 在 CI 无 libclang | 低 | 提交预生成 `bindings.rs`（LiteParse 模式），`bindgen` feature 可选 |

---

## 7. 测试策略

| 层级 | 工具 | 目标 |
|---|---|---|
| Rust 单元 | `#[test]` | 几何转换、数据模型转换 |
| Rust 集成 | `tests/` 目录 | open/count/text/pixmap 正确性 |
| Python 兼容性 | pytest + PyMuPDF 测试集移植 | 通过率 ≥ 90% |
| 性能 | criterion | 文本提取 ≥ 10x、批量 ≥ 2-3x vs PyMuPDF |
| 回归 | 固定测试 PDF 集 | 含加密、扫描、复杂排版样本 |

---

## 8. 交付验收清单

- [ ] `mupdf-sys` / `mupdf` / `ritz` 三 crate 编译通过
- [ ] `pip install ritz` 在 Linux + macOS 可用
- [ ] `import ritz; ritz.open("x.pdf")[0].get_text("dict")` 返回与 PyMuPDF 结构一致
- [ ] `get_images(include_data=True)` 返回含 bbox + data
- [ ] `get_text_batch()` 批量提取比逐页快 ≥ 2x
- [ ] 多文档 rayon 并行近线性加速
- [ ] Rust 单元测试覆盖率 ≥ 70%
- [ ] 性能基准达 [plan_v1 §4.1](./01-plan-v1.md) 目标（±10%）
- [ ] LICENSE 为 AGPL-3.0
- [ ] README 含安装、快速开始、API 差异、性能对比

---

## 9. 与 plan_v1 的差异说明

| plan_v1 说法 | plan_v2 调整 | 原因 |
|---|---|---|
| "14 天单人" | 改为 5-7 周分阶段 | 完整范围 + 子模块 + 批量扩展工作量实测更大 |
| 未提生命周期擦除 | 明确 Rc<Context> 方案 | PyO3 不便处理生命周期参数 |
| stext 遍历未细化 | 强制 C 层扁平化 | longjmp 安全的关键约束 |
| Windows 同等支持 | 降为兼容（预编译库） | 子模块 Windows 编译复杂度过高 |
| 补丁自动应用 | 补丁最小化，扩展放独立文件 | 降低上游同步成本 |

---

## 10. 立即可执行的下一步

1. `git init` 仓库，`git submodule add` MuPDF
2. 写 `crates/mupdf-sys/build.rs`，目标：`make` 编出 `libmupdf.a`
3. 写最小 `error_wrapper.c`（context + open + count + drop）
4. 写 `mupdf-sys/src/lib.rs` + `mupdf/src/lib.rs`，目标：`cargo test` 打开 PDF 取页数

完成这 4 步即阶段一达成，是最关键的去风险化里程碑。
