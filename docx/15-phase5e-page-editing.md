# Phase 5e：页面编辑（new_page / delete_page / delete_pages / move_page / copy_page / insert_pdf）

> v0.3.0（2026-06）落地。本文记录 6 个页面编辑 API 的实现细节、refcount 陷阱和踩坑记录。

## 总览

Phase 5e 是 ritz 的最后一个 plan_v1 KPI——PDF 页面编辑能力。落地后 plan_v1 全部 KPI 达成，可发版 0.3.0。

| API | 签名 | PyMuPDF 等价 | 用途 |
|-----|------|--------------|------|
| `doc.new_page(pno=-1, width=595, height=842, rotate=0)` | → `Page` | `Document.new_page` | 插入空白页，返回新建 Page |
| `doc.delete_page(pno=-1)` | → `None` | `Document.delete_page` | 删除单页（-1 = 末页）|
| `doc.delete_pages(from_page=-1, to_page=-1)` | → `None` | `Document.delete_pages` | 删除闭区间 [from, to] |
| `doc.move_page(src, dst)` | → `None` | `Document.move_page` | 移动页位置 |
| `doc.copy_page(src, dst)` | → `None` | `Document.copy_page` | 同文档内克隆页 |
| `doc.insert_pdf(docsrc, start=0, end=-1, at=-1)` | → `None` | `Document.insert_pdf` | 把另一 PDF 的页范围插入 |

所有 API 都遵循不变量 #1（longjmp 隔离）：底层 MuPDF 调用全部包在 C 层 `fz_try/fz_catch`。

## 数据流

以 `doc.new_page()` 为例：

```
Python: doc.new_page()
  ↓ PyO3
Rust:   PyDocument::new_page(slf, pno, width, height, rotate)       [document.rs]
  ↓     mupdf_safe_new_blank_page(ctx, raw, at, width, height, rotate)
FFI:    error_wrapper.c: fz_try {
          pdf_add_page(mediabox, rotate, NULL, NULL)   → pdf_obj*
          pdf_count_pages                             → count
          pdf_insert_page(insert_at, page)             → 修改页树
        } fz_always { pdf_drop_obj(page); }
          fz_catch { set_error; return -1; }
  ↓
Rust:   mupdf_safe_count_pages + mupdf_safe_load_page   → 物化新 fz_page*
  ↓     PyPage::new(doc_handle, ctx, raw_doc, raw)
Python: 返回 ritz.Page
```

## C 层

### `mupdf_safe_new_blank_page` —— refcount 陷阱

```c
pdf_obj *page = NULL;
fz_try(ctx) {
    pdf_document *pdf = pdf_document_from_fz_document(ctx, doc);
    fz_rect mediabox = fz_make_rect(0, 0, width, height);
    page = pdf_add_page(ctx, pdf, mediabox, rotate, NULL, NULL);  /* refcount = 1 */
    int count = pdf_count_pages(ctx, pdf);
    int insert_at = (at < 0 || at >= count) ? count : at;
    pdf_insert_page(ctx, pdf, insert_at, page);  /* 内部 keep，refcount = 2 */
}
fz_always(ctx) {
    if (page) pdf_drop_obj(ctx, page);  /* 必须 drop 一次，refcount = 1（被页树持有）*/
}
fz_catch(ctx) { set_error(...); return -1; }
```

**关键 refcount 关系**：
- `pdf_add_page` 返回 refcount = 1（caller 持有）
- `pdf_insert_page` 把 page 加进 Kids 数组，内部 `pdf_array_push` 会 keep 一次 → refcount = 2
- 我们必须在 `fz_always` 里 drop 一次，让最终 refcount = 1（被页树独家持有）
- 文档关闭时页树 drop 所有 page obj，refcount 归零

**不 drop 会怎样**：每次 new_page 泄漏一个 pdf_obj；1000 次 new_page 累积 ~100KB。

参考 `vendor/mupdf/source/tools/murun.c:8028-8033` 的范式。

### `mupdf_safe_move_page` —— keep + drop 的另一面

```c
pdf_obj *page = NULL;
fz_try(ctx) {
    page = pdf_lookup_page_obj(ctx, pdf, src);  /* 不增加 refcount */
    pdf_keep_obj(ctx, page);                    /* refcount += 1（保命）*/
    pdf_delete_page(ctx, pdf, src);             /* 页树 drop 一次，但我们的 keep 救活了 */

    int new_count = pdf_count_pages(ctx, pdf);
    int insert_at = (dst >= new_count) ? new_count : dst;
    pdf_insert_page(ctx, pdf, insert_at, page); /* 页树 keep 一次 */
}
fz_always(ctx) {
    if (page) pdf_drop_obj(ctx, page);  /* 释放我们的 keep */
}
fz_catch(ctx) { set_error(...); return -1; }
```

**为什么 src 必须先 keep**：`pdf_delete_page` 会从 Kids 数组移除 page，内部 `pdf_array_delete` drop 一次。若 page 原本只有页树持有一份引用（refcount=1），delete 后 page 被释放，再 `pdf_insert_page(page)` 就是 use-after-free。`pdf_keep_obj` 把 refcount 抬到 2，delete 后还有 1，insert 后变 2，finally drop 后回到 1（页树独家持有）。

### `mupdf_safe_copy_page` —— 同文档 graft

```c
pdf_graft_page(ctx, pdf, dst, pdf, src);
```

`pdf_graft_page(ctx, dst_doc, dst_at, src_doc, src_at)` 内部用 `pdf_new_graft_map` + `pdf_graft_mapped_page`，会克隆 src 页的 Contents/Resources 等深拷贝到 dst。同文档内（src_doc = dst_doc）也能用，MuPDF 会建立 graft_map 缓存对象映射。

### `mupdf_safe_insert_pdf` —— 跨 ctx 陷阱

**核心约束**：`pdf_graft_page(ctx, dst, ...)` 中的 ctx 必须既用于 dst 的所有操作，也用于 src 的所有操作（看 `pdf_graft_mapped_page` 实现，line 226 of pdf-graft.c）。这意味着 **src 和 dst 必须共享同一 fz_context**。

但 ritz 里每个 `Document` 都有自己的 fz_context（`PyDocument::new` line 56 调 `mupdf_safe_new_context()`）。所以两个独立的 `ritz.open()` 得到的 doc 有不同的 ctx，直接 graft 是 UB。

**解决方案**：`mupdf_safe_insert_pdf` **不接受 src_doc 指针，而是接受 src 路径字符串**。内部用 dst 的 ctx 重新打开 src：

```c
int mupdf_safe_insert_pdf(fz_context *ctx, fz_document *dst_doc, int at,
                          const char *src_path, int start, int end);
```

```c
fz_try(ctx) {
    pdf_document *dst = pdf_document_from_fz_document(ctx, dst_doc);
    src_doc = fz_open_document(ctx, src_path);   /* 在 dst 的 ctx 里重开 */
    pdf_document *src = pdf_document_from_fz_document(ctx, src_doc);
    /* 此时 src/dst 共享 ctx，pdf_graft_page 安全 */
    for (int i = s; i < e; i++) {
        pdf_graft_page(ctx, dst, insert_at, src, i);
        insert_at++;
    }
}
fz_always(ctx) { if (src_doc) fz_drop_document(ctx, src_doc); }
fz_catch(ctx) { ... }
```

**Rust 侧**：`PyDocument` 增加 `filename: String` 字段（`new()` 时保存），`insert_pdf(docsrc)` 从 `docsrc.filename` 取路径传给 C 层。这要求 `docsrc` 仍指向一个有效可读路径。

### `mupdf_safe_delete_page_range` —— 半开 vs 闭区间

C 层遵循 MuPDF 原生语义 `[start, end)`。Rust 层做转换：

```rust
// PyMuPDF 用闭区间 [from, to]，C 用半开 [start, end)
let rc = unsafe {
    mupdf_safe_delete_page_range(self.ctx, self.raw, start, (to + 1))
};
```

### fz_always / fz_catch 必须与 fz_try 同级

**踩坑记录**（开发期 bug）：最初写成

```c
fz_try(ctx) {
    /* work */
    fz_always(ctx) { pdf_drop_obj(ctx, page); }  // ❌ 嵌套！
    fz_catch(ctx) {}                              // ❌ 嵌套！
}
fz_catch(ctx) { ... }
```

`fz_try/fz_always/fz_catch` 是 switch-case 风格的宏，必须**同级出现**。嵌套会导致第二次调用时控制流混乱，表现为"第一次 new_page 成功，第二次 SIGSEGV"。修复后 flatten 即可。

## Rust 层

### `PyDocument` 新增 `filename` 字段

为支持 `insert_pdf` 跨 ctx，`PyDocument` 增加 `pub(crate) filename: String`，在 `new()` 时记录。这是唯一新增的字段。

### `new_page` 返回 PyPage 的物化路径

`pdf_insert_page` 修改页树后，要返回新建的 `PyPage`，必须调用 `mupdf_safe_load_page` 把页树中的 page slot 物化成 `fz_page*`：

```rust
fn new_page(slf, pno, width, height, rotate) -> PyResult<Py<PyPage>> {
    /* FFI 调用 mupdf_safe_new_blank_page */
    let n = unsafe { mupdf_safe_count_pages(ctx, raw) };
    let target = if pno < 0 || pno >= n { (n - 1) as usize } else { pno as usize };
    Self::load_page(slf, target)   // 复用现有的 load_page 物化逻辑
}
```

### 缓存影响

`PyDocument` 唯一的缓存是 `metadata_cache: OnceLock<Py<PyAny>>`（metadata 字典）。页面编辑不修改 metadata，**无需失效**。`__len__` / `page_count` / `load_page` 每次都查 MuPDF（`pdf_count_pages` 直接读 `/Pages/Count`），也无 Rust 侧缓存。

## stale PyPage 警告

**重要**：调用 `delete_page` / `delete_pages` / `move_page` / `insert_pdf` 后，之前用 `doc[i]` 取出的 `PyPage` **位置可能失效**。MuPDF 内部 `fz_page*` 仍指向原页对象，但页树已重排——`doc[i]` 现在指向另一页。

**ritz 行为（与 PyMuPDF 不同）**：ritz 的 stale PyPage 不会报错，只是返回**空文本**（因为 fz_page 关联的 page_obj 在 delete 后被 drop，stext 构造拿不到内容）。PyMuPDF 则会抛 `AssertionError: page is None`。

**建议**：先做所有页面编辑，最后再 `doc[i]` 取页读取。需要 mid-edit 读取时，重新 `load_page`。**永远不要假设旧 PyPage 在写操作后还有意义**。

## 测试覆盖

`python/tests/test_ritz.py::TestPageEditing`，13 个用例（参见 `test_ritz.py:730+`）：

| 测试 | 验证 |
|------|------|
| `test_new_page_append` | 末尾追加，页数 +1，新页文本为空 |
| `test_new_page_at_position` | 在 index 0 插入，原首页内容后移 |
| `test_new_page_custom_size` | width/height/rotate 生效（mediabox）|
| `test_new_page_returns_page_object` | 返回值是 Page，可立即读 |
| `test_delete_page` | 删首页，页数 -1 |
| `test_delete_page_last` | pno=-1 删末页 |
| `test_delete_pages_range` | 闭区间 [0, 1] 删前两页 |
| `test_move_page` | 移动后顺序变化，页数不变 |
| `test_copy_page` | 复制后内容相同，源页保留 |
| `test_insert_pdf_basic` | 整文档插入 |
| `test_insert_pdf_partial` | start/end 半开范围插入 |
| `test_invalid_index_raises` | 越界抛 PyRuntimeError |
| `test_survives_save` | 综合：编辑 → save → reopen 状态保留 |

**测试总数**：62 → 75（+13），全绿。

## 踩坑记录

| 坑 | 症状 | 根因 | 修复 |
|----|------|------|------|
| fz_always/fz_catch 嵌套写法 | 第一次 new_page 成功，第二次 SIGSEGV | switch-case 宏不能嵌套 | flatten 到与 fz_try 同级 |
| `move_page` dst 调整错误 | `move(0, 1)` 没移动 | 误用 `dst > src ? dst-1 : dst` | 直接用 `dst`（pdf_insert_page 的 at 是相对新页树）|
| `insert_pdf` 跨 ctx | PyMuPDF 兼容场景必崩 | ritz 每个 doc 独立 ctx，pdf_graft_page 要求同 ctx | C 层用 dst ctx 重开 src |
| mediabox 旋转后两轴交换 | rotate=90 时 (200,300) 变 (300,200) | MuPDF `fz_bound_page_box` 考虑旋转 | 测试放宽：检查两轴排序而非顺序 |
| bindings.rs 未更新 | Rust 编译找不到符号 | bindings.rs 是 commit 进 git 的产物 | 手动加 extern "C" 块（不变量 §6.3）|

## 相关文件

| 文件 | 改动 |
|------|------|
| `crates/mupdf-sys/native/error_wrapper.c` | +6 函数（~180 行）|
| `crates/mupdf-sys/native/error_wrapper.h` | +1 section +6 声明 |
| `crates/mupdf-sys/bindings.rs` | +6 extern "C" 块（手动编辑）|
| `crates/ritz/src/document.rs` | +`filename` 字段 + 6 个 `#[pyo3]` 方法 |
| `python/tests/test_ritz.py` | +`TestPageEditing`（13 用例）|
| `docx/15-phase5e-page-editing.md` | 本文 |
| `docx/README.md` | +1 索引行 |
| `docx/00-onboarding-guide.md` | KPI/路线图/任务表 4 处更新 |
| `CHANGELOG.md` | [Unreleased] 写入 Phase 5e |
| `README.md` | API 速查 +6 行 |
