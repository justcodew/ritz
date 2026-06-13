//! PyPage — Python 侧 Page 类。

use crate::document::PyDocument;
use crate::pixmap::PyPixmap;
use mupdf_sys::{
    self, fz_context, fz_page, mupdf_safe_bound_page, mupdf_safe_drop_page,
    mupdf_safe_drop_stext_page, mupdf_safe_new_stext_page, mupdf_safe_render_pixmap,
    mupdf_safe_stext_to_html, mupdf_safe_stext_to_text, mupdf_safe_stext_to_xml, fz_pixmap,
    fz_stext_page,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::os::raw::{c_char, c_int};
use std::ptr;

/// Page — 单页（绑定到一个 Document 上）。
///
/// 安全性：Page 持有 `Py<PyDocument>` 引用计数，确保 Document 不会先于 Page 被 GC。
/// Drop 顺序：raw_page 先 drop（借用 ctx），然后 _doc 释放引用（可能触发 Document GC）。
#[pyclass(name = "Page")]
pub struct PyPage {
    pub(crate) ctx: *mut fz_context,
    pub(crate) raw: *mut fz_page,
    /// 持有父 Document 的引用，确保 Document 存活。
    #[allow(dead_code)]
    pub(crate) doc_handle: Py<PyDocument>,
}

impl PyPage {
    pub(crate) fn new(doc_handle: Py<PyDocument>, ctx: *mut fz_context, raw: *mut fz_page) -> Self {
        PyPage {
            ctx,
            raw,
            doc_handle,
        }
    }
}

#[pymethods]
impl PyPage {
    /// 页面矩形（x0, y0, x1, y1）。原点在左下。
    #[getter]
    fn rect(&self) -> (f32, f32, f32, f32) {
        let r = unsafe { mupdf_safe_bound_page(self.ctx, self.raw) };
        (r.x0, r.y0, r.x1, r.y1)
    }

    /// 页面宽度（pt）。
    #[getter]
    fn width(&self) -> f32 {
        let r = unsafe { mupdf_safe_bound_page(self.ctx, self.raw) };
        r.x1 - r.x0
    }

    /// 页面高度（pt）。
    #[getter]
    fn height(&self) -> f32 {
        let r = unsafe { mupdf_safe_bound_page(self.ctx, self.raw) };
        r.y1 - r.y0
    }

    /// get_text(mode="text") -> str
    ///
    /// 支持模式：text / html / xml（PyMuPDF 还有 words/blocks/rawdict/json 等，
    /// 阶段三逐步补充）。
    #[pyo3(signature = (mode = "text"))]
    fn get_text(&self, mode: &str) -> PyResult<String> {
        let mut stpage: *mut fz_stext_page = ptr::null_mut();
        let rc = unsafe { mupdf_safe_new_stext_page(self.ctx, self.raw, &mut stpage) };
        if rc != 0 || stpage.is_null() {
            return Err(PyDocument::last_error_pub());
        }

        let result = self.text_to_string(stpage, mode);
        unsafe { mupdf_safe_drop_stext_page(self.ctx, stpage) };
        result
    }

    /// get_pixmap(matrix=None, dpi=None, alpha=False) -> Pixmap
    ///
    /// matrix: 6 元组 (a,b,c,d,e,f)，目前只使用 a/d 作为缩放因子（保持简单）。
    /// dpi: 优先级高于 matrix，若设置则 zoom = dpi/72。
    /// 都不传时 zoom=1.0（72 DPI）。
    #[pyo3(signature = (matrix = None, dpi = None, alpha = false))]
    fn get_pixmap(
        &self,
        py: Python<'_>,
        matrix: Option<(f32, f32, f32, f32, f32, f32)>,
        dpi: Option<f32>,
        alpha: bool,
    ) -> PyResult<PyPixmap> {
        let zoom = if let Some(d) = dpi {
            d / 72.0
        } else if let Some(m) = matrix {
            // 仅取 a、d 作为水平/垂直缩放，取较大者作为整体 zoom。
            // 完整 matrix 变换留到阶段三（需要新增 fz_pre_scale 等绑定）。
            m.0.max(m.3)
        } else {
            1.0
        };

        let mut pix: *mut fz_pixmap = ptr::null_mut();
        let rc = unsafe {
            mupdf_safe_render_pixmap(self.ctx, self.raw, zoom, alpha as c_int, &mut pix)
        };
        if rc != 0 || pix.is_null() {
            return Err(PyDocument::last_error_pub());
        }
        // clone_ref 拿到 Py<PyDocument>，让 Pixmap 持有引用，避免 doc 先于 pix 被 GC
        Ok(PyPixmap::new(self.doc_handle.clone_ref(py), self.ctx, pix))
    }

    fn __repr__(&self) -> String {
        format!("<ritz.Page at {:p}>", self.raw)
    }
}

impl PyPage {
    fn text_to_string(&self, stpage: *mut fz_stext_page, mode: &str) -> PyResult<String> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut len: usize = 0;
        let rc = unsafe {
            match mode {
                "text" => mupdf_safe_stext_to_text(self.ctx, stpage, &mut ptr, &mut len),
                "html" => mupdf_safe_stext_to_html(self.ctx, stpage, &mut ptr, &mut len),
                "xml" => mupdf_safe_stext_to_xml(self.ctx, stpage, &mut ptr, &mut len),
                other => {
                    return Err(PyRuntimeError::new_err(format!(
                        "unsupported text mode: '{}' (supported: text, html, xml)",
                        other
                    )));
                }
            }
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        let s = if ptr.is_null() || len == 0 {
            String::new()
        } else {
            unsafe {
                let slice = std::slice::from_raw_parts(ptr as *const u8, len);
                String::from_utf8_lossy(slice).into_owned()
            }
        };
        if !ptr.is_null() {
            unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        }
        Ok(s)
    }
}

impl Drop for PyPage {
    fn drop(&mut self) {
        unsafe {
            if !self.raw.is_null() {
                mupdf_safe_drop_page(self.ctx, self.raw);
                self.raw = ptr::null_mut();
            }
        }
        // self.doc_handle 自动 drop（释放对 Document 的引用）
    }
}

unsafe impl Send for PyPage {}
unsafe impl Sync for PyPage {}
