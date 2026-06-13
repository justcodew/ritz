//! MuPDF 安全封装层。
//!
//! 设计原则（借鉴 LiteParse）：
//! - 所有句柄用 RAII 管理（实现 Drop 调用对应 mupdf_safe_drop_*）
//! - 生命周期标记：`Document<'ctx>` 借用 `Context`，编译期防止释放后使用
//! - 错误码转为 `Result<T, MuPdfError>`
//! - 坐标系转换集中在 Page 方法（左下原点 → 左上原点）
//! - 不暴露任何 unsafe 接口

mod context;
mod document;
mod error;
mod geometry;
mod page;
mod pixmap;
mod stext;

pub use context::Context;
pub use document::Document;
pub use error::{MuPdfError, Result};
pub use geometry::{Matrix, Point, Rect};
pub use page::Page;
pub use pixmap::Pixmap;
pub use stext::STextPage;
