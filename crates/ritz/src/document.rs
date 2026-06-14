//! PyDocument — Python 侧 Document 类。

use crate::page::PyPage;
use mupdf_sys::{
    self, fz_context, fz_document,
    mupdf_ext_extract_pages_text, mupdf_safe_authenticate_password, mupdf_safe_count_pages,
    mupdf_safe_copy_page, mupdf_safe_delete_page, mupdf_safe_delete_page_range,
    mupdf_safe_drop_document, mupdf_safe_drop_context, mupdf_safe_insert_pdf,
    mupdf_safe_load_outline, mupdf_safe_load_page, mupdf_safe_lookup_metadata,
    mupdf_safe_move_page, mupdf_safe_needs_password, mupdf_safe_new_blank_page,
    mupdf_safe_new_context, mupdf_safe_open_document, mupdf_safe_resolve_names,
    mupdf_safe_save_document, mupdf_safe_set_toc,
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
    /// 源文件路径。用于 insert_pdf 跨 ctx 复用（在 dst 的 ctx 里重新打开）。
    pub(crate) filename: String,
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

        Ok(PyDocument { ctx, raw, filename: filename.to_string(), metadata_cache: OnceLock::new() })
    }

    /// 源文件路径（只读属性）。
    /// insert_pdf 会用此路径在目标文档的 ctx 内重开源 PDF，避免跨 ctx UB。
    #[getter]
    fn filename(&self) -> String {
        self.filename.clone()
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

    /// save(path) -> None。
    /// 将文档保存到指定路径（PDF 格式）。
    /// 支持增量保存和完整重写，由 MuPDF 内部决定。
    fn save(&self, path: &str) -> PyResult<()> {
        let c_path = CString::new(path)
            .map_err(|_| PyRuntimeError::new_err("path contains NUL byte"))?;
        let rc = unsafe { mupdf_safe_save_document(self.ctx, self.raw, c_path.as_ptr()) };
        if rc != 0 {
            return Err(Self::last_error_pub());
        }
        Ok(())
    }

    /// set_toc(toc) -> None
    ///
    /// 设置文档大纲（目录）。格式与 get_toc() 返回值一致：
    /// 每条为 (level, title, page)，其中 level 从 1 开始，page 从 1 开始。
    /// 传空 list 清空大纲。
    ///
    /// 仅对 PDF 文档有效。
    fn set_toc(&self, toc: Vec<(i32, String, i32)>) -> PyResult<()> {
        if toc.is_empty() {
            let rc = unsafe {
                mupdf_safe_set_toc(self.ctx, self.raw, ptr::null(), ptr::null(), ptr::null(), 0)
            };
            if rc != 0 {
                return Err(Self::last_error_pub());
            }
            return Ok(());
        }

        let levels: Vec<c_int> = toc.iter().map(|(l, _, _)| *l as c_int).collect();
        let pages: Vec<c_int> = toc.iter().map(|(_, _, p)| *p as c_int).collect();
        let c_titles: Vec<CString> = toc.iter()
            .map(|(_, t, _)| CString::new(t.as_str()).unwrap_or_default())
            .collect();
        let title_ptrs: Vec<*const c_char> = c_titles.iter().map(|c| c.as_ptr()).collect();

        let rc = unsafe {
            mupdf_safe_set_toc(
                self.ctx, self.raw,
                levels.as_ptr(), pages.as_ptr(),
                title_ptrs.as_ptr(), toc.len() as c_int,
            )
        };
        if rc != 0 {
            return Err(Self::last_error_pub());
        }
        Ok(())
    }

    /// resolve_names() -> dict[str, (page, x, y)]
    ///
    /// 返回 PDF 所有命名目标（named destinations）的映射。
    /// 每个 key 是目标名称字符串，value 是 (page, x, y) 元组：
    /// - page: 0-based 页码
    /// - x, y: 目标坐标（PDF 用户空间）
    ///
    /// 文档无命名目标时返回空 dict。
    fn resolve_names(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut total_len: usize = 0;
        let mut n: c_int = 0;
        let rc = unsafe {
            mupdf_safe_resolve_names(self.ctx, self.raw, &mut ptr, &mut total_len, &mut n)
        };
        if rc != 0 {
            return Err(Self::last_error_pub());
        }

        let result = (|| -> PyResult<Py<PyAny>> {
            let dict = PyDict::new(py);
            if n == 0 || ptr.is_null() || total_len == 0 {
                return Ok(dict.into_any().unbind());
            }

            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, total_len) };
            let mut off = 0usize;
            for _ in 0..n {
                if off + 4 > bytes.len() {
                    return Err(PyRuntimeError::new_err("resolve_names: buffer underrun"));
                }
                let name_len = i32::from_ne_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
                off += 4;
                if off + name_len + 12 > bytes.len() {
                    return Err(PyRuntimeError::new_err("resolve_names: buffer underrun"));
                }
                let name = std::str::from_utf8(&bytes[off..off + name_len])
                    .map_err(|e| PyRuntimeError::new_err(format!("resolve_names: name not utf8: {}", e)))?;
                off += name_len;
                let page = i32::from_ne_bytes(bytes[off..off + 4].try_into().unwrap());
                let x = f32::from_ne_bytes(bytes[off + 4..off + 8].try_into().unwrap());
                let y = f32::from_ne_bytes(bytes[off + 8..off + 12].try_into().unwrap());
                off += 12;

                let val = (page, x, y).into_pyobject(py)?.into_any().unbind();
                dict.set_item(name, val)?;
            }
            Ok(dict.into_any().unbind())
        })();

        unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        result
    }

    /// new_page(pno=-1, width=595, height=842, rotate=0) -> Page
    ///
    /// 插入一张空白页并返回新建的 Page 对象（PyMuPDF 兼容）。
    /// - pno：插入位置（0-based）；-1 表示末尾追加
    /// - width/height：页面尺寸（PDF 用户空间单位，默认 A4）
    /// - rotate：0/90/180/270
    ///
    /// 注意：返回的 Page 对象在后续 insert/delete/move/copy 操作后**位置可能失效**。
    /// 建议先做所有页面编辑，最后再 load_page 取页读取。
    #[pyo3(signature = (pno=-1, width=595.0, height=842.0, rotate=0))]
    fn new_page(
        slf: &Bound<'_, Self>,
        pno: i32,
        width: f32,
        height: f32,
        rotate: i32,
    ) -> PyResult<Py<PyPage>> {
        {
            let inner = slf.borrow();
            let rc = unsafe {
                mupdf_safe_new_blank_page(inner.ctx, inner.raw, pno as c_int, width, height, rotate as c_int)
            };
            if rc != 0 {
                return Err(Self::last_error_pub());
            }
        }
        // 计算新页落在的位置：合法 pno 落在 pno，否则追加到末尾（len-1）
        let n = {
            let inner = slf.borrow();
            unsafe { mupdf_safe_count_pages(inner.ctx, inner.raw) }
        };
        if n < 0 {
            return Err(Self::last_error_pub());
        }
        let target = if pno < 0 || pno >= n { (n - 1) as usize } else { pno as usize };
        Self::load_page(slf, target)
    }

    /// delete_page(pno=-1) -> None
    ///
    /// 删除指定页（0-based）。pno=-1 删除最后一页。
    fn delete_page(&self, pno: i32) -> PyResult<()> {
        let rc = unsafe { mupdf_safe_delete_page(self.ctx, self.raw, pno as c_int) };
        if rc != 0 {
            return Err(Self::last_error_pub());
        }
        Ok(())
    }

    /// delete_pages(from_page=-1, to_page=-1) -> None
    ///
    /// 删除闭区间 [from_page, to_page] 内的所有页（PyMuPDF 兼容语义）。
    /// -1 表示边界：from_page=-1 → 0；to_page=-1 → 末页。
    /// 内部转成 MuPDF 半开区间 [start, end+1) 调用 C 层。
    #[pyo3(signature = (from_page=-1, to_page=-1))]
    fn delete_pages(&self, from_page: i32, to_page: i32) -> PyResult<()> {
        // 先取总页数，把 -1 转成实际值，并构造半开区间
        let n = unsafe { mupdf_safe_count_pages(self.ctx, self.raw) };
        if n < 0 {
            return Err(Self::last_error_pub());
        }
        let start = if from_page < 0 { 0 } else { from_page };
        let to = if to_page < 0 { n - 1 } else { to_page };
        if to < start {
            return Err(PyRuntimeError::new_err(format!(
                "delete_pages: invalid range [{}, {}]", from_page, to_page
            )));
        }
        // MuPDF pdf_delete_page_range 是半开 [start, end)，传入 to+1
        let rc = unsafe {
            mupdf_safe_delete_page_range(self.ctx, self.raw, start as c_int, (to + 1) as c_int)
        };
        if rc != 0 {
            return Err(Self::last_error_pub());
        }
        Ok(())
    }

    /// move_page(src, dst) -> None
    ///
    /// 把 src 位置的页移动到 dst 位置（0-based）。
    fn move_page(&self, src: i32, dst: i32) -> PyResult<()> {
        let rc = unsafe { mupdf_safe_move_page(self.ctx, self.raw, src as c_int, dst as c_int) };
        if rc != 0 {
            return Err(Self::last_error_pub());
        }
        Ok(())
    }

    /// copy_page(src, dst) -> None
    ///
    /// 在 dst 位置克隆一份 src 页（同文档内）。
    fn copy_page(&self, src: i32, dst: i32) -> PyResult<()> {
        let rc = unsafe { mupdf_safe_copy_page(self.ctx, self.raw, dst as c_int, src as c_int) };
        if rc != 0 {
            return Err(Self::last_error_pub());
        }
        Ok(())
    }

    /// insert_pdf(docsrc, start=0, end=-1, at=-1) -> None
    ///
    /// 把 docsrc 的 [start, end) 页（0-based 半开区间）复制插入到当前文档的 at 位置。
    /// - start=-1 → 0；end=-1 → docsrc 末尾；at=-1 → 当前文档末尾追加。
    ///
    /// 实现：内部用 docsrc 的 filename 在当前文档的 fz_context 里重新打开源文件，
    /// 避免 MuPDF 跨 ctx graft_page 的 UB。因此 docsrc 必须仍指向一个有效可读路径。
    #[pyo3(signature = (docsrc, start=0, end=-1, at=-1))]
    fn insert_pdf(
        slf: &Bound<'_, Self>,
        docsrc: &Bound<'_, PyDocument>,
        start: i32,
        end: i32,
        at: i32,
    ) -> PyResult<()> {
        let src_path = docsrc.borrow().filename.clone();
        let c_path = CString::new(src_path.as_str())
            .map_err(|_| PyRuntimeError::new_err("docsrc filename contains NUL byte"))?;
        let (dst_ctx, dst_raw) = {
            let inner = slf.borrow();
            (inner.ctx, inner.raw)
        };
        let rc = unsafe {
            mupdf_safe_insert_pdf(dst_ctx, dst_raw, at as c_int, c_path.as_ptr(), start as c_int, end as c_int)
        };
        if rc != 0 {
            return Err(Self::last_error_pub());
        }
        Ok(())
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
