"""Corpus benchmark：在公开 PDF 测试集上对比 ritz 和 PyMuPDF 的文本提取。

数据源（与 pdf_oxide README 一致）：
- veraPDF corpus (https://github.com/veraPDF/veraPDF-corpus)
- Mozilla pdf.js test/pdfs (https://github.com/mozilla/pdf.js/tree/master/test/pdfs)
- DARPA SafeDocs (https://github.com/pdf-association/safedocs)

输出：
- per-library: mean / p50 / p95 / p99 / pass_rate / count
- markdown 表格（可直接粘到 README）

用法：
    python benchmarks/bench_corpus.py           # 默认跑全部 corpus
    python benchmarks/bench_corpus.py --max 100 # 只跑前 100 个 PDF（调试用）
    python benchmarks/bench_corpus.py --libs ritz,pymupdf
"""

import argparse
import gc
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

CORPUS_DIR = Path(__file__).resolve().parent / "corpus"
TIMEOUT_S = 60.0  # 与 pdf_oxide 一致


# ---------------------------------------------------------------------------
# Corpus 下载
# ---------------------------------------------------------------------------
def ensure_corpus():
    """Shallow-clone 三个 corpus 仓库到 benchmarks/corpus/。"""
    CORPUS_DIR.mkdir(exist_ok=True)
    sources = [
        ("pdfjs", "https://github.com/mozilla/pdf.js.git"),
        ("verapdf", "https://github.com/veraPDF/veraPDF-corpus.git"),
        ("safedocs", "https://github.com/pdf-association/safedocs.git"),
    ]
    for name, url in sources:
        target = CORPUS_DIR / name
        if target.exists() and any(target.rglob("*.pdf")):
            print(f"  ✓ {name} 已存在")
            continue
        if target.exists():
            shutil.rmtree(target)
        print(f"  ↓ cloning {name} ...")
        # 不指定 --branch，让 git 用远端 HEAD（不同仓库默认分支名不一）
        try:
            subprocess.check_call([
                "git", "clone", "--depth", "1", url, str(target),
            ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            pdfs_count = sum(1 for _ in target.rglob("*.pdf"))
            print(f"    ✓ {pdfs_count} PDFs")
        except subprocess.CalledProcessError as e:
            print(f"    ✗ clone {name} failed: {e}")
            shutil.rmtree(target, ignore_errors=True)


def collect_pdfs(max_n=None):
    """返回 corpus 目录下所有 .pdf 路径列表。"""
    if not CORPUS_DIR.exists():
        return []
    pdfs = []
    for p in CORPUS_DIR.rglob("*.pdf"):
        # 跳过明显损坏/零字节
        try:
            if p.stat().st_size < 100:
                continue
        except OSError:
            continue
        pdfs.append(p)
        if max_n and len(pdfs) >= max_n:
            break
    return pdfs


# ---------------------------------------------------------------------------
# 提取函数
# ---------------------------------------------------------------------------
def extract_ritz(path, timeout=TIMEOUT_S):
    """返回 (elapsed_s, status, text_len)。status: 'pass' / 'fail' / 'timeout'."""
    import ritz
    t0 = time.perf_counter()
    try:
        d = ritz.open(str(path))
        n = len(d)
        total_len = 0
        for i in range(n):
            total_len += len(d[i].get_text("text"))
        elapsed = time.perf_counter() - t0
        if elapsed > timeout:
            return (elapsed, "timeout", 0)
        return (elapsed, "pass", total_len)
    except Exception:
        elapsed = time.perf_counter() - t0
        return (elapsed, "fail" if elapsed < timeout else "timeout", 0)


def extract_ritz_batch(path, timeout=TIMEOUT_S):
    """ritz 独有：用 doc.get_text_batch() 一次性拿全部页文本（1 次 FFI）。

    与 extract_ritz 做相同工作量，但省下 N-1 次 per-page PyO3 dispatch +
    Python 对象构造，是 ritz 针对小文档场景的优化路径。
    """
    import ritz
    t0 = time.perf_counter()
    try:
        d = ritz.open(str(path))
        # get_text_batch 默认 mode='text'，返回 list[str]
        texts = d.get_text_batch()
        total_len = sum(len(t) for t in texts)
        elapsed = time.perf_counter() - t0
        if elapsed > timeout:
            return (elapsed, "timeout", 0)
        return (elapsed, "pass", total_len)
    except Exception:
        elapsed = time.perf_counter() - t0
        return (elapsed, "fail" if elapsed < timeout else "timeout", 0)


def extract_pymupdf(path, timeout=TIMEOUT_S):
    import fitz
    t0 = time.perf_counter()
    try:
        d = fitz.open(str(path))
        total_len = sum(len(p.get_text("text")) for p in d)
        elapsed = time.perf_counter() - t0
        if elapsed > timeout:
            return (elapsed, "timeout", 0)
        return (elapsed, "pass", total_len)
    except Exception:
        elapsed = time.perf_counter() - t0
        return (elapsed, "fail" if elapsed < timeout else "timeout", 0)


# ---------------------------------------------------------------------------
# 统计
# ---------------------------------------------------------------------------
def percentile(sorted_list, p):
    """p ∈ [0, 100]。"""
    if not sorted_list:
        return float("nan")
    k = (len(sorted_list) - 1) * p / 100
    f = int(k)
    c = min(f + 1, len(sorted_list) - 1)
    return sorted_list[f] + (sorted_list[c] - sorted_list[f]) * (k - f)


def summarize(results):
    """results: list of (elapsed_s, status, text_len)。"""
    n = len(results)
    passes = [r[0] for r in results if r[1] == "pass"]
    fails = sum(1 for r in results if r[1] == "fail")
    timeouts = sum(1 for r in results if r[1] == "timeout")
    passes_sorted = sorted(passes)
    return {
        "n": n,
        "pass_n": len(passes),
        "pass_rate": (len(passes) / n * 100) if n else 0,
        "fail": fails,
        "timeout": timeouts,
        "mean_ms": (statistics.mean(passes) * 1000) if passes else float("nan"),
        "p50_ms": percentile(passes_sorted, 50) * 1000,
        "p95_ms": percentile(passes_sorted, 95) * 1000,
        "p99_ms": percentile(passes_sorted, 99) * 1000,
    }


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--max", type=int, default=None, help="只跑前 N 个 PDF（调试）")
    parser.add_argument("--libs", default="ritz,ritz_batch,pymupdf", help="逗号分隔")
    parser.add_argument("--no-download", action="store_true", help="跳过 corpus 下载")
    args = parser.parse_args()

    libs = args.libs.split(",")
    print(f"ritz corpus benchmark | libs={libs} | max={args.max}")

    if not args.no_download:
        print("\n=== 下载 corpus ===")
        ensure_corpus()

    pdfs = collect_pdfs(max_n=args.max)
    print(f"\n=== 收集到 {len(pdfs)} 个 PDF ===")
    if not pdfs:
        print("✗ 无 PDF。请检查 corpus/ 目录。")
        sys.exit(1)

    extractors = {
        "ritz": extract_ritz,
        "ritz_batch": extract_ritz_batch,
        "pymupdf": extract_pymupdf,
    }

    all_results = {}
    for lib in libs:
        print(f"\n=== 跑 {lib} ===")
        fn = extractors[lib]
        results = []
        t_start = time.time()
        for i, p in enumerate(pdfs):
            r = fn(p)
            results.append(r)
            if (i + 1) % 200 == 0:
                gc.collect()
                elapsed = time.time() - t_start
                print(f"  [{i+1}/{len(pdfs)}] elapsed={elapsed:.1f}s "
                      f"pass_rate={(sum(1 for x in results if x[1]=='pass')/len(results)*100):.1f}%")
        all_results[lib] = results

    # 汇总
    print("\n" + "=" * 70)
    print("汇总")
    print("=" * 70)
    summaries = {lib: summarize(res) for lib, res in all_results.items()}

    headers = ["Library", "Mean", "p50", "p95", "p99", "Pass Rate", "Count"]
    rows = []
    for lib, s in summaries.items():
        rows.append([
            lib,
            f"{s['mean_ms']:.1f}ms",
            f"{s['p50_ms']:.1f}ms",
            f"{s['p95_ms']:.1f}ms",
            f"{s['p99_ms']:.1f}ms",
            f"{s['pass_rate']:.1f}%",
            f"{s['pass_n']}/{s['n']}",
        ])

    # 打印 markdown 表格
    print()
    print("| " + " | ".join(headers) + " |")
    print("|" + "|".join(["---"] * len(headers)) + "|")
    for row in rows:
        print("| " + " | ".join(row) + " |")

    # 详细分布
    print("\n详细：")
    for lib, s in summaries.items():
        print(f"  {lib}: mean={s['mean_ms']:.2f}ms p50={s['p50_ms']:.2f} "
              f"p95={s['p95_ms']:.2f} p99={s['p99_ms']:.2f} "
              f"pass={s['pass_n']}/{s['n']} ({s['pass_rate']:.2f}%) "
              f"fail={s['fail']} timeout={s['timeout']}")


if __name__ == "__main__":
    main()
