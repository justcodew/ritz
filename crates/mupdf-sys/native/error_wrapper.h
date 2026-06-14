#ifndef MUPDF_ERROR_WRAPPER_H
#define MUPDF_ERROR_WRAPPER_H

/*
 * C 错误包装层
 *
 * 所有暴露给 Rust 的函数都在内部用 fz_try/fz_catch 捕获 MuPDF 异常，
 * 防止 longjmp 穿透 Rust 栈帧导致 Drop 不执行（UB）。
 *
 * 返回值约定：
 *   - 返回指针的函数：成功返回指针，失败返回 NULL，错误消息见 mupdf_last_error()
 *   - 返回 int 的函数：0 = 成功，-1 = 失败（页数函数返回 -1 表错误，否则返回页数）
 *   - 返回结构体的函数：按值返回，失败时返回零值结构
 */

#include <stddef.h>
#include <mupdf/fitz.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- 错误缓冲区（线程局部） ---- */
const char *mupdf_last_error(void);
void mupdf_clear_error(void);
/* 内部 helper：格式化写入 last_error（同时供 mupdf_extensions.c 使用）。 */
void set_error(const char *fmt, ...);

/* ---- 上下文 ---- */
fz_context *mupdf_safe_new_context(void);
fz_context *mupdf_safe_clone_context(fz_context *ctx);
void mupdf_safe_drop_context(fz_context *ctx);

/* ---- 文档 ---- */
fz_document *mupdf_safe_open_document(fz_context *ctx, const char *filename);
int mupdf_safe_open_document_with_stream(
    fz_context *ctx,
    const unsigned char *data, size_t len,
    const char *mimetype,
    fz_document **out);
void mupdf_safe_drop_document(fz_context *ctx, fz_document *doc);
int mupdf_safe_count_pages(fz_context *ctx, fz_document *doc);
int mupdf_safe_needs_password(fz_context *ctx, fz_document *doc);
int mupdf_safe_authenticate_password(fz_context *ctx, fz_document *doc, const char *pw);
/* metadata: 返回写入 buf 的字节数（不含结尾 0），-1 表失败 */
int mupdf_safe_lookup_metadata(fz_context *ctx, fz_document *doc,
                               const char *key, char *buf, int len);

/* ---- 页面 ---- */
fz_page *mupdf_safe_load_page(fz_context *ctx, fz_document *doc, int number);
void mupdf_safe_drop_page(fz_context *ctx, fz_page *page);
fz_rect mupdf_safe_bound_page(fz_context *ctx, fz_page *page);
/* box_type: 0=media,1=crop,2=bleed,3=trim,4=art */
fz_rect mupdf_safe_bound_page_box(fz_context *ctx, fz_page *page, int box_type);
/* rotation: 仅 PDF，读 /Rotate；非 PDF 或失败返回 0 */
int mupdf_safe_page_rotation(fz_context *ctx, fz_document *doc, fz_page *page);

/* ---- 结构化文本（stext） ---- */
int mupdf_safe_new_stext_page(fz_context *ctx, fz_page *page, fz_stext_page **out);
void mupdf_safe_drop_stext_page(fz_context *ctx, fz_stext_page *stpage);
/* 将 stext 页转为文本/html/xml，输出 malloc 的 buffer，调用方须 mupdf_free */
int mupdf_safe_stext_to_text(fz_context *ctx, fz_stext_page *stpage,
                             char **out, size_t *out_len);
int mupdf_safe_stext_to_html(fz_context *ctx, fz_stext_page *stpage,
                             char **out, size_t *out_len);
int mupdf_safe_stext_to_xml(fz_context *ctx, fz_stext_page *stpage,
                            char **out, size_t *out_len);
int mupdf_safe_stext_to_xhtml(fz_context *ctx, fz_stext_page *stpage,
                              char **out, size_t *out_len);
/* json: scale 是坐标缩放因子（PyMuPDF 用 1.0） */
int mupdf_safe_stext_to_json(fz_context *ctx, fz_stext_page *stpage, float scale,
                             char **out, size_t *out_len);
/*
 * 单词级扁平化：遍历 stext_page 的 block→line→char 链表，按空白分词。
 * 每个单词编码为 [4 float bbox][3 int (block_no,line_no,word_no)][4 int str_len][N utf8 bytes]，
 * 顺序写入 malloc 缓冲区。total_n 写入单词总数。
 * 调用方通过 mupdf_free 释放。
 */
int mupdf_safe_stext_to_words(fz_context *ctx, fz_stext_page *stpage,
                              char **out, size_t *out_len, int *total_n);
/*
 * 块级扁平化：每个 block 输出
 *   [4 float bbox][3 int (block_no, block_type, text_len)][N utf8 bytes]
 * text 块的 text 内多行用 \n 分隔。image 块 text_len=0。
 */
int mupdf_safe_stext_to_blocks(fz_context *ctx, fz_stext_page *stpage,
                               char **out, size_t *out_len, int *total_n);
/*
 * dict 模式：返回 PyMuPDF 兼容的嵌套结构（扁平化为二进制 buffer）。
 *
 * 布局（按字节顺序，little-endian / native）：
 *   page_header:
 *     [block_count: i32]
 *   for each block:
 *     [type: i32]               // 0=text, 1=image, 2=struct(skip)
 *     [bbox: 4 floats]
 *     [line_count: i32]         // 0 for image
 *     for each line:
 *       [bbox: 4 floats]
 *       [wmode: i32]
 *       [dir: 2 floats]
 *       [span_count: i32]
 *       for each span:
 *         [bbox: 4 floats]
 *         [color: i32]          // sRGB int (r<<16 | g<<8 | b)
 *         [flags: i32]
 *         [size: f32]
 *         [font_len: i32][font bytes]
 *         [text_len: i32][text bytes]
 *
 *
 * include_chars != 0 时（rawdict 模式），在每个 span 的 text 之后追加：
 *   [char_count: i32]
 *   for each char:
 *     [bbox: 4 floats]   // char quad 包围盒
 *     [origin: 2 floats] // baseline 上的参考点
 *     [utf8_len: i32][utf8 bytes]
 * 对应 PyMuPDF rawdict 中 span["chars"] = [{"bbox":.., "origin":.., "c":..}]。
 */
int mupdf_safe_stext_to_dict(fz_context *ctx, fz_stext_page *stpage,
                             int include_chars, char **out, size_t *out_len);

/* ---- 像素图（pixmap）渲染 ---- */
/* zoom=1.0 为原始 DPI(72)，alpha=1 带透明通道 */
int mupdf_safe_render_pixmap(fz_context *ctx, fz_page *page,
                             float zoom, int alpha, fz_pixmap **out);
void mupdf_safe_drop_pixmap(fz_context *ctx, fz_pixmap *pix);
int mupdf_safe_pixmap_width(fz_context *ctx, fz_pixmap *pix);
int mupdf_safe_pixmap_height(fz_context *ctx, fz_pixmap *pix);
int mupdf_safe_pixmap_stride(fz_context *ctx, fz_pixmap *pix);
int mupdf_safe_pixmap_components(fz_context *ctx, fz_pixmap *pix);
/* 返回借用指针，pixmap 存活期间有效 */
unsigned char *mupdf_safe_pixmap_samples(fz_context *ctx, fz_pixmap *pix);
/* 编码为 PNG，输出 malloc 的 buffer，调用方须 mupdf_free */
int mupdf_safe_pixmap_to_png(fz_context *ctx, fz_pixmap *pix,
                             unsigned char **out, size_t *out_len);

/* ---- 链接（links） ---- */
/*
 * 遍历 page 的 fz_link 链表，扁平化到 malloc 数组返回。
 * 每条链接 = 1 个 fz_rect(16B) + NUL 结尾 uri 字符串（连续存放）。
 * 返回 0 成功（out/total_n/out_bytes 写好），-1 失败。
 */
int mupdf_safe_load_links(fz_context *ctx, fz_page *page,
                          char **out, size_t *out_len, int *total_n);

/* ---- 图片提取 ---- */
/*
 * 遍历页面上所有图片（按 fz_image* 去重）。
 * 二进制布局：[image_count: i32] then per image:
 *   [xref: i32]                  // PDF 间接引用号；0 表示非 PDF 或未找到
 *   [bbox: 4 floats][w: i32][h: i32][bpc: i32]
 *   [cs_len: i32][cs bytes][imagemask: i32][has_data: i32]
 *   if has_data: [ext_len: i32][ext bytes][data_len: i32][data bytes]
 * include_data != 0 时返回原始压缩字节（JPEG/PNG/JPX）或 PNG fallback。
 * doc 参数：传入父 fz_document* 以查询 xref（仅 PDF 有效，传 NULL 不查 xref）。
 */
int mupdf_safe_get_images(fz_context *ctx, fz_document *doc, fz_page *page,
                          int include_data,
                          char **out, size_t *out_len, int *total_n);

/* ---- 内存释放 ---- */
void mupdf_free(void *ptr);

#ifdef __cplusplus
}
#endif

#endif /* MUPDF_ERROR_WRAPPER_H */
