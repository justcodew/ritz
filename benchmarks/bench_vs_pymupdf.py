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
    _ = ritz_doc.metadata("format")
    _ = ritz_doc.metadata("info:Title")
    _ = ritz_doc.metadata("info:Author")

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
