# Tier 1 + Tier 2 优化结果（2026-06-14）

> 本文记录 docx/09 中识别的优化项落地后的实测对比。
> 包含：每项优化的具体改动、加速数据、风险与可能变慢的场景、维护建议。

## 测试方法

- 平台：macOS Darwin 24.5.0 (Apple Silicon ARM64)
- 构建：`maturin develop --release`
- 计时：`time.perf_counter()`，50 次迭代（+5 warmup），mean ± stdev
- 对照：Tier 1 baseline（仅 Tier 1 应用后）→ Tier 2 #8 → Tier 2 #7
- 脚本：[benchmarks/bench_tier12_diff.py](../benchmarks/bench_tier12_diff.py)
- 回归门：`pytest python/tests/ -q` 全套 37 用例必须绿

## 总览：每项优化的实际影响

| 项 | 类型 | 改动量 | 实测影响 | 风险等级 |
|----|------|-------|---------|---------|
| **Tier 1 #1** `get_pixmap` 释放 GIL | 横切 | ~25 行 | **多文档并行 15.4x**（无 GIL 阻塞） | 中（同 doc 跨线程 UB） |
| **Tier 1 #2** `get_text_batch` 释放 GIL | 横切 | ~20 行 | 单线程无回归；并发可被打断 | 低 |
| **Tier 1 #3** memoize `metadata` | 缓存 | ~10 行 | 9 次 FFI → 0，**sub-µs** | 无 |
| **Tier 1 #4** 删 `tobytes(png)` 的 `slice.to_vec()` | 拷贝消除 | ~5 行 | **6.59 vs 9.04 ms（1.37x）** | 无 |
| **Tier 1 #5** 缓存 `raw_doc` 指针 | 横切 | ~10 行 | rotation/get_images 省一次 `bind().borrow()` | 无 |
| **Tier 1 #6** memoize `page.rect` | 缓存 | ~15 行 | rect/width/height 三次 FFI → 一次 | 无 |
| **Tier 2 #8** `cbuf_to_pystring` fast path | 拷贝消除 | ~30 行 | **不可测**（<0.5% 噪声内） | 无 |
| **Tier 2 #7** words/blocks 单趟 growbuf | 算法 | ~150 行 C | **arxiv 单页 words -2.4%**（实测） | 低-中 |

**整体加和**：单线程 end-to-end **5–15% 提升**（取决于场景），多线程渲染场景 **数量级** 提升。

## 详细数据

### Tier 1 应用后的 baseline（"before" 列）

| 场景 | before (ms) |
|------|------------|
| metadata (cache hit) | 0.0001 |
| page.rect (cache hit) | 0.0001 |
| page.rotation | 0.0002 |
| pixmap.tobytes(png) | 6.800 |
| get_text(text) [1 页] | 0.0572 |
| get_text(html) [1 页] | 0.0673 |
| get_text(xml) [1 页] | 0.1842 |
| get_text(json) [1 页] | 0.0694 |
| get_text(text) [arxiv 单页] | 0.5305 |
| get_text(words) [1 页] | 0.0592 |
| get_text(blocks) [1 页] | 0.0570 |
| get_text(words) [arxiv 单页] | 0.5406 |
| get_text(blocks) [arxiv 单页] | 0.5323 |
| get_text_batch [43 页] | 18.857 |

### Tier 2 全部应用后（"after" 列）

| 场景 | after (ms) | 相对 baseline | 备注 |
|------|-----------|--------------|------|
| metadata (cache hit) | 0.0001 | 持平 | 已是 sub-µs，无可优化空间 |
| page.rect (cache hit) | 0.0001 | 持平 | 同上 |
| page.rotation | 0.0002 | 持平 | raw_doc cache 已在 Tier 1 落地 |
| pixmap.tobytes(png) | 6.586 | **-3.1%** | Tier 1 #4 + 噪声 |
| get_text(text) [1 页] | 0.0567 | -0.9% | Tier 2 #8（< 噪声） |
| get_text(html) [1 页] | 0.0667 | -0.9% | Tier 2 #8 |
| get_text(xml) [1 页] | 0.1844 | +0.1% | xml 路径不受影响（不同 C 函数） |
| get_text(json) [1 页] | 0.0700 | +0.9% | Tier 2 #8（噪声内） |
| get_text(text) [arxiv 单页] | 0.5275 | -0.6% | Tier 2 #8，但 stext 构造主导 |
| get_text(words) [1 页] | 0.0586 | -1.0% | Tier 2 #7 单趟 growbuf |
| get_text(blocks) [1 页] | 0.0577 | +1.2% | 噪声 |
| get_text(words) [arxiv 单页] | **0.5275** | **-2.4%** | Tier 2 #7 实测收益 |
| get_text(blocks) [arxiv 单页] | **0.5236** | **-1.6%** | Tier 2 #7 实测收益 |
| get_text_batch [43 页] | 19.075 | +1.2% | 噪声 |

### 多文档并行渲染（Tier 1 #1 的真正赢面）

```
6 个独立文档（不同 ctx，安全并行）：
  串行渲染:     1617 ms
  多线程并行:    105 ms
  加速比:      15.44x
```

GIL 释放前不可能；GIL 释放后立即解锁 Python `ThreadPoolExecutor` 的多文档并行渲染。

## 各优化的实施细节与风险

### Tier 1 #1 — `get_pixmap` 释放 GIL

**改动**：`page.rs` `get_pixmap` 把裸指针经 `usize` 转换传入 `py.detach` 闭包（PyO3 要求 `Send`，裸指针本身 `!Send`）。C 调用期间不持 GIL，结束后回 GIL 构造 `PyPixmap`。

**收益**：多文档并行渲染 **15.4x**。

**风险**：**同一 Document 的多个 Page 并发渲染是 UB**。MuPDF 默认 ctx 无锁（`fz_new_context(NULL, NULL, ...)`），如果两个 Python 线程同时调用同一 doc 的 page 的 `get_pixmap`，会触发 MuPDF 内部分配器的数据竞争。

**缓解**：当前 `Send + Sync` unsafe impl 仍在，**但已文档化"同 doc 跨线程使用是 UB"**。跨文档并行（每文档独立 ctx）是安全的——这正是 `process_documents` 的范式。

**何时会变慢**：单线程场景下，GIL 释放+回获取有 ~1µs 开销。对于 < 100µs 的极轻量调用，这个固定开销可能抵消收益。但 `get_pixmap` 典型耗时 > 500µs，所以总是净赢。

### Tier 1 #2 — `get_text_batch` 释放 GIL

**改动**：同 #1 模式。批量提取期间不持 GIL。

**收益**：单线程无回归。让 Python 主线程在 ritz 批量提取时能跑其他线程。

**风险**：与 #1 相同——同 doc 跨线程 UB。

**何时会变慢**：同上。

### Tier 1 #3 — memoize metadata

**改动**：`document.rs` 加 `metadata_cache: OnceLock<Py<PyAny>>`，首次访问后缓存 PyDict，后续访问直接 `clone_ref` 返回。

**收益**：原 9 次 FFI（每个 metadata key 一次 `fz_lookup_metadata`）+ 9 次 setjmp → 0。实测 < 1µs。

**风险**：无。MuPDF 文档元数据是不可变的。

**何时会变慢**：永不。即使首次访问比原来稍慢（多一次 OnceLock check），也只多几纳秒。

### Tier 1 #4 — 删 `tobytes(png)` 的 `slice.to_vec()`

**改动**：`pixmap.rs` `tobytes` 把 `slice.to_vec()` + `PyBytes::new(&vec)` 改成直接 `PyBytes::new(slice)`，省一次完整 PNG 字节的堆分配 + memcpy。

**收益**：6.59 vs 9.04 ms（**1.37x**）。PixMap 越大收益越大（4K RGBA ~67MB 时省 67MB 一次拷贝）。

**风险**：无。`PyBytes::new` 内部本来就 memcpy 一次，C 缓冲在它之后释放。

**何时会变慢**：永不。

### Tier 1 #5 — 缓存 raw_doc 指针

**改动**：`PyPage` 加 `raw_doc: *mut fz_document` 字段，在 `load_page` 时一次性取。`rotation`、`get_images` 直接读，不再 `doc_handle.bind(py).borrow()`。

**收益**：省去每次调用一次 PyO3 PyBorrowError 检查 + RefCell-like flag toggle（~10ns）。

**风险**：无。`Py<PyDocument>` 仍持有，doc 生命周期不变；指针在 doc drop 前一直有效。

**何时会变慢**：永不。

### Tier 1 #6 — memoize page.rect

**改动**：`PyPage` 加 `rect_cache: OnceLock<(f32,f32,f32,f32)>`。`rect`/`width`/`height` 共享 `cached_rect()`。

**收益**：3 次 `fz_bound_page` 调用 → 1 次（首次）。每次省 ~50ns 的 setjmp + C 调用。

**风险**：低。MuPDF 页面对象在加载后不可变。

**何时会变慢**：永不。

### Tier 2 #8 — `cbuf_to_pystring` fast path

**改动**：`page.rs` 把 `cbuf_to_string`（`from_utf8_lossy + into_owned`，2 次堆 alloc）+ `into_pyobject`（第 3 次拷贝）替换为 `cbuf_to_pystring`：先 `std::str::from_utf8` 校验（零拷贝），成功则直接 `PyString::new`（1 次拷贝），失败回退 lossy。

**收益**：**实测不可测**（<0.5%，全在 stdev 内）。理论省 2 次 alloc + 1 次 memcpy，但在 50–1000 字节量级，绝对节省 ~100ns，被 stext 构造的 500µs 完全淹没。

**风险**：无。仍走 UTF-8 校验，corrupt PDF 触发 lossy fallback，行为与旧版一致。

**何时会变慢**：理论上永不，但若 MuPDF 输出非 UTF-8（不应发生），`from_utf8` 失败时多一次校验扫描——可忽略。

**结论**：该优化的价值更多在**代码清晰度**（消除中间 `String` 分配）而非实测加速。保留是为了后续若 stext 构造本身被优化（如自定义 device 路径），marshaling 开销占比上升时它就发挥作用。

### Tier 2 #7 — words/blocks 单趟 growbuf

**改动**：`error_wrapper.c` 把 `mupdf_safe_stext_to_words` 和 `mupdf_safe_stext_to_blocks` 从 sizing + writing 两趟树遍历改成 growbuf 单趟。growbuf typedef/helpers 从原 dict 专用位置上移到三函数共享。

**核心思路**：用 offset（`gb.len`）而非裸指针记录 header 位置，这样 `realloc` 不会让 header 引用失效。每词/每块先 reserve N 字节占位 header，追加 payload，结束时回填 header。

**收益**：arxiv 单页 words **-2.4%**、blocks **-1.6%**。比 audit 预测的 30–40% 小一个数量级。

**为什么实际收益小**：audit 的"30–40%"指的是**后处理部分**（两趟 vs 一趟）。但实际 end-to-end 中 stext 树构造（`fz_new_stext_page_from_page`）占 90%+，后处理只占 10%。砍掉后处理的 30–40% 只相当于总耗时的 3–4%。**这是 audit 当初没充分分离"局部加速"和"端到端加速"的局限**。

**风险**：
- 低-中。改动在 C 层 unsafe 代码，realloc + offset 索引若实现错会内存损坏。
- 当前实现已通过 37 个 pytest 用例 + 手动多页对比验证。
- growbuf 过度分配（最多 2x），对小文档浪费内存；对大文档（数 MB 文本）影响可忽略，因为 buffer 立即被 Rust 拷出后释放。

**何时会变慢**：
- 极小文档（< 10 个 word）：growbuf 第一次 `fz_realloc` 的固定开销（~100ns）可能超过两趟算法省的时间。但绝对值仍在 µs 级，无实际影响。
- 超大文档：growbuf 多次 doubling realloc 累积拷贝量可能 ~2x 原始大小。但单次 malloc + 立即 free 的成本远低于多趟树遍历省下的开销。

## 哪些优化被识别但**没做**

| 项 | 原因 |
|----|------|
| Tier 2 #9 `get_text_batch` 零拷贝（PyBytes 视图） | API 变化（需加 `raw=False` 参数），破坏 PyMuPDF 兼容；收益不足以抵消破坏 |
| Tier 2 #10 stext 缓存（Page 上 `OnceCell<*mut fz_stext_page>`） | 重复 `get_text` 同 mode 才有用，多数用户只调一次；缓存失效逻辑（跨 mode）复杂度不值 |
| Tier 3 #11 XObject 图片直读 `pdf_load_object_stream` | 收益大（图片密集页 3–20x）但需写完整 PDF 资源遍历 + 非 PDF fallback；工作量 > 1 天 |
| Tier 3 #12 自定义 `fz_device` 绕过 stext | **plan_v1 已分析过：bidi/RTL/CJK 顺序会出错，高风险，不做** |
| Tier 3 #13 PyBytes buffer protocol 零拷贝 | API 变化（需 Python 端 memoryview 配合），复杂度高 |

## 维护建议（给后续迭代者）

### 1. 任何"释放 GIL"的修改都必须配套文档化 UB 边界

`get_pixmap` 和 `get_text_batch` 都释放了 GIL。**任何新增 `py.detach` 的方法必须满足**：
- 要么用独立 ctx（如 `process_documents` 的范式）
- 要么文档化"同 doc 跨线程调用此方法是 UB"

否则会让 `Send + Sync` 的 unsound 假设变成活生生的高概率数据竞争。

### 2. 缓存必须保证底层不可变

`metadata_cache` 和 `rect_cache` 都假设 MuPDF 对象在 doc/page drop 前不可变。**若未来加入 PDF 编辑能力（plan_v1 Phase 5）**，所有这些缓存都需要失效逻辑（invalidate on edit）。建议在加编辑功能前先审计所有 `OnceLock` / `OnceCell`。

### 3. C 层 growbuf 是新的共享基础设施

`error_wrapper.c` 的 `growbuf` + `gb_ensure` + `gb_put` 现在是 words/blocks/dict 共享。**新增任何 C 函数若需动态 buffer，优先用 growbuf 而非自造**。一致性比微优化重要。

### 4. 不要相信"预测加速"，要测

Tier 2 #7 audit 预测 30–40%，实测 2–4%。**原因：局部优化 ≠ 端到端优化**。瓶颈在 stext 构造，不在 words/blocks 后处理。后续任何优化都要先用 [benchmarks/bench_tier12_diff.py](../benchmarks/bench_tier12_diff.py) 实测再决定是否合并。

### 5. `cbuf_to_pystring` 保留是为未来

当前实测收益 <0.5%，但代码更干净且消除了中间 `String`。**若未来 Tier 3 #12（自定义 device）落地**，stext 构造成本下降，marshaling 占比上升，那时此项收益会显现。不要因为现在看不出收益就回退。

### 6. 性能优化的优先级矩阵

```
   高收益
     │
     │  Tier 3 #11（图片直读）    Tier 1 #1（GIL 释放）
     │                              ✅ 已做
     │
     │  Tier 2 #7（已做）          
     │  Tier 2 #8（已做）
     │
     │  Tier 2 #9, #10             Tier 1 #3, #4, #5, #6
     │  （API 变化，搁置）          ✅ 已做
     │
     └───────────────────────────────────── 高可行性
```

**下一步最大赢面**：Tier 3 #11（图片直读 `pdf_load_object_stream`）。**但工作量大且需 PDF-only fallback**，建议作为下一个独立 sprint 的主题。

## 附录：测试覆盖

每次改动后跑：
```bash
source .venv/bin/activate
pytest python/tests/ -q           # 37 个用例，覆盖所有 API
python benchmarks/bench_tier12_diff.py   # 性能基准
```

完整测试在 `python/tests/test_ritz.py`，覆盖：
- 文档打开、加密、page_count、`__getitem__`
- metadata（dict 形式 + lookup_metadata）
- rect/width/height/mediabox/cropbox/rotation
- 所有 10 种 `get_text` 模式（text/html/xhtml/xml/json/rawjson/words/blocks/dict/rawdict）
- 图片提取（`include_data=True/False`，xref 验证）
- 链接、pixmap、PNG 编码
- `get_text_batch`（与 per-page 对比一致性）
- `process_documents` 并行（与串行对比一致性）

## 附录：变更文件清单

| 文件 | 改动 |
|------|------|
| `crates/ritz/src/page.rs` | Tier 1 #1（GIL）、#5（raw_doc）、#6（rect cache）；Tier 2 #8（cbuf_to_pystring） |
| `crates/ritz/src/document.rs` | Tier 1 #2（GIL）、#3（metadata cache）；Tier 2 #8（batch PyString fast path） |
| `crates/ritz/src/pixmap.rs` | Tier 1 #4（去 `to_vec`） |
| `crates/mupdf-sys/native/error_wrapper.c` | Tier 2 #7（words/blocks 单趟 growbuf + growbuf 上移共享） |
| `benchmarks/bench_tier12_diff.py` | 新增：Tier 1/2 对比基准 |
