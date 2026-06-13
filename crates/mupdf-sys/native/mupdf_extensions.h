#ifndef MUPDF_EXTENSIONS_H
#define MUPDF_EXTENSIONS_H

/*
 * 批量扩展层（阶段四）
 *
 * 与 error_wrapper.h 不同，这里只放"减少 FFI 往返"的批量扁平化函数。
 * 单次调用处理整本文档，避免 Python 侧 N 次循环调用的开销。
 *
 * 所有函数返回 0 成功，-1 失败（错误见 mupdf_last_error()）。
 */

#include <stddef.h>
#include <mupdf/fitz.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * 批量提取所有页的纯文本。
 *
 * 一次性遍历 [0, page_count) 所有页：
 *   fz_load_page → fz_new_stext_page → fz_print_stext_page_as_text → drop
 * 文本连续写入单个 malloc 缓冲区。
 *
 * 输出：
 *   out_text        —— 连续文本缓冲区（NUL 结尾）
 *   out_text_len     —— 文本字节数（不含结尾 NUL）
 *   out_offsets    —— page_count+1 个 size_t：每页文本起点，最后一个等于总长
 *   out_page_count  —— 实际页数
 *
 * 调用方必须用 mupdf_free() 释放 out_text 和 out_offsets。
 * 任一页失败立即返回 -1，已分配内存释放。
 */
int mupdf_ext_extract_pages_text(fz_context *ctx, fz_document *doc,
                         char **out_text, size_t *out_text_len,
                         size_t **out_offsets, int *out_page_count);

#ifdef __cplusplus
}
#endif

#endif /* MUPDF_EXTENSIONS_H */
