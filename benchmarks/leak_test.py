#!/usr/bin/env python3
"""ritz 内存与泄漏验证（plan_v1 §4.2 兑现）。

两类验证：

A. 常驻内存验证（plan_v1 §4.2 目标：1000 页 PDF ≤ 200 MB）
   - 打开 1000 页 PDF
   - 遍历全部页 get_text("text") / get_text("dict") / get_pixmap()
   - 记录峰值 RSS
   - 断言 ≤ 200 MB（plan 目标）或给出明确诊断

B. 泄漏压力测试
   - 1000 次 open/close 循环（Document 生命周期）
   - 1000 次 page 操作循环（Page/Pixmap/SText 生命周期）
   - 每 N 次采样 RSS，线性回归斜率应近零（容差 < 100 KB/iter）

通过条件：A 达标 或有明确诊断；B 斜率 < 阈值。
"""

import gc
import resource
import sys
import time
from pathlib import Path

import ritz

SAMPLE = Path(__file__).resolve().parents[1] / "crates" / "mupdf" / "tests" / "samples" / "sample.pdf"
BIG_1000 = Path("/tmp/ritz_big_1000.pdf")
BIG_200 = Path("/tmp/ritz_big_200.pdf")

MB = 1024 * 1024
KB = 1024

PLAN_RSS_LIMIT_MB = 200  # plan_v1 §4.2
LEAK_SLOPE_LIMIT_KB = 100  # 每 iter RSS 增量容差


def rss_mb() -> float:
    # ru_maxrss: macOS 单位是字节，Linux 是 KB
    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if sys.platform == "darwin":
        return rss / MB
    return rss / KB / MB


def rss_now_mb() -> float:
    """当前 RSS（非峰值），用 /proc 或 ps。macOS 用 ps."""
    import subprocess
    pid = __import__("os").getpid()
    if sys.platform == "darwin":
        out = subprocess.check_output(["ps", "-o", "rss=", "-p", str(pid)]).decode().strip()
        return int(out) * KB / MB
    else:
        try:
            with open(f"/proc/{pid}/statm") as f:
                return int(f.read().split()[1]) * 4096 / MB
        except Exception:
            return rss_mb()


# ============================================================================
# A. 常驻内存验证
# ============================================================================

def test_resident_memory_big_pdf():
    """plan_v1 §4.2：打开 1000 页 PDF，全页提取后常驻内存 ≤ 200 MB。"""
    if not BIG_1000.exists():
        print(f"  SKIP: {BIG_1000} not found (run make_big_pdf.py first)")
        return None

    gc.collect()
    rss_before = rss_now_mb()
    t0 = time.perf_counter()
    doc = ritz.open(str(BIG_1000))
    n = len(doc)
    print(f"  opened {n}-page PDF, RSS={rss_now_mb():.1f} MB (Δ {rss_now_mb()-rss_before:+.1f})")

    # 全页文本提取
    for i in range(n):
        _ = doc[i].get_text("text")
    rss_after_text = rss_now_mb()
    print(f"  after {n}× get_text(text), RSS={rss_after_text:.1f} MB")

    # 全页 dict 提取（更重的结构）
    for i in range(n):
        _ = doc[i].get_text("dict")
    rss_after_dict = rss_now_mb()
    print(f"  after {n}× get_text(dict), RSS={rss_after_dict:.1f} MB")

    # 抽样渲染（不做全部 1000 页，太慢；每 100 页一次）
    for i in range(0, n, 100):
        _ = doc[i].get_pixmap(dpi=72)
    rss_after_pix = rss_now_mb()
    print(f"  after {n//100}× get_pixmap, RSS={rss_after_pix:.1f} MB")

    t1 = time.perf_counter()
    doc = None
    gc.collect()
    rss_freed = rss_now_mb()
    print(f"  after doc drop+GC, RSS={rss_freed:.1f} MB")
    print(f"  total wall: {(t1-t0):.1f} s")

    peak = max(rss_after_text, rss_after_dict, rss_after_pix)
    status = "PASS" if peak <= PLAN_RSS_LIMIT_MB else "FAIL"
    print(f"  [{status}] peak RSS={peak:.1f} MB vs plan limit {PLAN_RSS_LIMIT_MB} MB")
    return peak


# ============================================================================
# B. 泄漏压力测试
# ============================================================================

def _slope_kbpiter(samples):
    """简单线性回归斜率（KB/iter）。samples: list of (iter, rss_kb)."""
    n = len(samples)
    if n < 2:
        return 0.0
    xs = [s[0] for s in samples]
    ys = [s[1] for s in samples]
    mx = sum(xs) / n
    my = sum(ys) / n
    num = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    den = sum((x - mx) ** 2 for x in xs)
    if den == 0:
        return 0.0
    return num / den  # KB/iter


def test_leak_open_close(iters=1000, sample_every=100):
    """1000 次 open/close，每 100 次采样 RSS，斜率应 < 100 KB/iter。"""
    gc.collect()
    samples = []
    rss0 = rss_now_mb()
    print(f"  start RSS={rss0:.1f} MB")
    for i in range(iters):
        d = ritz.open(str(SAMPLE))
        _ = len(d)
        d = None
        if (i + 1) % sample_every == 0:
            gc.collect()
            rss = rss_now_mb()
            samples.append((i + 1, rss * 1024))  # KB
            print(f"    iter {i+1}: RSS={rss:.1f} MB")
    slope = _slope_kbpiter(samples)
    status = "PASS" if slope < LEAK_SLOPE_LIMIT_KB else "FAIL"
    print(f"  [{status}] open/close leak slope={slope:.1f} KB/iter (limit {LEAK_SLOPE_LIMIT_KB})")
    return slope


def test_leak_page_ops(iters=1000, sample_every=100):
    """1000 次 page 重负载操作（text/dict/pixmap/images），斜率应 < 100 KB/iter。"""
    gc.collect()
    doc = ritz.open(str(SAMPLE))
    samples = []
    print(f"  start RSS={rss_now_mb():.1f} MB")
    for i in range(iters):
        p = doc[0]
        _ = p.get_text("dict")
        _ = p.get_pixmap(dpi=72)
        _ = p.get_images(include_data=True)
        if (i + 1) % sample_every == 0:
            gc.collect()
            rss = rss_now_mb()
            samples.append((i + 1, rss * 1024))
            print(f"    iter {i+1}: RSS={rss:.1f} MB")
    doc = None
    slope = _slope_kbpiter(samples)
    status = "PASS" if slope < LEAK_SLOPE_LIMIT_KB else "FAIL"
    print(f"  [{status}] page-ops leak slope={slope:.1f} KB/iter (limit {LEAK_SLOPE_LIMIT_KB})")
    return slope


# ============================================================================
# Main
# ============================================================================

def main():
    print("=" * 70)
    print("A. 常驻内存验证（plan_v1 §4.2：1000 页 PDF ≤ 200 MB）")
    print("=" * 70)
    peak = test_resident_memory_big_pdf()

    print()
    print("=" * 70)
    print("B. 泄漏压力测试（1000 次循环，斜率 < 100 KB/iter）")
    print("=" * 70)
    print("\n[B1] open/close × 1000")
    s1 = test_leak_open_close()
    print("\n[B2] page-ops × 1000 (dict + pixmap + images)")
    s2 = test_leak_page_ops()

    print()
    print("=" * 70)
    print("汇总")
    print("=" * 70)
    print(f"  A. peak RSS (1000 页):       {peak:.1f} MB" if peak else "  A. SKIP")
    if peak:
        print(f"     plan limit:              {PLAN_RSS_LIMIT_MB} MB")
        print(f"     结果:                   {'PASS' if peak <= PLAN_RSS_LIMIT_MB else 'FAIL'}")
    print(f"  B1. open/close leak slope:   {s1:.1f} KB/iter ({'PASS' if s1 < LEAK_SLOPE_LIMIT_KB else 'FAIL'})")
    print(f"  B2. page-ops leak slope:     {s2:.1f} KB/iter ({'PASS' if s2 < LEAK_SLOPE_LIMIT_KB else 'FAIL'})")


if __name__ == "__main__":
    main()
