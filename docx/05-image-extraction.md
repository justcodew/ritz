# 一级图片提取

> plan_v1 §3.2 的核心特性。一次调用同时返回 bbox + 原始图像字节，消除 PyMuPDF 的两步调用。本文档解释实现和关键优化。

## PyMuPDF 的问题：两步调用

PyMuPDF 提取页面图片需要两步：

```python
# 第一步：获取 xref 列表（不含 bbox、不含数据）
for img in page.get_images(full=True):
    xref = img[0]
    # 第二步：按 xref 加载图像数据
    data = doc.extract_image(xref)  # {image: bytes, ext: "png", ...}
```

问题：
1. **两步调用**——先遍历页面资源拿 xref，再逐个加载
2. **无 bbox**——PyMuPDF 的 `get_images` 不返回图片在页面上的坐标
3. **每个图片一次 FFI**——N 张图 = N + 1 次跨语言调用

## ritz 的方案：一次 device 遍历

ritz 用自定义 `fz_device` 子类，在渲染流水线中拦截 `fill_image` 回调，一次遍历捕获所有图片：

```python
# 一次调用，返回 bbox + 原始字节
for img in page.get_images(include_data=True):
    print(img["bbox"])      # (x0, y0, x1, y1) 页面坐标
    print(img["width"])     # 原始像素宽
    print(img["ext"])       # "jpeg" / "png" / "jpx"
    open(f"out.{img['ext']}", "wb").write(img["image"])
```

### 实现：image_capture_device

```c
// error_wrapper.c
typedef struct {
    fz_device dev;              // MuPDF device 接口
    growbuf gb;                 // 输出 buffer
    int include_data;
    fz_image **seen;            // 去重表（按 fz_image* 指针）
    int seen_count;
} image_capture_device;

static void cap_fill_image(fz_context *ctx, fz_device *dev_,
                           fz_image *img, fz_matrix ctm, ...) {
    image_capture_device *cap = (image_capture_device*)dev_;

    // 去重：同一张图（同一 fz_image* 指针）只捕获一次
    if (img_seen(cap, img)) return;
    img_add_seen(ctx, cap, img);

    // bbox 从变换矩阵算出
    fz_rect r = fz_transform_rect(fz_unit_rect, ctm);
    gb_put(ctx, &cap->gb, &r.x0, 16);  // 4 个 float

    // 图像属性
    gb_put(ctx, &cap->gb, &img->w, 4);      // width
    gb_put(ctx, &cap->gb, &img->h, 4);      // height
    gb_put(ctx, &cap->gb, &img->bpc, 4);    // bits per component
    // colorspace name, imagemask flag ...

    if (cap->include_data) {
        // 关键优化：返回原始压缩字节（见下文）
    }
}
```

调用方式：

```c
cap->dev.fill_image = cap_fill_image;
fz_run_display_list(ctx, list, (fz_device*)cap, ...);
// 运行结束后，cap->gb 包含所有图片的扁平化数据
```

**一次 `fz_run_display_list` = 所有图片的 bbox + 属性 + 数据**。

### 去重

PDF 中同一张图可能被引用多次（如背景水印）。ritz 按 `fz_image*` 指针去重：

```c
static int img_seen(image_capture_device *cap, fz_image *img) {
    for (int i = 0; i < cap->seen_count; i++) {
        if (cap->seen[i] == img) return 1;  // 已见过
    }
    return 0;
}
```

MuPDF 对同一图像对象返回同一指针，所以指针比较等价于图像去重。

## 关键优化：原始压缩字节

### 问题：decode + re-encode 是浪费

早期版本对每张图都做：

```c
// ❌ 慢路径：先解码到像素，再编码成 PNG
pix = fz_get_pixmap_from_image(ctx, img, ...);     // JPEG → 像素（decode）
buf = fz_new_buffer_from_pixmap_as_png(ctx, pix);   // 像素 → PNG（encode）
```

对于 JPEG 图，这是**双重浪费**：
1. JPEG decode（开销大）
2. PNG encode（开销更大，zlib 压缩）

原始 JPEG 字节本身就是可用的图像文件，为什么要解码再重编码？

### 优化：直接返回原始流

MuPDF 提供 `fz_compressed_image_buffer` 获取图像的原始压缩数据：

```c
// ✅ 快路径：直接返回原始字节，零 decode 零 encode
fz_compressed_buffer *cbuf = fz_compressed_image_buffer(ctx, img);
if (cbuf && cbuf->buffer) {
    int t = cbuf->params.type;
    const char *ext = NULL;
    if (t == FZ_IMAGE_JPEG) ext = "jpeg";       // DCTDecode
    else if (t == FZ_IMAGE_PNG) ext = "png";     // 内嵌 PNG
    else if (t == FZ_IMAGE_JPX) ext = "jpx";     // JPEG2000

    if (ext) {
        // 直接 memcpy 原始流到输出 buffer
        unsigned char *src = NULL;
        size_t len = fz_buffer_storage(ctx, cbuf->buffer, &src);
        gb_put(&extlen, 4);
        gb_put(ext, extlen);
        gb_put(&len, 4);
        gb_put(src, len);  // 零 decode 零 encode
    }
}
```

### 格式分类

PDF 图像的压缩类型决定了能否走快路径：

| MuPDF 类型 | PDF filter | 独立图像？ | ritz 处理 |
|-----------|-----------|----------|----------|
| FZ_IMAGE_JPEG | DCTDecode | ✅ 是（原始 JPEG 文件） | 返回原始字节 |
| FZ_IMAGE_PNG | (内嵌 PNG) | ✅ 是 | 返回原始字节 |
| FZ_IMAGE_JPX | JPXDecode | ✅ 是（JPEG2000） | 返回原始字节 |
| FZ_IMAGE_FLATE | FlateDecode | ❌ 否（Flate 压缩的原始像素） | fallback: decode + PNG |
| FZ_IMAGE_RAW | (无压缩) | ❌ 否（裸像素） | fallback: decode + PNG |
| FZ_IMAGE_FAX | CCITTFaxDecode | ⚠️ CCITT G4（罕见） | fallback: decode + PNG |

**为什么 FlateDecode 不能返回原始流？** 因为 PDF 的 FlateDecode 压缩的是**裸像素行**，不是独立的图像文件格式。解压后得到的是 `RGBRGBRGB...` 的像素流，不是任何图像查看器能直接打开的格式。必须重新包装成 PNG/BMP。

### fallback 路径

对于非独立格式，退回 decode + PNG 编码：

```c
if (!use_original) {
    pix = fz_get_pixmap_from_image(ctx, img, ...);
    buf = fz_new_buffer_from_pixmap_as_png(ctx, pix);
    ext = "png";
    // 写入 PNG 字节
}
```

## 性能影响

| PDF 类型 | 图像格式 | ritz 路径 | vs PyMuPDF |
|---------|---------|----------|-----------|
| 扫描文档 | 全 JPEG | 原始字节 | **6.2x** |
| 论文/截图 | FlateDecode 为主 | decode + PNG | 1.1x（持平） |
| 混合 | 混合 | 混合 | 1~6x 之间 |

**关键发现**：真实世界的论文 PDF（如 arxiv）大多是 FlateDecode（截屏/矢量图），加速比接近 1x。扫描文档（全 JPEG）能到 6x。plan_v1 的 10x 目标仅在 JPEG-heavy 场景接近达成。

## 输出格式

每个图片 dict 的字段：

```python
{
    "bbox": (x0, y0, x1, y1),  # 页面坐标（MuPDF 坐标系）
    "width": 800,               # 原始像素宽
    "height": 600,
    "bpc": 8,                   # bits per component
    "colorspace": "DeviceRGB",  # 色彩空间名
    "imagemask": 0,
    # include_data=True 时额外：
    "ext": "jpeg",              # 格式（jpeg/png/jpx）
    "image": b"\xff\xd8\xff...", # 原始压缩字节
}
```

`ext` 字段告诉用户数据格式——JPEG 直接存 `.jpg`，PNG 存 `.png`，无需猜测。
