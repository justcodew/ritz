/*
 * 手写的最小 FFI 绑定（阶段一）。
 *
 * 这是预生成绑定的初始版本，仅含阶段一所需类型和函数。
 * 后续可用 `cargo build -p mupdf-sys --features bindgen` 生成完整绑定替换此文件。
 *
 * 注：类型布局必须与 MuPDF C 头一致，用 #[repr(C)] 保证 ABI 兼容。
 */

/* ---- 值类型（与 MuPDF 二进制兼容） ---- */

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct fz_point {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct fz_rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct fz_matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct fz_irect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct fz_quad {
    pub ul: fz_point,
    pub ur: fz_point,
    pub ll: fz_point,
    pub lr: fz_point,
}

/* ---- 不透明类型（只通过指针使用） ---- */

#[repr(C)]
pub struct fz_context {
    _private: [u8; 0],
}

#[repr(C)]
pub struct fz_document {
    _private: [u8; 0],
}

#[repr(C)]
pub struct fz_page {
    _private: [u8; 0],
}

#[repr(C)]
pub struct fz_stream {
    _private: [u8; 0],
}

#[repr(C)]
pub struct fz_stext_page {
    _private: [u8; 0],
}

#[repr(C)]
pub struct fz_pixmap {
    _private: [u8; 0],
}

/* ---- 常量 ---- */

pub const FZ_STORE_DEFAULT: usize = 256 << 20; // 256 MB

/* ---- 错误包装层函数（来自 error_wrapper.h） ---- */

extern "C" {
    pub fn mupdf_last_error() -> *const std::os::raw::c_char;
    pub fn mupdf_clear_error();

    pub fn mupdf_safe_new_context() -> *mut fz_context;
    pub fn mupdf_safe_clone_context(ctx: *mut fz_context) -> *mut fz_context;
    pub fn mupdf_safe_drop_context(ctx: *mut fz_context);

    pub fn mupdf_safe_open_document(
        ctx: *mut fz_context,
        filename: *const std::os::raw::c_char,
    ) -> *mut fz_document;

    pub fn mupdf_safe_open_document_with_stream(
        ctx: *mut fz_context,
        data: *const std::os::raw::c_uchar,
        len: usize,
        mimetype: *const std::os::raw::c_char,
        out: *mut *mut fz_document,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_drop_document(ctx: *mut fz_context, doc: *mut fz_document);

    pub fn mupdf_safe_count_pages(
        ctx: *mut fz_context,
        doc: *mut fz_document,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_needs_password(
        ctx: *mut fz_context,
        doc: *mut fz_document,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_authenticate_password(
        ctx: *mut fz_context,
        doc: *mut fz_document,
        pw: *const std::os::raw::c_char,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_lookup_metadata(
        ctx: *mut fz_context,
        doc: *mut fz_document,
        key: *const std::os::raw::c_char,
        buf: *mut std::os::raw::c_char,
        len: std::os::raw::c_int,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_load_page(
        ctx: *mut fz_context,
        doc: *mut fz_document,
        number: std::os::raw::c_int,
    ) -> *mut fz_page;

    pub fn mupdf_safe_drop_page(ctx: *mut fz_context, page: *mut fz_page);

    pub fn mupdf_safe_bound_page(ctx: *mut fz_context, page: *mut fz_page) -> fz_rect;

    pub fn mupdf_safe_bound_page_box(
        ctx: *mut fz_context,
        page: *mut fz_page,
        box_type: std::os::raw::c_int,
    ) -> fz_rect;

    pub fn mupdf_safe_page_rotation(
        ctx: *mut fz_context,
        doc: *mut fz_document,
        page: *mut fz_page,
    ) -> std::os::raw::c_int;

    /* ---- 结构化文本（stext） ---- */

    pub fn mupdf_safe_new_stext_page(
        ctx: *mut fz_context,
        page: *mut fz_page,
        out: *mut *mut fz_stext_page,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_drop_stext_page(ctx: *mut fz_context, stpage: *mut fz_stext_page);

    pub fn mupdf_safe_stext_to_text(
        ctx: *mut fz_context,
        stpage: *mut fz_stext_page,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_stext_to_html(
        ctx: *mut fz_context,
        stpage: *mut fz_stext_page,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_stext_to_xml(
        ctx: *mut fz_context,
        stpage: *mut fz_stext_page,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_stext_to_xhtml(
        ctx: *mut fz_context,
        stpage: *mut fz_stext_page,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_stext_to_json(
        ctx: *mut fz_context,
        stpage: *mut fz_stext_page,
        scale: f32,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_stext_to_words(
        ctx: *mut fz_context,
        stpage: *mut fz_stext_page,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
        total_n: *mut std::os::raw::c_int,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_stext_to_blocks(
        ctx: *mut fz_context,
        stpage: *mut fz_stext_page,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
        total_n: *mut std::os::raw::c_int,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_stext_to_dict(
        ctx: *mut fz_context,
        stpage: *mut fz_stext_page,
        include_chars: std::os::raw::c_int,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
    ) -> std::os::raw::c_int;

    /* ---- 像素图（pixmap） ---- */

    pub fn mupdf_safe_render_pixmap(
        ctx: *mut fz_context,
        page: *mut fz_page,
        zoom: f32,
        alpha: std::os::raw::c_int,
        out: *mut *mut fz_pixmap,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_drop_pixmap(ctx: *mut fz_context, pix: *mut fz_pixmap);

    pub fn mupdf_safe_pixmap_width(ctx: *mut fz_context, pix: *mut fz_pixmap) -> std::os::raw::c_int;

    pub fn mupdf_safe_pixmap_height(ctx: *mut fz_context, pix: *mut fz_pixmap) -> std::os::raw::c_int;

    pub fn mupdf_safe_pixmap_stride(ctx: *mut fz_context, pix: *mut fz_pixmap) -> std::os::raw::c_int;

    pub fn mupdf_safe_pixmap_components(
        ctx: *mut fz_context,
        pix: *mut fz_pixmap,
    ) -> std::os::raw::c_int;

    pub fn mupdf_safe_pixmap_samples(
        ctx: *mut fz_context,
        pix: *mut fz_pixmap,
    ) -> *mut std::os::raw::c_uchar;

    pub fn mupdf_safe_pixmap_to_png(
        ctx: *mut fz_context,
        pix: *mut fz_pixmap,
        out: *mut *mut std::os::raw::c_uchar,
        out_len: *mut usize,
    ) -> std::os::raw::c_int;

    /* ---- 链接（links） ---- */

    pub fn mupdf_safe_load_links(
        ctx: *mut fz_context,
        page: *mut fz_page,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
        total_n: *mut std::os::raw::c_int,
    ) -> std::os::raw::c_int;

    /* ---- 图片提取 ---- */

    pub fn mupdf_safe_get_images(
        ctx: *mut fz_context,
        page: *mut fz_page,
        include_data: std::os::raw::c_int,
        out: *mut *mut std::os::raw::c_char,
        out_len: *mut usize,
        total_n: *mut std::os::raw::c_int,
    ) -> std::os::raw::c_int;

    /* ---- 内存释放 ---- */

    pub fn mupdf_free(ptr: *mut std::ffi::c_void);

    /* ---- 批量扩展（mupdf_extensions.c） ---- */

    pub fn mupdf_ext_extract_pages_text(
        ctx: *mut fz_context,
        doc: *mut fz_document,
        out_text: *mut *mut std::os::raw::c_char,
        out_text_len: *mut usize,
        out_offsets: *mut *mut usize,
        out_page_count: *mut std::os::raw::c_int,
    ) -> std::os::raw::c_int;
}
