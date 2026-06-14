#!/usr/bin/env python3
"""Tier 1/2 优化前后对比基准。

聚焦受影响场景，输出可复现的 mean ± stdev 数据。
不与 PyMuPDF 比，只与 ritz 自己的历史版本比（git checkout 验证用）。

用法：
    # 当前版本基线
    python benchmarks/bench_tier12_diff.py

    # 对比旧版本：
    git stash && python benchmarks/bench_tier12_diff.py > /tmp/old.txt
    git stash pop && python benchmarks/bench_tier12_diff.py > /tmp/new.txt
    diff /tmp/old.txt /tmp/new.txt
"""

from __future__ import annotations

import statistics
import time
from pathlib import Path

import ritz

REPO = Path(__file__).resolve().parent.parent
SAMPLE = REPO / "crates" / "mupdf" / "tests" / "samples" / "sample.pdf"
ARXIV = Path(
    "/Users/xiongzhaolong/Downloads/claude-pro/code_2026/"
    "arxiv_2603.09677_translated.pdf"
)


def bench(fn, n: int = 50, warmup: int = 5) -> tuple[float, float]:
    """返回 (mean_ms, stdev_ms)。"""
    for _ in range(warmup):
        fn()
    samples = []
    for _ in range(n):
        t0 = time.perf_counter()
        fn()
        samples.append((time.perf_counter() - t0) * 1000)
    return statistics.mean(samples), statistics.stdev(samples)


def fmt(mean: float, stdev: float) -> str:
    if mean < 1:
        return f"{mean:7.4f} ± {stdev:6.4f} ms"
    return f"{mean:7.3f} ± {stdev:6.3f} ms"


def main():
    print(f"ritz {ritz.__version__}")
    print(f"sample: {SAMPLE.name} (1 页)")
    print(f"arxiv:  {ARXIV.name} ({ARXIV.stat().st_size // 1024 // 1024} MB)")
    print(f"iterations: 50 (+5 warmup)")
    print("=" * 70)

    # 文档对象
    rd_small = ritz.open(str(SAMPLE))
    rd_big = ritz.open(str(ARXIV))
    page_small = rd_small[0]
    page_big = rd_big[0]

    print(f"\n{'场景':<40} | {'耗时':<22}")
    print("-" * 70)

    # === Tier 1 影响场景 ===
    # metadata cache（缓存命中 vs 首次）
    # 先触发首次访问（填充缓存）
    _ = rd_big.metadata
    m, s = bench(lambda: rd_big.metadata)
    print(f"{'metadata (cache hit)':<40} | {fmt(m, s)}")

    # rect cache
    _ = page_big.rect
    m, s = bench(lambda: page_big.rect)
    print(f"{'page.rect (cache hit)':<40} | {fmt(m, s)}")
    m, s = bench(lambda: page_big.width)
    print(f"{'page.width (cache hit)':<40} | {fmt(m, s)}")
    m, s = bench(lambda: page_big.height)
    print(f"{'page.height (cache hit)':<40} | {fmt(m, s)}")

    # rotation (raw_doc cache，不走 borrow)
    m, s = bench(lambda: page_big.rotation)
    print(f"{'page.rotation (raw_doc cache)':<40} | {fmt(m, s)}")

    # PNG tobytes（去 to_vec）
    pix = page_big.get_pixmap()
    m, s = bench(lambda: pix.tobytes("png"), n=20, warmup=3)
    print(f"{'pixmap.tobytes(png)':<40} | {fmt(m, s)}")

    # === Tier 2 #8 影响场景：text/html/xml/json ===
    m, s = bench(lambda: page_small.get_text("text"))
    print(f"{'get_text(text) [1 页]':<40} | {fmt(m, s)}")
    m, s = bench(lambda: page_small.get_text("html"))
    print(f"{'get_text(html) [1 页]':<40} | {fmt(m, s)}")
    m, s = bench(lambda: page_small.get_text("xml"))
    print(f"{'get_text(xml) [1 页]':<40} | {fmt(m, s)}")
    m, s = bench(lambda: page_small.get_text("json"))
    print(f"{'get_text(json) [1 页]':<40} | {fmt(m, s)}")

    # arxiv 单页（大文档单页）
    m, s = bench(lambda: page_big.get_text("text"), n=30, warmup=3)
    print(f"{'get_text(text) [arxiv 单页]':<40} | {fmt(m, s)}")

    # === Tier 2 #7 影响场景：words/blocks ===
    m, s = bench(lambda: page_small.get_text("words"))
    print(f"{'get_text(words) [1 页]':<40} | {fmt(m, s)}")
    m, s = bench(lambda: page_small.get_text("blocks"))
    print(f"{'get_text(blocks) [1 页]':<40} | {fmt(m, s)}")
    m, s = bench(lambda: page_big.get_text("words"), n=30, warmup=3)
    print(f"{'get_text(words) [arxiv 单页]':<40} | {fmt(m, s)}")
    m, s = bench(lambda: page_big.get_text("blocks"), n=30, warmup=3)
    print(f"{'get_text(blocks) [arxiv 单页]':<40} | {fmt(m, s)}")

    # === 批量文本（Tier 1 GIL 释放，不变路径，作对照） ===
    m, s = bench(lambda: rd_big.get_text_batch(), n=10, warmup=2)
    print(f"{'get_text_batch [43 页]':<40} | {fmt(m, s)}")


if __name__ == "__main__":
    main()
