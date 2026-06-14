# Phase 5d：命名目标解析

> v0.2.0（2026-06-14）落地。本文记录 `doc.resolve_names()` 的实现细节。

## 总览

Phase 5d 实现了 PDF 命名目标（named destinations）的解析，返回文档中所有命名目标的映射。

| API | PyMuPDF 等价 | 用途 |
|-----|-------------|------|
| `doc.resolve_names()` | — | 返回 `dict[str, (page, x, y)]` |

PyMuPDF 没有直接等价 API（其 `get_names()` 返回原始结构，需手动解析）。

## PDF 命名目标机制

PDF 有两种命名目标存储方式，取决于 PDF 版本：

- **PDF 1.1**：`/Root -> /Dests` 直接字典
- **PDF 1.2+**：`/Root -> /Names -> /Dests` 名称树（name tree）

名称树是一种平衡树结构，叶节点包含 key-value 对数组（`[name1, dest1, name2, dest2, ...]`），中间节点通过 `/Kids` 指向子树。

每个目标值可以是：
- **显式目标数组**：`[page_ref, /XYZ, left, top, zoom]`
- **字典**：`{ /D: [page_ref, /XYZ, ...] }`（action 引用）
- **字符串**：另一个命名目标名称（间接引用）

## `doc.resolve_names()` 实现

### 方案选择：`pdf_resolve_link_dest` per entry

两种方案：

**方案 A**：`pdf_load_name_tree` + 手动解析目标数组。需要移植 MuPDF 的 `resolve_dest` + `populate_destination` 逻辑（~130 行 C），处理所有目标格式变体。

**方案 B**：`pdf_load_name_tree` 获取扁平化字典，对每个条目构造 `"#nameddest=<name>"` URI，调用 `pdf_resolve_link_dest` 一次性解析。

选择**方案 B**——`pdf_resolve_link_dest` 已经处理了所有目标格式（显式数组、字典、间接引用），包括 `pdf_page_obj_transform` 坐标转换。代码更简洁，维护成本低。

### 数据流

```
Python: doc.resolve_names()
  ↓ PyO3
Rust:   PyDocument::resolve_names(py)                          [document.rs]
  ↓     mupdf_safe_resolve_names(ctx, doc, &ptr, &len, &n)     [FFI]
C:      pdf_load_name_tree(ctx, pdf, PDF_NAME(Dests))
        → pdf_dict_len / pdf_dict_get_key / pdf_dict_get_val
        → 遍历每个条目
        → snprintf("#nameddest=%s", name)
        → pdf_resolve_link_dest(ctx, pdf, uri)
        → gb_put 写入 buffer
  ↓     返回 char* + size_t + int
Rust:   slice::from_raw_parts → 逐条解析 → dict
Python: dict[str, (page, x, y)]
```

### C 层实现

```c
int mupdf_safe_resolve_names(fz_context *ctx, fz_document *doc,
                             char **out, size_t *out_len, int *total_n) {
    pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
    pdf_obj *tree = pdf_load_name_tree(ctx, pdf, PDF_NAME(Dests));
    if (!tree || !pdf_is_dict(ctx, tree)) return 0;  // 无命名目标

    int len = pdf_dict_len(ctx, tree);
    for (int i = 0; i < len; i++) {
        pdf_obj *key = pdf_dict_get_key(ctx, tree, i);
        const char *name = pdf_to_name(ctx, key);

        // 使用 pdf_resolve_link_dest 解析目标
        char uri[1024];
        snprintf(uri, sizeof(uri), "#nameddest=%s", name);
        fz_link_dest dest = pdf_resolve_link_dest(ctx, pdf, uri);

        // 写入 buffer
        int name_len = strlen(name);
        int page = dest.loc.page;  // 0-based
        float x = dest.x, y = dest.y;
        gb_put(ctx, &gb, &name_len, 4);
        gb_put(ctx, &gb, name, name_len);
        gb_put(ctx, &gb, &page, 4);
        gb_put(ctx, &gb, &x, 4);
        gb_put(ctx, &gb, &y, 4);
    }
    pdf_drop_obj(ctx, tree);
}
```

### buffer 协议

每条命名目标在 buffer 中的布局：

| 偏移 | 长度 | 字段 | 说明 |
|------|------|------|------|
| 0 | 4 | name_len | 名称字符串长度（int32） |
| 4 | name_len | name | UTF-8 名称 |
| ... | 4 | page | 0-based 页码（int32），-1 表示无法解析 |
| ... | 4 | x | 目标 x 坐标（float32） |
| ... | 4 | y | 目标 y 坐标（float32） |

### `pdf_resolve_link_dest` 的作用

这个函数是 MuPDF 的高层 API，接受 URI 字符串（`"#nameddest=foo"` 或 `"#page=5&zoom=100,100,200"`），返回 `fz_link_dest` 结构体：

```c
typedef struct {
    fz_location loc;     // .page 是 0-based 页码
    fz_link_dest_type type;  // XYZ / Fit / FitH / FitV / ...
    float x, y, w, h, zoom;
} fz_link_dest;
```

内部处理：
1. 如果目标是字典，提取 `/D` 条目
2. 如果目标是字符串/名称，递归查找 `pdf_lookup_dest`
3. 解析目标数组：page ref → `pdf_lookup_page_number` 转为页码
4. 应用 `pdf_page_obj_transform` 处理页面变换矩阵

## Rust 层：`doc.resolve_names()`

返回 `dict[str, (page, x, y)]`：

```python
names = doc.resolve_names()
# names = {
#     "chapter1": (0, 0.0, 720.0),
#     "section2.1": (5, 72.0, 600.0),
#     ...
# }
```

page 是 0-based（与 ritz 其他 API 一致）。

## 测试覆盖

### TestResolveNames（3 个用例）

| 用例 | 验证点 |
|------|--------|
| `test_resolve_names_empty` | sample.pdf 无命名目标 → 返回空 dict |
| `test_resolve_names_type` | 返回值类型检查：dict，value 是 (int, float, float) 元组 |
| `test_resolve_names_survives_save` | save+reopen 后仍可调用 |

**局限**：sample.pdf 没有命名目标，只能测试"空"场景。要测试有数据的场景需要带命名目标的 PDF（如从 Word 导出的 PDF，或有 `/Names /Dests` 树的 PDF）。

## 性能特征

`pdf_load_name_tree` 递归遍历名称树并扁平化为一个字典（O(N)），然后 O(N) 遍历字典条目。每条目一次 `pdf_resolve_link_dest` 调用。

对于有 N 个命名目标的文档，总 FFI 次数 = 2（load_name_tree + drop）+ N × 1（resolve_link_dest）。

如果 N 很大（>100），可考虑在 C 层一次性处理所有条目（当前实现已经是这样）。

## 相关文件

| 文件 | 改动 |
|------|------|
| `crates/mupdf-sys/native/error_wrapper.c` | `mupdf_safe_resolve_names` |
| `crates/mupdf-sys/native/error_wrapper.h` | 对应声明 |
| `crates/mupdf-sys/bindings.rs` | 手动添加 FFI 声明 |
| `crates/ritz/src/document.rs` | `resolve_names()` |
| `python/tests/test_ritz.py` | `TestResolveNames`（3 个用例） |
