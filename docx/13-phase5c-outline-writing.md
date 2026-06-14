# Phase 5c：大纲写入 + 文档保存

> v0.2.0（2026-06-14）落地。本文记录 `doc.set_toc()` 和 `doc.save()` 的实现细节。

## 总览

Phase 5c 实现了 ritz 的**首个写路径 API**——大纲写入和文档保存。这是注释写入（Phase 5b）的前置依赖，因为所有写操作都需要 `doc.save()` 才能持久化。

| API | PyMuPDF 等价 | 用途 |
|-----|-------------|------|
| `doc.save(path)` | `doc.save(path)` | 保存 PDF 到文件 |
| `doc.set_toc(toc)` | `doc.set_toc(toc)` | 设置文档大纲（目录） |

## `doc.save()` 实现

最简单的写路径 API。C 层调用 `pdf_save_document`：

```c
int mupdf_safe_save_document(fz_context *ctx, fz_document *doc, const char *filename) {
    pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
    if (!pdf) fz_throw(ctx, FZ_ERROR_GENERIC, "not a pdf document");
    pdf_save_document(ctx, pdf, filename, NULL);  // NULL = 默认选项
}
```

Rust 侧简单的 `CString` 转换 + FFI 调用。

## `doc.set_toc()` 实现

### 为什么不使用 `fz_outline_iterator`

最初尝试使用 MuPDF 的 `fz_outline_iterator` API（`insert`/`down`/`up`/`next`/`prev`）。但发现了一个**设计缺陷**：

`fz_outline_iterator` 使用内部 `modifier` 状态机（`MOD_NONE`/`MOD_BELOW`/`MOD_AFTER`），`down()` 要求 `modifier == MOD_NONE`。但 `insert()` 后 modifier 变为 `MOD_AFTER`，必须先调 `prev()` 重置为 `MOD_NONE` 才能 `down()`。

更严重的问题是**根级导航**：从根级子项（如 "Part I"）调用 `up()` 失败，因为 `up()` 要求 parent 的 parent 存在（grandparent），而根级子项的 parent 是 `/Outlines` 字典，没有 grandparent。这导致在根级条目之间插入兄弟节点时无法实现。

```
Root (/Outlines)
├── Part I
│   ├── Chapter 1
│   └── Chapter 2    ← 从这里 up() 到 Part I，再 up() 失败（无 grandparent）
└── Part II           ← 无法到达这里
```

参考 `vendor/mupdf/source/tools/pdfmerge.c` 的 `copy_item` 函数，确认了 `prev(); down();` 的模式，但仍然无法解决根级兄弟导航问题。

### 最终方案：直接操作 PDF 字典

放弃 iterator，直接操作 PDF outline 字典对象：

```c
// 每条 outline entry 是一个 pdf dict：
// {
//   /Title: "Chapter 1"
//   /Dest: [page_obj /Fit]
//   /Parent: parent_dict_ref
//   /First: first_child_ref  (如果有子条目)
//   /Last: last_child_ref    (如果有子条目)
//   /Next: next_sibling_ref
//   /Prev: prev_sibling_ref
// }
```

### 树结构构建算法

使用两个栈数组跟踪状态：

```c
#define MAX_TOC_DEPTH 16
pdf_obj *stack[MAX_TOC_DEPTH + 1] = {0};      // stack[level] = 该层最后一条目
pdf_obj *parent_of[MAX_TOC_DEPTH + 1] = {0};   // parent_of[level] = 该层父对象
parent_of[1] = outlines;  // level 1 的父是 /Outlines 字典

for (int i = 0; i < count; i++) {
    int level = levels[i];  // 1-based

    // 创建 entry
    pdf_obj *entry = pdf_add_new_dict(ctx, pdf, 8);
    pdf_dict_put_text_string(ctx, entry, PDF_NAME(Title), titles[i]);

    // 设置目标
    pdf_obj *page_obj = pdf_lookup_page_obj(ctx, pdf, page_num - 1);
    pdf_obj *dest = pdf_add_new_array(ctx, pdf, 2);
    pdf_array_push(ctx, dest, page_obj);
    pdf_array_push(ctx, dest, PDF_NAME(Fit));
    pdf_dict_put(ctx, entry, PDF_NAME(Dest), dest);

    // 链接到树
    pdf_obj *parent = parent_of[level];
    pdf_dict_put(ctx, entry, PDF_NAME(Parent), parent);

    pdf_obj *last_child = stack[level];
    if (!last_child) {
        pdf_dict_put(ctx, parent, PDF_NAME(First), entry);
    } else {
        pdf_dict_put(ctx, last_child, PDF_NAME(Next), entry);
        pdf_dict_put(ctx, entry, PDF_NAME(Prev), last_child);
    }
    pdf_dict_put(ctx, parent, PDF_NAME(Last), entry);

    // 更新栈
    stack[level] = entry;
    parent_of[level + 1] = entry;
    for (int d = level + 1; d <= MAX_TOC_DEPTH; d++) stack[d] = NULL;
}
```

### `/Outlines` 字典初始化

文档首次设置大纲时，需要创建 `/Outlines` 字典并注册到 catalog：

```c
if (!pdf_is_dict(ctx, outlines)) {
    outlines = pdf_add_new_dict(ctx, pdf, 4);
    pdf_dict_put(ctx, catalog, PDF_NAME(Outlines), outlines);
    pdf_dict_put(ctx, outlines, PDF_NAME(Type), PDF_NAME(Outlines));
}
```

### 清空现有大纲

`clear_outline_entries` 递归删除所有子条目：

```c
static void clear_outline_entries(fz_context *ctx, pdf_obj *outlines) {
    pdf_obj *child = pdf_dict_get(ctx, outlines, PDF_NAME(First));
    while (child) {
        if (pdf_dict_get(ctx, child, PDF_NAME(First)))
            clear_outline_entries(ctx, child);  // 递归删除子级
        pdf_obj *next = pdf_dict_get(ctx, child, PDF_NAME(Next));
        pdf_dict_del(ctx, child, PDF_NAME(Parent));
        // ... 清除其他键 ...
        child = next;
    }
    pdf_dict_del(ctx, outlines, PDF_NAME(First));
    pdf_dict_del(ctx, outlines, PDF_NAME(Last));
}
```

### 页码验证

创建 outline entry 前验证页码范围，否则 MuPDF 在 save 时报 "cannot find page N in page tree"：

```c
int page_count = fz_count_pages(ctx, doc);
if (page_num < 1 || page_num > page_count)
    fz_throw(ctx, FZ_ERROR_GENERIC, "page number %d out of range (1..%d)", page_num, page_count);
```

## Rust 层：`doc.set_toc(toc)`

接受与 `get_toc()` 相同格式的 `Vec<(i32, String, i32)>`，实现对称：

```python
doc.set_toc([
    (1, "Chapter 1", 1),
    (2, "Section 1.1", 1),
    (1, "Chapter 2", 2),
])
```

Rust 侧将 Vec 拆分为三个 parallel arrays（levels、pages、titles），titles 转为 `CString` 数组，传给 C 层。

## 测试覆盖

### TestSave（3 个用例）

| 用例 | 验证点 |
|------|--------|
| `test_save_basic` | save 到临时文件，验证文件存在且大小 > 0 |
| `test_save_roundtrip` | save → 重新 open → 验证 page_count 一致 |
| `test_save_invalid_path` | 不存在的目录 → RuntimeError |

### TestSetToc（4 个用例）

| 用例 | 验证点 |
|------|--------|
| `test_set_toc_basic` | 2 条平级大纲 → save → reopen → get_toc 验证 |
| `test_set_toc_nested` | 3 层嵌套（5 条）→ 验证 level/titles |
| `test_set_toc_empty` | 传空 list → 清空大纲 |
| `test_set_toc_survives_save` | 7 条大纲 → save → reopen → 验证持久化 |

## 踩坑记录

| 坑 | 症状 | 根因 | 修复 |
|----|------|------|------|
| `fz_outline_iterator` 根级导航 | 根级兄弟无法插入 | `up()` 要求 grandparent 存在 | 放弃 iterator，直接操作 PDF dict |
| `parent_stack[level]` 为 NULL | "not a dict (null)" crash | 数组索引错误：`parent_stack[1]` 未初始化 | 改用 `parent_of[level]`，level 从 1 开始 |
| 页码超出范围 | "cannot find page 2 in page tree" | sample.pdf 只有 1 页 | 添加 `fz_count_pages` 验证 |
| `fz_always` 嵌套 `fz_catch` | 外层 catch 不执行 | MuPDF 不支持嵌套 try/catch | 分离清理逻辑 |

## 相关文件

| 文件 | 改动 |
|------|------|
| `crates/mupdf-sys/native/error_wrapper.c` | `mupdf_safe_save_document` + `mupdf_safe_set_toc` + `clear_outline_entries` |
| `crates/mupdf-sys/native/error_wrapper.h` | 对应声明 |
| `crates/mupdf-sys/bindings.rs` | 手动添加 FFI 声明 |
| `crates/ritz/src/document.rs` | `save()` + `set_toc()` |
| `python/tests/test_ritz.py` | `TestSave` + `TestSetToc`（7 个用例） |
