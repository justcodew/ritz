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
 */
int mupdf_safe_stext_to_words(fz_context *ctx, fz_stext_page *stpage,
                              char **out, size_t *out_len, int *total_n) {
    if (!ctx || !stpage || !out || !out_len || !total_n) return -1;
    *out = NULL;
    *out_len = 0;
    *total_n = 0;

    /* 第一遍：计算总长度和单词数 */
    size_t total = 0;
    int n_words = 0;
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
            size_t word_bytes = 0;
            for (fz_stext_char *ch = line->first_char; ch; ch = ch->next) {
                if (ch->c == ' ' || ch->c == '\t') {
                    if (in_word) {
                        total += 32 + word_bytes;
                        n_words++;
                        word_no++;
                        in_word = 0;
                        word_bytes = 0;
                    }
                } else {
                    /* utf8 编码长度 */
                    int len = 0;
                    fz_try(ctx) {
                        char tmp[8];
                        len = fz_runetochar(tmp, ch->c);
                    }
                    fz_catch(ctx) {
                        len = 0;
                    }
                    word_bytes += len;
                    in_word = 1;
                }
            }
            if (in_word) {
                total += 32 + word_bytes;
                n_words++;
                word_no++;
            }
            line_no++;
        }
        block_no++;
    }

    if (n_words == 0) {
        return 0;
    }

    char *buf = (char *)malloc(total);
    if (!buf) {
        set_error("stext words: out of memory (%zu bytes)", total);
        return -1;
    }

    /* 第二遍：写入数据 */
    char *p = buf;
    block_no = 0;
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
            char *word_start = NULL; /* 指向当前 word 在 buf 中的 utf8 起点 */

            /* 预留 32 字节头，再写 utf8 */
            char *header = p;
            p += 32;
            word_start = p;

            for (fz_stext_char *ch = line->first_char; ch; ch = ch->next) {
                if (ch->c == ' ' || ch->c == '\t') {
                    if (in_word) {
                        /* 收尾当前 word */
                        memcpy(header,     &wx0, 4);
                        memcpy(header + 4, &wy0, 4);
                        memcpy(header + 8, &wx1, 4);
                        memcpy(header + 12, &wy1, 4);
                        int hdr_block = block_no, hdr_line = line_no, hdr_word = word_no;
                        int hdr_len = (int)word_bytes;
                        memcpy(header + 16, &hdr_block, 4);
                        memcpy(header + 20, &hdr_line, 4);
                        memcpy(header + 24, &hdr_word, 4);
                        memcpy(header + 28, &hdr_len, 4);
                        /* 准备下一 word 的 header */
                        header = p;
                        p += 32;
                        word_start = p;
                        word_no++;
                        in_word = 0;
                        word_bytes = 0;
                        wx0 = wy0 = wx1 = wy1 = 0;
                    }
                } else {
                    /* 累积 bbox 并写 utf8 */
                    fz_quad q = ch->quad;
                    float cx0 = q.ul.x, cy0 = q.ul.y;
                    float cx1 = q.lr.x, cy1 = q.lr.y;
                    /* 取四点包围盒 */
                    if (q.ll.x < cx0) cx0 = q.ll.x;
                    if (q.ur.x > cx1) cx1 = q.ur.x;
                    if (q.ur.y < cy0) cy0 = q.ur.y;
                    if (q.ll.y > cy1) cy1 = q.ll.y;
                    if (!in_word) {
                        wx0 = cx0; wy0 = cy0; wx1 = cx1; wy1 = cy1;
                    } else {
                        if (cx0 < wx0) wx0 = cx0;
                        if (cy0 < wy0) wy0 = cy0;
                        if (cx1 > wx1) wx1 = cx1;
                        if (cy1 > wy1) wy1 = cy1;
                    }
                    char tmp[8];
                    int len = fz_runetochar(tmp, ch->c);
                    memcpy(p, tmp, len);
                    p += len;
                    word_bytes += len;
                    in_word = 1;
                }
            }
            if (in_word) {
                memcpy(header,     &wx0, 4);
                memcpy(header + 4, &wy0, 4);
                memcpy(header + 8, &wx1, 4);
                memcpy(header + 12, &wy1, 4);
                int hdr_block = block_no, hdr_line = line_no, hdr_word = word_no;
                int hdr_len = (int)word_bytes;
                memcpy(header + 16, &hdr_block, 4);
                memcpy(header + 20, &hdr_line, 4);
                memcpy(header + 24, &hdr_word, 4);
                memcpy(header + 28, &hdr_len, 4);
                /* 末尾 word 不再预留 header */
            } else {
                /* 没用到预留的 header，回滚 */
                p -= 32;
            }
            line_no++;
        }
        block_no++;
    }

    *out = buf;
    *out_len = total;
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

    /* 第一遍：统计总长度和块数 */
    size_t total = 0;
    int n_blocks = 0;
    int block_no = 0;
    for (fz_stext_block *blk = stpage->first_block; blk; blk = blk->next) {
        size_t text_bytes = 0;
        if (blk->type == FZ_STEXT_BLOCK_TEXT) {
            int line_idx = 0;
            for (fz_stext_line *line = blk->u.t.first_line; line; line = line->next) {
                if (line_idx > 0) text_bytes += 1; /* \n */
                for (fz_stext_char *ch = line->first_char; ch; ch = ch->next) {
                    char tmp[8];
                    int len = fz_runetochar(tmp, ch->c);
                    text_bytes += len;
                }
                line_idx++;
            }
        }
        total += 28 + text_bytes;
        n_blocks++;
        block_no++;
    }

    if (n_blocks == 0) {
        return 0;
    }

    char *buf = (char *)malloc(total);
    if (!buf) {
        set_error("stext blocks: out of memory (%zu bytes)", total);
        return -1;
    }

    /* 第二遍：写入 */
    char *p = buf;
    block_no = 0;
    for (fz_stext_block *blk = stpage->first_block; blk; blk = blk->next) {
        /* bbox */
        float coords[4] = { blk->bbox.x0, blk->bbox.y0, blk->bbox.x1, blk->bbox.y1 };
        memcpy(p, coords, sizeof(coords));
        p += 16;
        int bno = block_no;
        int btype = (blk->type == FZ_STEXT_BLOCK_IMAGE) ? 1 : 0;
        /* 先记 text 起点，写完 text 后回头填 text_len */
        memcpy(p, &bno, 4);
        memcpy(p + 4, &btype, 4);
        char *len_slot = p + 8;
        p += 12;

        size_t text_bytes = 0;
        if (blk->type == FZ_STEXT_BLOCK_TEXT) {
            int line_idx = 0;
            for (fz_stext_line *line = blk->u.t.first_line; line; line = line->next) {
                if (line_idx > 0) {
                    *p++ = '\n';
                    text_bytes += 1;
                }
                for (fz_stext_char *ch = line->first_char; ch; ch = ch->next) {
                    char tmp[8];
                    int len = fz_runetochar(tmp, ch->c);
                    memcpy(p, tmp, len);
                    p += len;
                    text_bytes += len;
                }
                line_idx++;
            }
        }
        int hdr_len = (int)text_bytes;
        memcpy(len_slot, &hdr_len, 4);

        block_no++;
    }

    *out = buf;
    *out_len = total;
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

/*
 * 简易动态缓冲区（可按偏移写），避免 fz_buffer 不能原地修改的限制。
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

static void cap_fill_image(fz_context *ctx, fz_device *dev_, fz_image *img,
                           fz_matrix ctm, float alpha, fz_color_params cp) {
    (void)alpha; (void)cp;
    image_capture_device *cap = (image_capture_device *)dev_;
    if (cap->error) return;
    if (img_seen(cap, img)) return;
    if (img_add_seen(ctx, cap, img)) { cap->error = 1; return; }

    fz_try(ctx) {
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
            fz_pixmap *pix = NULL;
            fz_buffer *buf = NULL;
            unsigned char *data = NULL;
            size_t dlen = 0;

            pix = fz_get_pixmap_from_image(ctx, img, NULL, NULL, NULL, NULL);
            if (pix) {
                buf = fz_new_buffer_from_pixmap_as_png(ctx, pix, fz_default_color_params);
                if (buf) {
                    dlen = fz_buffer_extract(ctx, buf, &data);
                    int dl32 = (int)dlen;
                    gb_put(ctx, &cap->gb, &dl32, 4);
                    if (dlen > 0) gb_put(ctx, &cap->gb, data, dlen);
                    if (data) fz_free(ctx, data);
                    fz_drop_buffer(ctx, buf);
                } else {
                    int zero = 0;
                    gb_put(ctx, &cap->gb, &zero, 4);
                }
                fz_drop_pixmap(ctx, pix);
            } else {
                int zero = 0;
                gb_put(ctx, &cap->gb, &zero, 4);
            }
        }
        cap->image_count++;
    }
    fz_catch(ctx) {
        cap->error = 1;
    }
}

int mupdf_safe_get_images(fz_context *ctx, fz_page *page, int include_data,
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
            gb_data = cap->gb.data;
            gb_len = cap->gb.len;
            gb_count = cap->image_count;
            gb_error = cap->error;
            cap->gb.data = NULL;  /* 防止 drop_device 内部误用（虽然不会） */
            if (cap->seen) {
                fz_free(ctx, cap->seen);
                cap->seen = NULL;
            }
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
 * 内存释放
 * ==================================================================== */

void mupdf_free(void *ptr) {
    if (ptr) free(ptr);
}
