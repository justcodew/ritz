# ritz 技术文档

> fitz, reforged in Rust.

本目录是 ritz 的关键技术决策文档，面向想要理解内部实现或贡献代码的开发者。

## 文档索引

| 文档 | 主题 | 核心问题 |
|------|------|---------|
| [00-onboarding-guide.md](00-onboarding-guide.md) | **接手指南** | **新维护者必读：2 小时入门 + 尸体地图** |
| [01-architecture.md](01-architecture.md) | 四层架构总览 | 为什么要分四层？每层职责？ |
| [02-longjmp-isolation.md](02-longjmp-isolation.md) | longjmp 隔离 | MuPDF 的 setjmp/longjmp 如何穿透 Rust 栈导致 UB？如何隔离？ |
| [03-c-layer-flattening.md](03-c-layer-flattening.md) | C 层扁平化策略 | 为何链接读取能快 27x？树/链表遍历为什么放 C 层？ |
| [04-memory-safety.md](04-memory-safety.md) | 内存安全与生命周期 | `*mut fz_context` 是裸指针，如何保证不悬挂？Drop 顺序？ |
| [05-image-extraction.md](05-image-extraction.md) | 一级图片提取 | 如何一次调用拿到 bbox + 原始字节？为何跳过 decode+re-encode？ |
| [06-parallel-and-batch.md](06-parallel-and-batch.md) | rayon 并行与批量提取 | fz_context 不是线程安全的，如何并行？批量如何省 FFI？ |
| [07-performance-analysis.md](07-performance-analysis.md) | 性能分析 | 27x 链接 / 2x 文本 / 6x JPEG——加速比的根因分类 |
| [08-comparison-with-pdf-oxide.md](08-comparison-with-pdf-oxide.md) | pdf_oxide 对比 | ritz vs pdf_oxide 谁更快？MIT vs AGPL？ |
| [09-acceleration-opportunities.md](09-acceleration-opportunities.md) | 加速机会审计 | plan_v1 之后还能再快吗？实现层的低垂果实 |
| [10-tier1-tier2-results.md](10-tier1-tier2-results.md) | Tier 1+2 优化结果 | 哪些优化真有效、哪些是噪声、哪些有风险 |

## 快速导航

**我想理解……**
- **我刚接手项目** → [00-onboarding-guide](00-onboarding-guide.md) ⭐ 必读
- 整体设计 → [01-architecture](01-architecture.md)
- 为什么 Rust 调 C 库会 UB → [02-longjmp-isolation](02-longjmp-isolation.md)
- 27x 加速怎么来的 → [03-c-layer-flattening](03-c-layer-flattening.md)
- 裸指针如何安全 → [04-memory-safety](04-memory-safety.md)
- 图片提取为什么返回原始字节 → [05-image-extraction](05-image-extraction.md)
- 多文档并行怎么做到的 → [06-parallel-and-batch](06-parallel-and-batch.md)
- 哪些场景能 10x，哪些不能 → [07-performance-analysis](07-performance-analysis.md)
- 跟 pdf_oxide 比怎么样 → [08-comparison-with-pdf-oxide](08-comparison-with-pdf-oxide.md)
- 还能再加速吗 → [09-acceleration-opportunities](09-acceleration-opportunities.md)
- Tier 1/2 优化做完了吗，效果如何 → [10-tier1-tier2-results](10-tier1-tier2-results.md)

## 相关文件

- 性能基准报告：[../benchmarks/benchmark_report.md](../benchmarks/benchmark_report.md)
- 用户文档（README）：[../README.md](../README.md)
- 交互演示：[../docs/demo.ipynb](../docs/demo.ipynb)
