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

int mupdf_safe_page_rotation(fz_context *ctx, fz_page *page) {
    int rot = 0;
    if (!ctx || !page) return 0;
    fz_try(ctx) {
        rot = fz_page_rotation(ctx, page);
    }
    fz_catch(ctx) {
        rot = 0;
    }
    return rot;
}

/* ====================================================================
 * 内存释放
 * ==================================================================== */

void mupdf_free(void *ptr) {
    if (ptr) free(ptr);
}
