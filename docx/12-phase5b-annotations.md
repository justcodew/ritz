# Phase 5b：注释读写

> v0.2.0（2026-06-14）落地。本文记录注释（annotation）读写的实现细节。

## 总览

Phase 5b 实现了 PDF 注释的读取、创建和删除，覆盖四种最常用类型：

| API | PyMuPDF 等价 | 用途 |
|-----|-------------|------|
| `page.get_annotations()` | `page.annots()` | 读取页面所有注释 |
| `page.add_highlight_annot(quads, ...)` | `page.add_highlight_annot(quads)` | 添加高亮注释 |
| `page.add_underline_annot(quads, ...)` | `page.add_underline_annot(quads)` | 添加下划线注释 |
| `page.add_strikeout_annot(quads, ...)` | `page.add_strikeout_annot(quads)` | 添加删除线注释 |
| `page.add_text_annot(rect, contents, ...)` | `page.add_text_annot(point, contents)` | 添加文本注释（便签） |
| `page.delete_annot(index)` | `annot.delete()` | 按索引删除注释 |

## `page.get_annotations()` 实现

### 数据流

```
Python: page.get_annotations()
  ↓ PyO3
Rust:   PyPage::get_annotations(py)                            [page.rs]
  ↓     mupdf_safe_get_annotations(ctx, doc, page, &ptr, &len, &n) [FFI]
C:      pdf_first_annot → 遍历 → 读 PDF dict 属性 → growbuf
  ↓     返回 char* + size_t + int
Rust:   slice::from_raw_parts → 逐条解析 → list[dict]
Python: list[dict]  每条含 type/rect/flags/contents/author/color/quads
```

### C 层：直接读 PDF 字典

**关键设计决策**：不使用 MuPDF 的注释属性访问函数（如 `pdf_annot_rect`、`pdf_annot_color`），而是直接从 PDF 字典读取。原因是 MuPDF 的访问函数可能有副作用（触发 appearance stream 合成、修改链表状态），在遍历过程中会导致崩溃。

```c
// error_wrapper.c — 直接从字典读属性
pdf_obj *aobj = pdf_annot_obj(ctx, annot);
pdf_obj *robj = pdf_dict_get(ctx, aobj, PDF_NAME(Rect));
// ... 直接 pdf_to_real 取值
```

### 注释遍历的 refcount 陷阱

**核心发现**：`pdf_first_annot` / `pdf_next_annot` 返回的指针**不增加 refcount**——注释由 page 持有。如果在遍历循环中调用 `pdf_drop_annot`，会释放 page 持有的内存，导致链表损坏。

```c
// ❌ 错误：会破坏链表
for (pdf_annot *annot = pdf_first_annot(ctx, ppage); annot; ) {
    // ... 读属性 ...
    pdf_drop_annot(ctx, annot);  // ← 释放了 page 持有的 annot!
    annot = pdf_next_annot(ctx, annot);  // ← 链表已损坏，无限循环或 crash
}

// ✅ 正确：不 drop
for (pdf_annot *annot = pdf_first_annot(ctx, ppage); annot;
     annot = pdf_next_annot(ctx, annot)) {
    // ... 读属性 ...
    // 不 drop —— annot 由 page 持有
}
```

这个 bug 导致了 2+ 注释时的无限循环，花了大量时间排查。

### Highlight/Underline/StrikeOut 的 Rect 问题

这三种注释类型在 PDF 中**没有 /Rect 条目**——它们只有 /QuadPoints。C 层需要从 QuadPoints 计算包围盒：

```c
if (rect.x0 == 0 && rect.y0 == 0 && rect.x1 == 0 && rect.y1 == 0
    && pdf_is_array(ctx, qp) && pdf_array_len(ctx, qp) >= 8) {
    float min_x = 1e30f, min_y = 1e30f, max_x = -1e30f, max_y = -1e30f;
    for (int pi = 0; pi < total_pts; pi += 2) {
        float px = pdf_to_real(ctx, pdf_array_get(ctx, qp, pi));
        float py = pdf_to_real(ctx, pdf_array_get(ctx, qp, pi + 1));
        if (px < min_x) min_x = px;
        // ...
    }
    rect.x0 = min_x; rect.y0 = min_y;
    rect.x1 = max_x; rect.y1 = max_y;
}
```

### buffer 协议

每条注释在 buffer 中的布局：

| 偏移 | 长度 | 字段 | 说明 |
|------|------|------|------|
| 0 | 4 | type | 注释类型（int32，pdf_annot_type 枚举） |
| 4 | 16 | rect | 包围盒 x0,y0,x1,y1（4 × float32） |
| 20 | 4 | flags | 标志位（int32） |
| 24 | 4 | color_n | 颜色分量数（0/3/4） |
| 28 | 16 | color | 颜色值（4 × float32） |
| 44 | 4 | contents_len | 内容文本长度 |
| 48 | 4 | author_len | 作者名长度 |
| 52 | 4 | quad_count | QuadPoints 数量 |
| 56 | 变长 | contents | UTF-8 内容 |
| ... | 变长 | author | UTF-8 作者名 |
| ... | 32×N | quads | 每个 quad = 8 floats |

Rust 侧解析后，type 映射为字符串名（"Highlight"、"Underline" 等），与 PyMuPDF 一致。

## `page.add_*_annot()` 实现

### 创建注释的 refcount 模式

使用 `pdf_create_annot_raw` 而非 `pdf_create_annot`：

```c
pdf_annot *annot = pdf_create_annot_raw(ctx, ppage, type);
// pdf_create_annot_raw 返回 refcount=2（内部 keep + page 持有）
// ... 设置属性（quads、color、contents）...
pdf_drop_annot(ctx, annot);
// refcount 降回 1（page 持有）
```

`pdf_create_annot` 会调用 `pdf_begin_operation` / `pdf_end_operation`，在某些场景下导致链表损坏。`pdf_create_annot_raw` 跳过这些副作用。

**不调用 `pdf_update_annot`**：appearance stream 在 `doc.save()` 时自动合成，不需要在创建时生成。

### 注释类型编码

| Python 方法 | pdf_annot_type 枚举值 |
|-------------|---------------------|
| `add_text_annot` | 0 (PDF_ANNOT_TEXT) |
| `add_highlight_annot` | 8 (PDF_ANNOT_HIGHLIGHT) |
| `add_underline_annot` | 9 (PDF_ANNOT_UNDERLINE) |
| `add_strikeout_annot` | 11 (PDF_ANNOT_STRIKE_OUT) |

### Text 注释的 Rect 设置

文本注释需要指定位置（不像 highlight 基于 quads）。创建后单独调用 `mupdf_safe_set_annot_rect`：

```rust
// page.rs
fn add_text_annot(&self, rect, contents, ...) -> PyResult<()> {
    self.create_annot_impl(0, &[], color, contents)?;  // type=0 (TEXT)
    mupdf_safe_set_annot_rect(ctx, doc, page, index, rect);  // 设置位置
}
```

## `page.delete_annot(index)` 实现

遍历到第 index 个注释，调用 `pdf_delete_annot`：

```c
pdf_annot *annot = pdf_first_annot(ctx, ppage);
for (int i = 0; i < index && annot; i++)
    annot = pdf_next_annot(ctx, annot);
pdf_delete_annot(ctx, ppage, annot);
```

## 测试覆盖

### TestAnnotations（3 个用例）

| 用例 | 验证点 |
|------|--------|
| `test_get_annotations_empty` | 无注释页面返回空 list |
| `test_add_and_read_annotations` | 添加 4 种注释 → 读取验证 type/rect/color → save+reopen 验证持久化 |
| `test_delete_annot` | 添加 → 删除 → 验证数量减少 |

**关键约束**：不能同时打开同一 PDF 的多个文档对象写注释——MuPDF 不支持并发写。测试中将所有注释操作合并到单个 test 函数。

## 踩坑记录

| 坑 | 症状 | 根因 | 修复 |
|----|------|------|------|
| `pdf_drop_annot` in loop | 2+ 注释时无限循环 | `pdf_first_annot`/`pdf_next_annot` 不 keep，drop 释放 page 持有的内存 | 移除循环中的 drop |
| `pdf_create_annot` | 链表损坏 | `pdf_begin_operation` 副作用 | 改用 `pdf_create_annot_raw` |
| Highlight rect = (0,0,0,0) | get_annotations 返回零 rect | Highlight 无 /Rect，只有 /QuadPoints | 从 QuadPoints 计算包围盒 |
| 返回 type 为 int | 测试断言失败 | get_annotations 返回字符串名（"Highlight"）不是 int（8） | 修正测试断言 |

## 相关文件

| 文件 | 改动 |
|------|------|
| `crates/mupdf-sys/native/error_wrapper.c` | `mupdf_safe_get_annotations` + `mupdf_safe_create_annot` + `mupdf_safe_delete_annot` + `mupdf_safe_set_annot_rect` |
| `crates/mupdf-sys/native/error_wrapper.h` | 对应声明 |
| `crates/mupdf-sys/bindings.rs` | 手动添加 FFI 声明 |
| `crates/ritz/src/page.rs` | `get_annotations()` + `add_*_annot()` + `delete_annot()` + `create_annot_impl()` |
| `python/tests/test_ritz.py` | `TestAnnotations`（3 个用例） |
