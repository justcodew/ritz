#!/usr/bin/env python3
"""生成大页数 PDF fixture，用于批量/内存基准。

ritz 当前无 PDF 写出能力（Phase 5 待实现），借用 PyMuPDF 做一次性 fixture 生成
是合理且明确的工程妥协——fixture 只生成一次，不入版本库。

输出：
  /tmp/ritz_big_200.pdf   — 200 页（批量基准，放大 N×FFI 累积优势）
  /tmp/ritz_big_1000.pdf  — 1000 页（plan 4.2 内存验证）

策略：以 sample.pdf 第 0 页为模板，复制 N 次插入新文档。sample 页含文本+图，
能代表真实负载。
"""

import sys
from pathlib import Path

import fitz  # PyMuPDF

SAMPLE = Path(__file__).resolve().parents[1] / "crates" / "mupdf" / "tests" / "samples" / "sample.pdf"

OUT_200 = Path("/tmp/ritz_big_200.pdf")
OUT_1000 = Path("/tmp/ritz_big_1000.pdf")
OUT_JPEG = Path("/tmp/ritz_jpeg_heavy.pdf")  # JPEG-heavy：每页 1 张 JPEG，验证原始字节路径


def make(src_pdf: Path, out_pdf: Path, n_pages: int) -> None:
    if out_pdf.exists():
        sz = out_pdf.stat().st_size
        # 验证页数
        d = fitz.open(str(out_pdf))
        actual = len(d)
        d.close()
        if actual == n_pages:
            print(f"  exists & valid: {out_pdf} ({sz/1024/1024:.1f} MB, {actual} pages)")
            return
        print(f"  exists but wrong page count ({actual} != {n_pages}), regenerating")
        out_pdf.unlink()

    src = fitz.open(str(src_pdf))
    if len(src) == 0:
        print(f"ERROR: source {src_pdf} has 0 pages", file=sys.stderr)
        sys.exit(1)
    # 用 insert_pdf 复制（比逐页 insert 更快）
    out = fitz.open()
    # 循环插入 src 全部内容 n_pages // src_pages 次，余数补足
    src_pages = len(src)
    full_copies = n_pages // src_pages
    remainder = n_pages % src_pages
    for _ in range(full_copies):
        out.insert_pdf(src)
    if remainder:
        # 插入前 remainder 页
        tmp = fitz.open()
        tmp.insert_pdf(src, from_page=0, to_page=remainder - 1)
        out.insert_pdf(tmp)
        tmp.close()
    out.save(str(out_pdf))
    out.close()
    src.close()
    sz = out_pdf.stat().st_size
    print(f"  generated: {out_pdf} ({sz/1024/1024:.1f} MB, {n_pages} pages)")


def make_jpeg_heavy(out_pdf: Path, n_pages: int = 20) -> None:
    """造一个每页含 1 张 JPEG 的 PDF（DCTDecode filter）。
    用于验证 ritz 原始字节优化路径（跳过 decode+re-encode）。
    """
    import glob
    if out_pdf.exists():
        d = fitz.open(str(out_pdf))
        actual = len(d)
        d.close()
        if actual == n_pages:
            print(f"  exists & valid: {out_pdf} ({out_pdf.stat().st_size/1024/1024:.1f} MB, {actual} pages)")
            return

    src_dir = Path(__file__).resolve().parents[1] / "vendor" / "mupdf" / "thirdparty"
    jpgs = sorted(glob.glob(str(src_dir / "leptonica" / "**" / "*.jpg"), recursive=True))
    jpgs += sorted(glob.glob(str(src_dir / "libjpeg" / "*.jpg")))
    if len(jpgs) < 3:
        print(f"  WARN: only {len(jpgs)} JPEGs found, need >= 3")
        if not jpgs:
            return

    out = fitz.open()
    for i in range(n_pages):
        jpg = jpgs[i % len(jpgs)]
        img = fitz.open(jpg)  # 打开 JPEG 作为 1 页文档
        rect = img[0].rect
        page = out.new_page(width=rect.width, height=rect.height)
        page.insert_image(rect, filename=jpg)
        img.close()
    out.save(str(out_pdf))
    out.close()
    print(f"  generated: {out_pdf} ({out_pdf.stat().st_size/1024/1024:.1f} MB, {n_pages} pages, all JPEG)")


def main():
    if not SAMPLE.exists():
        print(f"ERROR: sample PDF not found: {SAMPLE}", file=sys.stderr)
        sys.exit(1)
    print(f"source: {SAMPLE} ({SAMPLE.stat().st_size/1024:.1f} KB)")
    make(SAMPLE, OUT_200, 200)
    make(SAMPLE, OUT_1000, 1000)
    make_jpeg_heavy(OUT_JPEG, 20)


if __name__ == "__main__":
    main()
