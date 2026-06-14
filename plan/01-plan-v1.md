# 从零构建高性能 PDF 处理库

> 借鉴 LiteParse 分层架构 + Rennie 数据模型，魔改 MuPDF

---

## 1. 项目愿景与目标

### 现状

PyMuPDF（fitz）是 Python 生态中功能最全的 PDF 库，底层为 C 语言 MuPDF 引擎，通过 SWIG 生成绑定。该方案存在跨语言调用开销大、内存复制频繁、难以并行、且无法直接提取嵌入图片精确坐标等问题。

### 参考案例

- **LiteParse**：采用 Rust + PDFium，通过手工 FFI + 安全封装 + PyO3，实现了比 PyMuPDF 快 88 倍的文本提取，并提供了简洁的 Python API。其分层架构（FFI 层 → 安全封装层 → 高层 Rust API → PyO3）是本项目架构设计的核心参考。
- **Rennie 案例**：开发者 Jeffrey Rennie 用 Rust 重写 PDF 软件时，通过 **用 enum 替代继承树、返回副本而非指针、紧凑内存布局**，实现了比原 C++ 版本快 3 倍的性能。其数据模型重构思想是本项目实现性能飞跃的关键。

### 本项目目标

从零构建一个全新的高性能 PDF 处理库，对标并超越 PyMuPDF。采用 **Rust + 可魔改的 MuPDF** 技术栈，借鉴 LiteParse 的分层架构，借鉴 Rennie 的 enum 数据模型，最终输出与 fitz API 部分兼容的 Python 包。

### 核心突破

- **极致性能**：在文本提取、图片提取、批量处理及多文档并行场景下实现数量级提升（10x~30x）。
- **一级图片提取**：提供原生 API 直接返回嵌入图片的二进制数据及精确页面坐标，消除两步调用。
- **完全控制底层**：将 MuPDF C 源码作为子模块，可按需任意修改以优化特定功能（新增批量 API、优化内存布局等）。
- **安全与健壮**：正确处理 MuPDF 的 longjmp 异常机制，确保内存安全、线程安全，无资源泄漏。

### 许可证

由于 MuPDF 为 AGPL，本项目的所有 Rust 及 Python 代码同样以 **AGPL-3.0** 开源。若未来有闭源需求，需购买商业许可，但本版本仅面向开源社区。

---

## 2. 核心架构设计（借鉴 LiteParse 分层设计）

本项目完全借鉴 LiteParse 的 **四层架构**，并结合 MuPDF 后端进行实现：

```text
┌─────────────────────────────────────────────┐
│           Python 层（用户代码）              │
│   import fitz   # 与原始 fitz 接口兼容       │
└─────────────────────────────────────────────┘
                       │ PyO3 绑定
┌─────────────────────────────────────────────┐
│      高层 Rust API（`pymupdf` crate）        │
│  Document, Page, Pixmap, Rect, Matrix 等    │
│  - 对下调用安全封装                          │
│  - 向上提供 Python 友好的接口                │
│  - 在此层实现 Rennie 式 enum 数据模型        │
└─────────────────────────────────────────────┘
                       │
┌─────────────────────────────────────────────┐
│      MuPDF 安全封装（`mupdf` crate）         │
│  MuPdfContext, PdfDocument, PdfPage         │
│  - RAII 管理所有 MuPDF 资源                  │
│  - 错误码转为 Rust Result                    │
│  - 坐标系转换（左下 → 左上）                 │
│  - 不暴露任何 unsafe 接口                    │
└─────────────────────────────────────────────┘
                       │
┌─────────────────────────────────────────────┐
│      MuPDF FFI 绑定（`mupdf-sys` crate）     │
│  - bindgen 自动生成的原始 C 类型绑定         │
│  - 手写的 C 错误包装层函数声明               │
│  - 构建脚本：编译 MuPDF 源码及扩展 C 文件    │
└─────────────────────────────────────────────┘
                       │
┌─────────────────────────────────────────────┐
│          MuPDF C 源码（Git 子模块）          │
│  - 官方 MuPDF 源码（可裁剪）                 │
│  - 项目自有的 C 扩展文件（批量提取等）       │
└─────────────────────────────────────────────┘
```

### 分层职责（借鉴 LiteParse）

| 本项目 crate | 对应 LiteParse crate | 职责 |
| --- | --- | --- |
| `mupdf-sys` | `liteparse-pdfium-sys` | FFI 绑定、动态库加载、C 错误包装 |
| `mupdf` | `liteparse-pdfium` | 安全 RAII 封装、错误转换、坐标系转换 |
| `pymupdf` | `liteparse` | 高层 Rust API 和 Python 绑定 |

---

## 3. 功能范围（MVP 首批支持）

本版本聚焦于最高频、最核心的 PDF 处理能力，其余高级功能在后续版本中逐步实现。

### 3.1 与 PyMuPDF 兼容的 API（首批实现）

> **[待补充]** 原飞书文档中的 API 兼容性表格未导出，需补充首批支持的 API 清单（如 `Document.open`、`Page.get_text`、`Page.get_images` 等）。

**未实现功能处理**：所有未列入上表的 API（如注释、表单、编辑、OCR 等）均抛出 `NotImplementedError`，并附带清晰提示，避免静默失败。

### 3.2 新增/增强功能（超越原版）

**图片一级提取**：`Page.get_images(include_data=True)` 返回的每个图片字典包含：

```json
{
    "xref": 123,
    "width": 800,
    "height": 600,
    "bbox": [x0, y0, x1, y1],
    "ext": "jpeg",
    "data": "..."
}
```

- `bbox`：**新增**——精确页面坐标
- `data`：当 `include_data=True` 时返回原始字节

**批量文本提取**：`Page.get_text_batch(pages: List[int]) -> Dict[int, Any]`，一次调用返回多页的结构化文本数据，大幅减少 Python 与 Rust 间的往返次数。

---

## 4. 非功能需求

### 4.1 性能目标（对比 PyMuPDF v1.18+）

> **[待补充]** 原飞书文档中的性能基准表格未导出，需补充各场景（文本提取、图片提取、批量、并行）的具体加速比目标。

**实现路径**：

- **借鉴 Rennie 案例**：使用 enum 紧凑内存布局、返回副本而非指针，提升缓存命中率。
- **借鉴 LiteParse**：分层调用 + 批量数据传递，减少跨语言调用次数。
- **魔改 MuPDF**：新增 C 层批量 API，一次调用返回多页数据。

### 4.2 资源限制

- **内存**：打开 1000 页 PDF 后的常驻内存 ≤ 200 MB（不预缓存页面内容）。
- **包体积**：静态链接所有依赖的 wheel 包 ≤ 35 MB（Linux/macOS）；Windows 使用预编译动态库时 ≤ 15 MB。

### 4.3 平台支持

- **优先**：Linux (x86_64, aarch64)、macOS (x86_64, aarch64)
- **兼容**：Windows (x86_64)，采用预编译 MuPDF 动态库 + 头文件方式，不要求用户安装编译工具链

---

## 5. 核心技术方案（借鉴两个案例）

### 5.1 错误处理与 longjmp 隔离（沿用 LiteParse 思路）

**问题**：MuPDF 大量使用 `fz_try` / `fz_catch`（底层为 `setjmp` / `longjmp`），异常会跨过 Rust 栈帧，导致 `Drop` 无法执行，引发资源泄漏和未定义行为。

**方案**：在 `mupdf-sys` 内添加一个 C 错误包装层（`error_wrapper.c`）。为每个需要暴露给 Rust 的 MuPDF 函数实现一个 `_safe` 版本，在内部捕获异常并转为错误码返回。

```c
// error_wrapper.c
int mupdf_safe_open_document(fz_context *ctx, fz_document **doc, const char *filename) {
    fz_try(ctx) {
        *doc = fz_open_document(ctx, filename);
    }
    fz_catch(ctx) {
        return fz_caught(ctx);   // 返回错误码
    }
    return 0;                    // 成功
}
```

Rust 安全封装层（`mupdf` crate）仅调用这些 `_safe` 函数，永不会触发 longjmp 穿透。此方案完全不修改 MuPDF 源码，风险极低。

### 5.2 数据模型重构（借鉴 Rennie 案例）

**核心思想**：不要将 MuPDF 的 C 结构体直接暴露给上层，而是在 `pymupdf` 层完全转换为 Rust 原生枚举和紧凑结构体，实现缓存友好的内存布局。

**具体做法**：

定义 Rust 原生类型（例如文本块、图片信息、PDF 对象），全部使用 `enum` 和 `struct`，无指针：

```rust
pub struct TextBlock {
    pub text: String,
    pub bbox: Rect,
    pub font_name: String,
    pub font_size: f32,
}

pub enum PdfValue {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    String(String),
    Name(String),
    Array(Vec<PdfValue>),
    Dict(HashMap<String, PdfValue>),
    Stream { dict: Box<PdfValue>, data: Vec<u8> },
}
```

- 在 `pymupdf` 层实现 `From` traits，将 MuPDF 的 C 结构体（如 `fz_stext_page`、`fz_image`）一次性复制转换为上述 Rust 类型。
- **为什么复制反而更快**：Rennie 的实践证明，紧凑的栈上数据（或连续堆数据）比散布在堆中的指针链具有更好的缓存局部性，即使加上复制开销，整体性能仍大幅提升。

### 5.3 并行策略（借鉴 LiteParse + 魔改）

- **多文档并行（主推）**：每个线程创建独立的 `fz_context` 并独立打开文档，通过 `rayon` 实现多文档并行处理，可达到近线性加速。适用于批量处理任务。
- **单文档内批量操作**：不依赖多线程并发，而是通过魔改 MuPDF 新增 C 层批量 API（如 `fz_extract_pages_text`）一次性提取多页数据，减少 Rust ↔ C 调用次数，实现 2-3 倍提速。
- **不承诺单文档多页并行**：因 `fz_document` 非线程安全，强行并行需每页独立打开文档，内存开销大且收益不稳定，首版不提供。

### 5.4 坐标系统一

MuPDF 内部原点在页面左下角，Y 轴向上；Python API 期望原点在左上角，Y 轴向下。所有坐标转换集中在 `mupdf` 安全封装层处理：

```text
y_new = page_height - y_old
```

### 5.5 魔改 MuPDF 与长期维护（借鉴 Rennie + 社区最佳实践）

- **扩展原则**：不修改 MuPDF 已有函数的行为，仅在独立的 C 文件（如 `src/bridge/mupdf_extensions.c`）中新增辅助函数，并使用 `mupdf_ext_` 前缀命名。
- **补丁管理**：所有修改通过 `git format-patch` 生成补丁文件，存储在仓库中，`build.rs` 在编译前自动应用补丁。
- **向上游贡献**：将通用性强的扩展函数整理后尝试提交 PR 至 MuPDF 官方仓库。
- **定期同步**：每季度合并上游主分支，并重新适配补丁。

---

## 6. 实施计划（5–7 周，单人全职，分 4 阶段）

> 基于 plan_v2.md 的工程量实测推算：完整 API 兼容 + 子模块编译 + 批量 C 扩展三项叠加，原 14 天排期不现实。改为 4 阶段渐进交付，每阶段产出可用版本。

### 阶段一：构建基础设施打通（1 周）

**目标**：MuPDF 子模块能编译，Rust 能 `open("x.pdf").page_count()`。

- [ ] 初始化 git 仓库，加 MuPDF 子模块
- [ ] 写 `build.rs`：`make` 编译 MuPDF，链接 `libmupdf.a`
- [ ] `error_wrapper.c` 实现：context / open / count_pages / load_page / bound_page / drop_*
- [ ] bindgen 生成 `bindings.rs`（含预生成 fallback）
- [ ] `mupdf-sys` crate 编译通过
- [ ] `mupdf` crate：Context + Document + 基础方法
- [ ] Rust 集成测试：打开测试 PDF，断言 page_count 正确

**交付物**：`cargo test` 能打开 PDF 取页数。**这是全项目风险最高的去风险化里程碑。**

### 阶段二：文本提取 + PyO3 最小可用（1.5 周）

**目标**：`pip install` 后能 `fitz.open("x.pdf")[0].get_text("text")`。

- [ ] `error_wrapper.c` 增加：stext 相关、渲染 pixmap
- [ ] `mupdf` crate：Page、STextPage、Pixmap、Rect/Matrix/Point
- [ ] `pymupdf` crate：PyO3 绑定 Document/Page/Pixmap
- [ ] 实现 `get_text("text")` 和 `get_text("blocks")`
- [ ] 实现 `get_pixmap()` + `Pixmap.tobytes("png")`
- [ ] `fitz/__init__.py` 重导出 + `pyproject.toml`（maturin 构建）

**交付物**：可 `pip install`，核心文本提取 + 渲染可用。

### 阶段三：完整 API + Rennie 数据模型（1.5 周）

**目标**：`get_text` 全模式、`get_images` 含 bbox、`get_links`。

- [ ] `get_text("dict")` / `"rawdict"`：TextPageData + 嵌套转换（block→line→span→char）
- [ ] `get_text("words")` / `"json"` / `"rawjson"` / `"html"` / `"xhtml"` / `"xml"`
- [ ] 图片一级提取：`get_images(include_data=True)` 含 bbox + data
- [ ] `get_links()` 返回链接字典列表
- [ ] metadata、rotation、cropbox、mediabox 属性
- [ ] Rennie `PdfValue` enum（为后续对象操作铺路）

**交付物**：与 PyMuPDF 核心文本/图片/链接 API 兼容。

### 阶段四：批量扩展 + 并行 + 打磨（1 周）

**目标**：性能优势兑现。

- [ ] `mupdf_extensions.c`：`mupdf_ext_extract_pages_text` 批量扁平化
- [ ] `get_text_batch()` Python API
- [ ] rayon 多文档并行：每文档独立 cloned context，近线性加速
- [ ] criterion 性能基准：对比 PyMuPDF，产出对比报告
- [ ] PyMuPDF 测试用例移植：通过率 ≥ 90%
- [ ] 文档：README + Jupyter 演示

**交付物**：性能基准报告 + 完整测试套件。

> 详细 crate 结构、关键代码骨架（error_wrapper.c / 安全封装 / PyO3）、风险矩阵见 [plan_v2.md](./02-plan-v2.md)。

---

## 7. 风险与应对

> **[待补充]** 原飞书文档中的风险矩阵未导出，需补充风险项、影响等级与应对策略。

---

## 8. 交付物与验收标准

1. **源码仓库**：包含 `mupdf-sys`、`mupdf`、`pymupdf` 三个 Rust crate 及 Python 绑定。
2. **Python 包 `fitz`**：可通过 `pip install` 安装，在目标平台上导入并使用首批支持的 API。
3. **文档**：
   - `README.md` 包含安装说明、快速开始、与 PyMuPDF 的 API 差异、性能对比数据。
   - Jupyter Notebook 演示核心功能（文本/图片提取、批量操作、并行处理）。
4. **测试**：
   - Rust 单元测试覆盖率 ≥ 70%。
   - 从 PyMuPDF 官方测试集中移植的核心用例通过率 ≥ 90%。
   - 性能基准测试报告显示达到第 4.1 节目标（允许 ±10% 偏差）。
5. **许可证**：源码树包含 `LICENSE` 文件明确 AGPL-3.0 许可。

---

## 9. 参考资料

- **LiteParse 源码**：<https://github.com/run-llama/liteparse>
  - 重点参考 `liteparse-pdfium-sys` 和 `liteparse-pdfium` 的分层架构、动态库加载、PyO3 绑定。
- **Rennie 文章**：[03-rennie-data-model.md](./03-rennie-data-model.md)（*I built the same software 3 times, then Rust showed me a better way* 翻译）
  - 重点借鉴其 **enum 替代继承树、返回副本、紧凑内存布局** 思想。
- **mupdf-rs**：<https://github.com/jrmuizel/mupdf-rs>
  - 参考其 `build.rs` 与绑定生成方式。
- **PyMuPDF 源码**：<https://github.com/pymupdf/PyMuPDF>
  - 查阅原始 Python API 定义以保持兼容。
- **MuPDF 官方文档**：<https://mupdf.com/docs/index.html>
  - 重点阅读 `fitz.h` 和 `pdf.h` 中的 API。
