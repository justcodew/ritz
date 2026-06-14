"""ritz — fitz, reforged in Rust.

PyMuPDF 兼容 API 的 Rust 实现（基于 MuPDF）。

示例:
    import ritz
    doc = ritz.open("file.pdf")
    page = doc[0]
    print(page.get_text())
    pix = page.get_pixmap(dpi=144)
    pix.tobytes("png")  # PNG bytes
"""

from ._ritz import (
    Document,
    Page,
    PdfValue,
    Pixmap,
    __version__,
    process_documents,
)

# 与 PyMuPDF 兼容：顶层 open() 函数。
open = Document

__all__ = [
    "Document",
    "Page",
    "PdfValue",
    "Pixmap",
    "open",
    "process_documents",
    "__version__",
]
