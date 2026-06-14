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
#include <mupdf/pdf.h>
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

void set_error(const char *fmt, ...) {
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

fz_rect mupdf_safe_bound_page_box(fz_context *ctx, fz_page *page, int box_type) {
    fz_rect r = {0, 0, 0, 0};
    if (!ctx || !page) return r;
    static const fz_box_type kBoxes[5] = {
        FZ_MEDIA_BOX, FZ_CROP_BOX, FZ_BLEED_BOX, FZ_TRIM_BOX, FZ_ART_BOX,
    };
    if (box_type < 0 || box_type > 4) box_type = 1; /* 默认 cropbox */
    mupdf_clear_error();
    fz_try(ctx) {
        r = fz_bound_page_box(ctx, page, kBoxes[box_type]);
    }
    fz_catch(ctx) {
        set_error("bound page box: %s", fz_caught_message(ctx));
        r = fz_make_rect(0, 0, 0, 0);
    }
    return r;
}

/* rotation 仅对 PDF 文档有意义：读 /Rotate（0/90/180/270）。
 * 非 PDF 返回 0。fz_page 自身没有 rotation 概念。 */
int mupdf_safe_page_rotation(fz_context *ctx, fz_document *doc, fz_page *page) {
    if (!ctx || !doc || !page) return 0;
    int rot = 0;
    mupdf_clear_error();
    fz_try(ctx) {
        pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
        if (!pdf) {
            fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf document");
        }
        pdf_page *pg = (pdf_page *)page;
        if (!pg->obj) {
            fz_throw(ctx, FZ_ERROR_GENERIC, "page has no pdf obj");
        }
        rot = pdf_dict_get_inheritable_int(ctx, pg->obj, PDF_NAME(Rotate));
    }
    fz_catch(ctx) {
        /* 静默：rotation 对非 PDF 是 0，错误也算 0 */
        rot = 0;
    }
    return rot;
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

int mupdf_safe_stext_to_xhtml(fz_context *ctx, fz_stext_page *stpage,
                              char **out, size_t *out_len) {
    return stext_to_buffer(ctx, stpage, fz_print_stext_page_as_xhtml, out, out_len);
}

/*
 * JSON 模式：fz_print_stext_page_as_json 签名为 (ctx, out, page, float scale)，
 * 与 text/html/xml 都不同，所以单独实现。
 */
int mupdf_safe_stext_to_json(fz_context *ctx, fz_stext_page *stpage, float scale,
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
        fz_print_stext_page_as_json(ctx, stm, stpage, scale);
        fz_close_output(ctx, stm);
        len = fz_buffer_extract(ctx, buf, &data);
    }
    fz_always(ctx) {
        fz_drop_output(ctx, stm);
        fz_drop_buffer(ctx, buf);
    }
    fz_catch(ctx) {
        set_error("stext json: %s", fz_caught_message(ctx));
        if (data) free(data);
        return -1;
    }

    *out = (char *)data;
    *out_len = len;
    return 0;
}

/*
 * 简易动态缓冲区（Tier 2 #7：words/blocks/dict 共享）。
 * 用 offset（而非 pointer）记录位置，realloc 不失效。
 */
typedef struct {
    unsigned char *data;
    size_t len;
    size_t cap;
} growbuf;

static int gb_ensure(fz_context *ctx, growbuf *gb, size_t extra) {
    if (gb->len + extra <= gb->cap) return 0;
    size_t newcap = gb->cap ? gb->cap : 1024;
    while (newcap < gb->len + extra) newcap *= 2;
    unsigned char *nd = (unsigned char *)fz_realloc(ctx, gb->data, newcap);
    if (!nd) return -1;
    gb->data = nd;
    gb->cap = newcap;
    return 0;
}

static void gb_put(fz_context *ctx, growbuf *gb, const void *src, size_t n) {
    if (gb_ensure(ctx, gb, n)) fz_throw(ctx, FZ_ERROR_SYSTEM, "oom");
    memcpy(gb->data + gb->len, src, n);
    gb->len += n;
}

/*
 * 单词级提取：与 PyMuPDF get_text("words") 对齐。
 *
 * 遍历 stext_page 的 block→line→char 链表（仅处理文本块），
 * 在每行内按字符 c == ' ' 分词。每词的 bbox 由 char.quad 的并集计算。
 *
 * 输出格式（每条记录连续写入）：
 *   [x0,y0,x1,y1: 4 * float]            16B
 *   [block_no,line_no,word_no: 3 * int] 12B
 *   [str_len: int]                       4B
 *   [utf8 bytes]                         str_len bytes（无 NUL）
 *
 * Tier 2 #7：原 sizing+writing 两趟改成 growbuf 单趟。每词先 reserve
 * 32 字节占位 header（记 offset），追加 utf8，结束时回填 header。
 * offset 不受 realloc 影响，安全。
 */
int mupdf_safe_stext_to_words(fz_context *ctx, fz_stext_page *stpage,
                              char **out, size_t *out_len, int *total_n) {
    if (!ctx || !stpage || !out || !out_len || !total_n) return -1;
    *out = NULL;
    *out_len = 0;
    *total_n = 0;

    growbuf gb = {0, 0, 0};
    int n_words = 0;

    mupdf_clear_error();
    fz_try(ctx) {
        char header_zero[32] = {0};
        int block_no = 0;
        for (fz_stext_block *blk = stpage->first_block; blk; blk = blk->next) {
            if (blk->type != FZ_STEXT_BLOCK_TEXT) {
                block_no++;
                continue;
            }
            int line_no = 0;
            for (fz_stext_line *line = blk->u.t.first_line; line; line = line->next) {
                int word_no = 0;
                int in_word = 0;
                float wx0 = 0, wy0 = 0, wx1 = 0, wy1 = 0;
                size_t word_bytes = 0;
                size_t header_off = 0;

                for (fz_stext_char *ch = line->first_char; ch; ch = ch->next) {
                    if (ch->c == ' ' || ch->c == '\t') {
                        if (in_word) {
                            /* 回填 header：offset 不受 realloc 影响 */
                            int hdr_block = block_no, hdr_line = line_no, hdr_word = word_no;
                            int hdr_len = (int)word_bytes;
                            unsigned char *h = gb.data + header_off;
                            memcpy(h,      &wx0, 4);
                            memcpy(h + 4,  &wy0, 4);
                            memcpy(h + 8,  &wx1, 4);
                            memcpy(h + 12, &wy1, 4);
                            memcpy(h + 16, &hdr_block, 4);
                            memcpy(h + 20, &hdr_line, 4);
                            memcpy(h + 24, &hdr_word, 4);
                            memcpy(h + 28, &hdr_len, 4);
                            n_words++;
                            word_no++;
                            in_word = 0;
                            word_bytes = 0;
                        }
                    } else {
                        char tmp[8];
                        int len = fz_runetochar(tmp, ch->c);
                        fz_quad q = ch->quad;
                        float cx0 = q.ul.x, cy0 = q.ul.y;
                        float cx1 = q.lr.x, cy1 = q.lr.y;
                        if (q.ll.x < cx0) cx0 = q.ll.x;
                        if (q.ur.x > cx1) cx1 = q.ur.x;
                        if (q.ur.y < cy0) cy0 = q.ur.y;
                        if (q.ll.y > cy1) cy1 = q.ll.y;
                        if (!in_word) {
                            wx0 = cx0; wy0 = cy0; wx1 = cx1; wy1 = cy1;
                            /* 占位 32 字节 header，记录 offset，utf8 紧随其后 */
                            header_off = gb.len;
                            gb_put(ctx, &gb, header_zero, 32);
                            gb_put(ctx, &gb, tmp, len);
                            word_bytes = len;
                            in_word = 1;
                        } else {
                            if (cx0 < wx0) wx0 = cx0;
                            if (cy0 < wy0) wy0 = cy0;
                            if (cx1 > wx1) wx1 = cx1;
                            if (cy1 > wy1) wy1 = cy1;
                            gb_put(ctx, &gb, tmp, len);
                            word_bytes += len;
                        }
                    }
                }
                if (in_word) {
                    int hdr_block = block_no, hdr_line = line_no, hdr_word = word_no;
                    int hdr_len = (int)word_bytes;
                    unsigned char *h = gb.data + header_off;
                    memcpy(h,      &wx0, 4);
                    memcpy(h + 4,  &wy0, 4);
                    memcpy(h + 8,  &wx1, 4);
                    memcpy(h + 12, &wy1, 4);
                    memcpy(h + 16, &hdr_block, 4);
                    memcpy(h + 20, &hdr_line, 4);
                    memcpy(h + 24, &hdr_word, 4);
                    memcpy(h + 28, &hdr_len, 4);
                    n_words++;
                }
                line_no++;
            }
            block_no++;
        }
    }
    fz_catch(ctx) {
        set_error("stext words: %s", fz_caught_message(ctx));
        if (gb.data) fz_free(ctx, gb.data);
        return -1;
    }

    if (n_words == 0) {
        if (gb.data) fz_free(ctx, gb.data);
        return 0;
    }

    *out = (char *)gb.data;
    *out_len = gb.len;
    *total_n = n_words;
    return 0;
}

/*
 * 块级提取：与 PyMuPDF get_text("blocks") 对齐。
 *
 * 对每个 stext_block：
 *   - text 块：text = 各 line 的字符串联，line 间用 \n
 *   - image 块：text_len = 0
 * 输出格式（每条记录连续写入）：
 *   [x0,y0,x1,y1: 4 * float]   16B
 *   [block_no, block_type, text_len: 3 * int]  12B
 *   [utf8 bytes]                text_len B
 */
int mupdf_safe_stext_to_blocks(fz_context *ctx, fz_stext_page *stpage,
                               char **out, size_t *out_len, int *total_n) {
    if (!ctx || !stpage || !out || !out_len || !total_n) return -1;
    *out = NULL;
    *out_len = 0;
    *total_n = 0;

    /* Tier 2 #7：growbuf 单趟。每块先 reserve 28 字节 header，写 text，回填 text_len。 */
    growbuf gb = {0, 0, 0};
    int n_blocks = 0;

    mupdf_clear_error();
    fz_try(ctx) {
        char header_zero[28] = {0};
        int block_no = 0;
        for (fz_stext_block *blk = stpage->first_block; blk; blk = blk->next) {
            size_t header_off = gb.len;
            gb_put(ctx, &gb, header_zero, 28);

            float coords[4] = { blk->bbox.x0, blk->bbox.y0, blk->bbox.x1, blk->bbox.y1 };
            memcpy(gb.data + header_off, coords, 16);
            int bno = block_no;
            int btype = (blk->type == FZ_STEXT_BLOCK_IMAGE) ? 1 : 0;
            memcpy(gb.data + header_off + 16, &bno, 4);
            memcpy(gb.data + header_off + 20, &btype, 4);
            /* text_len 在 text 写完后回填到 header_off + 24 */

            size_t text_bytes = 0;
            if (blk->type == FZ_STEXT_BLOCK_TEXT) {
                int line_idx = 0;
                for (fz_stext_line *line = blk->u.t.first_line; line; line = line->next) {
                    if (line_idx > 0) {
                        char nl = '\n';
                        gb_put(ctx, &gb, &nl, 1);
                        text_bytes += 1;
                    }
                    for (fz_stext_char *ch = line->first_char; ch; ch = ch->next) {
                        char tmp[8];
                        int len = fz_runetochar(tmp, ch->c);
                        gb_put(ctx, &gb, tmp, len);
                        text_bytes += len;
                    }
                    line_idx++;
                }
            }
            int hdr_len = (int)text_bytes;
            memcpy(gb.data + header_off + 24, &hdr_len, 4);

            n_blocks++;
            block_no++;
        }
    }
    fz_catch(ctx) {
        set_error("stext blocks: %s", fz_caught_message(ctx));
        if (gb.data) fz_free(ctx, gb.data);
        return -1;
    }

    if (n_blocks == 0) {
        if (gb.data) fz_free(ctx, gb.data);
        return 0;
    }

    *out = (char *)gb.data;
    *out_len = gb.len;
    *total_n = n_blocks;
    return 0;
}

/* ====================================================================
 * dict / rawdict 模式
 * ==================================================================== */

/*
 * span 边界判断：相邻 char 的 (font,size,color,flags) 相同时归为同一 span。
 */
static int span_compat(fz_stext_char *a, fz_stext_char *b) {
    return a->font == b->font
        && a->size == b->size
        && a->argb == b->argb
        && a->flags == b->flags;
}

int mupdf_safe_stext_to_dict(fz_context *ctx, fz_stext_page *stpage,
                             int include_chars, char **out, size_t *out_len) {
    if (!ctx || !stpage || !out || !out_len) return -1;
    *out = NULL;
    *out_len = 0;

    growbuf gb = {0, 0, 0};

    mupdf_clear_error();
    fz_try(ctx) {
        /* 占位 block_count，最后回填 */
        size_t block_count_pos = gb.len;
        int block_count = 0;
        gb_put(ctx, &gb, &(int){0}, 4);

        for (fz_stext_block *blk = stpage->first_block; blk; blk = blk->next) {
            if (blk->type != FZ_STEXT_BLOCK_TEXT && blk->type != FZ_STEXT_BLOCK_IMAGE) {
                continue;
            }
            int btype = (blk->type == FZ_STEXT_BLOCK_IMAGE) ? 1 : 0;
            gb_put(ctx, &gb, &btype, 4);
            float bbox[4] = { blk->bbox.x0, blk->bbox.y0, blk->bbox.x1, blk->bbox.y1 };
            gb_put(ctx, &gb, bbox, sizeof(bbox));

            if (btype == 1) {
                int zero = 0;
                gb_put(ctx, &gb, &zero, 4);
                block_count++;
                continue;
            }

            /* text 块：line_count 占位 */
            size_t lc_pos = gb.len;
            int line_count = 0;
            gb_put(ctx, &gb, &(int){0}, 4);

            for (fz_stext_line *line = blk->u.t.first_line; line; line = line->next) {
                float lbbox[4] = { line->bbox.x0, line->bbox.y0, line->bbox.x1, line->bbox.y1 };
                gb_put(ctx, &gb, lbbox, sizeof(lbbox));
                gb_put(ctx, &gb, &line->wmode, 4);
                float dir[2] = { line->dir.x, line->dir.y };
                gb_put(ctx, &gb, dir, sizeof(dir));

                /* span_count 占位 */
                size_t sc_pos = gb.len;
                int span_count = 0;
                gb_put(ctx, &gb, &(int){0}, 4);

                fz_stext_char *ch = line->first_char;
                while (ch) {
                    fz_stext_char *span_start = ch;
                    fz_stext_char *span_end = ch;
                    while (span_end->next && span_compat(span_end, span_end->next)) {
                        span_end = span_end->next;
                    }

                    /* span bbox = char quads 并集 */
                    float sx0 = 1e30f, sy0 = 1e30f, sx1 = -1e30f, sy1 = -1e30f;
                    for (fz_stext_char *c = span_start; ; c = c->next) {
                        fz_quad q = c->quad;
                        float cx0 = q.ul.x < q.ll.x ? q.ul.x : q.ll.x;
                        float cy0 = q.ul.y < q.ur.y ? q.ul.y : q.ur.y;
                        float cx1 = q.lr.x > q.ur.x ? q.lr.x : q.ur.x;
                        float cy1 = q.lr.y > q.ll.y ? q.lr.y : q.ll.y;
                        if (cx0 < sx0) sx0 = cx0;
                        if (cy0 < sy0) sy0 = cy0;
                        if (cx1 > sx1) sx1 = cx1;
                        if (cy1 > sy1) sy1 = cy1;
                        if (c == span_end) break;
                    }
                    float sbbox[4] = { sx0, sy0, sx1, sy1 };
                    gb_put(ctx, &gb, sbbox, sizeof(sbbox));

                    /* color: sRGB int (r<<16 | g<<8 | b) */
                    uint32_t argb = span_start->argb;
                    int color = (int)(((argb >> 16) & 0xff) << 16 |
                                      ((argb >> 8) & 0xff) << 8 |
                                      (argb & 0xff));
                    gb_put(ctx, &gb, &color, 4);

                    gb_put(ctx, &gb, &span_start->flags, 4);
                    gb_put(ctx, &gb, &span_start->size, 4);

                    /* font name */
                    const char *fname = fz_font_name(ctx, span_start->font);
                    if (!fname) fname = "";
                    int fname_len = (int)strlen(fname);
                    gb_put(ctx, &gb, &fname_len, 4);
                    if (fname_len > 0) gb_put(ctx, &gb, fname, fname_len);

                    /* text: 先写 len 占位，遍历后回填 */
                    size_t text_len_pos = gb.len;
                    int text_len = 0;
                    gb_put(ctx, &gb, &(int){0}, 4);
                    for (fz_stext_char *c = span_start; ; c = c->next) {
                        char tmp[8];
                        int n = fz_runetochar(tmp, c->c);
                        gb_put(ctx, &gb, tmp, n);
                        text_len += n;
                        if (c == span_end) break;
                    }
                    memcpy(gb.data + text_len_pos, &text_len, 4);

                    /* rawdict：每个 span 追加 chars 列表 */
                    if (include_chars) {
                        size_t cc_pos = gb.len;
                        int char_count = 0;
                        gb_put(ctx, &gb, &(int){0}, 4);
                        for (fz_stext_char *c = span_start; ; c = c->next) {
                            fz_quad q = c->quad;
                            float cx0 = q.ul.x < q.ll.x ? q.ul.x : q.ll.x;
                            float cy0 = q.ul.y < q.ur.y ? q.ul.y : q.ur.y;
                            float cx1 = q.lr.x > q.ur.x ? q.lr.x : q.ur.x;
                            float cy1 = q.lr.y > q.ll.y ? q.lr.y : q.ll.y;
                            float cbbox[4] = { cx0, cy0, cx1, cy1 };
                            gb_put(ctx, &gb, cbbox, sizeof(cbbox));
                            float origin[2] = { c->origin.x, c->origin.y };
                            gb_put(ctx, &gb, origin, sizeof(origin));
                            char tmp[8];
                            int n = fz_runetochar(tmp, c->c);
                            gb_put(ctx, &gb, &n, 4);
                            if (n > 0) gb_put(ctx, &gb, tmp, n);
                            char_count++;
                            if (c == span_end) break;
                        }
                        memcpy(gb.data + cc_pos, &char_count, 4);
                    }

                    span_count++;
                    ch = span_end->next;
                }

                memcpy(gb.data + sc_pos, &span_count, 4);
                line_count++;
            }

            memcpy(gb.data + lc_pos, &line_count, 4);
            block_count++;
        }

        memcpy(gb.data + block_count_pos, &block_count, 4);
    }
    fz_catch(ctx) {
        set_error("stext dict: %s", fz_caught_message(ctx));
        if (gb.data) fz_free(ctx, gb.data);
        return -1;
    }

    *out = (char *)gb.data;
    *out_len = gb.len;
    return 0;
}

/* ====================================================================
 * 链接（links）
 * ==================================================================== */

/*
 * 链接扁平化：遍历 fz_link 链表，每条记录编码为
 *   [rect.x0, rect.y0, rect.x1, rect.y1] (4 floats = 16B)
 *   + uri 字符串（含 NUL 结尾）
 * 顺序写入 malloc 缓冲区。调用方通过 mupdf_free 释放。
 *
 * 返回值：0 成功，-1 失败。
 * *total_n 写入链接条数。
 * *out_len 写入总字节数。
 */
int mupdf_safe_load_links(fz_context *ctx, fz_page *page,
                          char **out, size_t *out_len, int *total_n) {
    if (!ctx || !page || !out || !out_len || !total_n) return -1;
    *out = NULL;
    *out_len = 0;
    *total_n = 0;

    fz_link *head = NULL;
    char *buf = NULL;

    mupdf_clear_error();
    fz_try(ctx) {
        head = fz_load_links(ctx, page);
    }
    fz_catch(ctx) {
        set_error("load links: %s", fz_caught_message(ctx));
        return -1;
    }

    /* 第一遍：计算总字节数 */
    size_t total = 0;
    int n = 0;
    for (fz_link *l = head; l; l = l->next) {
        const char *uri = l->uri ? l->uri : "";
        size_t uri_len = strlen(uri) + 1;
        total += 4 * sizeof(float) + uri_len;
        n++;
    }

    if (total == 0) {
        /* 无链接 */
        fz_drop_link(ctx, head);
        *total_n = 0;
        *out_len = 0;
        *out = NULL;
        return 0;
    }

    buf = (char *)malloc(total);
    if (!buf) {
        fz_drop_link(ctx, head);
        set_error("links: out of memory (%zu bytes)", total);
        return -1;
    }

    /* 第二遍：写入数据 */
    char *p = buf;
    for (fz_link *l = head; l; l = l->next) {
        const char *uri = l->uri ? l->uri : "";
        size_t uri_len = strlen(uri) + 1;
        float coords[4] = { l->rect.x0, l->rect.y0, l->rect.x1, l->rect.y1 };
        memcpy(p, coords, sizeof(coords));
        p += sizeof(coords);
        memcpy(p, uri, uri_len);
        p += uri_len;
    }

    fz_drop_link(ctx, head);
    *out = buf;
    *out_len = total;
    *total_n = n;
    return 0;
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
 * 图片提取（get_images）
 * ==================================================================== */

/*
 * 通过自定义 fz_device 捕获 fill_image 回调，遍历页面上所有图片。
 * 按 fz_image* 指针去重（同一张图复用多次只输出一次）。
 *
 * 输出二进制布局：
 *   [image_count: i32]
 *   for each image:
 *     [bbox: 4 floats]
 *     [width: i32]
 *     [height: i32]
 *     [bpc: i32]
 *     [colorspace_len: i32][colorspace bytes]
 *     [imagemask: i32]
 *     [has_data: i32]
 *     if has_data:
 *       [data_len: i32][png bytes]
 */
typedef struct {
    fz_device dev;
    growbuf gb;
    int include_data;
    int image_count;
    int error;
    fz_image **seen;
    int seen_count;
    int seen_cap;
    /* xref 查找表：fz_image* → xref 号（来自 PDF 页面资源） */
    fz_image **xref_imgs;
    int *xref_nums;
    int xref_count;
} image_capture_device;

static int img_seen(image_capture_device *cap, fz_image *img) {
    for (int i = 0; i < cap->seen_count; i++) {
        if (cap->seen[i] == img) return 1;
    }
    return 0;
}

static int img_add_seen(fz_context *ctx, image_capture_device *cap, fz_image *img) {
    if (cap->seen_count == cap->seen_cap) {
        int nc = cap->seen_cap ? cap->seen_cap * 2 : 8;
        fz_image **n = (fz_image **)fz_realloc(ctx, cap->seen, nc * sizeof(fz_image *));
        if (!n) return -1;
        cap->seen = n;
        cap->seen_cap = nc;
    }
    cap->seen[cap->seen_count++] = img;
    return 0;
}

/* 在 xref 查找表中按 fz_image* 指针找 xref 号；找不到返回 0 */
static int img_find_xref(image_capture_device *cap, fz_image *img) {
    int i;
    for (i = 0; i < cap->xref_count; i++) {
        if (cap->xref_imgs[i] == img) return cap->xref_nums[i];
    }
    return 0;
}

static void cap_fill_image(fz_context *ctx, fz_device *dev_, fz_image *img,
                           fz_matrix ctm, float alpha, fz_color_params cp) {
    (void)alpha; (void)cp;
    image_capture_device *cap = (image_capture_device *)dev_;
    if (cap->error) return;
    if (img_seen(cap, img)) return;
    if (img_add_seen(ctx, cap, img)) { cap->error = 1; return; }

    fz_try(ctx) {
        /* 先写 xref（0 表示非 PDF 或未在资源表里找到） */
        int xref = img_find_xref(cap, img);
        gb_put(ctx, &cap->gb, &xref, 4);

        fz_rect r = fz_transform_rect(fz_unit_rect, ctm);
        float bbox[4] = { r.x0, r.y0, r.x1, r.y1 };
        gb_put(ctx, &cap->gb, bbox, sizeof(bbox));

        int w = img->w, h = img->h;
        int bpc = img->bpc ? img->bpc : 8;
        gb_put(ctx, &cap->gb, &w, 4);
        gb_put(ctx, &cap->gb, &h, 4);
        gb_put(ctx, &cap->gb, &bpc, 4);

        const char *cs = img->colorspace ? fz_colorspace_name(ctx, img->colorspace) : "";
        int cslen = (int)strlen(cs);
        gb_put(ctx, &cap->gb, &cslen, 4);
        if (cslen > 0) gb_put(ctx, &cap->gb, cs, cslen);

        int mask = (int)img->imagemask;
        gb_put(ctx, &cap->gb, &mask, 4);

        int has_data = cap->include_data ? 1 : 0;
        gb_put(ctx, &cap->gb, &has_data, 4);

        if (has_data) {
            /* 优先返回原始压缩流（JPEG/PNG/JPX），跳过 decode + re-encode。
             * 这是图片提取 10x+ 加速的关键：大多数 PDF 嵌入图是 JPEG，
             * 原始字节直接可写，无需 fz_get_pixmap_from_image + PNG 重编码。
             * 协议：ext_len(4) | ext(ext_len) | dlen(4) | data(dlen)
             * 仅 JPEG/PNG/JPX 返回原始流；RAW/FLATE/FAX 等退回 PNG fallback。
             */
            int wrote = 0;
            fz_try(ctx) {
                fz_compressed_buffer *cbuf = fz_compressed_image_buffer(ctx, img);
                if (cbuf && cbuf->buffer) {
                    int t = cbuf->params.type;
                    const char *ext = NULL;
                    if (t == FZ_IMAGE_JPEG) ext = "jpeg";
                    else if (t == FZ_IMAGE_PNG) ext = "png";
                    else if (t == FZ_IMAGE_JPX) ext = "jpx";
                    if (ext) {
                        unsigned char *src = NULL;
                        size_t len = fz_buffer_storage(ctx, cbuf->buffer, &src);
                        if (len > 0 && src) {
                            int extlen = (int)strlen(ext);
                            gb_put(ctx, &cap->gb, &extlen, 4);
                            gb_put(ctx, &cap->gb, ext, extlen);
                            int dl32 = (int)len;
                            gb_put(ctx, &cap->gb, &dl32, 4);
                            gb_put(ctx, &cap->gb, src, len);
                            wrote = 1;
                        }
                    }
                }
            }
            fz_catch(ctx) {
                wrote = 0;
            }

            if (!wrote) {
                /* Fallback：RAW/FLATE/FAX 等非独立格式 → decode + PNG 编码 */
                fz_pixmap *pix = NULL;
                fz_buffer *buf = NULL;
                fz_try(ctx) {
                    pix = fz_get_pixmap_from_image(ctx, img, NULL, NULL, NULL, NULL);
                    if (pix) {
                        buf = fz_new_buffer_from_pixmap_as_png(ctx, pix, fz_default_color_params);
                        unsigned char *data = NULL;
                        size_t dlen = buf ? fz_buffer_extract(ctx, buf, &data) : 0;
                        const char *ext = "png";
                        int extlen = 3;
                        gb_put(ctx, &cap->gb, &extlen, 4);
                        gb_put(ctx, &cap->gb, ext, extlen);
                        int dl32 = (int)dlen;
                        gb_put(ctx, &cap->gb, &dl32, 4);
                        if (dlen > 0 && data) gb_put(ctx, &cap->gb, data, dlen);
                        if (data) fz_free(ctx, data);
                    } else {
                        const char *ext = "png";
                        int extlen = 3, zero = 0;
                        gb_put(ctx, &cap->gb, &extlen, 4);
                        gb_put(ctx, &cap->gb, ext, extlen);
                        gb_put(ctx, &cap->gb, &zero, 4);
                    }
                }
                fz_always(ctx) {
                    if (buf) fz_drop_buffer(ctx, buf);
                    if (pix) fz_drop_pixmap(ctx, pix);
                }
                fz_catch(ctx) {
                    cap->error = 1;
                }
            }
        }
        cap->image_count++;
    }
    fz_catch(ctx) {
        cap->error = 1;
    }
}

int mupdf_safe_get_images(fz_context *ctx, fz_document *doc, fz_page *page,
                          int include_data,
                          char **out, size_t *out_len, int *total_n) {
    if (!ctx || !page || !out || !out_len || !total_n) return -1;
    *out = NULL;
    *out_len = 0;
    *total_n = 0;

    image_capture_device *cap = NULL;
    fz_display_list *list = NULL;
    size_t count_pos = 0;

    /* 在 fz_drop_device 前把 growbuf 所有权转移到这里，避免 use-after-free */
    unsigned char *gb_data = NULL;
    size_t gb_len = 0;
    int gb_count = 0;
    int gb_error = 0;

    mupdf_clear_error();
    fz_try(ctx) {
        list = fz_new_display_list_from_page(ctx, page);
        cap = fz_new_derived_device(ctx, image_capture_device);
        cap->gb.data = NULL; cap->gb.len = 0; cap->gb.cap = 0;
        cap->include_data = include_data;
        cap->image_count = 0;
        cap->error = 0;
        cap->seen = NULL; cap->seen_count = 0; cap->seen_cap = 0;
        cap->xref_imgs = NULL; cap->xref_nums = NULL; cap->xref_count = 0;

        /* 预扫描 PDF 页面资源，构建 fz_image* → xref 映射表。
         * cap_fill_image 在 device 遍历时会按指针查这张表。
         * 非 PDF 文档（doc==NULL 或非 PDF）跳过，xref 默认 0。 */
        if (doc) {
            pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
            if (pdf) {
                pdf_page *ppage = pdf_page_from_fz_page(ctx, page);
                if (ppage) {
                    pdf_obj *res = pdf_page_resources(ctx, ppage);
                    if (res) {
                        pdf_obj *xobj = pdf_dict_get(ctx, res, PDF_NAME(XObject));
                        int n = pdf_dict_len(ctx, xobj);
                        if (n > 0) {
                            cap->xref_imgs = (fz_image **)fz_calloc(ctx, n, sizeof(fz_image *));
                            cap->xref_nums = (int *)fz_calloc(ctx, n, sizeof(int));
                            int cnt = 0;
                            int i;
                            for (i = 0; i < n; i++) {
                                pdf_obj *indirect = pdf_dict_get_val(ctx, xobj, i);
                                pdf_obj *sub = pdf_dict_get(ctx, indirect, PDF_NAME(Subtype));
                                if (pdf_name_eq(ctx, sub, PDF_NAME(Image))) {
                                    int xref = pdf_to_num(ctx, indirect);
                                    fz_image *im = pdf_load_image(ctx, pdf, indirect);
                                    if (im && cnt < n) {
                                        cap->xref_imgs[cnt] = im;
                                        cap->xref_nums[cnt] = xref;
                                        cnt++;
                                    } else if (im) {
                                        fz_drop_image(ctx, im);
                                    }
                                }
                            }
                            cap->xref_count = cnt;
                        }
                    }
                }
            }
        }

        cap->dev.fill_image = cap_fill_image;
        /* fill_image_mask 签名不同（含 colorspace + color），暂不捕获，罕见场景 */

        /* image_count 占位 */
        count_pos = cap->gb.len;
        int placeholder = 0;
        gb_put(ctx, &cap->gb, &placeholder, 4);

        fz_run_display_list(ctx, list, (fz_device *)cap, fz_identity, fz_infinite_rect, NULL);
        fz_close_device(ctx, (fz_device *)cap);
    }
    fz_always(ctx) {
        /* 在 drop_device 前抢救 growbuf 和状态到本地 */
        if (cap) {
            int i;
            gb_data = cap->gb.data;
            gb_len = cap->gb.len;
            gb_count = cap->image_count;
            gb_error = cap->error;
            cap->gb.data = NULL;  /* 防止 drop_device 内部误用（虽然不会） */
            if (cap->seen) {
                fz_free(ctx, cap->seen);
                cap->seen = NULL;
            }
            /* 释放 xref 表里的 image 引用（display_list 持有自己的 ref） */
            for (i = 0; i < cap->xref_count; i++) {
                if (cap->xref_imgs[i]) fz_drop_image(ctx, cap->xref_imgs[i]);
            }
            if (cap->xref_imgs) fz_free(ctx, cap->xref_imgs);
            if (cap->xref_nums) fz_free(ctx, cap->xref_nums);
            cap->xref_imgs = NULL;
            cap->xref_nums = NULL;
            cap->xref_count = 0;
            fz_drop_device(ctx, (fz_device *)cap);
        }
        if (list) fz_drop_display_list(ctx, list);
    }
    fz_catch(ctx) {
        set_error("get_images: %s", fz_caught_message(ctx));
        if (gb_data) {
            fz_free(ctx, gb_data);
        }
        return -1;
    }

    if (gb_error) {
        set_error("get_images: image capture failed");
        if (gb_data) {
            fz_free(ctx, gb_data);
        }
        return -1;
    }

    if (gb_data) {
        memcpy(gb_data + count_pos, &gb_count, 4);
    }

    *out = (char *)gb_data;
    *out_len = gb_len;
    *total_n = gb_count;
    return 0;
}

/* ====================================================================
 * 文本搜索（plan_v1 Phase 5a）
 * ==================================================================== */

/* 搜索回调：把每个 hit 的所有 quads 追加到 growbuf。
 * 在 fz_try 内被调用，gb_put 失败时 fz_throw 会被外层 fz_catch 接住。 */
struct search_collect_state {
    growbuf gb;
    int quad_count;
};

static int search_collect_cb(fz_context *ctx, void *opaque,
                             int num_quads, fz_quad *hit_bbox) {
    struct search_collect_state *st = (struct search_collect_state *)opaque;
    /* fz_quad = 4 fz_point = 8 floats = 32 bytes */
    gb_put(ctx, &st->gb, hit_bbox, sizeof(fz_quad) * (size_t)num_quads);
    st->quad_count += num_quads;
    return 0; /* 0 = 继续搜索 */
}

int mupdf_safe_search_stext_page(fz_context *ctx, fz_stext_page *stpage,
                                 const char *needle,
                                 float **out, int *out_n) {
    if (!ctx || !stpage || !needle || !out || !out_n) return -1;
    *out = NULL;
    *out_n = 0;

    struct search_collect_state st = {0};

    mupdf_clear_error();
    fz_try(ctx) {
        int hits = fz_search_stext_page_cb(
            ctx, stpage, needle, search_collect_cb, &st);
        (void)hits; /* 我们关心的是 quad 总数，不是 hit 数 */
    }
    fz_catch(ctx) {
        if (st.gb.data) fz_free(ctx, st.gb.data);
        set_error("search stext page: %s", fz_caught_message(ctx));
        return -1;
    }

    if (st.quad_count == 0) {
        if (st.gb.data) fz_free(ctx, st.gb.data);
        *out = NULL;
        *out_n = 0;
        return 0;
    }

    *out = (float *)st.gb.data;
    *out_n = st.quad_count;
    return 0;
}

/* ====================================================================
 * 大纲 / 书签（plan_v1 Phase 5a）
 * ==================================================================== */

/* 递归前序遍历 outline 树，扁平化到 growbuf。
 * 在 fz_try 内调用，fz_page_number_from_location / gb_put 失败时 fz_throw
 * 由外层 fz_catch 接住。 */
static void walk_outline(fz_context *ctx, fz_document *doc,
                         fz_outline *node, int level,
                         growbuf *gb, int *count) {
    for (; node; node = node->next) {
        int raw_page = fz_page_number_from_location(ctx, doc, node->page);
        int py_page = (raw_page >= 0) ? raw_page + 1 : -1;
        const char *title = node->title ? node->title : "";
        int title_len = (int)strlen(title);
        int flags = (int)node->flags;
        int level_i = level;

        gb_put(ctx, gb, &level_i, sizeof(int));
        gb_put(ctx, gb, &py_page, sizeof(int));
        gb_put(ctx, gb, &flags, sizeof(int));
        gb_put(ctx, gb, &title_len, sizeof(int));
        if (title_len > 0) {
            gb_put(ctx, gb, title, (size_t)title_len);
        }
        (*count)++;

        if (node->down) {
            walk_outline(ctx, doc, node->down, level + 1, gb, count);
        }
    }
}

int mupdf_safe_load_outline(fz_context *ctx, fz_document *doc,
                            char **out, size_t *out_len, int *total_n) {
    if (!ctx || !doc || !out || !out_len || !total_n) return -1;
    *out = NULL;
    *out_len = 0;
    *total_n = 0;

    fz_outline *root = NULL;
    growbuf gb = {0};
    int count = 0;
    int failed = 0;

    mupdf_clear_error();
    fz_try(ctx) {
        root = fz_load_outline(ctx, doc);
        if (root) {
            walk_outline(ctx, doc, root, 1, &gb, &count);
        }
    }
    fz_always(ctx) {
        if (root) fz_drop_outline(ctx, root);
    }
    fz_catch(ctx) {
        failed = 1;
        set_error("load outline: %s", fz_caught_message(ctx));
    }

    if (failed) {
        if (gb.data) fz_free(ctx, gb.data);
        return -1;
    }

    if (count == 0) {
        if (gb.data) fz_free(ctx, gb.data);
        *out = NULL;
        *out_len = 0;
        *total_n = 0;
        return 0;
    }

    *out = (char *)gb.data;
    *out_len = gb.len;
    *total_n = count;
    return 0;
}

/* ====================================================================
 * 文档保存
 * ==================================================================== */

int mupdf_safe_save_document(fz_context *ctx, fz_document *doc, const char *filename) {
    if (!ctx || !doc || !filename) return -1;
    mupdf_clear_error();
    fz_try(ctx) {
        pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
        if (!pdf) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf document");
        pdf_save_document(ctx, pdf, filename, NULL);
    }
    fz_catch(ctx) {
        set_error("save document: %s", fz_caught_message(ctx));
        return -1;
    }
    return 0;
}

/* ====================================================================
 * 注释读写（Phase 5b）
 * ==================================================================== */

/* Buffer 协议（每个注释一条记录）：
 *   [type: i32]         enum pdf_annot_type
 *   [rect: 4*f32]      x0, y0, x1, y1
 *   [flags: i32]
 *   [color_n: i32]      颜色分量数 (0=无颜色, 3=RGB, 4=CMYK)
 *   [color: 4*f32]      颜色值（有效个数由 color_n 决定）
 *   [contents_len: i32]
 *   [author_len: i32]
 *   [quad_count: i32]
 *   [contents utf8 bytes]
 *   [author utf8 bytes]
 *   [quad0: 8*f32] ... [quadN: 8*f32]   fz_quad = ul,ur,ll,lr 每点 2 floats
 */

int mupdf_safe_get_annotations(fz_context *ctx, fz_document *doc, fz_page *page,
                               char **out, size_t *out_len, int *total_n) {
    if (!ctx || !doc || !page || !out || !out_len || !total_n) return -1;
    *out = NULL; *out_len = 0; *total_n = 0;

    growbuf gb = {0};
    int count = 0;

    mupdf_clear_error();
    fz_try(ctx) {
        pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
        if (!pdf) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf document");
        pdf_page *ppage = pdf_page_from_fz_page(ctx, page);
        if (!ppage) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf page");

        for (pdf_annot *annot = pdf_first_annot(ctx, ppage);
             annot;
             annot = pdf_next_annot(ctx, annot), count++) {

            /* 读取注释属性 */
            int type = (int)pdf_annot_type(ctx, annot);
            fz_rect rect = {0, 0, 0, 0};
            int flags = 0;
            const char *contents = "";
            const char *author = "";
            int color_n = 0;
            float color[4] = {0};
            int quad_count = 0;

            /* 仅从字典读属性，不调用 display/bound/synthesis 函数 */
            pdf_obj *aobj = pdf_annot_obj(ctx, annot);
            pdf_obj *robj = pdf_dict_get(ctx, aobj, PDF_NAME(Rect));
            if (pdf_is_array(ctx, robj)) {
                rect.x0 = pdf_to_real(ctx, pdf_array_get(ctx, robj, 0));
                rect.y0 = pdf_to_real(ctx, pdf_array_get(ctx, robj, 1));
                rect.x1 = pdf_to_real(ctx, pdf_array_get(ctx, robj, 2));
                rect.y1 = pdf_to_real(ctx, pdf_array_get(ctx, robj, 3));
            }
            pdf_obj *cobj = pdf_dict_get(ctx, aobj, PDF_NAME(Contents));
            if (pdf_is_string(ctx, cobj)) contents = pdf_to_str_buf(ctx, cobj);
            pdf_obj *color_arr = pdf_dict_get(ctx, aobj, PDF_NAME(C));
            if (pdf_is_array(ctx, color_arr)) {
                color_n = pdf_array_len(ctx, color_arr);
                for (int ci = 0; ci < color_n && ci < 4; ci++)
                    color[ci] = pdf_to_real(ctx, pdf_array_get(ctx, color_arr, ci));
            }
            pdf_obj *qp = pdf_dict_get(ctx, aobj, PDF_NAME(QuadPoints));
            if (pdf_is_array(ctx, qp)) quad_count = pdf_array_len(ctx, qp) / 8;

            /* Highlight/Underline/StrikeOut 没有 Rect，从 QuadPoints 计算包围盒 */
            if (rect.x0 == 0 && rect.y0 == 0 && rect.x1 == 0 && rect.y1 == 0
                && pdf_is_array(ctx, qp) && pdf_array_len(ctx, qp) >= 8) {
                float min_x = 1e30f, min_y = 1e30f, max_x = -1e30f, max_y = -1e30f;
                int total_pts = pdf_array_len(ctx, qp);
                for (int pi = 0; pi < total_pts; pi += 2) {
                    float px = pdf_to_real(ctx, pdf_array_get(ctx, qp, pi));
                    float py = pdf_to_real(ctx, pdf_array_get(ctx, qp, pi + 1));
                    if (px < min_x) min_x = px;
                    if (py < min_y) min_y = py;
                    if (px > max_x) max_x = px;
                    if (py > max_y) max_y = py;
                }
                rect.x0 = min_x; rect.y0 = min_y;
                rect.x1 = max_x; rect.y1 = max_y;
            }

            int contents_len = contents ? (int)strlen(contents) : 0;
            int author_len = author ? (int)strlen(author) : 0;

            /* header */
            gb_put(ctx, &gb, &type, 4);
            gb_put(ctx, &gb, &rect.x0, 4);
            gb_put(ctx, &gb, &rect.y0, 4);
            gb_put(ctx, &gb, &rect.x1, 4);
            gb_put(ctx, &gb, &rect.y1, 4);
            gb_put(ctx, &gb, &flags, 4);
            gb_put(ctx, &gb, &color_n, 4);
            gb_put(ctx, &gb, color, 16);
            gb_put(ctx, &gb, &contents_len, 4);
            gb_put(ctx, &gb, &author_len, 4);
            gb_put(ctx, &gb, &quad_count, 4);

            /* variable-length fields */
            if (contents_len > 0) gb_put(ctx, &gb, contents, (size_t)contents_len);
            if (author_len > 0) gb_put(ctx, &gb, author, (size_t)author_len);

            /* quad points — 直接从字典读取，避免 pdf_annot_quad_point 的副作用 */
            pdf_obj *qp_arr = pdf_dict_get(ctx, aobj, PDF_NAME(QuadPoints));
            if (pdf_is_array(ctx, qp_arr)) {
                for (int qi = 0; qi < quad_count * 8; qi++) {
                    float v = pdf_to_real(ctx, pdf_array_get(ctx, qp_arr, qi));
                    gb_put(ctx, &gb, &v, 4);
                }
            }

            /* 不 drop —— pdf_first_annot/pdf_next_annot 不 keep，
               annot 由 page 持有，不应在此释放。 */
        }
    }
    fz_catch(ctx) {
        set_error("get annotations: %s", fz_caught_message(ctx));
        if (gb.data) fz_free(ctx, gb.data);
        return -1;
    }

    if (count == 0) {
        if (gb.data) fz_free(ctx, gb.data);
        return 0;
    }

    *out = (char *)gb.data;
    *out_len = gb.len;
    *total_n = count;
    return 0;
}

int mupdf_safe_create_annot(fz_context *ctx, fz_document *doc, fz_page *page,
                            int annot_type,
                            const float *quads, int quad_count,
                            int color_n, const float *color,
                            const char *contents,
                            int *out_index) {
    if (!ctx || !doc || !page) return -1;

    mupdf_clear_error();
    fz_try(ctx) {
        pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
        if (!pdf) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf document");
        pdf_page *ppage = pdf_page_from_fz_page(ctx, page);
        if (!ppage) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf page");

        /* 计算新注释的 index（创建前的注释数量） */
        if (out_index) {
            int idx = 0;
            for (pdf_annot *a = pdf_first_annot(ctx, ppage); a; a = pdf_next_annot(ctx, a)) {
                idx++;
            }
            *out_index = idx;
        }

        /* 使用 pdf_create_annot_raw 而非 pdf_create_annot，避免
           pdf_begin_operation / pdf_end_operation 导致的链表损坏。 */
        pdf_annot *annot = pdf_create_annot_raw(ctx, ppage, (enum pdf_annot_type)annot_type);

        if (quads && quad_count > 0) {
            for (int i = 0; i < quad_count; i++) {
                const float *q = &quads[i * 8];
                fz_quad quad;
                quad.ul.x = q[0]; quad.ul.y = q[1];
                quad.ur.x = q[2]; quad.ur.y = q[3];
                quad.ll.x = q[4]; quad.ll.y = q[5];
                quad.lr.x = q[6]; quad.lr.y = q[7];
                pdf_add_annot_quad_point(ctx, annot, quad);
            }
        }

        if (color && color_n > 0) {
            pdf_set_annot_color(ctx, annot, color_n, color);
        }

        if (contents && contents[0]) {
            pdf_set_annot_contents(ctx, annot, contents);
        }

        /* 不调用 pdf_update_annot —— appearance stream 在 doc.save() 时自动合成。 */

        /* pdf_create_annot_raw 返回 refcount=2（内部 keep + page 持有）。
           drop 一次使 refcount 降回 1（page 持有）。 */
        pdf_drop_annot(ctx, annot);
    }
    fz_catch(ctx) {
        set_error("create annot: %s", fz_caught_message(ctx));
        return -1;
    }
    return 0;
}

int mupdf_safe_delete_annot(fz_context *ctx, fz_document *doc, fz_page *page, int index) {
    if (!ctx || !doc || !page || index < 0) return -1;

    mupdf_clear_error();
    fz_try(ctx) {
        pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
        if (!pdf) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf document");
        pdf_page *ppage = pdf_page_from_fz_page(ctx, page);
        if (!ppage) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf page");

        pdf_annot *annot = pdf_first_annot(ctx, ppage);
        for (int i = 0; i < index && annot; i++) {
            annot = pdf_next_annot(ctx, annot);
        }
        if (!annot) fz_throw(ctx, FZ_ERROR_GENERIC, "annot index out of range");
        pdf_delete_annot(ctx, ppage, annot);
    }
    fz_catch(ctx) {
        set_error("delete annot: %s", fz_caught_message(ctx));
        return -1;
    }
    return 0;
}

int mupdf_safe_set_annot_rect(fz_context *ctx, fz_document *doc, fz_page *page,
                              int index, const float *rect) {
    if (!ctx || !doc || !page || !rect || index < 0) return -1;

    mupdf_clear_error();
    fz_try(ctx) {
        pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
        if (!pdf) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf document");
        pdf_page *ppage = pdf_page_from_fz_page(ctx, page);
        if (!ppage) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf page");

        pdf_annot *annot = pdf_first_annot(ctx, ppage);
        for (int i = 0; i < index && annot; i++) {
            annot = pdf_next_annot(ctx, annot);
        }
        if (!annot) fz_throw(ctx, FZ_ERROR_GENERIC, "annot index out of range");

        fz_rect r;
        r.x0 = rect[0]; r.y0 = rect[1];
        r.x1 = rect[2]; r.y1 = rect[3];
        pdf_set_annot_rect(ctx, annot, r);
        pdf_update_annot(ctx, annot);
    }
    fz_catch(ctx) {
        set_error("set annot rect: %s", fz_caught_message(ctx));
        return -1;
    }
    return 0;
}

/* ====================================================================
 * 大纲写入（Phase 5c）
 *
 * 直接操作 PDF outline 对象构建树结构，不使用 fz_outline_iterator
 * （iterator 的 up/down 在 root-child ↔ root 转换时有设计缺陷）。
 * ==================================================================== */

static void clear_outline_entries(fz_context *ctx, pdf_obj *outlines) {
    pdf_obj *child = pdf_dict_get(ctx, outlines, PDF_NAME(First));
    while (child) {
        if (pdf_dict_get(ctx, child, PDF_NAME(First))) {
            clear_outline_entries(ctx, child);
        }
        pdf_obj *next = pdf_dict_get(ctx, child, PDF_NAME(Next));
        pdf_dict_del(ctx, child, PDF_NAME(Parent));
        pdf_dict_del(ctx, child, PDF_NAME(First));
        pdf_dict_del(ctx, child, PDF_NAME(Last));
        pdf_dict_del(ctx, child, PDF_NAME(Next));
        pdf_dict_del(ctx, child, PDF_NAME(Prev));
        child = next;
    }
    pdf_dict_del(ctx, outlines, PDF_NAME(First));
    pdf_dict_del(ctx, outlines, PDF_NAME(Last));
    pdf_dict_del(ctx, outlines, PDF_NAME(Count));
}

int mupdf_safe_set_toc(fz_context *ctx, fz_document *doc,
                       const int *levels, const int *pages,
                       const char **titles, int count) {
    if (!ctx || !doc) return -1;
    if (count < 0) return -1;
    if (count > 0 && (!levels || !pages || !titles)) return -1;

    mupdf_clear_error();
    fz_try(ctx) {
        pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
        if (!pdf) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf document");

        int page_count = fz_count_pages(ctx, doc);

        pdf_obj *trailer = pdf_trailer(ctx, pdf);
        if (!trailer) fz_throw(ctx, FZ_ERROR_GENERIC, "pdf_trailer returned NULL");

        pdf_obj *catalog = pdf_dict_get(ctx, trailer, PDF_NAME(Root));
        if (!catalog) fz_throw(ctx, FZ_ERROR_GENERIC, "catalog (Root) not found in trailer");

        /* 清空现有大纲 */
        pdf_obj *outlines = pdf_dict_get(ctx, catalog, PDF_NAME(Outlines));
        if (pdf_is_dict(ctx, outlines)) {
            clear_outline_entries(ctx, outlines);
        }

        if (count == 0) {
            /* 清空：移除 catalog 中的 /Outlines 引用 */
            if (outlines) pdf_dict_del(ctx, catalog, PDF_NAME(Outlines));
            fz_always(ctx) {}
            fz_catch(ctx) {}
            return 0;
        }

        /* 创建或复用 /Outlines 字典 */
        if (!pdf_is_dict(ctx, outlines)) {
            outlines = pdf_add_new_dict(ctx, pdf, 4);
            pdf_dict_put(ctx, catalog, PDF_NAME(Outlines), outlines);
            pdf_dict_put(ctx, outlines, PDF_NAME(Type), PDF_NAME(Outlines));
        }

        /*
         * 逐条构建 outline 树。
         *
         * stack[level] = 该 level 当前最后一个 outline entry（用于链接 Next）。
         * parent_of[level] = 该 level 的父 pdf_obj（用于设置 Parent 和 First/Last）。
         *
         * level 从 1 开始。level 1 的父是 outlines 字典本身。
         */
        #define MAX_TOC_DEPTH 16
        pdf_obj *stack[MAX_TOC_DEPTH + 1] = {0};
        pdf_obj *parent_of[MAX_TOC_DEPTH + 1] = {0};
        parent_of[1] = outlines; /* level 1 条目的父是 /Outlines 字典 */

        for (int i = 0; i < count; i++) {
            int level = levels[i];
            if (level < 1) level = 1;
            if (level > MAX_TOC_DEPTH) level = MAX_TOC_DEPTH;

            int page_num = pages[i];
            if (page_num < 1 || page_num > page_count) {
                fz_throw(ctx, FZ_ERROR_GENERIC,
                    "page number %d out of range (1..%d)", page_num, page_count);
            }

            /* 创建 outline entry 对象 */
            pdf_obj *entry = pdf_add_new_dict(ctx, pdf, 8);

            /* Title */
            pdf_dict_put_text_string(ctx, entry, PDF_NAME(Title), titles[i]);

            /* Dest: [page_obj /Fit] */
            pdf_obj *page_obj = pdf_lookup_page_obj(ctx, pdf, page_num - 1);
            if (page_obj) {
                pdf_obj *dest = pdf_add_new_array(ctx, pdf, 2);
                pdf_array_push(ctx, dest, page_obj);
                pdf_array_push(ctx, dest, PDF_NAME(Fit));
                pdf_dict_put(ctx, entry, PDF_NAME(Dest), dest);
            }

            /* 链接到树结构 */
            pdf_obj *parent = parent_of[level];
            pdf_dict_put(ctx, entry, PDF_NAME(Parent), parent);

            pdf_obj *last_child = stack[level];
            if (!last_child) {
                /* 该 level 尚无条目 → 设为父的 First */
                pdf_dict_put(ctx, parent, PDF_NAME(First), entry);
            } else {
                /* 链接到同级前一条目 */
                pdf_dict_put(ctx, last_child, PDF_NAME(Next), entry);
                pdf_dict_put(ctx, entry, PDF_NAME(Prev), last_child);
            }
            pdf_dict_put(ctx, parent, PDF_NAME(Last), entry);

            /* 更新栈：当前 level 指向新条目，子 level 以新条目为父 */
            stack[level] = entry;
            if (level + 1 <= MAX_TOC_DEPTH)
                parent_of[level + 1] = entry;
            /* 清除更深层级的栈（新条目尚无子条目） */
            for (int d = level + 1; d <= MAX_TOC_DEPTH; d++) {
                stack[d] = NULL;
            }
        }
        #undef MAX_TOC_DEPTH
    }
    fz_always(ctx) {}
    fz_catch(ctx) {
        set_error("set toc: %s", fz_caught_message(ctx));
        return -1;
    }
    return 0;
}

/* ====================================================================
 * 命名目标解析（Phase 5d）
 * ==================================================================== */

int mupdf_safe_resolve_names(fz_context *ctx, fz_document *doc,
                             char **out, size_t *out_len, int *total_n) {
    if (!ctx || !doc || !out || !out_len || !total_n) return -1;
    *out = NULL; *out_len = 0; *total_n = 0;

    growbuf gb = {0};
    int count = 0;
    pdf_obj *tree = NULL;

    mupdf_clear_error();
    fz_try(ctx) {
        pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
        if (!pdf) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf document");

        tree = pdf_load_name_tree(ctx, pdf, PDF_NAME(Dests));
        if (!tree || !pdf_is_dict(ctx, tree)) {
            /* 无命名目标 */
            fz_always(ctx) {
                if (tree) { pdf_drop_obj(ctx, tree); tree = NULL; }
            }
            fz_catch(ctx) {}
            return 0;
        }

        int len = pdf_dict_len(ctx, tree);
        for (int i = 0; i < len; i++) {
            pdf_obj *key = pdf_dict_get_key(ctx, tree, i);
            pdf_obj *val = pdf_dict_get_val(ctx, tree, i);
            if (!key) continue;

            const char *name = pdf_to_name(ctx, key);
            int name_len = name ? (int)strlen(name) : 0;

            /* 使用 pdf_resolve_link_dest 解析目标 */
            char uri[1024];
            snprintf(uri, sizeof(uri), "#nameddest=%s", name ? name : "");
            fz_link_dest dest = pdf_resolve_link_dest(ctx, pdf, uri);

            int page = dest.loc.page; /* 0-based, -1 if unresolvable */
            float x = dest.x;
            float y = dest.y;

            /* 写入 buffer: [name_len][name bytes][page][x][y] */
            gb_put(ctx, &gb, &name_len, 4);
            if (name_len > 0) gb_put(ctx, &gb, name, (size_t)name_len);
            gb_put(ctx, &gb, &page, 4);
            gb_put(ctx, &gb, &x, 4);
            gb_put(ctx, &gb, &y, 4);
            count++;
        }
    }
    fz_always(ctx) {
        if (tree) pdf_drop_obj(ctx, tree);
    }
    fz_catch(ctx) {
        set_error("resolve names: %s", fz_caught_message(ctx));
        if (gb.data) fz_free(ctx, gb.data);
        return -1;
    }

    if (count == 0) {
        if (gb.data) fz_free(ctx, gb.data);
        return 0;
    }

    *out = (char *)gb.data;
    *out_len = gb.len;
    *total_n = count;
    return 0;
}

/* ====================================================================
 * 内存释放
 * ==================================================================== */

void mupdf_free(void *ptr) {
    if (ptr) free(ptr);
}
