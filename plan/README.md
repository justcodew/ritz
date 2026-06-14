# plan/ — 项目起源文档

> **为什么做、做什么、灵感从哪来**。这里是项目最原始的构想，比 `docx/` 早，比 `CHANGELOG` 更顶层。
>
> **三层文档的关系**：
> - `plan/` —— **初衷与范围**（为什么做、做什么、灵感来源）
> - `docx/` —— **技术决策记录**（怎么做、关键不变量、风险地图）
> - `CHANGELOG.md` —— **版本发布账本**（实际做了什么）

## 阅读顺序

| 顺序 | 文档 | 一句话 |
|------|------|--------|
| 1 | [01-plan-v1.md](01-plan-v1.md) | 项目愿景、4 项核心 KPI（性能/图片/控制/安全）、四层架构方向 |
| 2 | [02-plan-v2.md](02-plan-v2.md) | 工程化方案：crate 结构、关键代码骨架、分阶段交付估算 |
| 3 | [03-rennie-data-model.md](03-rennie-data-model.md) | Jeffrey Rennie 文章翻译——PdfValue enum 数据模型的灵感来源 |

## 各文档定位

### 01-plan-v1.md —— 愿景与目标

立项文档。立了 4 项核心突破：

- **极致性能**：文本/图片/批量/多文档并行 10x~30x
- **一级图片提取**：原生 API 直接返回嵌入图片字节 + 精确坐标
- **完全控制底层**：MuPDF 作为子模块，可按需魔改
- **安全健壮**：longjmp 隔离、内存安全、无泄漏

> ⚠️ **诚实提示**：plan_v1 的 "10x~30x" 在 MuPDF 后端上**只对纯调度场景成立**（如链接读取 27x）。计算密集场景（文本提取、Flate 图片）双方共享 MuPDF C 引擎，天花板 ~2x。详见 [docx/07-performance-analysis.md](../docx/07-performance-analysis.md) 和 [benchmarks/benchmark_report.md](../benchmarks/benchmark_report.md) 的"plan_v1 KPI 兑现账本"。

### 02-plan-v2.md —— 工程方案

基于 plan_v1 细化的可执行工程方案。重点：

- 4 个 crate 的职责划分（mupdf-sys / mupdf / ritz / python）
- 关键代码骨架（longjmp 隔离、PyO3 绑定、批量 FFI）
- 4 阶段交付估算（每阶段产出可用版本）
- 与 LiteParse / gomupdf / PyMuPDF 源码深挖对照

### 03-rennie-data-model.md —— PdfValue 灵感来源

Jeffrey Rennie 的文章翻译（[原文](https://itnext.io/i-built-the-same-software-3-times-then-rust-showed-me-a-better-way)）。
他用 Rust 重写 PDF 软件，通过**用 enum 替代继承树、返回副本而非指针、紧凑内存布局**，比 C++ 版本快 3 倍。

ritz 的 `PdfValue` enum（10 种 PDF 对象类型）就是这个思路的直接应用。详见 [docx/01-architecture.md §"关键技术决策"](../docx/01-architecture.md)。

## 历史背景

- **2026 年初**：plan_v1 立项，参考 LiteParse（Rust + PDFium，88x 文本提取）和 Rennie 文章定下方向
- **2026 年中**：plan_v2 细化为可执行方案，引入 MuPDF 子模块（替代 PDFium）和 patches/ 工作流
- **2026-06-14**：0.1.0 发布，含 plan_v1 阶段 1–4 全部 API + Tier 1/2 实现层优化

详见 [CHANGELOG.md](../CHANGELOG.md) 和 [docx/00-onboarding-guide.md](../docx/00-onboarding-guide.md)。
