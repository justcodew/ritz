//! ritz PyO3 绑定：PyMuPDF 兼容 API。
//!
//! 暴露给 Python 的模块名是 `_ritz`（与 PyMuPDF 的 `_fitz` 对应）。
//! Python 侧用 `ritz/__init__.py` re-export。

use pyo3::prelude::*;

mod batch;
mod document;
mod page;
mod pixmap;
pub mod value;

pub use batch::process_documents;
pub use document::PyDocument;
pub use page::PyPage;
pub use pixmap::PyPixmap;
pub use value::{PdfValue, PyPdfValue};

#[pymodule]
fn _ritz(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<PyDocument>()?;
    m.add_class::<PyPage>()?;
    m.add_class::<PyPixmap>()?;
    m.add_class::<PyPdfValue>()?;
    m.add_function(wrap_pyfunction!(process_documents, m)?)?;
    Ok(())
}
