#!/usr/bin/env python3
"""ritz vs PyMuPDF 性能对比基准。

参考 gomupdf-main/benchmark/benchmark_report.md 的测试结构：
  1. 文档打开/关闭
  2. 元数据读取
  3. 页面加载
  4. 页面渲染 1x
  5. 页面渲染 2x
  6. 全页面渲染 1x（所有页）
  7. 文本提取
  8. 文本搜索（ritz 暂未实现 search，跳过）
  9. 大纲读取（ritz 暂未实现 outline，跳过）
 10. PDF 保存到内存（ritz 暂未实现 write，跳过）
 11. 插入页（ritz 暂未实现，跳过）
 12. 删除页（ritz 暂未实现，跳过）
 13. Pixmap→PNG
 14. Pixmap→JPEG（ritz 暂未实现 JPEG，跳过）
 15. 链接读取

用法：
  python benchmarks/bench_vs_pymupdf.py [path/to/test.pdf]
"""

import os
import statistics
import sys
import time
from pathlib import Path

# 默认测试 PDF（与 gomupdf 基准同款，便于横向对比）
DEFAULT_PDF = "/Users/xiongzhaolong/Downloads/claude-pro/code_2026/arxiv_2603.09677_translated.pdf"
PDF = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_PDF

if not Path(PDF).exists():
    print(f"ERROR: test PDF not found: {PDF}")
    sys.exit(1)

import ritz
import fitz  # PyMuPDF

ITERATIONS = 10
WARMUP = 2


def bench(fn, iters=ITERATIONS, warmup=WARMUP):
    """Run fn iters times after warmup, return (mean_ms, min_ms, max_ms)."""
    for _ in range(warmup):
        fn()
    times = []
    for _ in range(iters):
        t0 = time.perf_counter()
        fn()
        t1 = time.perf_counter()
        times.append((t1 - t0) * 1000.0)
    return statistics.mean(times), min(times), max(times)


# ============================================================================
# Test scenarios
# ============================================================================

def fmt(mean, lo, hi):
    return f"{mean:.3f} [{lo:.3f}-{ha:.3f}]".replace("ha", "hi") if False else f"{mean:.3f} ms"


def fmt_row(name, ritz_m, fitz_m, ritz_minmax, fitz_minmax):
    if fitz_m == 0:
        rate = "N/A"
        winner = "N/A"
    else:
        rate = ritz_m / fitz_m
        if rate < 0.9:
            winner = "**ritz**"
        elif rate > 1.1:
            winner = "**PyMuPDF**"
        else:
            winner = "持平"
        rate = f"{rate:.2f}x"
    return f"| {name} | {ritz_m:.3f} | {fitz_m:.3f} | {rate} | {winner} |"


print(f"测试文件：{PDF}")
print(f"文件大小：{os.path.getsize(PDF) / 1024 / 1024:.2f} MB")
print(f"迭代次数：{ITERATIONS}")
print()

results = []

# ---------------------------------------------------------------------------
# 1. 文档打开/关闭
# ---------------------------------------------------------------------------
def ritz_open_close():
    d = ritz.open(PDF)
    _ = len(d)

def fitz_open_close():
    d = fitz.open(PDF)
    _ = len(d)
    d.close()

ritz_mean, ritz_lo, ritz_hi = bench(ritz_open_close)
fitz_mean, fitz_lo, fitz_hi = bench(fitz_open_close)
results.append(("文档打开/关闭", ritz_mean, fitz_mean, (ritz_lo, ritz_hi), (fitz_lo, fitz_hi)))

# 准备共享 doc 实例
ritz_doc = ritz.open(PDF)
fitz_doc = fitz.open(PDF)
PAGE_COUNT = len(ritz_doc)
print(f"页数：{PAGE_COUNT}")

# ---------------------------------------------------------------------------
# 2. 元数据读取
# ---------------------------------------------------------------------------
def ritz_metadata():
    _ = ritz_doc.metadata["format"]
    _ = ritz_doc.metadata["title"]
    _ = ritz_doc.metadata["author"]

def fitz_metadata():
    _ = fitz_doc.metadata.get("format")
    _ = fitz_doc.metadata.get("title")
    _ = fitz_doc.metadata.get("author")

ritz_mean, ritz_lo, ritz_hi = bench(ritz_metadata)
fitz_mean, fitz_lo, fitz_hi = bench(fitz_metadata)
results.append(("元数据读取", ritz_mean, fitz_mean, (ritz_lo, ritz_hi), (fitz_lo, fitz_hi)))

# ---------------------------------------------------------------------------
# 3. 页面加载
# ---------------------------------------------------------------------------
def ritz_load_page():
    p = ritz_doc[0]

def fitz_load_page():
    p = fitz_doc[0]

ritz_mean, ritz_lo, ritz_hi = bench(ritz_load_page)
fitz_mean, fitz_lo, fitz_hi = bench(fitz_load_page)
results.append(("页面加载", ritz_mean, fitz_mean, (ritz_lo, ritz_hi), (fitz_lo, fitz_hi)))

# ---------------------------------------------------------------------------
# 4. 页面渲染 1x
# ---------------------------------------------------------------------------
def ritz_render_1x():
    pix = ritz_doc[0].get_pixmap(dpi=72)

def fitz_render_1x():
    pix = fitz_doc[0].get_pixmap(dpi=72)

ritz_mean, ritz_lo, ritz_hi = bench(ritz_render_1x)
fitz_mean, fitz_lo, fitz_hi = bench(fitz_render_1x)
results.append(("页面渲染 1x", ritz_mean, fitz_mean, (ritz_lo, ritz_hi), (fitz_lo, fitz_hi)))

# ---------------------------------------------------------------------------
# 5. 页面渲染 2x
# ---------------------------------------------------------------------------
def ritz_render_2x():
    pix = ritz_doc[0].get_pixmap(dpi=144)

def fitz_render_2x():
    pix = fitz_doc[0].get_pixmap(dpi=144)

ritz_mean, ritz_lo, ritz_hi = bench(ritz_render_2x)
fitz_mean, fitz_lo, fitz_hi = bench(fitz_render_2x)
results.append(("页面渲染 2x", ritz_mean, fitz_mean, (ritz_lo, ritz_hi), (fitz_lo, fitz_hi)))

# ---------------------------------------------------------------------------
# 6. 全页面渲染 1x
# ---------------------------------------------------------------------------
def ritz_render_all():
    for i in range(PAGE_COUNT):
        _ = ritz_doc[i].get_pixmap(dpi=72)

def fitz_render_all():
    for i in range(PAGE_COUNT):
        _ = fitz_doc[i].get_pixmap(dpi=72)

# 减少迭代次数避免太慢
ritz_mean, ritz_lo, ritz_hi = bench(ritz_render_all, iters=max(2, ITERATIONS // 5), warmup=1)
fitz_mean, fitz_lo, fitz_hi = bench(fitz_render_all, iters=max(2, ITERATIONS // 5), warmup=1)
results.append((f"全页面渲染 1x ({PAGE_COUNT}页)", ritz_mean, fitz_mean, (ritz_lo, ritz_hi), (fitz_lo, fitz_hi)))

# ---------------------------------------------------------------------------
# 7. 文本提取
# ---------------------------------------------------------------------------
def ritz_text():
    _ = ritz_doc[0].get_text("text")

def fitz_text():
    _ = fitz_doc[0].get_text("text")

ritz_mean, ritz_lo, ritz_hi = bench(ritz_text)
fitz_mean, fitz_lo, fitz_hi = bench(fitz_text)
results.append(("文本提取", ritz_mean, fitz_mean, (ritz_lo, ritz_hi), (fitz_lo, fitz_hi)))

# ---------------------------------------------------------------------------
# 13. Pixmap → PNG
# ---------------------------------------------------------------------------
def ritz_png():
    pix = ritz_doc[0].get_pixmap(dpi=72)
    _ = pix.tobytes("png")

def fitz_png():
    pix = fitz_doc[0].get_pixmap(dpi=72)
    _ = pix.tobytes("png")

ritz_mean, ritz_lo, ritz_hi = bench(ritz_png)
fitz_mean, fitz_lo, fitz_hi = bench(fitz_png)
results.append(("Pixmap→PNG", ritz_mean, fitz_mean, (ritz_lo, ritz_hi), (fitz_lo, fitz_hi)))

# ---------------------------------------------------------------------------
# 15. 链接读取
# ---------------------------------------------------------------------------
def ritz_links():
    _ = ritz_doc[0].get_links()

def fitz_links():
    _ = fitz_doc[0].get_links()

ritz_mean, ritz_lo, ritz_hi = bench(ritz_links)
fitz_mean, fitz_lo, fitz_hi = bench(fitz_links)
results.append(("链接读取", ritz_mean, fitz_mean, (ritz_lo, ritz_hi), (fitz_lo, fitz_hi)))


# ============================================================================
# plan_v1 KPI 兑现场景：图片提取 / 批量 / 多文档并行
# （这三项是 plan_v1 点名的 10x~30x 目标核心）
# ============================================================================

import glob
import multiprocessing as mp

# macOS Python 3.8+ 默认 spawn，会重新导入主模块导致死循环。强制 fork。
try:
    mp.set_start_method("fork", force=True)
except RuntimeError:
    pass
from multiprocessing import Pool

BIG_200 = "/tmp/ritz_big_200.pdf"
BIG_1000 = "/tmp/ritz_big_1000.pdf"

print()
print("=" * 70)
print("plan_v1 KPI 兑现场景（图片 / 批量 / 多文档并行）")
print("=" * 70)

# ---------------------------------------------------------------------------
# KPI-1. 图片提取 head-to-head（plan_v1 §3.2 一级提取）
# ---------------------------------------------------------------------------
# ritz: 一次调用 get_images(include_data=True) 取 bbox + PNG bytes
# PyMuPDF: 两步 — get_images(full=True) 取 xref，再循环 extract_image(xref)
def ritz_images_all_pages():
    total = 0
    for i in range(PAGE_COUNT):
        imgs = ritz_doc[i].get_images(include_data=True)
        for im in imgs:
            if "image" in im:
                total += len(im["image"])
    return total

def fitz_images_all_pages():
    total = 0
    for i in range(PAGE_COUNT):
        for im in fitz_doc[i].get_images(full=True):
            xref = im[0]
            d = fitz_doc.extract_image(xref)
            total += len(d["image"])
    return total

print("\n[KPI-1] 图片提取（全文档，含 PNG bytes）")
ritz_img_mean, ritz_img_lo, ritz_img_hi = bench(ritz_images_all_pages)
fitz_img_mean, fitz_img_lo, fitz_img_hi = bench(fitz_images_all_pages)
ritz_img_bytes = ritz_images_all_pages()
img_rate = fitz_img_mean / ritz_img_mean if ritz_img_mean > 0 else 0
print(f"  ritz 一次调用: {ritz_img_mean:.3f} ms ({ritz_img_bytes/1024:.0f} KB images)")
print(f"  PyMuPDF 两步:  {fitz_img_mean:.3f} ms")
print(f"  加速:          {img_rate:.1f}x")
results.append(("图片提取(全文档)", ritz_img_mean, fitz_img_mean, (ritz_img_lo, ritz_img_hi), (fitz_img_lo, fitz_img_hi)))

# KPI-1b. JPEG-heavy PDF：验证原始字节路径（跳过 decode+re-encode）
JPEG_PDF = "/tmp/ritz_jpeg_heavy.pdf"
if Path(JPEG_PDF).exists():
    ritz_jd = ritz.open(JPEG_PDF)
    fitz_jd = fitz.open(JPEG_PDF)
    JN = len(ritz_jd)

    def ritz_jpeg_images():
        total = 0
        for i in range(JN):
            for im in ritz_jd[i].get_images(include_data=True):
                if "image" in im:
                    total += len(im["image"])
        return total

    def fitz_jpeg_images():
        total = 0
        for i in range(JN):
            for im in fitz_jd[i].get_images(full=True):
                d = fitz_jd.extract_image(im[0])
                total += len(d["image"])
        return total

    print("\n[KPI-1b] 图片提取（JPEG-heavy PDF，验证原始字节路径）")
    ritz_jm, ritz_jlo, ritz_jhi = bench(ritz_jpeg_images, iters=5, warmup=1)
    fitz_jm, fitz_jlo, fitz_jhi = bench(fitz_jpeg_images, iters=5, warmup=1)
    ritz_jb = ritz_jpeg_images()
    jrate = fitz_jm / ritz_jm if ritz_jm > 0 else 0
    print(f"  ritz 原始字节: {ritz_jm:.3f} ms ({ritz_jb/1024:.0f} KB JPEG)")
    print(f"  PyMuPDF 两步:  {fitz_jm:.3f} ms")
    print(f"  加速:          {jrate:.1f}x")
    results.append(("图片提取(JPEG-heavy)", ritz_jm, fitz_jm, (ritz_jlo, ritz_jhi), (fitz_jlo, fitz_jhi)))
    ritz_jd = None
    fitz_jd.close()

# ---------------------------------------------------------------------------
# KPI-2. 批量文本提取 head-to-head（plan_v1 §3.2 / 阶段4）
# ---------------------------------------------------------------------------
# 用 200 页 PDF 放大 N×FFI 累积优势
if Path(BIG_200).exists():
    print("\n[KPI-2] 批量文本提取（200 页 PDF）")
    ritz_big = ritz.open(BIG_200)
    fitz_big = fitz.open(BIG_200)
    BIG_N = len(ritz_big)
    print(f"  文档: {BIG_N} 页")

    def ritz_batch():
        _ = ritz_big.get_text_batch()

    def ritz_per_page():
        _ = [ritz_big[i].get_text("text") for i in range(BIG_N)]

    def fitz_per_page():
        _ = [fitz_big[i].get_text("text") for i in range(BIG_N)]

    ritz_batch_m, ritz_batch_lo, ritz_batch_hi = bench(ritz_batch, iters=max(3, ITERATIONS // 2), warmup=1)
    ritz_pp_m, _, _ = bench(ritz_per_page, iters=max(3, ITERATIONS // 2), warmup=1)
    fitz_pp_m, fitz_pp_lo, fitz_pp_hi = bench(fitz_per_page, iters=max(3, ITERATIONS // 2), warmup=1)

    # 验证内容一致
    batch_content = ritz_big.get_text_batch()
    pp_content = [ritz_big[i].get_text("text") for i in range(BIG_N)]
    print(f"  ritz batch vs ritz per-page 内容一致: {batch_content == pp_content}")
    print(f"  ritz batch:     {ritz_batch_m:.3f} ms")
    print(f"  ritz per-page:  {ritz_pp_m:.3f} ms  (batch 加速 {ritz_pp_m/ritz_batch_m:.1f}x)")
    print(f"  PyMuPDF per-page: {fitz_pp_m:.3f} ms")
    print(f"  ritz batch vs PyMuPDF per-page: {fitz_pp_m/ritz_batch_m:.1f}x")
    # 加入主表（ritz batch vs PyMuPDF per-page，因 PyMuPDF 无原生批量）
    results.append(("批量提取(200页)", ritz_batch_m, fitz_pp_m, (ritz_batch_lo, ritz_batch_hi), (fitz_pp_lo, fitz_pp_hi)))
    ritz_big = None
    fitz_big.close()
else:
    print("\n[KPI-2] SKIP: 200 页 fixture 不存在，先跑 make_big_pdf.py")

# ---------------------------------------------------------------------------
# KPI-3. 多文档并行 head-to-head（plan_v1 §5.3）
# ---------------------------------------------------------------------------
# 准备多个 PDF 做并行压测
def gather_pdfs(limit=8):
    paths = [PDF]  # arxiv
    # 补充 vendor 里的 PDF
    for p in sorted(glob.glob("../gomupdf-main/*.pdf")):
        if Path(p).exists() and p not in paths:
            paths.append(str(Path(p).resolve()))
    for p in sorted(glob.glob("vendor/mupdf/thirdparty/**/*.pdf", recursive=True)):
        if len(paths) >= limit:
            break
        sz = Path(p).stat().st_size
        if sz > 50_000:  # 跳过太小的
            paths.append(str(Path(p).resolve()))
    return paths[:limit]

PDFS = gather_pdfs(8)
print(f"\n[KPI-3] 多文档并行（{len(PDFS)} 个 PDF）")
for p in PDFS:
    print(f"  - {Path(p).name} ({Path(p).stat().st_size/1024:.0f} KB)")

def ritz_process_docs_serial():
    out = []
    for p in PDFS:
        try:
            d = ritz.open(p)
            out.append(d.get_text_batch())
        except Exception:
            out.append(None)
    return out

def ritz_process_docs_parallel():
    return ritz.process_documents(PDFS)

def _fitz_one(p):
    try:
        d = fitz.open(p)
        out = [d[i].get_text("text") for i in range(len(d))]
        d.close()
        return out
    except Exception:
        return None

def fitz_process_docs_serial():
    return [_fitz_one(p) for p in PDFS]

def fitz_process_docs_parallel():
    with Pool(min(len(PDFS), 8)) as pool:
        return pool.map(_fitz_one, PDFS)

print("\n  串行/并行各跑 3 次（并行含 fork/pool 启动开销）")
ritz_ser_m, _, _ = bench(ritz_process_docs_serial, iters=3, warmup=1)
ritz_par_m, _, _ = bench(ritz_process_docs_parallel, iters=3, warmup=1)
fitz_ser_m, _, _ = bench(fitz_process_docs_serial, iters=3, warmup=1)
fitz_par_m, _, _ = bench(fitz_process_docs_parallel, iters=3, warmup=1)

print(f"  ritz 串行:    {ritz_ser_m:.1f} ms")
print(f"  ritz 并行:    {ritz_par_m:.1f} ms  (rayon 加速 {ritz_ser_m/ritz_par_m:.2f}x)")
print(f"  fitz 串行:    {fitz_ser_m:.1f} ms")
print(f"  fitz 并行:    {fitz_par_m:.1f} ms  (multiprocessing 加速 {fitz_ser_m/fitz_par_m:.2f}x)")
print(f"  ritz 并行 vs fitz 并行: {fitz_par_m/ritz_par_m:.2f}x")
# 加入主表：ritz 并行 vs fitz 并行
results.append(("多文档并行", ritz_par_m, fitz_par_m, (ritz_par_m, ritz_par_m), (fitz_par_m, fitz_par_m)))


# ============================================================================
# 输出 markdown 表格
# ============================================================================

print()
print("## 测试结果")
print()
print("| # | 测试项目 | ritz (ms) | PyMuPDF (ms) | 倍率 | 胜者 |")
print("|---|---------|----------|-------------|------|------|")
for i, (name, rm, fm, _, _) in enumerate(results, 1):
    if fm == 0:
        rate = "N/A"
        winner = "N/A"
    else:
        rate = rm / fm
        if rate < 0.9:
            winner = "**ritz**"
        elif rate > 1.1:
            winner = "**PyMuPDF**"
        else:
            winner = "持平"
        rate = f"{rate:.2f}x"
    print(f"| {i} | {name} | {rm:.3f} | {fm:.3f} | {rate} | {winner} |")

# 详细表
print()
print("## 详细数据（含 min-max）")
print()
print("| 测试 | ritz (ms) | PyMuPDF (ms) |")
print("|------|----------|-------------|")
for name, rm, fm, (rl, rh), (fl, fh) in results:
    print(f"| {name} | {rm:.3f} [{rl:.3f}-{rh:.3f}] | {fm:.3f} [{fl:.3f}-{fh:.3f}] |")

ritz_doc = None  # let GC close
fitz_doc.close()
