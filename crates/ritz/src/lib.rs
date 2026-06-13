//! ritz — fitz, reforged in Rust.
//!
//! 高层 Rust API + PyO3 Python 绑定。
//! 阶段一为桩，阶段二起填充 PyO3 类和 get_text 等方法。

pub use mupdf;

/// 占位：阶段二将替换为 #[pymodule] ritz。
pub fn version() -> &'static str {
    "0.1.0-phase1"
}
