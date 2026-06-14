//! PyPixmap — Python 侧 Pixmap 类。

use crate::document::PyDocument;
use mupdf_sys::{
    self, fz_context, fz_pixmap, mupdf_safe_drop_pixmap, mupdf_safe_pixmap_components,
    mupdf_safe_pixmap_height, mupdf_safe_pixmap_samples, mupdf_safe_pixmap_stride,
    mupdf_safe_pixmap_to_png, mupdf_safe_pixmap_width,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::os::raw::c_uchar;
use std::ptr;

/// Pixmap — 像素图。
///
/// 安全性：与 Page 相同，持有 `Py<PyDocument>` 引用以确保 Document 存活，
/// 防止 ctx 在 Pixmap drop 之前被释放（GC 顺序不可控）。
#[pyclass(name = "Pixmap")]
pub struct PyPixmap {
    pub(crate) ctx: *mut fz_context,
    pub(crate) raw: *mut fz_pixmap,
    #[allow(dead_code)]
    pub(crate) doc_handle: Py<PyDocument>,
}

impl PyPixmap {
    pub(crate) fn new(
        doc_handle: Py<PyDocument>,
        ctx: *mut fz_context,
        raw: *mut fz_pixmap,
    ) -> Self {
        PyPixmap {
            ctx,
            raw,
            doc_handle,
        }
    }
}

#[pymethods]
impl PyPixmap {
    #[getter]
    fn width(&self) -> i32 {
        unsafe { mupdf_safe_pixmap_width(self.ctx, self.raw) }
    }

    #[getter]
    fn height(&self) -> i32 {
        unsafe { mupdf_safe_pixmap_height(self.ctx, self.raw) }
    }

    #[getter]
    fn stride(&self) -> i32 {
        unsafe { mupdf_safe_pixmap_stride(self.ctx, self.raw) }
    }

    /// 每个像素的分量数（RGB=3，RGBA=4）。
    #[getter]
    fn components(&self) -> i32 {
        unsafe { mupdf_safe_pixmap_components(self.ctx, self.raw) }
    }

    /// samples —— 直接返回像素字节数据（bytes）。
    #[getter]
    fn samples<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let p = unsafe { mupdf_safe_pixmap_samples(self.ctx, self.raw) };
        if p.is_null() {
            return Ok(PyBytes::new(py, &[]));
        }
        let len = (self.stride() as usize) * (self.height() as usize);
        let bytes = unsafe { std::slice::from_raw_parts(p, len) };
        Ok(PyBytes::new(py, bytes))
    }

    /// tobytes(format="png") -> bytes
    ///
    /// 当前仅支持 png。PyMuPDF 还支持 pam/pgm/ppm 等，阶段三补充。
    ///
    /// 优化（Tier 1 #4）：直接从 C 缓冲构造 PyBytes，省一次 Vec 拷贝。
    #[pyo3(signature = (format = "png"))]
    fn tobytes<'py>(&self, format: &str, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        match format {
            "png" => {
                let mut ptr: *mut c_uchar = ptr::null_mut();
                let mut len: usize = 0;
                let rc = unsafe {
                    mupdf_safe_pixmap_to_png(self.ctx, self.raw, &mut ptr, &mut len)
                };
                if rc != 0 {
                    return Err(PyDocument::last_error_pub());
                }
                let result = (|| -> PyResult<Bound<'py, PyBytes>> {
                    if ptr.is_null() || len == 0 {
                        return Ok(PyBytes::new(py, &[]));
                    }
                    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
                    Ok(PyBytes::new(py, slice))
                })();
                if !ptr.is_null() {
                    unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
                }
                result
            }
            other => Err(PyRuntimeError::new_err(format!(
                "unsupported pixmap format: '{}' (only 'png' supported)",
                other
            ))),
        }
    }

    /// 便捷：保存为 PNG 文件。
    #[pyo3(signature = (filename))]
    fn save_png(&self, filename: &str) -> PyResult<()> {
        let py = unsafe { Python::assume_attached() };
        let bytes = self.tobytes("png", py)?;
        std::fs::write(filename, bytes.as_bytes())
            .map_err(|e| PyRuntimeError::new_err(format!("write {}: {}", filename, e)))
    }

    fn __repr__(&self) -> String {
        format!(
            "<ritz.Pixmap {}x{} at {:p}>",
            self.width(),
            self.height(),
            self.raw
        )
    }
}

impl Drop for PyPixmap {
    fn drop(&mut self) {
        unsafe {
            if !self.raw.is_null() {
                mupdf_safe_drop_pixmap(self.ctx, self.raw);
                self.raw = ptr::null_mut();
            }
        }
    }
}

unsafe impl Send for PyPixmap {}
unsafe impl Sync for PyPixmap {}
