/*
 * 批量扩展层实现（阶段四）
 *
 * 与 error_wrapper.c 共享错误缓冲区和 growbuf 思路，但单独成文件便于维护。
 * 通过 error_wrapper.h 引入 mupdf_last_error / mupdf_clear_error / set_error。
 */

#include "error_wrapper.h"
#include "mupdf_extensions.h"
#include <mupdf/pdf.h>
#include <string.h>
#include <stdlib.h>

/* ---- growbuf（与 error_wrapper.c 中的等价，复制以保持单文件可测） ---- */
typedef struct {
    unsigned char *data;
    size_t len;
    size_t cap;
} ext_growbuf;

static int ext_gb_ensure(ext_growbuf *gb, size_t extra) {
    if (gb->len + extra <= gb->cap) return 0;
    size_t newcap = gb->cap ? gb->cap : 4096;
    while (newcap < gb->len + extra) newcap *= 2;
    unsigned char *nd = (unsigned char *)realloc(gb->data, newcap);
    if (!nd) return -1;
    gb->data = nd;
    gb->cap = newcap;
    return 0;
}

static void ext_gb_put(ext_growbuf *gb, const void *src, size_t n) {
    if (ext_gb_ensure(gb, n)) return; /* OOM 由调用方检查 */
    memcpy(gb->data + gb->len, src, n);
    gb->len += n;
}

/* fz_output 的 write 回调，直接写入 ext_growbuf，避免二次拷贝。 */
static void ext_write_to_growbuf(fz_context *ctx, void *state_,
                                  const void *buf, size_t n) {
    (void)ctx;
    ext_growbuf *gb = (ext_growbuf *)state_;
    ext_gb_put(gb, buf, n);
}

int mupdf_ext_extract_pages_text(fz_context *ctx, fz_document *doc,
                         char **out_text, size_t *out_text_len,
                         size_t **out_offsets, int *out_page_count) {
    if (!ctx || !doc || !out_text || !out_text_len || !out_offsets || !out_page_count) {
        return -1;
    }
    *out_text = NULL;
    *out_text_len = 0;
    *out_offsets = NULL;
    *out_page_count = 0;

    ext_growbuf gb = {0, 0, 0};
    size_t *offsets = NULL;
    int page_count = 0;

    mupdf_clear_error();
    fz_try(ctx) {
        page_count = fz_count_pages(ctx, doc);
        if (page_count < 0) {
            fz_throw(ctx, FZ_ERROR_GENERIC, "cannot count pages");
        }

        offsets = (size_t *)fz_calloc(ctx, (size_t)page_count + 1, sizeof(size_t));
        if (!offsets) {
            fz_throw(ctx, FZ_ERROR_SYSTEM, "oom allocating offsets");
        }

        for (int i = 0; i < page_count; i++) {
            fz_page *page = NULL;
            fz_stext_page *stpage = NULL;
            fz_output *stm = NULL;
            size_t before = gb.len;

            fz_try(ctx) {
                offsets[i] = gb.len;

                page = fz_load_page(ctx, doc, i);
                stpage = fz_new_stext_page_from_page(ctx, page, NULL);

                /* 直接流到 growbuf，无中间 buffer 拷贝 */
                stm = fz_new_output(ctx, 4096, &gb, ext_write_to_growbuf, NULL, NULL);
                fz_print_stext_page_as_text(ctx, stm, stpage);
                fz_close_output(ctx, stm);
            }
            fz_always(ctx) {
                if (stm) fz_drop_output(ctx, stm);
                if (stpage) fz_drop_stext_page(ctx, stpage);
                if (page) fz_drop_page(ctx, page);
            }
            fz_catch(ctx) {
                fz_rethrow_if(ctx, FZ_ERROR_SYSTEM);
                /* 非致命：保持 offset 不变（回退） */
                gb.len = before;
            }
        }
        offsets[page_count] = gb.len;

        /* 追加 NUL 便于 C 端打印 */
        char nul = '\0';
        ext_gb_put(&gb, &nul, 1);
    }
    fz_catch(ctx) {
        set_error("extract_pages_text: %s", fz_caught_message(ctx));
        if (gb.data) free(gb.data);
        if (offsets) fz_free(ctx, offsets);
        return -1;
    }

    *out_text = (char *)gb.data;
    *out_text_len = gb.len > 0 ? gb.len - 1 : 0;
    *out_offsets = offsets;
    *out_page_count = page_count;
    return 0;
}
