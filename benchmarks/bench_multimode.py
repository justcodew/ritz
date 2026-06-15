"""多模式同页 benchmark：验证 Phase 6 stext 缓存的收益。

场景：真实用户常对同一页同时取 text + words + blocks + dict + 搜索，
如果 stext 不缓存，每次调用都要 fz_new_stext_page_from_page（~200-500μs）。
Phase 6 缓存后，第二次起命中 cache，省下 stext 重建成本。

对比：
- ritz (with cache): 当前实现
- pymupdf (no cache): 每次重建 stext

跑法：
    python benchmarks/bench_multimode.py
"""
import statistics
import time
from pathlib import Path

SAMPLE = Path(__file__).resolve().parent.parent / "crates/mupdf/tests/samples/sample.pdf"
ITERS = 200  # 同一 page 重复 200 轮，每轮 4 次提取


def bench_ritz_multimode(path, iters=ITERS):
    """对同一 page 调 text + words + blocks + dict，共 4 次提取 × iters 轮。"""
    import ritz
    d = ritz.open(str(path))
    p = d[0]
    # warmup
    p.get_text("text")
    t0 = time.perf_counter()
    for _ in range(iters):
        p.get_text("text")
        p.get_text("words")
        p.get_text("blocks")
        p.get_text("dict")
    return (time.perf_counter() - t0) / iters * 1000  # ms per round


def bench_ritz_text_only(path, iters=ITERS):
    """同一 page 只调 text（基准：cache 对单模式帮助有限）。"""
    import ritz
    d = ritz.open(str(path))
    p = d[0]
    p.get_text("text")
    t0 = time.perf_counter()
    for _ in range(iters):
        p.get_text("text")
    return (time.perf_counter() - t0) / iters * 1000


def bench_pymupdf_multimode(path, iters=ITERS):
    """PyMuPDF 同等工作量（每次都重建 stext，无 cache）。"""
    import fitz
    d = fitz.open(str(path))
    p = d[0]
    p.get_text("text")
    t0 = time.perf_counter()
    for _ in range(iters):
        p.get_text("text")
        p.get_text("words")
        p.get_text("blocks")
        p.get_text("dict")
    return (time.perf_counter() - t0) / iters * 1000


def bench_pymupdf_text_only(path, iters=ITERS):
    import fitz
    d = fitz.open(str(path))
    p = d[0]
    p.get_text("text")
    t0 = time.perf_counter()
    for _ in range(iters):
        p.get_text("text")
    return (time.perf_counter() - t0) / iters * 1000


def main():
    if not SAMPLE.exists():
        print(f"✗ sample not found: {SAMPLE}")
        return
    print(f"sample: {SAMPLE.name}")
    print(f"iters: {ITERS} rounds, each round = 1 or 4 get_text calls on same page\n")

    # 跑 5 次取 min（减少 GC/调度抖动）
    def repeat(fn, n=5):
        return min(fn(SAMPLE) for _ in range(n))

    print("=== 多模式同页（4 次/轮：text + words + blocks + dict） ===")
    ritz_mm = repeat(bench_ritz_multimode)
    pymupdf_mm = repeat(bench_pymupdf_multimode)
    print(f"  ritz     (cache):   {ritz_mm:.3f} ms/round")
    print(f"  pymupdf  (no cache): {pymupdf_mm:.3f} ms/round")
    print(f"  speedup: {pymupdf_mm / ritz_mm:.2f}x")

    print("\n=== 单模式 text（1 次/轮） ===")
    ritz_t = repeat(bench_ritz_text_only)
    pymupdf_t = repeat(bench_pymupdf_text_only)
    print(f"  ritz     (cache):   {ritz_t:.3f} ms/round")
    print(f"  pymupdf  (no cache): {pymupdf_t:.3f} ms/round")
    print(f"  speedup: {pymupdf_t / ritz_t:.2f}x")


if __name__ == "__main__":
    main()
