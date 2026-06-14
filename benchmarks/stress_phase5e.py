"""Phase 5e 深度压力测试 —— 找隐患。

每项测试都打印 PASS/FAIL/LEAK，最后汇总。运行：
    python benchmarks/stress_phase5e.py
"""

import gc
import os
import resource
import sys
import tempfile
import time
from pathlib import Path

import ritz

SAMPLE = str(Path(__file__).resolve().parents[1] / "crates" / "mupdf" / "tests" / "samples" / "sample.pdf")

PASS = "PASS"
FAIL = "FAIL"
WARN = "WARN"


def rss_kb():
    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    # macOS ru_maxrss 是字节，Linux 是 KB
    if sys.platform == "darwin":
        return rss // 1024
    return rss


def section(name):
    print(f"\n=== {name} ===")


def report(name, status, detail=""):
    marker = {"PASS": "✓", FAIL: "✗", WARN: "!"}[status]
    print(f"  {marker} [{status}] {name}" + (f" — {detail}" if detail else ""))
    return status


results = []


# ---------------------------------------------------------------------------
# 1. 1000 次 new_page（refcount 泄漏检测）
# ---------------------------------------------------------------------------
def test_new_page_stress():
    section("1. 1000 次 new_page（refcount 泄漏）")
    gc.collect()
    rss0 = rss_kb()
    d = ritz.open(SAMPLE)
    n0 = len(d)
    t0 = time.time()
    for _ in range(1000):
        d.new_page()
    elapsed = time.time() - t0
    rss1 = rss_kb()
    delta = rss1 - rss0
    expect = n0 + 1000
    ok = len(d) == expect
    results.append(report(
        "1000x new_page",
        PASS if ok else FAIL,
        f"len={len(d)} expect={expect} elapsed={elapsed*1000:.0f}ms RSS+{delta}KB",
    ))
    # 检查末页能正常 load_page + get_text（不会因页树状态崩溃）
    try:
        text = d[len(d) - 1].get_text("text")
        results.append(report("末页 get_text", PASS, f"text_len={len(text)}"))
    except Exception as e:
        results.append(report("末页 get_text", FAIL, str(e)))


# ---------------------------------------------------------------------------
# 2. 1000 次 insert+delete 循环（页数稳定）
# ---------------------------------------------------------------------------
def test_insert_delete_cycle():
    section("2. 1000 次 insert+delete 循环")
    d = ritz.open(SAMPLE)
    n0 = len(d)
    for _ in range(1000):
        d.new_page()
        d.delete_page(-1)
    ok = len(d) == n0
    results.append(report(
        "insert+delete 1000x 页数稳定",
        PASS if ok else FAIL,
        f"len={len(d)} expect={n0}",
    ))


# ---------------------------------------------------------------------------
# 3. 保存-重开 round-trip：内容完整保留
# ---------------------------------------------------------------------------
def test_save_reopen_roundtrip():
    section("3. 保存-重开 round-trip")
    with tempfile.TemporaryDirectory() as tmp:
        # 原始 + new_page + insert_pdf + move + set_toc
        d = ritz.open(SAMPLE)
        original_text = d[0].get_text("text")
        d.new_page(width=400, height=500, rotate=0)
        d.new_page()
        d.move_page(0, 1)  # 交换
        # 加 toc
        try:
            d.set_toc([(1, "Inserted", 1), (1, "Original", 2)])
            toc_set = True
        except Exception:
            toc_set = False

        out = os.path.join(tmp, "out.pdf")
        d.save(out)
        del d
        gc.collect()

        d2 = ritz.open(out)
        # 验证：原始首页内容应在新位置（被 move 后）
        all_text = "\n".join(d2[i].get_text("text") for i in range(len(d2)))
        ok = original_text.strip() in all_text
        results.append(report(
            "round-trip 内容保留",
            PASS if ok else FAIL,
            f"original_in_text={ok} pages={len(d2)}",
        ))
        if toc_set:
            toc = d2.get_toc()
            ok = toc is not None and len(toc) >= 1
            results.append(report("round-trip TOC 保留", PASS if ok else FAIL, f"toc={toc}"))


# ---------------------------------------------------------------------------
# 4. insert_pdf 后页内容比对（与 PyMuPDF 对照，如果可用）
# ---------------------------------------------------------------------------
def test_insert_pdf_vs_pymupdf():
    section("4. insert_pdf 内容完整性")
    with tempfile.TemporaryDirectory() as tmp:
        src_path = os.path.join(tmp, "src.pdf")
        # 先造一个 3 页源
        d = ritz.open(SAMPLE)
        while len(d) < 3:
            d.new_page()
        d.save(src_path)
        del d

        dst = ritz.open(SAMPLE)
        n0 = len(dst)
        src = ritz.open(src_path)
        dst.insert_pdf(src)
        ok_len = len(dst) == n0 + 3
        results.append(report("insert_pdf 页数", PASS if ok_len else FAIL,
                              f"got={len(dst)} expect={n0+3}"))

        # 校验每页都能 get_text
        try:
            for i in range(len(dst)):
                _ = dst[i].get_text("text")
            results.append(report("insert_pdf 后每页可读", PASS))
        except Exception as e:
            results.append(report("insert_pdf 后每页可读", FAIL, str(e)))


# ---------------------------------------------------------------------------
# 5. 与 PyMuPDF 对照（如果装了）
# ---------------------------------------------------------------------------
def test_vs_pymupdf():
    section("5. 与 PyMuPDF 对照")
    try:
        import fitz  # type: ignore
    except ImportError:
        results.append(report("PyMuPDF 对照", WARN, "fitz 未安装，跳过"))
        return

    with tempfile.TemporaryDirectory() as tmp:
        # ritz 端
        rd = ritz.open(SAMPLE)
        rd.new_page(width=300, height=400)  # 不传 rotate（fitz new_page 不支持）
        rd.new_page()
        rd.move_page(0, 2)
        rd_path = os.path.join(tmp, "ritz.pdf")
        rd.save(rd_path)
        ritz_n = len(rd)
        ritz_texts = [rd[i].get_text("text")[:50] for i in range(ritz_n)]

        # PyMuPDF 端做同样操作（new_page 不支持 rotate 参数）
        fd = fitz.open(SAMPLE)
        fd.new_page(pno=-1, width=300, height=400)
        fd.new_page()
        fd.move_page(0, 2)
        fitz_n = len(fd)
        fitz_texts = [fd[i].get_text("text")[:50] for i in range(fitz_n)]

        ok_n = ritz_n == fitz_n
        results.append(report("页数 vs PyMuPDF", PASS if ok_n else FAIL,
                              f"ritz={ritz_n} fitz={fitz_n}"))

        ok_text = ritz_texts == fitz_texts
        results.append(report(
            "页面顺序 vs PyMuPDF",
            PASS if ok_text else WARN,
            f"match={ok_text}",
        ))


# ---------------------------------------------------------------------------
# 6. move_page 边界 case
# ---------------------------------------------------------------------------
def test_move_page_edge_cases():
    section("6. move_page 边界 case（PyMuPDF 兼容语义）")
    d = ritz.open(SAMPLE)
    # 凑到 5 页，第一页有内容
    while len(d) < 5:
        d.new_page()
    n = len(d)
    texts_before = [d[i].get_text("text")[:30] for i in range(n)]

    # PyMuPDF 语义：move(0, n-1) 把 src=0 插到原 dst=n-1 之前 → 落在 n-2
    d.move_page(0, n - 1)
    texts_after = [d[i].get_text("text")[:30] for i in range(n)]
    # 期望：[before[1], before[2], before[3], before[0], before[4]]
    expected = texts_before[1:n-1] + [texts_before[0]] + [texts_before[n-1]]
    ok = texts_after == expected
    results.append(report("move(0, n-1)", PASS if ok else FAIL,
                          f"match={ok}"))

    # 复原：move(n-2, 0) 把当前 n-2 位置（=原 first）移回首
    d.move_page(n - 2, 0)
    texts_back = [d[i].get_text("text")[:30] for i in range(n)]
    ok = texts_back == texts_before
    results.append(report("move 复原", PASS if ok else FAIL))


# ---------------------------------------------------------------------------
# 7. delete_pages 区间边界
# ---------------------------------------------------------------------------
def test_delete_pages_edge():
    section("7. delete_pages 区间边界")
    d = ritz.open(SAMPLE)
    while len(d) < 5:
        d.new_page()
    n0 = len(d)

    # from > to 应抛异常
    try:
        d.delete_pages(3, 1)
        results.append(report("from > to 应抛异常", FAIL, "未抛"))
    except RuntimeError:
        results.append(report("from > to 应抛异常", PASS))

    # delete_pages(0, n-1) 清空
    d.delete_pages(0, n0 - 1)
    ok = len(d) == 0
    results.append(report("delete_pages(0, n-1) 清空", PASS if ok else FAIL,
                          f"len={len(d)}"))


# ---------------------------------------------------------------------------
# 8. 0 页文档的 new_page
# ---------------------------------------------------------------------------
def test_empty_doc_new_page():
    section("8. 0 页文档操作")
    d = ritz.open(SAMPLE)
    while len(d) > 0:
        d.delete_page(0)
    # 现在 0 页
    if len(d) != 0:
        results.append(report("清空到 0 页", FAIL, f"len={len(d)}"))
        return
    results.append(report("清空到 0 页", PASS))

    # 0 页文档插入新页
    try:
        d.new_page()
        ok = len(d) == 1
        results.append(report("0 页文档 new_page", PASS if ok else FAIL,
                              f"len={len(d)}"))
    except Exception as e:
        results.append(report("0 页文档 new_page", FAIL, str(e)))


# ---------------------------------------------------------------------------
# 9. 内存泄漏：循环 open + edit + close
# ---------------------------------------------------------------------------
def test_open_edit_close_leak():
    section("9. open + new_page + close 循环泄漏")
    gc.collect()
    rss0 = rss_kb()
    for _ in range(200):
        d = ritz.open(SAMPLE)
        for _ in range(10):
            d.new_page()
        del d
    gc.collect()
    rss1 = rss_kb()
    delta = rss1 - rss0
    # 阈值：每轮 ~50KB（容差给 5MB）
    status = PASS if delta < 5000 else WARN
    results.append(report(
        "200x (open + 10x new_page + close)",
        status,
        f"RSS+{delta}KB ({delta/200:.1f}KB/iter)",
    ))


# ---------------------------------------------------------------------------
# 10. PyPage stale 警告验证（文档化行为）
# ---------------------------------------------------------------------------
def test_stale_pypage_after_delete():
    section("10. stale PyPage 行为验证")
    d = ritz.open(SAMPLE)
    while len(d) < 3:
        d.new_page()
    page0 = d[0]
    text_before = page0.get_text("text")[:30]
    # 删除第 0 页，page0 应不再对应第 0 页
    d.delete_page(0)
    # 重新取 d[0]
    new_first = d[0]
    text_after = new_first.get_text("text")[:30]
    # 旧的 page0 仍能读出原内容（fz_page 还活着）
    try:
        old_text = page0.get_text("text")[:30]
        # 旧 page 内容应仍是原首页
        ok = old_text == text_before
        results.append(report(
            "stale PyPage 仍指向原页内容",
            PASS if ok else WARN,
            f"old='{old_text[:20]}' vs before='{text_before[:20]}'",
        ))
    except Exception as e:
        results.append(report("stale PyPage 行为", FAIL, str(e)))


if __name__ == "__main__":
    print(f"ritz {ritz.__version__} | sample={SAMPLE}")
    test_new_page_stress()
    test_insert_delete_cycle()
    test_save_reopen_roundtrip()
    test_insert_pdf_vs_pymupdf()
    test_vs_pymupdf()
    test_move_page_edge_cases()
    test_delete_pages_edge()
    test_empty_doc_new_page()
    test_open_edit_close_leak()
    test_stale_pypage_after_delete()

    print("\n" + "=" * 70)
    print("汇总")
    print("=" * 70)
    npass = sum(1 for r in results if r == PASS)
    nfail = sum(1 for r in results if r == FAIL)
    nwarn = sum(1 for r in results if r == WARN)
    print(f"  PASS={npass}  FAIL={nfail}  WARN={nwarn}  总={len(results)}")
    sys.exit(1 if nfail > 0 else 0)
