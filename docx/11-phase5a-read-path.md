# Phase 5a：读路径扩展（文本搜索 + 大纲读取）

> v0.2.0（2026-06-14）落地。本文记录 `page.search_for()` 和 `doc.get_toc()` 的实现细节。

## 总览

Phase 5a 补上了 ritz 相对 PyMuPDF 的两个最常用读路径 API：

| API | PyMuPDF 等价 | 用途 |
|-----|-------------|------|
| `page.search_for(needle, quads=False)` | `page.search_for()` | 页面内文本搜索，返回命中位置坐标 |
| `doc.get_toc()` | `doc.get_toc(simple=True)` | 文档大纲（目录）读取 |

两者都遵循 ritz 的核心原则：**C 层遍历 + growbuf 扁平化 + 一次 FFI 返回**。

## `page.search_for()` 实现

### 数据流

```
Python: page.search_for("keyword", quads=True)
  ↓ PyO3
Rust:   PyPage::search_for(py, needle, quads)          [page.rs:214]
  ↓     mupdf_safe_new_stext_page(ctx, raw, &stpage)   [FFI #1]
  ↓     mupdf_safe_search_stext_page(ctx, stpage, ...)  [FFI #2]
C:      fz_search_stext_page_cb() → search_collect_cb → growbuf
  ↓     返回 float* (连续 quads) + int (quad 数量)
Rust:   slice::from_raw_parts → 逐 quad 构造 tuple
  ↓     mupdf_safe_drop_stext_page                      [FFI #3]
Python: list[(x0,y0,x1,y1)] 或 list[(ul_x,ul_y,...,ll_x,ll_y)]
```

### C 层：回调式搜索

MuPDF 提供 `fz_search_stext_page_cb` 回调 API——每找到一个命中就调回调函数，避免了传统的 count+fill 两趟模式：

```c
// error_wrapper.c
struct search_collect_state {
    growbuf gb;
    int quad_count;
};

static int search_collect_cb(fz_context *ctx, void *opaque,
                             int num_quads, fz_quad *hit_bbox) {
    struct search_collect_state *st = opaque;
    gb_put(ctx, &st->gb, hit_bbox, sizeof(fz_quad) * num_quads);
    st->quad_count += num_quads;
    return 0;  // 0 = 继续搜索
}
```

`fz_quad` 内存布局：4 个 `fz_point`（每点 2 floats）= 8 floats = 32 bytes。MuPDF 的顺序是 ul→ur→ll→lr。

### Rust 侧：quad → rect 转换

```rust
// page.rs:241-265
for i in 0..n as usize {
    let base = i * 8;
    let (ul_x, ul_y) = (slice[base], slice[base + 1]);
    let (ur_x, ur_y) = (slice[base + 2], slice[base + 3]);
    let (ll_x, ll_y) = (slice[base + 4], slice[base + 5]);
    let (lr_x, lr_y) = (slice[base + 6], slice[base + 7]);

    if quads {
        // PyMuPDF Quad 顺序：ul→ur→lr→ll（顺时针）
        out.push((ul_x, ul_y, ur_x, ur_y, lr_x, lr_y, ll_x, ll_y));
    } else {
        // Rect = 包围盒
        let x0 = ul_x.min(ll_x);
        let y0 = ul_y.min(ur_y);
        let x1 = ur_x.max(lr_x);
        let y1 = ll_y.max(lr_y);
        out.push((x0, y0, x1, y1));
    }
}
```

注意 MuPDF 内部 quad 顺序（ul→ur→**ll**→lr）与 PyMuPDF 输出顺序（ul→ur→**lr**→ll）不同，Rust 侧做了重排。

### 与 PyMuPDF 的行为一致性

- **大小写不敏感**：MuPDF `fz_search_stext_page` 默认 case-insensitive
- **跨行命中**：一次命中跨多行时，MuPDF 返回多个 quad（每行一个），ritz 原样返回，与 PyMuPDF 行为一致
- **CJK 搜索**：复用 stext 基础设施，UTF-8 原生支持

## `doc.get_toc()` 实现

### 数据流

```
Python: doc.get_toc()
  ↓ PyO3
Rust:   PyDocument::get_toc(py)                         [document.rs:182]
  ↓     mupdf_safe_load_outline(ctx, doc, &ptr, &len, &n) [FFI #1]
C:      fz_load_outline → walk_outline 递归 → growbuf
  ↓     返回 char* (连续 buffer) + size_t + int
Rust:   slice::from_raw_parts → 逐 entry 解析 level/page/title
  ↓     mupdf_free(ptr)                                  [FFI #2]
Python: list[(level, title, page)] | None
```

### C 层：递归前序遍历 + growbuf 扁平化

```c
// error_wrapper.c
static void walk_outline(fz_context *ctx, fz_document *doc,
                         fz_outline *node, int level,
                         growbuf *gb, int *count) {
    for (; node; node = node->next) {
        int raw_page = fz_page_number_from_location(ctx, doc, node->page);
        int py_page = (raw_page >= 0) ? raw_page + 1 : -1;  // 1-based, 外链=-1
        const char *title = node->title ? node->title : "";
        int title_len = (int)strlen(title);
        int flags = (int)node->flags;

        // 每条 entry：[level:4][page:4][flags:4][title_len:4][title:变长]
        gb_put(ctx, gb, &level_i, sizeof(int));
        gb_put(ctx, gb, &py_page, sizeof(int));
        gb_put(ctx, gb, &flags, sizeof(int));
        gb_put(ctx, gb, &title_len, sizeof(int));
        if (title_len > 0) gb_put(ctx, gb, title, title_len);
        (*count)++;

        if (node->down) walk_outline(ctx, doc, node->down, level + 1, gb, count);
    }
}
```

### buffer 协议

每条 outline entry 在 buffer 中的布局：

| 偏移 | 长度 | 字段 | 说明 |
|------|------|------|------|
| 0 | 4 | level | 1-based 层级（int32） |
| 4 | 4 | page | 1-based 页码，外链 = -1（int32） |
| 8 | 4 | flags | 标志位（当前未暴露给 Python） |
| 12 | 4 | title_len | UTF-8 标题字节长度（int32） |
| 16 | title_len | title | UTF-8 标题内容 |

Rust 侧按此协议逐条解析，构造 `(level, title, page)` tuple。

### PyMuPDF 兼容性

- **返回值**：`list[(level, title, page)]` 与 `fitz.get_toc(simple=True)` 一致
- **无大纲时**：返回 `None`（不是空 list），与 PyMuPDF 一致
- **page 编号**：1-based（PyMuPDF 约定），外链目标 page = -1
- **level 编号**：1-based（顶层 = 1）

## 测试覆盖

### TestSearchFor（7 个用例）

| 用例 | 验证点 |
|------|--------|
| `test_basic_search` | 基本搜索返回非空 list，每条是 4 元组 |
| `test_case_insensitive` | 大小写不敏感（"EASY" == "easy"） |
| `test_no_hits` | 不存在的词返回空 list |
| `test_quads_mode` | `quads=True` 返回 8 元组 |
| `test_rect_is_bbox_of_quad` | rect 模式是 quad 的包围盒（数值验证） |
| `test_cjk_search` | CJK 字符搜索（arxiv 翻译版） |
| `test_matches_pymupdf` | 与 PyMuPDF 搜索结果数量一致 |

### TestGetToc（5 个用例）

| 用例 | 验证点 |
|------|--------|
| `test_no_outline` | 无大纲文档返回 None |
| `test_returns_none_or_list` | 返回类型检查 |
| `test_arxiv_toc` | arxiv PDF 有大纲，每条是 (level, title, page) |
| `test_levels_are_nested` | 大纲含多层级（level > 1） |
| `test_matches_pymupdf` | 与 PyMuPDF 大纲结果对照 |

## 性能特征

两个 API 都遵循 ritz 的标准模式：**1 次 FFI 拿全部数据**。

- `search_for`：3 次 FFI（new_stext_page + search + drop_stext_page），与 PyMuPDF 的 N 次 FFI（每 hit 一次）相比，命中数越多优势越大
- `get_toc`：2 次 FFI（load_outline + free），一次拿全部 entry

由于两者都复用 stext 基础设施，瓶颈仍在 MuPDF 内部计算（stext 构造 + 搜索遍历），预期加速比 ~2x（与文本提取类似）。

## 相关文件

| 文件 | 改动 |
|------|------|
| `crates/mupdf-sys/native/error_wrapper.c` | `mupdf_safe_search_stext_page` + `mupdf_safe_load_outline` + `walk_outline` |
| `crates/mupdf-sys/native/error_wrapper.h` | 对应声明 |
| `crates/mupdf-sys/bindings.rs` | 重新生成 |
| `crates/ritz/src/page.rs` | `search_for()` 方法 |
| `crates/ritz/src/document.rs` | `get_toc()` 方法 |
| `python/tests/test_ritz.py` | `TestSearchFor` + `TestGetToc`（12 个用例） |
