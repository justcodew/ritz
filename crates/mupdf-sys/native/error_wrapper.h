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

/* ---- 内存释放 ---- */
void mupdf_free(void *ptr);

#ifdef __cplusplus
}
#endif

#endif /* MUPDF_ERROR_WRAPPER_H */
