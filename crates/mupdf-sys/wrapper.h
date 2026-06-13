/*
 * bindgen 入口头文件。
 * 只包含我们实际使用的 MuPDF 头，并用 allowlist 限制生成范围。
 */
#include <mupdf/fitz.h>
#include <mupdf/pdf.h>

/* 手写 C 包装层头文件 */
#include "error_wrapper.h"
