#!/usr/bin/env python3
"""ritz vs pdf_oxide vs PyMuPDF 三方对比基准。

测试维度（与 docx/08-comparison-with-pdf-oxide.md 对应）：
  1. 文档打开（小文档 / 大文档）
  2. 单页文本 / 多页全文
  3. 单图提取
  4. 单页渲染（1x / 2x）
  5. 多页渲染
  6. 多文档串行 / 并行
  7. 链接读取

用法：
    cd <ritz repo root>
    source .venv/bin/activate
    pip install pdf_oxide pymupdf
    python benchmarks/bench_vs_pdf_oxide.py
"""

from __future__ import annotations

import multiprocessing as mp
import os
import sys
import time
from pathlib import Path

mp.set_start_method("fork", force=True)

import fitz
import ritz
from pdf_oxide import PdfDocument

# ---------------------------------------------------------------------------
# 测试样本
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parent.parent
SAMPLE = REPO_ROOT / "crates" / "mupdf" / "tests" / "samples" / "sample.pdf"
ARXIV = Path(
    "/Users/xiongzhaolong/Downloads/claude-pro/code_2026/"
    "arxiv_2603.09677_translated.pdf"
)
IMAGE_PDF = REPO_ROOT / "vendor" / "mupdf" / "thirdparty" / "extract" / "test" / "text_graphic_image.pdf"

for p in (SAMPLE, ARXIV):
    if not p.exists():
        sys.exit(f"missing test pdf: {p}")

# ---------------------------------------------------------------------------
# 工具
# ---------------------------------------------------------------------------

def bench(fn, n: int = 10, warmup: int = 2) -> float:
    """返回 mean ms。"""
    for _ in range(warmup):
        fn()
    t0 = time.perf_counter()
    for _ in range(n):
        fn()
    return (time.perf_counter() - t0) / n * 1000


def fmt_ms(x: float | None) -> str:
    if x is None:
        return "    —    "
    if x < 10:
        return f"{x:7.3f} ms"
    return f"{x:7.1f} ms"


def ratio(other: float | None, ritz_t: float) -> str:
    if other is None or ritz_t == 0:
        return "  —  "
    r = other / ritz_t
    flag = "✅" if r >= 1 else "❌"
    return f"{r:5.2f}x {flag}"


def proc_text(p: str) -> list[str]:
    d = fitz.open(p)
    return [d[i].get_text("text") for i in range(len(d))]


# ---------------------------------------------------------------------------
# 测试场景
# ---------------------------------------------------------------------------

def s_open_small():
    ritz_t = bench(lambda: ritz.open(str(SAMPLE)))
    po_t = bench(lambda: PdfDocument(str(SAMPLE)))
    fitz_t = bench(lambda: fitz.open(str(SAMPLE)))
    return ritz_t, po_t, fitz_t


def s_open_big():
    ritz_t = bench(lambda: ritz.open(str(ARXIV)))
    po_t = bench(lambda: PdfDocument(str(ARXIV)), n=5)
    fitz_t = bench(lambda: fitz.open(str(ARXIV)), n=5)
    return ritz_t, po_t, fitz_t


def s_text_single():
    rd = ritz.open(str(SAMPLE))
    fd = fitz.open(str(SAMPLE))
    pod = PdfDocument(str(SAMPLE))

    def po_text():
        for i in range(len(pod)):
            _ = pod[i].text

    ritz_t = bench(lambda: rd[0].get_text("text"))
    po_t = bench(po_text)
    fitz_t = bench(lambda: fd[0].get_text("text"))
    return ritz_t, po_t, fitz_t


def s_text_full():
    rd = ritz.open(str(ARXIV))
    fd = fitz.open(str(ARXIV))
    pod = PdfDocument(str(ARXIV))

    def ritz_all():
        for i in range(len(rd)):
            _ = rd[i].get_text("text")

    def po_all():
        for i in range(len(pod)):
            _ = pod[i].text

    def fitz_all():
        for i in range(len(fd)):
            _ = fd[i].get_text("text")

    ritz_t = bench(ritz_all, n=5)
    po_t = bench(po_all, n=5)
    fitz_t = bench(fitz_all, n=5)
    return ritz_t, po_t, fitz_t


def s_text_batch():
    rd = ritz.open(str(ARXIV))
    ritz_t = bench(lambda: rd.get_text_batch(), n=5)
    return ritz_t, None, None


def s_image_single():
    rd = ritz.open(str(IMAGE_PDF))
    fd = fitz.open(str(IMAGE_PDF))
    pod = PdfDocument(str(IMAGE_PDF))

    def ritz_imgs():
        _ = rd[0].get_images(include_data=True)

    def po_imgs():
        for i in range(len(pod)):
            _ = pod[i].extract_image_bytes()

    def fitz_imgs():
        imgs = fd[0].get_images(full=True)
        for im in imgs:
            _ = fd.extract_image(im[0])

    ritz_t = bench(ritz_imgs)
    po_t = bench(po_imgs)
    fitz_t = bench(fitz_imgs)
    return ritz_t, po_t, fitz_t


def s_render_1x():
    rd = ritz.open(str(SAMPLE))
    fd = fitz.open(str(SAMPLE))
    pod = PdfDocument(str(SAMPLE))

    def po_render():
        pod[0].render()

    ritz_t = bench(lambda: rd[0].get_pixmap())
    po_t = bench(po_render, n=5)
    fitz_t = bench(lambda: fd[0].get_pixmap())
    return ritz_t, po_t, fitz_t


def s_render_2x():
    rd = ritz.open(str(SAMPLE))
    fd = fitz.open(str(SAMPLE))
    pod = PdfDocument(str(SAMPLE))

    def po_render():
        pod[0].render(scale=2.0)

    ritz_t = bench(lambda: rd[0].get_pixmap(zoom=2.0))
    po_t = bench(po_render, n=5)
    fitz_t = bench(lambda: fd[0].get_pixmap(matrix=fitz.Matrix(2, 2)))
    return ritz_t, po_t, fitz_t


def s_render_big():
    rd = ritz.open(str(ARXIV))
    fd = fitz.open(str(ARXIV))
    pod = PdfDocument(str(ARXIV))

    def ritz_all():
        for i in range(len(rd)):
            _ = rd[i].get_pixmap()

    def po_all():
        for i in range(len(pod)):
            _ = pod[i].render()

    def fitz_all():
        for i in range(len(fd)):
            _ = fd[i].get_pixmap()

    ritz_t = bench(ritz_all, n=3, warmup=1)
    po_t = bench(po_all, n=3, warmup=1)
    fitz_t = bench(fitz_all, n=3, warmup=1)
    return ritz_t, po_t, fitz_t


def s_multi_serial():
    paths = [str(ARXIV)] * 8

    def po_serial():
        for p in paths:
            d = PdfDocument(p)
            for i in range(len(d)):
                _ = d[i].text

    def ritz_serial():
        for p in paths:
            d = ritz.open(p)
            _ = d.get_text_batch()

    def fitz_serial():
        for p in paths:
            d = fitz.open(p)
            for i in range(len(d)):
                _ = d[i].get_text("text")

    po_t = _wall(po_serial)
    ritz_t = _wall(ritz_serial)
    fitz_t = _wall(fitz_serial)
    return ritz_t, po_t, fitz_t


def s_multi_parallel():
    paths = [str(ARXIV)] * 8

    ritz_t = _wall(lambda: ritz.process_documents(paths))
    fitz_t = _wall(lambda: _mp_run(paths))
    return ritz_t, None, fitz_t


def _wall(fn) -> float:
    fn()  # warmup
    t0 = time.perf_counter()
    fn()
    return (time.perf_counter() - t0) * 1000


def _mp_run(paths):
    with mp.Pool(8) as pool:
        pool.map(proc_text, paths)


def s_links():
    rd = ritz.open(str(SAMPLE))
    fd = fitz.open(str(SAMPLE))
    ritz_t = bench(lambda: rd[0].get_links(), n=100)
    fitz_t = bench(lambda: fd[0].get_links(), n=100)
    return ritz_t, None, fitz_t


# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------

SCENARIOS = [
    ("文档打开 sample (1 页)",          s_open_small),
    ("文档打开 arxiv (43 页)",          s_open_big),
    ("单页文本 sample",                 s_text_single),
    ("arxiv 43 页全文本",               s_text_full),
    ("arxiv 全文本（batch 一次调用）",  s_text_batch),
    ("单图提取 (1 JPEG)",               s_image_single),
    ("单页渲染 1x sample",              s_render_1x),
    ("单页渲染 2x sample",              s_render_2x),
    ("arxiv 43 页全渲染 1x",            s_render_big),
    ("8 文档串行文本",                  s_multi_serial),
    ("8 文档并行文本",                  s_multi_parallel),
    ("链接读取 sample",                 s_links),
]


def main():
    print("=" * 80)
    print("ritz vs pdf_oxide vs PyMuPDF")
    print(f"sample:   {SAMPLE}")
    print(f"arxiv:    {ARXIV} ({ARXIV.stat().st_size // 1024 // 1024} MB)")
    if IMAGE_PDF.exists():
        print(f"image:    {IMAGE_PDF}")
    print("=" * 80)

    print(f"\n{'场景':<32} | {'ritz':>10} | {'pdf_oxide':>10} | {'PyMuPDF':>10} | "
          f"{'ritz vs pdf_oxide':>16} | {'ritz vs PyMuPDF':>16}")
    print("-" * 110)

    for name, fn in SCENARIOS:
        try:
            ritz_t, po_t, fitz_t = fn()
        except Exception as e:
            print(f"{name:<32} | ERROR: {e}")
            continue
        print(
            f"{name:<32} | {fmt_ms(ritz_t)} | {fmt_ms(po_t)} | {fmt_ms(fitz_t)} | "
            f"{ratio(po_t, ritz_t):>16} | {ratio(fitz_t, ritz_t):>16}"
        )

    print()
    print("倍率 = 对手时间 / ritz 时间。倍率 > 1 表示 ritz 更快。")
    print("✅ = ritz 领先，❌ = 对手领先，— = 对手不支持该场景。")


if __name__ == "__main__":
    main()
