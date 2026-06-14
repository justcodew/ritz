//! PyDocument — Python 侧 Document 类。

use crate::page::PyPage;
use mupdf_sys::{
    self, fz_context, fz_document,
    mupdf_ext_extract_pages_text, mupdf_safe_authenticate_password, mupdf_safe_count_pages,
    mupdf_safe_drop_document, mupdf_safe_drop_context, mupdf_safe_load_outline,
    mupdf_safe_load_page, mupdf_safe_lookup_metadata, mupdf_safe_needs_password,
    mupdf_safe_new_context, mupdf_safe_open_document,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::ptr;
use std::sync::OnceLock;

/// Document(filename) — 打开 PDF 文档。
///
/// 加速缓存（plan_v1 后 Tier 1 优化）：
/// - `metadata_cache`：metadata @property 首次访问后缓存 Py<PyAny>，避免每次访问都走 9 次 FFI
#[pyclass(name = "Document")]
pub struct PyDocument {
    /// 原始 ctx 指针。Drop 顺序：doc 先 drop，再 drop ctx。
    pub(crate) ctx: *mut fz_context,
    pub(crate) raw: *mut fz_document,
    metadata_cache: OnceLock<Py<PyAny>>,
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

        Ok(PyDocument { ctx, raw, metadata_cache: OnceLock::new() })
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

    /// metadata — 与 PyMuPDF 兼容的 @property，返回完整元数据 dict。
    ///
    /// 字段：format, title, author, subject, keywords, creator, producer,
    ///       creationDate, modDate, encryption
    /// 缺失字段值为 None。
    ///
    /// 缓存：首次访问后 memoize（OnceLock），后续访问直接返回缓存 dict。
    #[getter]
    fn metadata(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(cached) = self.metadata_cache.get() {
            return Ok(cached.clone_ref(py));
        }
        let d = PyDict::new(py);
        // PyMuPDF 标准字段 → MuPDF metadata key
        // info:* 取 info dict 的字段；format/encryption 直接走 lookup。
        let keys: &[(&str, &str)] = &[
            ("format", "format"),
            ("title", "info:Title"),
            ("author", "info:Author"),
            ("subject", "info:Subject"),
            ("keywords", "info:Keywords"),
            ("creator", "info:Creator"),
            ("producer", "info:Producer"),
            ("creationDate", "info:CreationDate"),
            ("modDate", "info:ModDate"),
        ];
        for (field, mupdf_key) in keys {
            let val = self.lookup_metadata(mupdf_key)?;
            d.set_item(field, val)?;
        }
        // encryption：基于 is_encrypted 给字符串或 None
        let enc = if self.is_encrypted() {
            Some("yes".to_string())
        } else {
            None
        };
        d.set_item("encryption", enc)?;
        let bound = d.into_any().unbind();
        // get_or_init 防并发：若另一线程先填了，使用其结果（语义等价）。
        let stored = self.metadata_cache.get_or_init(|| bound.clone_ref(py));
        Ok(stored.clone_ref(py))
    }

    /// lookup_metadata(key) -> str | None
    ///
    /// 单字段查询，对应 PyMuPDF `doc.metadata.get(key)` 的底层调用。
    /// 常用 key: "format", "info:Title", "info:Author" 等。
    fn lookup_metadata(&self, key: &str) -> PyResult<Option<String>> {
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

    /// get_toc() -> list[[level, title, page]] | None
    ///
    /// 返回文档大纲（目录）。PyMuPDF `get_toc(simple=True)` 兼容格式。
    /// 每条为 `[level, title, page]`：
    /// - `level`：1-based 层级（顶层 = 1）
    /// - `title`：UTF-8 字符串
    /// - `page`：1-based 页码；外部链接或无目标为 -1
    ///
    /// 文档无大纲时返回 None（与 PyMuPDF 一致）。
    fn get_toc(&self, py: Python<'_>) -> PyResult<Option<Vec<Py<PyAny>>>> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut total_len: usize = 0;
        let mut n: c_int = 0;
        let rc = unsafe { mupdf_safe_load_outline(self.ctx, self.raw, &mut ptr, &mut total_len, &mut n) };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        if n == 0 || ptr.is_null() || total_len == 0 {
            return Ok(None);
        }

        let result = (|| -> PyResult<Option<Vec<Py<PyAny>>>> {
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, total_len) };
            let mut out = Vec::with_capacity(n as usize);
            let mut off = 0usize;
            for _ in 0..n {
                if off + 16 > bytes.len() {
                    return Err(PyRuntimeError::new_err(
                        "get_toc: buffer underrun reading entry header",
                    ));
                }
                let level = i32::from_ne_bytes(bytes[off..off + 4].try_into().unwrap());
                let page = i32::from_ne_bytes(bytes[off + 4..off + 8].try_into().unwrap());
                let _flags = i32::from_ne_bytes(bytes[off + 8..off + 12].try_into().unwrap());
                let title_len = i32::from_ne_bytes(bytes[off + 12..off + 16].try_into().unwrap()) as usize;
                off += 16;
                if off + title_len > bytes.len() {
                    return Err(PyRuntimeError::new_err(
                        "get_toc: buffer underrun reading title",
                    ));
                }
                let title = std::str::from_utf8(&bytes[off..off + title_len])
                    .map_err(|e| PyRuntimeError::new_err(format!("get_toc: title not utf8: {}", e)))?
                    .to_owned();
                off += title_len;

                let tuple = (level, title, page)
                    .into_pyobject(py)?
                    .into_any()
                    .unbind();
                out.push(tuple);
            }
            Ok(Some(out))
        })();

        unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        result
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
        let page = PyPage::new(doc_handle, ctx, raw_doc, raw);
        Py::new(py, page)
    }

    /// doc[i] — 等价于 doc.load_page(i)。
    fn __getitem__(slf: &Bound<'_, Self>, index: usize) -> PyResult<Py<PyPage>> {
        Self::load_page(slf, index)
    }

    /// get_text_batch() -> list[str]
    ///
    /// 一次性提取所有页的纯文本，避免 N 次 Python↔Rust 往返。
    /// 内部走 mupdf_ext_extract_pages_text，单次 C 调用遍历全文档。
    /// 等价于 [doc[i].get_text("text") for i in range(len(doc))]，但更快。
    ///
    /// GIL 释放：批量提取期间不持 GIL，允许其他 Python 线程并行。
    #[pyo3(signature = ())]
    fn get_text_batch(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        // 裸指针经由 usize 传入 closure（py.detach 要求 Send）。
        let ctx_addr = self.ctx as usize;
        let raw_addr = self.raw as usize;
        let extracted: (c_int, usize, usize, usize, usize, c_int) = py.detach(move || {
            let ctx = ctx_addr as *mut fz_context;
            let doc = raw_addr as *mut fz_document;
            let mut text_ptr: *mut std::os::raw::c_char = ptr::null_mut();
            let mut text_len: usize = 0;
            let mut offsets_ptr: *mut usize = ptr::null_mut();
            let mut page_count: c_int = 0;
            let rc = unsafe {
                mupdf_ext_extract_pages_text(
                    ctx,
                    doc,
                    &mut text_ptr,
                    &mut text_len,
                    &mut offsets_ptr,
                    &mut page_count,
                )
            };
            (rc, text_ptr as usize, text_len, offsets_ptr as usize, 0, page_count)
        });
        let (rc, text_ptr_addr, text_len, offsets_ptr_addr, _, page_count) = extracted;
        let text_ptr = text_ptr_addr as *mut std::os::raw::c_char;
        let offsets_ptr = offsets_ptr_addr as *mut usize;

        if rc != 0 {
            return Err(Self::last_error_pub());
        }

        let result = (|| -> PyResult<Vec<Py<PyAny>>> {
            if page_count == 0 || text_ptr.is_null() || offsets_ptr.is_null() {
                return Ok(Vec::new());
            }
            let bytes = unsafe {
                std::slice::from_raw_parts(text_ptr as *const u8, text_len)
            };
            let offsets = unsafe {
                std::slice::from_raw_parts(offsets_ptr, page_count as usize + 1)
            };

            let mut out = Vec::with_capacity(page_count as usize);
            for i in 0..page_count as usize {
                let start = offsets[i];
                let end = offsets[i + 1];
                let slice = if start <= end && end <= bytes.len() {
                    &bytes[start..end]
                } else {
                    &bytes[start..]
                };
                // Tier 2 #8：fast path 直走 PyString，省 String::from_utf8_lossy + into_owned
                let py_str = match std::str::from_utf8(slice) {
                    Ok(s) => pyo3::types::PyString::new(py, s).into_any().unbind(),
                    Err(_) => {
                        let lossy = String::from_utf8_lossy(slice);
                        pyo3::types::PyString::new(py, &lossy).into_any().unbind()
                    }
                };
                out.push(py_str);
            }
            Ok(out)
        })();

        unsafe {
            if !text_ptr.is_null() {
                mupdf_sys::mupdf_free(text_ptr as *mut _);
            }
            if !offsets_ptr.is_null() {
                mupdf_sys::mupdf_free(offsets_ptr as *mut _);
            }
        }
        result
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
