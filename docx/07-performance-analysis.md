# 性能分析：加速比的根因

> plan_v1 立了"10x~30x"目标。实测有的场景 27x，有的只有 1.1x。本章解释**为什么不同场景的加速比差异如此巨大**，以及哪些目标可达、哪些是理论天花板。

## 核心定律：加速比由瓶颈类型决定

任何 PDF 操作的耗时由两部分构成：

```
总耗时 = 语言调度开销 + MuPDF 内部计算
         ↑ ritz 能优化      ↑ ritz 无法优化（双方共享同一引擎）
```

| 瓶颈类型 | 典型加速比 | 场景示例 |
|----------|----------|---------|
| **纯语言调度**（Python 对象构造） | 10x~30x | 链接读取 **27x** |
| **FFI 调度**（PyO3 vs Python C ext） | 2x~6x | 文档打开 **5.6x**、JPEG 图片 **6.2x** |
| **MuPDF 内部计算**（双方共享） | 1x~1.2x | 文本提取 **2.1x**、Flate 图片 **1.1x** |

**判断方法**：把 ritz 和 PyMuPDF 调的 MuPDF 函数列出来。如果两边调的是**同一个 C 函数**且该函数是瓶颈，加速比上限 ~2x。如果瓶颈在"调用次数/对象构造"，加速比可以到 10x+。

## 逐场景分析

### 1. 链接读取：27x（纯调度场景）

**瓶颈**：Python dict 构造。

PyMuPDF 为每个链接：
```
fz_load_links → 遍历链表 → 构造 Python dict → set_item × N → fz_drop_links
```
12 个链接 = 12 个 dict + 12 × N 个 set_item 调用。每个 dict ~500ns，总开销 6μs+。

ritz：
```
C 层遍历链表 → 写入连续 buffer → 1 次 FFI 返回 → Rust 侧切片构造 list[dict]
```
1 次 FFI，buffer 遍历是 memcpy 级别。总开销 0.01ms。

**为什么 27x**：PyMuPDF 的瓶颈是"构造 12 个 Python dict"，这是纯语言开销。ritz 用 C 层扁平化彻底消除了它。

详见 [03-c-layer-flattening.md](03-c-layer-flattening.md)。

### 2. 文档打开：5.6x（FFI 调度场景）

**瓶颈**：对象初始化开销。

PyMuPDF 的 `fitz.open()`：
```
Python → C 扩展 → fz_open_document → 构造完整 Document 对象
  （含 lazy 状态、异常 hook、属性缓存、weakref 支持...）
```

ritz：
```
Python → PyO3 → mupdf_safe_open_document → 构造 PyDocument
  （只含 ctx + raw 两个指针）
```

**为什么 5.6x**：PyO3 的对象构造比 Python C 扩展轻量。PyDocument 只有 2 个字段，PyMuPDF 的 Document 有十几个属性初始化。

### 3. 文本提取：2.1x（MuPDF 内部计算场景）

**瓶颈**：`fz_new_stext_page_from_page`（stext 树构造）。

两个工具调的是**完全相同的 C 函数链**：
```
fz_new_stext_page_from_page  → 构造 block→line→char 树（~0.4ms）
fz_print_stext_page_as_text  → 遍历树输出文本（~0.1ms）
```

stext 构造占 0.4ms / 总 0.55ms = **73%**。这部分是 MuPDF 内部 C 代码，ritz 和 PyMuPDF 共享，无法优化。

ritz 的 2.1x 优势来自剩余 27% 的常数因子：
- growbuf 直接接收 `fz_output` 字节流，无中间 buffer 拷贝
- `String::from_utf8_lossy` 一次性构造 str

**为什么不是 10x**：MuPDF 所有文本 API 都穿过 stext 构造（`util.h` 里的 `fz_new_buffer_from_page` 等都是 stext 的封装）。没有轻量级纯文本路径。要破 10x 必须写自定义 `fill_text` device 跳过 stext，但会丢失 bidi/排版顺序——对 CJK/RTL 文本出错。

**plan_v1 预期 10x 的依据是 LiteParse（基于 PDFium）的 88x**。但 PDFium 的文本提取路径与 MuPDF 完全不同。MuPDF 的 stext 是高度优化的 C 代码，已是性能地板。

### 4. 图片提取：JPEG 6.2x / FlateDecode 1.1x

**瓶颈取决于图像格式**：

| 格式 | 瓶颈 | ritz 路径 | 加速比 |
|------|------|----------|--------|
| JPEG（DCTDecode） | 语言调度 | 返回原始 JPEG 字节，零 decode 零 encode | **6.2x** |
| FlateDecode | PNG 编码 | decode + PNG 编码（双方共享） | 1.1x |

JPEG 场景：PyMuPDF 需要 `get_images()` + `extract_image(xref)` 两步，每张图一次 FFI。ritz 一次 device 遍历 + 返回原始字节。

FlateDecode 场景：Flate 压缩的是裸像素，不是独立图像格式。必须 decode → PNG 编码才能返回可用字节。PNG 编码（zlib 压缩）占 99% 耗时，双方共享 `fz_new_buffer_from_pixmap_as_png`。

**真实世界**：论文 PDF（arxiv）18/19 张图是 FlateDecode → 1.1x。扫描文档（全 JPEG）→ 6.2x。详见 [05-image-extraction.md](05-image-extraction.md)。

### 5. 批量提取：2.4x（vs PyMuPDF）

**误区**：plan_v1 预期 batch 相比逐页循环有 10x。

**实测**：ritz batch vs ritz 逐页 = **1.0x**（无差异）。

原因：stext 构造占 99%。减少 FFI 往返（200 次 → 1 次）省下的时间被淹没。batch 的 2.4x 优势完全来自 PyO3 vs Python C 扩展的调度差异，与批量本身无关。

### 6. 多文档并行：2.1x（vs PyMuPDF multiprocessing）

ritz rayon：54ms（8 文档）。
PyMuPDF multiprocessing.Pool：111ms。

PyMuPDF 的 multiprocessing 几乎无加速（1.02x）——fork + pickle 序列化开销抵消并行收益。ritz rayon 无 IPC，1.5x 内部加速（受限于小文档负载不足）。

## 三方对比中的根因

与 gomupdf（Go + CGO）对比时，FFI 调度差异更明显：

| 场景 | ritz vs gomupdf | 根因 |
|------|----------------|------|
| 页面加载 | **133x** | Go CGO 每次调用 ~200ns 栈切换；PyO3 几乎零成本 |
| 渲染 1x | **7x** | 同上，频繁调用累积 |
| Pixmap→PNG | **3x** | ritz 1 次拷贝；Go 经 GC barrier 2 次 |
| 链接读取 | 0.2x（gomupdf 赢） | gomupdf 返回 Go struct（零转换）；ritz 返回 Python dict（构造税） |
| 文本提取 | 1.1x（持平） | 共享 MuPDF stext 地板 |

**Go CGO 的栈切换开销**是 gomupdf 在 FFI 密集场景落后的根因。每次 CGO 调用要做：保存 goroutine 状态 → 切到系统栈 → 调 C → 切回 → 恢复。PyO3 没有这个开销（Rust 和 C 共享栈）。

## plan_v1 KPI 兑现总结

| plan_v1 点名场景 | 目标 | 实测 | 达标 | 未达标根因 |
|----------------|------|------|------|-----------|
| 文本提取 | 10x~30x | 2.1x | ❌ | stext 构造是双方共享地板 |
| 图片提取 | 10x~30x | JPEG 6.2x / Flate 1.1x | ⚠️ 部分 | FlateDecode 必须 decode+PNG 编码 |
| 批量 | 10x~30x | 2.4x | ❌ | 同文本，stext 占 99% |
| 多文档并行 | 10x~30x | 2.1x | ❌ | rayon 1.5x 受限于小文档 |

| 非点名但实测大幅领先 | 倍率 | 根因 |
|---------------------|------|------|
| 链接读取 | **27x** | C 层扁平化消除 Python dict 构造 |
| 文档打开 | **5.6x** | PyO3 零成本对象构造 |
| 页面加载 | **1.9x** | 同上 |

## 教训

1. **10x+ 只在纯调度场景成立**：当瓶颈是 Python 对象构造（dict/list）而非 C 计算，C 层扁平化能拿到数量级优势
2. **共享引擎限制天花板**：ritz 和 PyMuPDF 都用 MuPDF，MuPDF 内部的 stext 构造、PNG 编码、Flate 解压是双方共享的地板
3. **plan_v1 的 10x 预期基于 PDFium**：LiteParse 的 88x 是因为 PDFium 的文本提取路径有更多可优化空间；MuPDF 已是优化过的 C 代码
4. **加速比类型化**：报告性能数据时，区分"调度优势"和"计算优势"——前者可到 10x+，后者上限 ~2x

## 如何预测一个新场景的加速比

回答三个问题：

1. **瓶颈是 C 计算还是语言调度？**
   - 如果是 C 计算（渲染、编码、stext）→ 预期 1~2x
   - 如果是语言调度（对象构造、FFI 往返）→ 预期 5~30x

2. **输出是单个 blob 还是多个对象？**
   - 单个 blob（文本字符串、像素数组）→ 直接流式输出，1~2x
   - 多个对象（链接列表、图片列表、嵌套 dict）→ C 层扁平化，5~30x

3. **PyMuPDF 的实现是否低效？**
   - 如果 PyMuPDF 做了明显多余的工作（如每 link 一个 dict）→ 大空间
   - 如果 PyMuPDF 已是最优路径 → 小空间

这三条能准确预测 ritz 在新场景的表现。
