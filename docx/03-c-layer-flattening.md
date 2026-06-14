# C 层扁平化策略

> 这是 ritz 性能优势的头号来源。链接读取快 27x、图片提取一次调用取全部，都归功于这个策略。

## 问题：N 次 FFI + N 个 Python 对象 = 慢

假设要在 Rust 里遍历 MuPDF 的链接链表：

```rust
// ❌ 慢路径：每个 link 一次 FFI + 一次 Python dict 构造
let mut link = unsafe { mupdf_safe_load_first_link(ctx, page) };
while !link.is_null() {
    let rect = unsafe { get_link_rect(link) };        // FFI #1
    let uri = unsafe { get_link_uri(link) };           // FFI #2
    let dict = PyDict::new(py);
    dict.set_item("from", rect_to_tuple(rect))?;       // Python 对象构造
    dict.set_item("uri", uri_to_string(uri)?)?;        // Python 对象构造
    out.push(dict);
    link = unsafe { get_next_link(link) };             // FFI #3
}
```

12 个链接 = 36 次 FFI 调用 + 12 个 dict 构造 + 36 次 set_item。每次 FFI 约 5~10ns，每次 dict 构造约 500ns。总开销远超实际工作。

这正是 PyMuPDF 的做法——它为每个 link 单独调用 `fz_load_links`，构造 Python dict，处理 xref 解析。

## 解决方案：C 层遍历 + 扁平化 buffer

把遍历逻辑下沉到 C 层，在 C 侧走完整个链表/树，把结果写入**一段连续的 malloc buffer**，然后一次返回给 Rust：

```
C 层：遍历链表 → 写入连续 buffer（rect + uri 紧挨着）
         ↓ 一次返回
Rust：slice::from_raw_parts → 一次切片构造 list[dict]
```

### 链接读取实现

```c
// error_wrapper.c — mupdf_safe_load_links
int mupdf_safe_load_links(fz_context *ctx, fz_page *page,
                          char **out, size_t *out_len, int *count) {
    growbuf gb = {0};

    fz_try(ctx) {
        // 在 C 层遍历整个 fz_link 链表
        for (fz_link *link = fz_load_links(ctx, page);
             link;
             link = link->next) {

            // rect：4 个 float，直接写入 buffer
            gb_put(ctx, &gb, &link->rect.x0, 4);
            gb_put(ctx, &gb, &link->rect.y0, 4);
            gb_put(ctx, &gb, &link->rect.x1, 4);
            gb_put(ctx, &gb, &link->rect.y1, 4);

            // uri：变长字符串，先写长度再写内容
            int ulen = (int)strlen(link->uri);
            gb_put(ctx, &gb, &ulen, 4);
            gb_put(ctx, &gb, link->uri, ulen);
        }
    }
    fz_catch(ctx) { ... }

    *out = (char*)gb.data;  // 连续 buffer，一次返回
    *out_len = gb.len;
    *count = n;
    return 0;
}
```

Rust 侧一次切片解析：

```rust
// page.rs — get_links
let bytes = unsafe { slice::from_raw_parts(ptr, len) };
let mut off = 0;
for _ in 0..count {
    let x0 = read_f32(&bytes[off..]); off += 4;
    let y0 = read_f32(&bytes[off..]); off += 4;
    // ... 读取 rect + uri
    let dict = PyDict::new(py);
    dict.set_item("from", (x0,y0,x1,y1))?;
    dict.set_item("uri", uri)?;
    out.push(dict);
}
```

**结果**：1 次 FFI 调用（不管多少个链接），1 段连续内存。这比 PyMuPDF 的 N 次 FFI + N 个对象快 **27 倍**。

### 自定义 growbuf

C 层用自定义的 `growbuf` 做动态 buffer，比 MuPDF 的 `fz_buffer` 更轻量：

```c
typedef struct {
    unsigned char *data;
    size_t len;
    size_t cap;
} growbuf;

static void gb_put(fz_context *ctx, growbuf *gb, const void *src, size_t n) {
    if (gb->len + n > gb->cap) {
        size_t nc = gb->cap ? gb->cap * 2 : 256;
        while (nc < gb->len + n) nc *= 2;
        gb->data = fz_realloc(ctx, gb->data, nc);
        gb->cap = nc;
    }
    memcpy(gb->data + gb->len, src, n);
    gb->len += n;
}
```

特点：
- **指数扩容**（×2），均摊 O(1)
- **连续内存**，Rust 侧 `slice::from_raw_parts` 零成本
- 用 MuPDF 的 `fz_realloc` 分配，生命周期由 Rust 侧 `mupdf_free` 管理

### 图片提取：device callback 扁平化

图片提取更复杂——需要运行 display list 才能知道页面上的图片位置。ritz 用自定义 `fz_device` 子类：

```c
typedef struct {
    fz_device dev;          // MuPDF device 接口（函数指针表）
    growbuf gb;
    int include_data;
    fz_image **seen;        // 去重：按 fz_image* 指针
    int seen_count;
} image_capture_device;

// 实现 fill_image 回调——MuPDF 渲染时遇到图片就调这个
static void cap_fill_image(fz_context *ctx, fz_device *dev_,
                           fz_image *img, fz_matrix ctm, ...) {
    image_capture_device *cap = (image_capture_device*)dev_;

    // 去重：同一个 fz_image* 只捕获一次
    if (img_seen(cap, img)) return;
    img_add_seen(ctx, cap, img);

    // 从 ctm 矩阵算出 bbox（页面坐标）
    fz_rect r = fz_transform_rect(fz_unit_rect, ctm);

    // 写入 buffer：bbox + width + height + bpc + colorspace + data
    gb_put(ctx, &cap->gb, &r.x0, 4);  // ... 4 个 float
    // ... w, h, bpc, cs
    if (cap->include_data) {
        // 原始压缩字节或 PNG fallback（见 05-image-extraction.md）
    }
}
```

然后一次 `fz_run_display_list` 就捕获了页面上的所有图片：

```c
cap->dev.fill_image = cap_fill_image;  // 注册回调
fz_run_display_list(ctx, list, (fz_device*)cap, fz_identity, fz_infinite_rect, NULL);
// 运行结束后，cap->gb 里就是所有图片的扁平化数据
```

**一次 device 遍历 = 所有图片的 bbox + data**，这是 [plan_v1 §3.2](../plan/01-plan-v1.md) "一级图片提取"的实现。

### dict 模式：嵌套结构扁平化

`get_text("dict")` 返回嵌套的 `block → line → span` 结构。C 层用**前序遍历 + 计数回填**：

```c
// 先写 block_count 占位
int count_pos = gb.len;
gb_put(&placeholder, 4);  // 占位

for each block:
    gb_put(block_bbox, 32);
    int line_count_pos = gb.len;
    gb_put(&placeholder, 4);  // line_count 占位

    for each line:
        gb_put(line_bbox, 32);
        int span_count_pos = gb.len;
        gb_put(&placeholder, 4);  // span_count 占位

        for each span:
            gb_put(span fields...);
            if (include_chars):
                for each char: gb_put(char bbox + origin + char);

    // 回填 line_count
    memcpy(gb.data + line_count_pos, &actual_line_count, 4);

// 回填 block_count
memcpy(gb.data + count_pos, &actual_block_count, 4);
```

Rust 侧按前序读取，遇到 count 就知道后面有多少个子节点。这避免了指针/引用——整段 buffer 是自描述的扁平数据。

## 什么时候用扁平化？

| 场景 | 是否扁平化 | 原因 |
|------|----------|------|
| 链接读取 | ✅ | 链表遍历 + 大量 dict 构造，扁平化收益 27x |
| 图片提取 | ✅ | device 回调 + 去 + data 编码，一次遍历全部捕获 |
| dict/rawdict 文本 | ✅ | 深度嵌套树，Rust 侧递归有 longjmp 风险 |
| 纯文本 | ❌ | 直接用 `fz_print_stext_page_as_text` 流式输出到 growbuf，无树遍历 |
| 渲染 pixmap | ❌ | 输出是单个像素 buffer，本身就是扁平的 |

**判断标准**：如果输出涉及"遍历 MuPDF 数据结构 + 构造多个 Python 对象"，就用 C 层扁平化。如果输出是单个 blob（文本字符串、像素数组），直接流式输出即可。
