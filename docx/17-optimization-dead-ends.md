# 优化死路记录（2026-06-15）

> Phase 6 之后尝试的两条优化路径全部失败。本文档记录**假设、PoC、实测数据、失败根因**，
> 避免未来重复同样的尝试。这是一份"已经撞过的墙"清单。

## 背景

Phase 6（stext 缓存，多模式 71x）兑现后，剩余的加速机会都集中在"单文档单次提取"场景。
corpus benchmark 显示 ritz 单文档 mean 1.0ms vs PyMuPDF 0.7ms，仍有 ~50% 差距。
本次尝试两条路径缩小这个差距，**两条都失败**。

## 死路 1：LTO + codegen-units=1

### 假设

release profile 启用 `lto=true` + `codegen-units=1`，让 Rust 编译器跨 crate 内联，
PyO3 调度路径（每次 get_text 必经）会变快，全场景普涨 5-15%。

### 实施

`Cargo.toml` workspace 末尾加：

```toml
[profile.release]
lto = true
codegen-units = 1
```

构建时间从 ~5s 涨到 ~10s（链接阶段变慢），符合 LTO 预期。

### 实测数据

corpus benchmark（2907 PDFs，单文档单次提取）：

| 库 | baseline mean | LTO 后 mean | 变化 |
|----|---------------|-------------|------|
| ritz | 1.0ms | 1.03ms | ±3%（噪声） |
| ritz_batch | 0.9ms | 0.88ms | 噪声 |
| PyMuPDF | 0.7ms | 0.7ms | 基准 |

多模式同页（Phase 6 缓存场景）：

| 场景 | baseline | LTO 后 |
|------|----------|--------|
| 4 模式/轮 | 0.009ms (71.9x) | 0.010ms (69.9x) |
| 单模式 text | 169x | 163.8x |

### 失败根因

**LTO 只作用于 Rust 层**。ritz 的瓶颈分布：
- fz_new_stext_page_from_page（C 内部）：~200-500μs，**LTO 碰不到**
- fz_run_page PDF 解释器（C 内部）：变量，**LTO 碰不到**
- Rust 调度层（PyO3 + mupdf 安全包装）：~5-15μs，LTO 优化对象

Rust 调度层已经被 Tier 1+2 + Phase 6 优化压到 5-15μs，LTO 在这部分能省的 ≤2μs，
被 MuPDF C 内部的几百微秒噪声完全盖住。

### 教训

凡是 Rust 编译器层面的优化（LTO/PGO/codegen-units）对 **C-bound 场景**全部无效。
判断方法：如果场景的瓶颈在 `fz_*` 系列函数（C 库内部），不要尝试编译器优化。

### 处置

**保留 LTO 配置**——无害（运行时无退化），构建慢一点但可接受。
未来若 Rust 层逻辑变重（如引入新的 Rust 原生解析路径），LTO 会自动受益。
**不发布、不宣传**——无用户可见收益。

## 死路 2：轨道 A（自定义 fz_device 绕过 stext）

### 假设

`page.get_text("text")` 当前路径是 `fz_new_stext_page_from_page`（构造完整 block→line→char 树）→ 遍历打印。
假设 stext 树构造占主导成本（plan_v1 估计 200-500μs）。
自定义一个 minimal `fz_device`，只实现 `fill_text` 回调，直接把 `fz_text_span.items[].ucs`
拼成 UTF-8 字节流，**跳过 stext 树构造**。

预期收益：纯文本提取 3-10x（飞书文档预估）。PyMuPDF 也走 stext，ritz 绕过 = 反超。

### 关键侦察发现

`fz_text` 对象的 `items[].ucs` 字段**已经包含 unicode**（stext-device.c:1172 直接读这个字段）。
所以 minimal device 不需要 font 查询、不需要 glyph→unicode 映射，零额外开销就能拿字符。

### PoC 实施

C 层新增 `mupdf_safe_fast_text`（`error_wrapper.c`）：

```c
typedef struct {
    fz_device base;
    growbuf *gb;  /* 外部所有，drop_device 不释放 */
} fast_text_device;

static void fast_text_fill_text(fz_context *ctx, fz_device *dev, const fz_text *text, ...) {
    for (const fz_text_span *span = text->head; span; span = span->next) {
        for (int i = 0; i < span->len; i++) {
            int c = span->items[i].ucs;
            if (c < 0 || c == 0xFFFD) continue;
            char tmp[8];
            int n = fz_runetochar(tmp, c);
            /* ... memcpy to gb ... */
        }
    }
}
```

只实现 fill_text / clip_text / ignore_text / drop_device 四个回调。
不做坐标距离判断（无空格/换行），输出纯字符流。

### PoC 实测数据

**正确性**：sample.pdf 上与 stext 路径**字符级 100% 一致**（只差 stext 插入的 \n）。
但大 PDF（2.6MB）上 fast 比 stext 多 10 个字符（重复 fill_text 调用未去重）。

**性能**（每次新 page 实例 + 提取一次，corpus 真实场景）：

| 场景 | ritz fast_text | ritz stext | fast vs stext |
|------|---------------|------------|---------------|
| sample.pdf（小，162 字符） | 48.6μs | 57.8μs | **1.19x**（微弱） |
| 大 PDF（2.6MB，100 字符） | **75.5μs** | **43.7μs** | **0.58x（反而更慢）** |

相对 PyMuPDF 仍有 1.9-2.8x 加速，但相对 ritz 已有的 stext 路径**没有优势，甚至更慢**。

### 失败根因

**"stext 构造是瓶颈"这个假设是错的**。真实瓶颈分布：

1. **stext device 有 `lasttext` 去重**（stext-device.c:1436）：
   ```c
   if (text == tdev->lasttext && ...) return;
   ```
   MuPDF 内部缓存上次 text 指针，相同则跳过整个 span 处理。
   minimal device 没这个优化，重复文本被多次处理 → 大 PDF 反而更慢 + 多出字符。

2. **`fz_run_page` 是双方共享地板**：PDF 解释器 + operator 调度成本，
   fast 和 stext 都要走。stext device 的额外成本（pool allocator + 树构造）
   在大文档上被 lasttext 去重抵消了。

3. **stext device 经过 Artifex 多年优化**：bidi 重排、actualtext 处理、坐标距离判断、
   pool allocator 都高度调优。minimal device 重写很难超过它。

4. **stext 树构造实际只占 9μs**（sample.pdf 实测：fast 48μs vs stext 57μs），
   不是 plan_v1 估计的 200-500μs。原估计混淆了"fz_run_page 总成本"和"stext 构造成本"。

### 教训

1. **不要假设 C 库内部成本分布**——必须实测。plan_v1 估计的"stext 构造 200-500μs"
   把 fz_run_page 的成本算进了 stext 构造，导致轨道 A 的收益预估虚高 10 倍。

2. **不要重写经过多年优化的库内部**——stext device 的 lasttext 去重、pool allocator
   都是 Artifex 多年调优的产物，minimal 重写在首个 benchmark 就会暴露短板。

3. **PoC 验证必须覆盖多种文档规模**——sample.pdf（小）上 fast_text 微弱领先，
   大 PDF 上反而更慢。只测小文档会得到误导性结论。

### 处置

**删除全部 PoC 代码**：
- `error_wrapper.c` 的 fast_text_device 结构 + 4 个回调 + `mupdf_safe_fast_text` 函数（~95 行）
- `error_wrapper.h` 的 `mupdf_safe_fast_text` 声明
- `bindings.rs` 的 extern "C" 块
- `page.rs` 的 `_fast_text_poc` 方法

**未保留任何痕迹**——失败假设不值得占用代码库。本文档是唯一记录。

## 共同教训：ritz 的优化天花板

两条死路共同证明：**MuPDF 1.27 stext 引擎是双方共享地板**。
ritz 已经通过三种手段建立对 PyMuPDF 的优势：

1. **C 层扁平化**（链接 27x、JPEG 6.2x）—— 减少 FFI 往返
2. **stext 缓存**（多模式 71x）—— Phase 6，跳过同页多次重建
3. **dispatch 层压薄**（单文档 ~2x）—— Phase 6 buffer borrow + UTF-8 skip

**单文档单次提取**场景的剩余 50% 差距（ritz 1.0ms vs PyMuPDF 0.7ms）
全部来自 MuPDF C 内部（fz_run_page + fz_new_stext_page_from_page），
ritz 这层无法突破，除非：
- 修改 MuPDF 源码（违反"不修改上游"原则）
- 替换 stext 引擎（如用 pdf_oxide 的 Rust 原生提取，工作量巨大）

## 剩余可行方向

只剩一条：**3.2 页面级并行**（rayon + fz_clone_context）。
它解决的是**大文档吞吐**问题（单文档 >50 页时 N 核 ~N 倍），
**不解决 corpus 单文档场景**（文档太小，clone_context 的 ~ms 开销大于收益）。
plan_v1 已评估"小文档不划算"。是否实施取决于是否有大文档用户场景。

## 时间线

- 2026-06-14：Phase 6 stext 缓存上线（commit a2d631e），多模式 71x
- 2026-06-15 上午：LTO 验证失败（保留配置，无收益）
- 2026-06-15 下午：轨道 A PoC 验证失败（清理代码）
- 2026-06-15 晚：本文档定稿，v0.4.0 发布
