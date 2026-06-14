"""ritz 核心测试：移植 PyMuPDF 关键场景。

覆盖：
  - Document: open / page_count / metadata / __len__ / __getitem__
  - Page: rect / mediabox / cropbox / rotation
  - get_text: text/html/xhtml/xml/json/rawjson/words/blocks/dict/rawdict
  - get_images: 不带 data / 带 data
  - get_links
  - Pixmap: get_pixmap + tobytes("png") / save_png
  - get_text_batch
  - process_documents (并行)
  - 异常路径：不存在文件、越界页号、未知 mode

运行：pytest python/tests/
"""

import json
import os
import struct
from pathlib import Path

import pytest

import ritz

SAMPLE_PDF = Path(__file__).resolve().parents[2] / "crates" / "mupdf" / "tests" / "samples" / "sample.pdf"
IMAGE_PDF = Path(__file__).resolve().parents[2] / "vendor" / "mupdf" / "thirdparty" / "extract" / "test" / "text_graphic_image.pdf"
# 带大纲的 PDF（仅 search/toc 测试用，缺失时自动 skip）
ARXIV_PDF = Path("/Users/xiongzhaolong/Downloads/claude-pro/code_2026/arxiv_2603.09677_translated.pdf")


def sample_pdf():
    return str(SAMPLE_PDF)


def image_pdf():
    return str(IMAGE_PDF if IMAGE_PDF.exists() else IMAGE_PDF)


@pytest.fixture(scope="module")
def doc():
    d = ritz.open(sample_pdf())
    assert d is not None
    yield d
    # Document 没有 close 方法，靠 GC；不显式 drop


# ----------------------------------------------------------------------------
# Document 测试
# ----------------------------------------------------------------------------

class TestDocument:
    def test_page_count(self, doc):
        assert len(doc) >= 1
        assert doc.page_count == len(doc)

    def test_getitem(self, doc):
        p = doc[0]
        assert p is not None

    def test_getitem_out_of_range(self, doc):
        with pytest.raises(Exception):
            _ = doc[9999]

    def test_metadata_format(self, doc):
        # metadata 是 @property dict（PyMuPDF 兼容）
        md = doc.metadata
        assert isinstance(md, dict)
        # PyMuPDF 几乎总有 format
        assert md.get("format") is None or "PDF" in md["format"]

    def test_metadata_missing_key(self, doc):
        # 不存在的 key 通过 .get 返回 None
        assert doc.metadata.get("nonexistent_key_xyz") is None

    def test_lookup_metadata(self, doc):
        # lookup_metadata 直接查 MuPDF key
        fmt = doc.lookup_metadata("format")
        assert fmt is None or "PDF" in fmt

    def test_open_nonexistent(self):
        with pytest.raises(Exception):
            ritz.open("/nonexistent/file.pdf")

    def test_repr(self, doc):
        r = repr(doc)
        assert "ritz.Document" in r


# ----------------------------------------------------------------------------
# Page 测试
# ----------------------------------------------------------------------------

class TestPage:
    def test_rect(self, doc):
        r = doc[0].rect
        assert len(r) == 4
        x0, y0, x1, y1 = r
        assert x1 > x0
        assert y1 > y0

    def test_mediabox_cropbox(self, doc):
        mb = doc[0].mediabox
        cb = doc[0].cropbox
        assert mb != cb or mb == cb  # 可能相等也可能不等
        # mediabox 一定是有效矩形
        assert mb[2] > mb[0] and mb[3] > mb[1]

    def test_rotation(self, doc):
        rot = doc[0].rotation
        assert rot in (0, 90, 180, 270)

    def test_repr(self, doc):
        r = repr(doc[0])
        assert "ritz.Page" in r


# ----------------------------------------------------------------------------
# get_text 各模式
# ----------------------------------------------------------------------------

class TestGetText:
    @pytest.mark.parametrize("mode", [
        "text", "html", "xhtml", "xml",
    ])
    def test_string_modes(self, doc, mode):
        s = doc[0].get_text(mode)
        assert isinstance(s, str)
        assert len(s) > 0

    def test_text_mode_non_empty(self, doc):
        s = doc[0].get_text("text")
        assert len(s) > 0

    def test_json_structure(self, doc):
        j = doc[0].get_text("json")
        parsed = json.loads(j)
        assert "blocks" in parsed
        assert isinstance(parsed["blocks"], list)

    def test_rawjson_has_chars(self, doc):
        j = doc[0].get_text("rawjson")
        parsed = json.loads(j)
        # 至少有一个 block/line/span 里能找到 chars（如果页上有文本）
        if parsed["blocks"]:
            b = parsed["blocks"][0]
            if b.get("lines"):
                ln = b["lines"][0]
                if ln.get("spans"):
                    # rawjson 的 span 应该有 chars（如果该 span 非空）
                    assert "chars" in ln["spans"][0]

    def test_words_format(self, doc):
        words = doc[0].get_text("words")
        assert isinstance(words, list)
        # PyMuPDF: 每个单词是 (x0, y0, x1, y1, text, block_no, line_no, word_no)
        if words:
            assert len(words[0]) >= 5

    def test_blocks_format(self, doc):
        blocks = doc[0].get_text("blocks")
        assert isinstance(blocks, list)
        # PyMuPDF: 每个 block = (x0, y0, x1, y1, text, block_no, block_type)
        if blocks:
            assert len(blocks[0]) >= 5

    def test_dict_structure(self, doc):
        d = doc[0].get_text("dict")
        assert "blocks" in d
        if d["blocks"]:
            b = d["blocks"][0]
            assert "type" in b
            assert "bbox" in b
            if b.get("lines"):
                ln = b["lines"][0]
                assert "bbox" in ln
                assert "wmode" in ln
                assert "dir" in ln
                assert "spans" in ln
                if ln["spans"]:
                    s = ln["spans"][0]
                    # dict 模式不应有 chars
                    assert "chars" not in s
                    assert "bbox" in s
                    assert "color" in s
                    assert "font" in s
                    assert "text" in s

    def test_rawdict_has_chars(self, doc):
        d = doc[0].get_text("rawdict")
        assert "blocks" in d
        # 找到一个含文本的 span，验证它有 chars
        for b in d["blocks"]:
            for ln in b.get("lines", []):
                for s in ln.get("spans", []):
                    if s.get("text"):
                        assert "chars" in s
                        assert len(s["chars"]) == len(s["text"])
                        return
        pytest.skip("no text spans on this page")

    def test_unknown_mode_raises(self, doc):
        with pytest.raises(Exception):
            doc[0].get_text("nonexistent_mode")


# ----------------------------------------------------------------------------
# get_images
# ----------------------------------------------------------------------------

class TestGetImages:
    def test_no_data(self, doc):
        # sample.pdf 可能无图
        imgs = doc[0].get_images()
        assert isinstance(imgs, list)

    @pytest.mark.skipif(not IMAGE_PDF.exists(), reason="image test pdf missing")
    def test_image_extraction(self):
        d = ritz.open(str(IMAGE_PDF))
        imgs = d[0].get_images()
        assert len(imgs) > 0
        im = imgs[0]
        assert "bbox" in im
        assert "width" in im
        assert "height" in im
        assert "bpc" in im
        assert "colorspace" in im
        # plan_v1 §3.2: xref 字段必须存在
        assert "xref" in im
        assert isinstance(im["xref"], int)
        assert im["xref"] >= 0
        # 与 PyMuPDF xref 一致（若装了 fitz）
        try:
            import fitz
            fd = fitz.open(str(IMAGE_PDF))
            fitz_imgs = fd[0].get_images(full=True)
            if fitz_imgs:
                fitz_xref = fitz_imgs[0][0]
                assert im["xref"] == fitz_xref, f"xref mismatch: ritz={im['xref']} fitz={fitz_xref}"
        except ImportError:
            pass

    @pytest.mark.skipif(not IMAGE_PDF.exists(), reason="image test pdf missing")
    def test_image_with_data(self):
        d = ritz.open(str(IMAGE_PDF))
        imgs = d[0].get_images(include_data=True)
        assert len(imgs) > 0
        im = imgs[0]
        assert "image" in im
        assert "ext" in im
        b = im["image"]
        ext = im["ext"]
        # 按格式验证 magic bytes
        if ext == "jpeg":
            assert b[:3] == b"\xff\xd8\xff", f"jpeg magic mismatch: {b[:8].hex()}"
        elif ext == "png":
            assert b[:8] == b"\x89PNG\r\n\x1a\n", f"png magic mismatch: {b[:8].hex()}"
        elif ext == "jpx":
            assert b[:4] == b"\x00\x00\x00\x0c" or b[:4] == b"\xff\x4f\xffQ", f"jpx magic mismatch"
        # ext 应是已知格式之一
        assert ext in ("jpeg", "png", "jpx"), f"unexpected ext: {ext!r}"


# ----------------------------------------------------------------------------
# get_links
# ----------------------------------------------------------------------------

class TestGetLinks:
    def test_returns_list(self, doc):
        links = doc[0].get_links()
        assert isinstance(links, list)


# ----------------------------------------------------------------------------
# search_for (Phase 5a)
# ----------------------------------------------------------------------------

class TestSearchFor:
    def test_basic_search(self, doc):
        # sample.pdf 文本含 "easy"、"medium"、"hard" 等单词
        hits = doc[0].search_for("easy")
        assert isinstance(hits, list)
        assert len(hits) > 0
        # 默认 Rect 模式：每条是 4 元组
        first = hits[0]
        assert isinstance(first, tuple)
        assert len(first) == 4

    def test_case_insensitive(self, doc):
        # MuPDF 搜索默认大小写不敏感
        upper = doc[0].search_for("EASY")
        lower = doc[0].search_for("easy")
        assert len(upper) == len(lower)

    def test_no_hits(self, doc):
        hits = doc[0].search_for("nonexistent_word_xyz_123")
        assert hits == []

    def test_quads_mode(self, doc):
        hits = doc[0].search_for("easy", quads=True)
        assert len(hits) > 0
        # quads 模式：每条是 8 元组（ul_x, ul_y, ur_x, ur_y, lr_x, lr_y, ll_x, ll_y）
        first = hits[0]
        assert isinstance(first, tuple)
        assert len(first) == 8

    def test_rect_is_bbox_of_quad(self, doc):
        """Rect 模式返回值应是 quad 的包围盒。"""
        rects = doc[0].search_for("easy")
        quads = doc[0].search_for("easy", quads=True)
        assert len(rects) == len(quads)
        for rect, quad in zip(rects, quads):
            ul_x, ul_y, ur_x, ur_y, lr_x, lr_y, ll_x, ll_y = quad
            x0 = min(ul_x, ll_x)
            y0 = min(ul_y, ur_y)
            x1 = max(ur_x, lr_x)
            y1 = max(ll_y, lr_y)
            assert abs(rect[0] - x0) < 0.01
            assert abs(rect[1] - y0) < 0.01
            assert abs(rect[2] - x1) < 0.01
            assert abs(rect[3] - y1) < 0.01

    @pytest.mark.skipif(not ARXIV_PDF.exists(), reason="arxiv fixture missing")
    def test_cjk_search(self):
        """CJK 字符搜索（arxiv 翻译版含中文）。"""
        d = ritz.open(str(ARXIV_PDF))
        # 找一个肯定在文本中的中文词
        hits = d[1].search_for("方法")
        assert len(hits) > 0

    @pytest.mark.skipif(not ARXIV_PDF.exists(), reason="arxiv fixture missing")
    def test_matches_pymupdf(self):
        """与 PyMuPDF 结果对照（若装了 fitz）。"""
        try:
            import fitz
        except ImportError:
            pytest.skip("PyMuPDF not installed")
        d_ritz = ritz.open(str(ARXIV_PDF))
        d_fitz = fitz.open(str(ARXIV_PDF))
        for pno in range(min(5, len(d_ritz))):
            text = d_ritz[pno].get_text("text")
            # 从文本里取一个真实出现的英文 token
            tokens = [w for w in text.replace(".", " ").split() if w.isalpha() and len(w) >= 4]
            if not tokens:
                continue
            needle = tokens[0]
            r_ritz = d_ritz[pno].search_for(needle)
            r_fitz = d_fitz[pno].search_for(needle)
            assert len(r_ritz) == len(r_fitz), (
                f"p{pno} search {needle!r}: ritz={len(r_ritz)} fitz={len(r_fitz)}"
            )


# ----------------------------------------------------------------------------
# get_toc (Phase 5a)
# ----------------------------------------------------------------------------

class TestGetToc:
    def test_no_outline(self, doc):
        """sample.pdf 没有大纲，返回 None（与 PyMuPDF 一致）。"""
        toc = doc.get_toc()
        assert toc is None

    def test_returns_none_or_list(self, doc):
        toc = doc.get_toc()
        assert toc is None or isinstance(toc, list)

    @pytest.mark.skipif(not ARXIV_PDF.exists(), reason="arxiv fixture missing")
    def test_arxiv_toc(self):
        d = ritz.open(str(ARXIV_PDF))
        toc = d.get_toc()
        assert toc is not None
        assert isinstance(toc, list)
        assert len(toc) > 0
        # 每条是 (level, title, page) 3 元组
        first = toc[0]
        assert len(first) == 3
        level, title, page = first
        assert isinstance(level, int)
        assert level >= 1
        assert isinstance(title, str)
        assert isinstance(page, int)
        # 顶层条目 page 应是有效页码（>=1）或是 -1（外链）
        assert page == -1 or page >= 1

    @pytest.mark.skipif(not ARXIV_PDF.exists(), reason="arxiv fixture missing")
    def test_levels_are_nested(self):
        """大纲应含多层级。"""
        d = ritz.open(str(ARXIV_PDF))
        toc = d.get_toc()
        levels = {entry[0] for entry in toc}
        assert 1 in levels  # 至少有顶层
        # 多层结构会出现 > 1 的层级
        assert any(l > 1 for l in levels), f"only level-1 found: {levels}"

    @pytest.mark.skipif(not ARXIV_PDF.exists(), reason="arxiv fixture missing")
    def test_matches_pymupdf(self):
        """与 PyMuPDF get_toc(simple=True) 对照。"""
        try:
            import fitz
        except ImportError:
            pytest.skip("PyMuPDF not installed")
        d_ritz = ritz.open(str(ARXIV_PDF))
        d_fitz = fitz.open(str(ARXIV_PDF))
        r = d_ritz.get_toc()
        f = d_fitz.get_toc(simple=True)
        if r is None and f == []:
            return
        assert r is not None and f != []
        assert len(r) == len(f), f"len mismatch: ritz={len(r)} fitz={len(f)}"
        # 逐条对照（title 可能因编码细微差异，对照 level 和 page）
        for i, (re, fe) in enumerate(zip(r, f)):
            assert re[0] == fe[0], f"entry {i} level mismatch: ritz={re[0]} fitz={fe[0]}"
            assert re[2] == fe[2], f"entry {i} page mismatch: ritz={re[2]} fitz={fe[2]} (title={re[1]!r})"


# ----------------------------------------------------------------------------
# Pixmap
# ----------------------------------------------------------------------------

class TestPixmap:
    def test_render_default(self, doc):
        pix = doc[0].get_pixmap()
        assert pix.width > 0
        assert pix.height > 0

    def test_render_dpi(self, doc):
        pix = doc[0].get_pixmap(dpi=144)
        # 144/72 = 2x
        pix_default = doc[0].get_pixmap()
        assert pix.width >= pix_default.width * 1.5

    def test_tobytes_png(self, doc):
        b = doc[0].get_pixmap().tobytes("png")
        assert b[:8] == b"\x89PNG\r\n\x1a\n"

    def test_save_png(self, doc, tmp_path):
        out = tmp_path / "out.png"
        doc[0].get_pixmap().save_png(str(out))
        assert out.exists()
        assert out.stat().st_size > 100

    def test_samples(self, doc):
        s = doc[0].get_pixmap().samples
        assert len(s) > 0


# ----------------------------------------------------------------------------
# get_text_batch
# ----------------------------------------------------------------------------

class TestTextBatch:
    def test_batch_matches_per_page(self, doc):
        batch = doc.get_text_batch()
        per_page = [doc[i].get_text("text") for i in range(len(doc))]
        assert batch == per_page

    def test_batch_count(self, doc):
        batch = doc.get_text_batch()
        assert len(batch) == len(doc)


# ----------------------------------------------------------------------------
# process_documents（并行）
# ----------------------------------------------------------------------------

class TestProcessDocuments:
    def test_parallel_serial_match(self):
        paths = [sample_pdf()]
        if IMAGE_PDF.exists():
            paths.append(str(IMAGE_PDF))

        serial = []
        for p in paths:
            try:
                d = ritz.open(p)
                serial.append(d.get_text_batch())
            except Exception:
                serial.append(None)

        parallel = ritz.process_documents(paths)
        assert len(parallel) == len(paths)
        for s, p in zip(serial, parallel):
            assert s == p

    def test_handles_missing_files(self):
        # 不存在的文件应返回 None（不抛异常）
        results = ritz.process_documents(["/nonexistent/a.pdf", "/nonexistent/b.pdf"])
        assert len(results) == 2
        assert all(r is None for r in results)
