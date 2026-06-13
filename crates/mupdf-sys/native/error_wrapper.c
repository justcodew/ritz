/*
 * C 错误包装层实现
 *
 * 每个 MuPDF 调用都用 fz_try/fz_catch 包裹，捕获 longjmp 异常，
 * 转为错误码 + 线程局部错误消息返回。
 *
 * 模式（借鉴 gomupdf bindings.c）：
 *   1. mupdf_clear_error() 清空缓冲区
 *   2. fz_try(ctx) { 实际工作 }
 *   3. fz_always(ctx) { 清理中间资源（无论是否异常都执行） }
 *   4. fz_catch(ctx) { 写错误消息，返回 NULL/-1/零值 }
 */

#include "error_wrapper.h"
#include <stdio.h>
#include <stdarg.h>
#include <string.h>
#include <stdlib.h>

/* ---- 线程局部错误缓冲区 ---- */
/* __thread 保证多线程下各自独立（Rust rayon 并行时关键） */
static __thread char last_error[512] = {0};

const char *mupdf_last_error(void) {
    return last_error;
}

void mupdf_clear_error(void) {
    last_error[0] = '\0';
}

static void set_error(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(last_error, sizeof(last_error), fmt, ap);
    va_end(ap);
}

/* ---- 静默警告回调，避免污染 stderr ---- */
static void warning_cb(void *user, const char *message) {
    /* 故意空实现：丢弃所有 MuPDF 警告 */
}

/* ====================================================================
 * 上下文
 * ==================================================================== */

fz_context *mupdf_safe_new_context(void) {
    mupdf_clear_error();
    fz_context *ctx = fz_new_context(NULL, NULL, FZ_STORE_DEFAULT);
    if (!ctx) {
        set_error("failed to allocate context");
        return NULL;
    }
    fz_register_document_handlers(ctx);
    fz_set_warning_callback(ctx, warning_cb, NULL);
    return ctx;
}

fz_context *mupdf_safe_clone_context(fz_context *ctx) {
    if (!ctx) return NULL;
    fz_context *clone = NULL;
    mupdf_clear_error();
    fz_try(ctx) {
        clone = fz_clone_context(ctx);
    }
    fz_catch(ctx) {
        set_error("clone context: %s", fz_caught_message(ctx));
        clone = NULL;
    }
    if (clone) {
        fz_set_warning_callback(clone, warning_cb, NULL);
    }
    return clone;
}

void mupdf_safe_drop_context(fz_context *ctx) {
    if (ctx) fz_drop_context(ctx);
}

/* ====================================================================
 * 文档
 * ==================================================================== */

fz_document *mupdf_safe_open_document(fz_context *ctx, const char *filename) {
    if (!ctx || !filename) return NULL;
    fz_document *doc = NULL;
    mupdf_clear_error();
    fz_try(ctx) {
        doc = fz_open_document(ctx, filename);
    }
    fz_catch(ctx) {
        set_error("open document '%s': %s", filename, fz_caught_message(ctx));
        doc = NULL;
    }
    return doc;
}

int mupdf_safe_open_document_with_stream(
    fz_context *ctx,
    const unsigned char *data, size_t len,
    const char *mimetype,
    fz_document **out) {
    if (!ctx || !data || !out) return -1;
    *out = NULL;
    fz_stream *stm = NULL;
    fz_document *doc = NULL;
    mupdf_clear_error();
    fz_try(ctx) {
        /* fz_open_memory 不复制数据，调用方须保证 data 在 doc 存活期间有效。
         * Rust 侧需用 Box::leak 或保持 Vec 不释放。 */
        stm = fz_open_memory(ctx, data, len);
        doc = fz_open_document_with_stream(ctx, mimetype, stm);
    }
    fz_always(ctx) {
        fz_drop_stream(ctx, stm);
    }
    fz_catch(ctx) {
        set_error("open stream: %s", fz_caught_message(ctx));
        if (doc) { fz_drop_document(ctx, doc); doc = NULL; }
        return -1;
    }
    *out = doc;
    return 0;
}

void mupdf_safe_drop_document(fz_context *ctx, fz_document *doc) {
    if (ctx && doc) fz_drop_document(ctx, doc);
}

int mupdf_safe_count_pages(fz_context *ctx, fz_document *doc) {
    if (!ctx || !doc) return -1;
    int n = -1;
    mupdf_clear_error();
    fz_try(ctx) {
        n = fz_count_pages(ctx, doc);
    }
    fz_catch(ctx) {
        set_error("count pages: %s", fz_caught_message(ctx));
        n = -1;
    }
    return n;
}

int mupdf_safe_needs_password(fz_context *ctx, fz_document *doc) {
    if (!ctx || !doc) return 0;
    int r = 0;
    fz_try(ctx) {
        r = fz_needs_password(ctx, doc);
    }
    fz_catch(ctx) {
        r = 0;
    }
    return r;
}

int mupdf_safe_authenticate_password(fz_context *ctx, fz_document *doc, const char *pw) {
    if (!ctx || !doc) return 0;
    int r = 0;
    mupdf_clear_error();
    fz_try(ctx) {
        r = fz_authenticate_password(ctx, doc, pw);
    }
    fz_catch(ctx) {
        set_error("authenticate: %s", fz_caught_message(ctx));
        r = 0;
    }
    return r;
}

int mupdf_safe_lookup_metadata(fz_context *ctx, fz_document *doc,
                               const char *key, char *buf, int len) {
    if (!ctx || !doc || !key || !buf || len <= 0) return -1;
    int n = -1;
    mupdf_clear_error();
    fz_try(ctx) {
        n = fz_lookup_metadata(ctx, doc, key, buf, len);
    }
    fz_catch(ctx) {
        set_error("lookup metadata: %s", fz_caught_message(ctx));
        n = -1;
    }
    return n;
}

/* ====================================================================
 * 页面
 * ==================================================================== */

fz_page *mupdf_safe_load_page(fz_context *ctx, fz_document *doc, int number) {
    if (!ctx || !doc) return NULL;
    fz_page *page = NULL;
    mupdf_clear_error();
    fz_try(ctx) {
        page = fz_load_page(ctx, doc, number);
    }
    fz_catch(ctx) {
        set_error("load page %d: %s", number, fz_caught_message(ctx));
        page = NULL;
    }
    return page;
}

void mupdf_safe_drop_page(fz_context *ctx, fz_page *page) {
    if (ctx && page) fz_drop_page(ctx, page);
}

fz_rect mupdf_safe_bound_page(fz_context *ctx, fz_page *page) {
    fz_rect r = {0, 0, 0, 0};
    if (!ctx || !page) return r;
    mupdf_clear_error();
    fz_try(ctx) {
        r = fz_bound_page(ctx, page);
    }
    fz_catch(ctx) {
        set_error("bound page: %s", fz_caught_message(ctx));
        r = fz_make_rect(0, 0, 0, 0);
    }
    return r;
}

/* ====================================================================
 * 结构化文本（stext）
 * ==================================================================== */

int mupdf_safe_new_stext_page(fz_context *ctx, fz_page *page, fz_stext_page **out) {
    if (!ctx || !page || !out) return -1;
    *out = NULL;
    mupdf_clear_error();
    fz_try(ctx) {
        *out = fz_new_stext_page_from_page(ctx, page, NULL);
    }
    fz_catch(ctx) {
        set_error("stext page: %s", fz_caught_message(ctx));
        return -1;
    }
    return 0;
}

void mupdf_safe_drop_stext_page(fz_context *ctx, fz_stext_page *stpage) {
    if (ctx && stpage) fz_drop_stext_page(ctx, stpage);
}

/*
 * 通用 helper：将 stext 页通过 fz_print_stext_page_as_* 输出到 buffer，
 * 然后提取为 malloc 的内存块返回给 Rust。
 *
 * MuPDF 1.27 的签名不统一：
 *   text:                          (ctx, out, page)
 *   html/xhtml/xml:                (ctx, out, page, int id)
 *   json:                          (ctx, out, page, float scale)
 * 统一用一个带 int 标签的函数指针，text 版用 wrapper 忽略 id。
 */
typedef void (*stext_print_fn)(fz_context *, fz_output *, fz_stext_page *, int);

static void text_print_wrapper(fz_context *ctx, fz_output *out,
                               fz_stext_page *page, int id) {
    (void)id;
    fz_print_stext_page_as_text(ctx, out, page);
}

static int stext_to_buffer(fz_context *ctx, fz_stext_page *stpage,
                           stext_print_fn print_fn,
                           char **out, size_t *out_len) {
    if (!ctx || !stpage || !out || !out_len) return -1;
    *out = NULL;
    *out_len = 0;

    fz_buffer *buf = NULL;
    fz_output *stm = NULL;
    unsigned char *data = NULL;
    size_t len = 0;

    mupdf_clear_error();
    fz_try(ctx) {
        buf = fz_new_buffer(ctx, 256);
        stm = fz_new_output_with_buffer(ctx, buf);
        print_fn(ctx, stm, stpage, 0);
        fz_close_output(ctx, stm);
        len = fz_buffer_extract(ctx, buf, &data); /* data 是 malloc 的 */
    }
    fz_always(ctx) {
        fz_drop_output(ctx, stm);
        fz_drop_buffer(ctx, buf);
    }
    fz_catch(ctx) {
        set_error("stext output: %s", fz_caught_message(ctx));
        if (data) free(data);
        return -1;
    }

    *out = (char *)data;
    *out_len = len;
    return 0;
}

int mupdf_safe_stext_to_text(fz_context *ctx, fz_stext_page *stpage,
                             char **out, size_t *out_len) {
    return stext_to_buffer(ctx, stpage, text_print_wrapper, out, out_len);
}

int mupdf_safe_stext_to_html(fz_context *ctx, fz_stext_page *stpage,
                             char **out, size_t *out_len) {
    return stext_to_buffer(ctx, stpage, fz_print_stext_page_as_html, out, out_len);
}

int mupdf_safe_stext_to_xml(fz_context *ctx, fz_stext_page *stpage,
                            char **out, size_t *out_len) {
    return stext_to_buffer(ctx, stpage, fz_print_stext_page_as_xml, out, out_len);
}

/* ====================================================================
 * 像素图（pixmap）
 * ==================================================================== */

int mupdf_safe_render_pixmap(fz_context *ctx, fz_page *page,
                             float zoom, int alpha, fz_pixmap **out) {
    if (!ctx || !page || !out) return -1;
    *out = NULL;

    fz_pixmap *pix = NULL;
    fz_device *dev = NULL;

    mupdf_clear_error();
    fz_try(ctx) {
        fz_rect bounds = fz_bound_page(ctx, page);
        fz_matrix ctm = fz_scale(zoom, zoom);
        fz_irect ibox = fz_irect_from_rect(fz_transform_rect(bounds, ctm));
        int w = ibox.x1 - ibox.x0;
        int h = ibox.y1 - ibox.y0;

        pix = fz_new_pixmap(ctx, fz_device_rgb(ctx), w, h, NULL, alpha);
        fz_clear_pixmap_with_value(ctx, pix, 0xFF); /* 白色背景 */

        dev = fz_new_draw_device(ctx, ctm, pix);
        fz_run_page(ctx, page, dev, ctm, NULL);
        fz_close_device(ctx, dev);
    }
    fz_always(ctx) {
        fz_drop_device(ctx, dev);
    }
    fz_catch(ctx) {
        if (pix) { fz_drop_pixmap(ctx, pix); pix = NULL; }
        set_error("render pixmap: %s", fz_caught_message(ctx));
        return -1;
    }

    *out = pix;
    return 0;
}

void mupdf_safe_drop_pixmap(fz_context *ctx, fz_pixmap *pix) {
    if (ctx && pix) fz_drop_pixmap(ctx, pix);
}

int mupdf_safe_pixmap_width(fz_context *ctx, fz_pixmap *pix) {
    if (!ctx || !pix) return 0;
    return fz_pixmap_width(ctx, pix);
}

int mupdf_safe_pixmap_height(fz_context *ctx, fz_pixmap *pix) {
    if (!ctx || !pix) return 0;
    return fz_pixmap_height(ctx, pix);
}

int mupdf_safe_pixmap_stride(fz_context *ctx, fz_pixmap *pix) {
    if (!ctx || !pix) return 0;
    return fz_pixmap_stride(ctx, pix);
}

int mupdf_safe_pixmap_components(fz_context *ctx, fz_pixmap *pix) {
    if (!ctx || !pix) return 0;
    return fz_pixmap_components(ctx, pix);
}

unsigned char *mupdf_safe_pixmap_samples(fz_context *ctx, fz_pixmap *pix) {
    if (!ctx || !pix) return NULL;
    return fz_pixmap_samples(ctx, pix);
}

int mupdf_safe_pixmap_to_png(fz_context *ctx, fz_pixmap *pix,
                             unsigned char **out, size_t *out_len) {
    if (!ctx || !pix || !out || !out_len) return -1;
    *out = NULL;
    *out_len = 0;

    fz_buffer *buf = NULL;
    unsigned char *data = NULL;
    size_t len = 0;

    mupdf_clear_error();
    fz_try(ctx) {
        buf = fz_new_buffer_from_pixmap_as_png(ctx, pix, fz_default_color_params);
        len = fz_buffer_extract(ctx, buf, &data);
    }
    fz_always(ctx) {
        fz_drop_buffer(ctx, buf);
    }
    fz_catch(ctx) {
        set_error("pixmap to png: %s", fz_caught_message(ctx));
        if (data) free(data);
        return -1;
    }

    *out = data;
    *out_len = len;
    return 0;
}

/* ====================================================================
 * 内存释放
 * ==================================================================== */

void mupdf_free(void *ptr) {
    if (ptr) free(ptr);
}
