//! PyDocument — Python 侧 Document 类。

use crate::page::PyPage;
use mupdf_sys::{
    self, fz_context, fz_document,
    mupdf_safe_authenticate_password, mupdf_safe_count_pages, mupdf_safe_drop_document,
    mupdf_safe_drop_context, mupdf_safe_load_page, mupdf_safe_lookup_metadata,
    mupdf_safe_needs_password, mupdf_safe_new_context, mupdf_safe_open_document,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::ffi::CString;
use std::os::raw::c_int;
use std::ptr;

/// Document(filename) — 打开 PDF 文档。
#[pyclass(name = "Document")]
pub struct PyDocument {
    /// 原始 ctx 指针。Drop 顺序：doc 先 drop，再 drop ctx。
    pub(crate) ctx: *mut fz_context,
    pub(crate) raw: *mut fz_document,
}

impl PyDocument {
    /// 内部 helper：从线程局部错误缓冲区构造 Python 异常。
    pub fn last_error_pub() -> PyErr {
        let msg = unsafe {
            let p = mupdf_sys::mupdf_last_error();
            if p.is_null() || *p == 0 {
                "unknown error".to_string()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        PyRuntimeError::new_err(msg)
    }
}

#[pymethods]
impl PyDocument {
    #[new]
    fn new(filename: &str) -> PyResult<Self> {
        let c_path = CString::new(filename)
            .map_err(|_| PyRuntimeError::new_err("invalid filename (contains NUL)"))?;

        let ctx = unsafe { mupdf_safe_new_context() };
        if ctx.is_null() {
            return Err(Self::last_error_pub());
        }

        let raw = unsafe { mupdf_safe_open_document(ctx, c_path.as_ptr()) };
        if raw.is_null() {
            unsafe { mupdf_safe_drop_context(ctx) };
            return Err(Self::last_error_pub());
        }

        Ok(PyDocument { ctx, raw })
    }

    /// 页数。
    #[getter]
    fn page_count(&self) -> PyResult<usize> {
        let n = unsafe { mupdf_safe_count_pages(self.ctx, self.raw) };
        if n < 0 {
            return Err(Self::last_error_pub());
        }
        Ok(n as usize)
    }

    /// 兼容别名：PyMuPDF 用 len(doc) 取页数。
    fn __len__(&self) -> PyResult<usize> {
        self.page_count()
    }

    /// 是否加密。
    #[getter]
    fn is_encrypted(&self) -> bool {
        unsafe { mupdf_safe_needs_password(self.ctx, self.raw) != 0 }
    }

    /// needs_password() — 与 PyMuPDF 一致。
    fn needs_password(&self) -> bool {
        self.is_encrypted()
    }

    /// authenticate(password) -> bool。成功解锁返回 True。
    fn authenticate(&self, password: &str) -> PyResult<bool> {
        let c_pw = CString::new(password).unwrap_or_default();
        let ok = unsafe {
            mupdf_safe_authenticate_password(self.ctx, self.raw, c_pw.as_ptr())
        };
        Ok(ok != 0)
    }

    /// metadata(key) -> str | None。
    /// 常用 key: "format", "info:Title", "info:Author" 等。
    fn metadata(&self, key: &str) -> PyResult<Option<String>> {
        let c_key = CString::new(key).unwrap_or_default();
        let mut buf = [0u8; 512];
        let n = unsafe {
            mupdf_safe_lookup_metadata(
                self.ctx,
                self.raw,
                c_key.as_ptr(),
                buf.as_mut_ptr() as *mut i8,
                buf.len() as c_int,
            )
        };
        if n <= 0 {
            return Ok(None);
        }
        // MuPDF 在某些情况下返回带尾部 '\0' 的字符串，去掉。
        let mut end = n as usize;
        while end > 0 && buf[end - 1] == 0 {
            end -= 1;
        }
        let s = String::from_utf8_lossy(&buf[..end]).into_owned();
        Ok(Some(s))
    }

    /// load_page(index) -> Page。
    /// index 从 0 开始，与 PyMuPDF 一致。
    fn load_page(slf: &Bound<'_, Self>, index: usize) -> PyResult<Py<PyPage>> {
        let py = slf.py();
        let doc_handle = slf.clone().unbind(); // Py<PyDocument>，引用计数 +1
        let inner = slf.borrow();
        let ctx = inner.ctx;
        let raw_doc = inner.raw;
        drop(inner);

        let n = unsafe { mupdf_safe_count_pages(ctx, raw_doc) };
        if n < 0 {
            return Err(Self::last_error_pub());
        }
        if index as i32 >= n {
            return Err(PyRuntimeError::new_err(format!(
                "page index {} out of range (page_count={})",
                index, n
            )));
        }
        let raw = unsafe { mupdf_safe_load_page(ctx, raw_doc, index as c_int) };
        if raw.is_null() {
            return Err(Self::last_error_pub());
        }
        let page = PyPage::new(doc_handle, ctx, raw);
        Py::new(py, page)
    }

    /// doc[i] — 等价于 doc.load_page(i)。
    fn __getitem__(slf: &Bound<'_, Self>, index: usize) -> PyResult<Py<PyPage>> {
        Self::load_page(slf, index)
    }

    fn __repr__(&self) -> String {
        format!("<ritz.Document page_count=? at {:p}>", self.raw)
    }
}

impl Drop for PyDocument {
    fn drop(&mut self) {
        unsafe {
            if !self.raw.is_null() {
                mupdf_safe_drop_document(self.ctx, self.raw);
                self.raw = ptr::null_mut();
            }
            if !self.ctx.is_null() {
                mupdf_safe_drop_context(self.ctx);
                self.ctx = ptr::null_mut();
            }
        }
    }
}

// ctx 内部对单线程已安全（默认无锁），多线程场景在阶段四加上锁。
// PyO3 0.29 要求 Send + Sync 才能跨 Python 线程持有。
unsafe impl Send for PyDocument {}
unsafe impl Sync for PyDocument {}
