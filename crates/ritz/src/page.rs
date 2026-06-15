//! PyPage — Python 侧 Page 类。

use crate::document::PyDocument;
use crate::pixmap::PyPixmap;
use mupdf_sys::{
    self, fz_context, fz_page, mupdf_safe_bound_page, mupdf_safe_bound_page_box,
    mupdf_safe_create_annot, mupdf_safe_delete_annot, mupdf_safe_drop_page,
    mupdf_safe_drop_stext_page, mupdf_safe_get_annotations, mupdf_safe_get_images,
    mupdf_safe_set_annot_rect,
    mupdf_safe_load_links, mupdf_safe_new_stext_page, mupdf_safe_page_rotation,
    mupdf_safe_render_pixmap, mupdf_safe_search_stext_page, mupdf_safe_stext_to_blocks,
    mupdf_safe_stext_to_dict, mupdf_safe_stext_to_json, mupdf_safe_stext_to_words,
    fz_pixmap, fz_stext_page,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyString};
use std::os::raw::{c_char, c_int};
use std::ptr;
use std::sync::OnceLock;

/// Page — 单页（绑定到一个 Document 上）。
///
/// 安全性：Page 持有 `Py<PyDocument>` 引用计数，确保 Document 不会先于 Page 被 GC。
/// Drop 顺序：raw_page 先 drop（借用 ctx），然后 doc_handle 释放引用（可能触发 Document GC）。
///
/// 加速缓存（plan_v1 后 Tier 1 优化 + Phase 6 stext 缓存）：
/// - `raw_doc`：load_page 时一次性取，省去每次方法调用时 `doc_handle.bind(py).borrow()` 的开销
/// - `rect_cache`：rect/width/height 三个 getter 共享一次 `fz_bound_page` 调用结果
/// - `stext_cache`：fz_stext_page 在首次 get_text/search_for 时构建，复用到 annot 写操作。
///   Mutex 而非 OnceLock 因为 annot 写操作要能 invalidate（OnceLock 无法 unset）。
///   锁在 PyO3 调用期间持有，但 Python 单线程内无竞争，开销 ~0。
#[pyclass(name = "Page")]
pub struct PyPage {
    pub(crate) ctx: *mut fz_context,
    pub(crate) raw: *mut fz_page,
    /// 父 doc 的裸指针。与 `doc_handle.raw` 同生命周期（doc_handle 持引用保活）。
    pub(crate) raw_doc: *mut mupdf_sys::fz_document,
    /// 持有父 Document 的引用，确保 Document 存活。
    #[allow(dead_code)]
    pub(crate) doc_handle: Py<PyDocument>,
    /// `fz_bound_page` 结果缓存。页面是不可变的，可安全缓存。
    rect_cache: OnceLock<(f32, f32, f32, f32)>,
    /// `fz_stext_page` 缓存。annot 写操作必须 invalidate（stext 包含 annot 文本）。
    stext_cache: std::sync::Mutex<Option<*mut fz_stext_page>>,
}

impl PyPage {
    pub(crate) fn new(
        doc_handle: Py<PyDocument>,
        ctx: *mut fz_context,
        raw_doc: *mut mupdf_sys::fz_document,
        raw: *mut fz_page,
    ) -> Self {
        PyPage {
            ctx,
            raw,
            raw_doc,
            doc_handle,
            rect_cache: OnceLock::new(),
            stext_cache: std::sync::Mutex::new(None),
        }
    }

    /// 返回缓存的 rect，首次访问时计算。
    fn cached_rect(&self) -> (f32, f32, f32, f32) {
        *self.rect_cache.get_or_init(|| {
            let r = unsafe { mupdf_safe_bound_page(self.ctx, self.raw) };
            (r.x0, r.y0, r.x1, r.y1)
        })
    }

    /// 返回缓存的 fz_stext_page，首次访问时构建。annot 写操作调用 invalidate_stext 后会重建。
    fn cached_stext(&self) -> PyResult<*mut fz_stext_page> {
        let mut guard = self.stext_cache.lock().unwrap();
        if let Some(stpage) = *guard {
            return Ok(stpage);
        }
        let mut stpage: *mut fz_stext_page = ptr::null_mut();
        let rc = unsafe { mupdf_safe_new_stext_page(self.ctx, self.raw, &mut stpage) };
        if rc != 0 || stpage.is_null() {
            return Err(PyDocument::last_error_pub());
        }
        *guard = Some(stpage);
        Ok(stpage)
    }

    /// annot 写操作调用：丢掉 stext 缓存。stext 包含 annot 文本，写 annot 后必须重建。
    fn invalidate_stext(&self) {
        if let Some(stpage) = self.stext_cache.lock().unwrap().take() {
            unsafe { mupdf_safe_drop_stext_page(self.ctx, stpage); }
        }
    }
}

#[pymethods]
impl PyPage {
    /// 页面矩形（x0, y0, x1, y1）。原点在左下。
    #[getter]
    fn rect(&self) -> (f32, f32, f32, f32) {
        self.cached_rect()
    }

    /// 页面宽度（pt）。
    #[getter]
    fn width(&self) -> f32 {
        let (x0, _, x1, _) = self.cached_rect();
        x1 - x0
    }

    /// 页面高度（pt）。
    #[getter]
    fn height(&self) -> f32 {
        let (_, y0, _, y1) = self.cached_rect();
        y1 - y0
    }

    /// mediabox（x0, y0, x1, y1）。
    #[getter]
    fn mediabox(&self) -> (f32, f32, f32, f32) {
        let r = unsafe { mupdf_safe_bound_page_box(self.ctx, self.raw, 0) };
        (r.x0, r.y0, r.x1, r.y1)
    }

    /// cropbox（x0, y0, x1, y1）。
    #[getter]
    fn cropbox(&self) -> (f32, f32, f32, f32) {
        let r = unsafe { mupdf_safe_bound_page_box(self.ctx, self.raw, 1) };
        (r.x0, r.y0, r.x1, r.y1)
    }

    /// rotation（0/90/180/270）。仅 PDF 文档有效，非 PDF 返回 0。
    #[getter]
    fn rotation(&self) -> i32 {
        unsafe { mupdf_safe_page_rotation(self.ctx, self.raw_doc, self.raw) as i32 }
    }

    /// get_text(mode="text") -> str | list | dict
    ///
    /// 文本模式（返回 str）：text / html / xml / json
    /// 结构模式（返回 list）：words / blocks
    /// 嵌套模式（返回 dict）：dict
    /// rawdict 将在后续补（含每个 char 的 bbox/origin）。
    #[pyo3(signature = (mode = "text"))]
    fn get_text(&self, py: Python<'_>, mode: &str) -> PyResult<Py<PyAny>> {
        // Phase 6：stext 缓存复用，避免每次 get_text/search_for 重建 fz_stext_page。
        // annot 写操作调用 invalidate_stext 后会重建。
        let stpage = self.cached_stext()?;

        match mode {
            "text" | "html" | "xhtml" | "xml" => self.stext_to_string(py, stpage, mode),
            "json" => self.stext_to_json(py, stpage, 1.0),
            "rawjson" => {
                // rawjson = json.dumps(rawdict)：MuPDF 无原生 rawjson 打印器，
                // 构造 rawdict 结构后用 Python 标准库序列化。
                let rawdict = self.stext_to_dict(py, stpage, true)?;
                let json = py.import("json")?;
                let dumps = json.getattr("dumps")?;
                let s = dumps.call1((rawdict,))?;
                Ok(s.into_any().unbind())
            }
            "words" => self.stext_to_words(py, stpage),
            "blocks" => self.stext_to_blocks(py, stpage),
            "dict" => self.stext_to_dict(py, stpage, false),
            "rawdict" => self.stext_to_dict(py, stpage, true),
            other => Err(PyRuntimeError::new_err(format!(
                "unsupported text mode: '{}' (supported: text, html, xhtml, xml, json, rawjson, words, blocks, dict, rawdict)",
                other
            ))),
        }
    }

    /// get_links() -> list[dict]
    ///
    /// PyMuPDF 兼容格式：每条链接返回字典 {kind, from, to, uri?, is_external}。
    fn get_links(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut total_len: usize = 0;
        let mut n: c_int = 0;
        let rc = unsafe {
            mupdf_safe_load_links(self.ctx, self.raw, &mut ptr, &mut total_len, &mut n)
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        if n == 0 || ptr.is_null() || total_len == 0 {
            return Ok(Vec::new());
        }

        let result = (|| -> PyResult<Vec<Py<PyAny>>> {
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, total_len) };
            let mut out = Vec::with_capacity(n as usize);
            let mut off = 0usize;
            for _ in 0..n {
                if off + 16 > bytes.len() {
                    break;
                }
                let x0 = f32::from_ne_bytes(bytes[off..off + 4].try_into().unwrap());
                let y0 = f32::from_ne_bytes(bytes[off + 4..off + 8].try_into().unwrap());
                let x1 = f32::from_ne_bytes(bytes[off + 8..off + 12].try_into().unwrap());
                let y1 = f32::from_ne_bytes(bytes[off + 12..off + 16].try_into().unwrap());
                off += 16;
                let end = bytes[off..].iter().position(|&b| b == 0).unwrap_or(0);
                let uri = std::str::from_utf8(&bytes[off..off + end])
                    .unwrap_or("")
                    .to_string();
                off += end + 1;

                let dict = PyDict::new(py);
                let is_external = uri.contains(':');
                dict.set_item("kind", if is_external { "uri" } else { "goto" })?;
                dict.set_item("from", (x0, y0, x1, y1))?;
                dict.set_item("to", (x0, y0, x1, y1))?;
                if !uri.is_empty() {
                    dict.set_item("uri", uri.clone())?;
                }
                dict.set_item("is_external", is_external)?;
                out.push(dict.unbind().into_any());
            }
            Ok(out)
        })();

        unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        result
    }

    /// search_for(needle, quads=False) -> list
    ///
    /// 在页面文本中搜索 needle（大小写不敏感）。PyMuPDF 兼容。
    ///
    /// - `quads=False`（默认）：返回每个命中位置的包围矩形
    ///   `list[(x0, y0, x1, y1)]`
    /// - `quads=True`：返回每个命中位置的 4 顶点（顺时针：ul→ur→lr→ll）
    ///   `list[(ul_x, ul_y, ur_x, ur_y, lr_x, lr_y, ll_x, ll_y)]`
    ///
    /// 一次命中跨多行会产生多个 quad/rect。
    #[pyo3(signature = (needle, quads=false))]
    fn search_for(&self, py: Python<'_>, needle: &str, quads: bool) -> PyResult<Vec<Py<PyAny>>> {
        // Phase 6：stext 缓存复用。get_text 后再 search_for 不重建 stext。
        let stpage = self.cached_stext()?;

        let mut quads_ptr: *mut f32 = ptr::null_mut();
        let result = (|| -> PyResult<Vec<Py<PyAny>>> {
            let mut n: c_int = 0;
            let c_needle = match std::ffi::CString::new(needle) {
                Ok(s) => s,
                Err(_) => return Err(PyRuntimeError::new_err("needle contains NUL byte")),
            };
            let rc = unsafe {
                mupdf_safe_search_stext_page(self.ctx, stpage, c_needle.as_ptr(), &mut quads_ptr, &mut n)
            };
            if rc != 0 {
                return Err(PyDocument::last_error_pub());
            }
            if n == 0 || quads_ptr.is_null() {
                return Ok(Vec::new());
            }

            let total_floats = (n as usize) * 8;
            let slice = unsafe { std::slice::from_raw_parts(quads_ptr, total_floats) };
            let mut out = Vec::with_capacity(n as usize);
            for i in 0..n as usize {
                let base = i * 8;
                // fz_quad 内存布局：ul, ur, ll, lr（每点 2 floats）
                let ul_x = slice[base];
                let ul_y = slice[base + 1];
                let ur_x = slice[base + 2];
                let ur_y = slice[base + 3];
                let ll_x = slice[base + 4];
                let ll_y = slice[base + 5];
                let lr_x = slice[base + 6];
                let lr_y = slice[base + 7];

                if quads {
                    // PyMuPDF Quad 顺序：ul→ur→lr→ll（顺时针）
                    out.push((
                        ul_x, ul_y, ur_x, ur_y, lr_x, lr_y, ll_x, ll_y,
                    ).into_pyobject(py)?.into_any().unbind());
                } else {
                    // Rect = 包围盒：(min_x, min_y, max_x, max_y)
                    let x0 = ul_x.min(ll_x);
                    let y0 = ul_y.min(ur_y);
                    let x1 = ur_x.max(lr_x);
                    let y1 = ll_y.max(lr_y);
                    out.push((x0, y0, x1, y1).into_pyobject(py)?.into_any().unbind());
                }
            }
            Ok(out)
        })();

        unsafe { mupdf_sys::mupdf_free(quads_ptr as *mut _) };
        result
    }

    // ---- 注释（Phase 5b） ----

    /// get_annotations() -> list[dict]
    ///
    /// 返回页面上所有注释的列表。每条为 dict，包含：
    /// - type: str ("Highlight"/"Underline"/"StrikeOut"/"Text"/...)
    /// - rect: (x0, y0, x1, y1)
    /// - contents: str
    /// - author: str
    /// - color: (r, g, b) 或 (r, g, b, a) 或 ()
    /// - quads: list of 8-tuples (仅 highlight/underline/strikeout)
    /// - flags: int
    fn get_annotations(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut total_len: usize = 0;
        let mut n: c_int = 0;
        let rc = unsafe {
            mupdf_safe_get_annotations(
                self.ctx, self.raw_doc, self.raw,
                &mut ptr, &mut total_len, &mut n,
            )
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        if n == 0 || ptr.is_null() || total_len == 0 {
            return Ok(Vec::new());
        }

        let result = (|| -> PyResult<Vec<Py<PyAny>>> {
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, total_len) };
            let mut out = Vec::with_capacity(n as usize);
            let mut off = 0usize;

            let type_names = [
                "Text", "Link", "FreeText", "Line", "Square", "Circle",
                "Polygon", "PolyLine", "Highlight", "Underline", "Squiggly",
                "StrikeOut", "Redact", "Stamp", "Caret", "Ink", "Popup",
                "FileAttachment", "Sound", "Movie", "RichMedia", "Widget",
                "Screen", "PrinterMark", "TrapNet", "Watermark", "3D", "Projection",
            ];

            for _ in 0..n as usize {
                if off + 56 > bytes.len() {
                    return Err(PyRuntimeError::new_err("get_annotations: buffer underrun"));
                }

                let type_val = i32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let x0 = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let y0 = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let x1 = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let y1 = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let flags = i32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let color_n = i32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let c0 = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let c1 = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let c2 = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let c3 = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                let contents_len = i32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()) as usize; off += 4;
                let author_len = i32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()) as usize; off += 4;
                let quad_count = i32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()) as usize; off += 4;

                if off + contents_len + author_len + quad_count * 32 > bytes.len() {
                    return Err(PyRuntimeError::new_err("get_annotations: buffer underrun in variable data"));
                }

                let contents = std::str::from_utf8(&bytes[off..off+contents_len]).unwrap_or("");
                off += contents_len;
                let author = std::str::from_utf8(&bytes[off..off+author_len]).unwrap_or("");
                off += author_len;

                let mut quads_list: Vec<Py<PyAny>> = Vec::with_capacity(quad_count);
                for _ in 0..quad_count {
                    let ul_x = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                    let ul_y = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                    let ur_x = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                    let ur_y = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                    let ll_x = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                    let ll_y = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                    let lr_x = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                    let lr_y = f32::from_ne_bytes(bytes[off..off+4].try_into().unwrap()); off += 4;
                    quads_list.push(
                        (ul_x, ul_y, ur_x, ur_y, lr_x, lr_y, ll_x, ll_y)
                            .into_pyobject(py)?.into_any().unbind()
                    );
                }

                let type_name = if type_val >= 0 && (type_val as usize) < type_names.len() {
                    type_names[type_val as usize]
                } else {
                    "Unknown"
                };

                let color_tuple: Py<PyAny> = match color_n {
                    3 => (c0, c1, c2).into_pyobject(py)?.into_any().unbind(),
                    4 => (c0, c1, c2, c3).into_pyobject(py)?.into_any().unbind(),
                    _ => ().into_pyobject(py)?.into_any().unbind(),
                };

                let dict = PyDict::new(py);
                dict.set_item("type", type_name)?;
                dict.set_item("rect", (x0, y0, x1, y1))?;
                dict.set_item("contents", contents)?;
                dict.set_item("author", author)?;
                dict.set_item("color", color_tuple)?;
                dict.set_item("quads", quads_list)?;
                dict.set_item("flags", flags)?;
                out.push(dict.into_any().unbind());
            }
            Ok(out)
        })();

        unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        result
    }

    /// add_highlight_annot(quads, color=(1,1,0), contents="", author="") -> None
    ///
    /// 添加高亮注释。quads 为 quad 列表，每个 quad 为 8 元组。
    #[pyo3(signature = (quads, color=(1.0, 1.0, 0.0), contents="", author=""))]
    fn add_highlight_annot(
        &self,
        quads: Vec<(f32, f32, f32, f32, f32, f32, f32, f32)>,
        color: (f32, f32, f32),
        contents: &str,
        author: &str,
    ) -> PyResult<()> {
        self.invalidate_stext();
        let _ = self.create_annot_impl(8, &quads, &[color.0, color.1, color.2], 3, contents, author)?;
        Ok(())
    }

    /// add_underline_annot(quads, color=(0, 1, 0), contents="", author="") -> None
    #[pyo3(signature = (quads, color=(0.0, 1.0, 0.0), contents="", author=""))]
    fn add_underline_annot(
        &self,
        quads: Vec<(f32, f32, f32, f32, f32, f32, f32, f32)>,
        color: (f32, f32, f32),
        contents: &str,
        author: &str,
    ) -> PyResult<()> {
        self.invalidate_stext();
        let _ = self.create_annot_impl(9, &quads, &[color.0, color.1, color.2], 3, contents, author)?;
        Ok(())
    }

    /// add_strikeout_annot(quads, color=(1, 0, 0), contents="", author="") -> None
    #[pyo3(signature = (quads, color=(1.0, 0.0, 0.0), contents="", author=""))]
    fn add_strikeout_annot(
        &self,
        quads: Vec<(f32, f32, f32, f32, f32, f32, f32, f32)>,
        color: (f32, f32, f32),
        contents: &str,
        author: &str,
    ) -> PyResult<()> {
        self.invalidate_stext();
        let _ = self.create_annot_impl(11, &quads, &[color.0, color.1, color.2], 3, contents, author)?;
        Ok(())
    }

    /// add_text_annot(rect, contents, color=(1, 1, 0), author="") -> None
    ///
    /// 添加文本注释（sticky note）。rect 为 (x0, y0, x1, y1)。
    #[pyo3(signature = (rect, contents, color=(1.0, 1.0, 0.0), author=""))]
    fn add_text_annot(
        &self,
        rect: (f32, f32, f32, f32),
        contents: &str,
        color: (f32, f32, f32),
        author: &str,
    ) -> PyResult<()> {
        self.invalidate_stext();
        // Text 注释用 rect 定位，不需要 quads
        let color_arr = [color.0, color.1, color.2];
        let idx = self.create_annot_impl(
            0, /* PDF_ANNOT_TEXT */
            &[],
            &color_arr, 3,
            contents, author,
        )?;
        // 设置 rect
        let rect_arr = [rect.0, rect.1, rect.2, rect.3];
        let rc = unsafe {
            mupdf_safe_set_annot_rect(
                self.ctx, self.raw_doc, self.raw,
                idx, rect_arr.as_ptr(),
            )
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        Ok(())
    }

    /// delete_annot(index) -> None
    ///
    /// 删除指定索引的注释。
    fn delete_annot(&self, index: i32) -> PyResult<()> {
        self.invalidate_stext();
        let rc = unsafe {
            mupdf_safe_delete_annot(self.ctx, self.raw_doc, self.raw, index)
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        Ok(())
    }

    /// get_images(include_data=False) -> list[dict]
    ///
    /// 列出页面上所有图片（按 fz_image* 去重）。
    /// 每条返回 {"xref", "bbox", "width", "height", "bpc", "colorspace", "imagemask"}；
    /// include_data=True 时额外含 "ext"（"jpeg"/"png"/"jpx"）和 "image"（原始压缩字节）。
    /// xref 为 PDF 间接引用号；非 PDF 或未在资源表中找到时为 0。
    #[pyo3(signature = (include_data=false))]
    fn get_images(
        &self,
        py: Python<'_>,
        include_data: bool,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut total_len: usize = 0;
        let mut n: c_int = 0;
        // raw_doc 在 Page 构造时一次性缓存（Py<PyDocument> 保证 doc 存活）
        let rc = unsafe {
            mupdf_safe_get_images(
                self.ctx,
                self.raw_doc,
                self.raw,
                if include_data { 1 } else { 0 },
                &mut ptr,
                &mut total_len,
                &mut n,
            )
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        if n == 0 || ptr.is_null() || total_len == 0 {
            return Ok(Vec::new());
        }

        let result = (|| -> PyResult<Vec<Py<PyAny>>> {
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, total_len) };
            let mut off = 0usize;
            let need = |n: usize, off: usize| -> PyResult<()> {
                if off + n > bytes.len() {
                    return Err(PyRuntimeError::new_err(format!(
                        "images buffer underrun at {} need {} have {}",
                        off, n, bytes.len()
                    )));
                }
                Ok(())
            };
            let take_i32 = |off: &mut usize| -> PyResult<i32> {
                need(4, *off)?;
                let v = i32::from_ne_bytes(bytes[*off..*off + 4].try_into().unwrap());
                *off += 4;
                Ok(v)
            };
            let take_f32 = |off: &mut usize| -> PyResult<f32> {
                need(4, *off)?;
                let v = f32::from_ne_bytes(bytes[*off..*off + 4].try_into().unwrap());
                *off += 4;
                Ok(v)
            };
            let take_str = |off: &mut usize| -> PyResult<String> {
                let len = take_i32(off)? as usize;
                need(len, *off)?;
                let s = std::str::from_utf8(&bytes[*off..*off + len])
                    .unwrap_or("")
                    .to_string();
                *off += len;
                Ok(s)
            };

            // 跳过 image_count（已经在 n 里）
            let _ = take_i32(&mut off)?;

            let mut out = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let xref = take_i32(&mut off)?;
                let bx0 = take_f32(&mut off)?;
                let by0 = take_f32(&mut off)?;
                let bx1 = take_f32(&mut off)?;
                let by1 = take_f32(&mut off)?;
                let w = take_i32(&mut off)?;
                let h = take_i32(&mut off)?;
                let bpc = take_i32(&mut off)?;
                let cs = take_str(&mut off)?;
                let mask = take_i32(&mut off)?;
                let has_data = take_i32(&mut off)?;

                let d = PyDict::new(py);
                d.set_item("xref", xref)?;
                d.set_item("bbox", (bx0, by0, bx1, by1))?;
                d.set_item("width", w)?;
                d.set_item("height", h)?;
                d.set_item("bpc", bpc)?;
                d.set_item("colorspace", cs)?;
                d.set_item("imagemask", mask)?;

                if has_data != 0 {
                    let ext = take_str(&mut off)?;
                    let dlen = take_i32(&mut off)? as usize;
                    need(dlen, off)?;
                    let py_bytes = PyBytes::new(py, &bytes[off..off + dlen]);
                    off += dlen;
                    d.set_item("ext", &ext)?;
                    d.set_item("image", py_bytes)?;
                }
                out.push(d.into_any().unbind());
            }
            Ok(out)
        })();

        unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        result
    }


    /// get_pixmap(matrix=None, dpi=None, alpha=False) -> Pixmap
    ///
    /// matrix: 6 元组 (a,b,c,d,e,f)，目前只使用 a/d 作为缩放因子（保持简单）。
    /// dpi: 优先级高于 matrix，若设置则 zoom = dpi/72。
    /// 都不传时 zoom=1.0（72 DPI）。
    ///
    /// GIL 释放：渲染期间不持 GIL，允许其他 Python 线程并行。
    /// 注意：**同一 Document 的多个 Page 并发渲染是 UB**（MuPDF ctx 无锁）。
    /// 跨文档并行是安全的（每个 doc 独立 ctx）。
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
            m.0.max(m.3)
        } else {
            1.0
        };

        // 把裸指针经由 usize 传入 closure（py.detach 要求 Send，裸指针本身 !Send）。
        // ctx/raw 都是 MuPDF 内部不变量，渲染期间只读，安全。
        let ctx_addr = self.ctx as usize;
        let raw_addr = self.raw as usize;
        let alpha_i = alpha as c_int;
        let result: (c_int, usize) = py.detach(move || {
            let ctx = ctx_addr as *mut fz_context;
            let raw = raw_addr as *mut fz_page;
            let mut pix: *mut fz_pixmap = ptr::null_mut();
            let rc = unsafe { mupdf_safe_render_pixmap(ctx, raw, zoom, alpha_i, &mut pix) };
            (rc, pix as usize)
        });
        let pix = result.1 as *mut fz_pixmap;
        if result.0 != 0 || pix.is_null() {
            return Err(PyDocument::last_error_pub());
        }
        Ok(PyPixmap::new(self.doc_handle.clone_ref(py), self.ctx, pix))
    }

    fn __repr__(&self) -> String {
        format!("<ritz.Page at {:p}>", self.raw)
    }
}

impl PyPage {
    fn stext_to_string<'py>(
        &self,
        py: Python<'py>,
        stpage: *mut fz_stext_page,
        mode: &str,
    ) -> PyResult<Py<PyAny>> {
        // Phase 6 #2：用借用版 buffer（fz_buffer_data，0 拷贝），省下 fz_buffer_extract 的
        // malloc+memcpy。data 仅在 release_buffer 之前有效。
        let mode_id: c_int = match mode {
            "text" => 0,
            "html" => 1,
            "xhtml" => 2,
            "xml" => 3,
            _ => unreachable!(),
        };
        let mut data: *const c_char = ptr::null();
        let mut len: usize = 0;
        let mut handle: *mut std::ffi::c_void = ptr::null_mut();
        let rc = unsafe {
            mupdf_sys::mupdf_safe_stext_to_buffer_borrow(
                self.ctx, stpage, mode_id, &mut data, &mut len, &mut handle,
            )
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        // 借用 data 构造 PyString（拷贝到 Python heap，data 不再被引用）
        let py_str = unsafe { cbuf_borrowed_to_pystring(py, data, len)? };
        // 现在可以释放 buf（data 失效）
        unsafe { mupdf_sys::mupdf_safe_release_buffer(self.ctx, handle); }
        Ok(py_str.into_any().unbind())
    }

    fn stext_to_json<'py>(
        &self,
        py: Python<'py>,
        stpage: *mut fz_stext_page,
        scale: f32,
    ) -> PyResult<Py<PyAny>> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut len: usize = 0;
        let rc = unsafe { mupdf_safe_stext_to_json(self.ctx, stpage, scale, &mut ptr, &mut len) };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        let py_str = cbuf_to_pystring(py, ptr, len)?;
        if !ptr.is_null() {
            unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        }
        Ok(py_str.into_any().unbind())
    }

    /// 单词级提取：返回 PyMuPDF 兼容的 list of tuples
    ///   (x0, y0, x1, y1, "word", block_no, line_no, word_no)
    fn stext_to_words(&self, py: Python<'_>, stpage: *mut fz_stext_page) -> PyResult<Py<PyAny>> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut total_len: usize = 0;
        let mut n: c_int = 0;
        let rc = unsafe {
            mupdf_safe_stext_to_words(self.ctx, stpage, &mut ptr, &mut total_len, &mut n)
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        if n == 0 || ptr.is_null() || total_len == 0 {
            return Ok(Vec::<(f32, f32, f32, f32, String, i32, i32, i32)>::new()
                .into_pyobject(py)?
                .into_any()
                .unbind());
        }

        let result = (|| -> PyResult<Py<PyAny>> {
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, total_len) };
            let mut items: Vec<(f32, f32, f32, f32, String, i32, i32, i32)> =
                Vec::with_capacity(n as usize);
            let mut off = 0usize;
            for _ in 0..n {
                if off + 32 > bytes.len() {
                    break;
                }
                let x0 = f32::from_ne_bytes(bytes[off..off + 4].try_into().unwrap());
                let y0 = f32::from_ne_bytes(bytes[off + 4..off + 8].try_into().unwrap());
                let x1 = f32::from_ne_bytes(bytes[off + 8..off + 12].try_into().unwrap());
                let y1 = f32::from_ne_bytes(bytes[off + 12..off + 16].try_into().unwrap());
                let block_no = i32::from_ne_bytes(bytes[off + 16..off + 20].try_into().unwrap());
                let line_no = i32::from_ne_bytes(bytes[off + 20..off + 24].try_into().unwrap());
                let word_no = i32::from_ne_bytes(bytes[off + 24..off + 28].try_into().unwrap());
                let str_len = i32::from_ne_bytes(bytes[off + 28..off + 32].try_into().unwrap()) as usize;
                off += 32;
                if off + str_len > bytes.len() {
                    break;
                }
                let s = std::str::from_utf8(&bytes[off..off + str_len])
                    .unwrap_or("")
                    .to_string();
                off += str_len;
                items.push((x0, y0, x1, y1, s, block_no, line_no, word_no));
            }
            Ok(items.into_pyobject(py)?.into_any().unbind())
        })();

        unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        result
    }

    /// 块级提取：返回 PyMuPDF 兼容 list of tuples
    ///   (x0, y0, x1, y1, "block text", block_no, block_type)
    fn stext_to_blocks(&self, py: Python<'_>, stpage: *mut fz_stext_page) -> PyResult<Py<PyAny>> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut total_len: usize = 0;
        let mut n: c_int = 0;
        let rc = unsafe {
            mupdf_safe_stext_to_blocks(self.ctx, stpage, &mut ptr, &mut total_len, &mut n)
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        if n == 0 || ptr.is_null() || total_len == 0 {
            return Ok(Vec::<(f32, f32, f32, f32, String, i32, i32)>::new()
                .into_pyobject(py)?
                .into_any()
                .unbind());
        }

        let result = (|| -> PyResult<Py<PyAny>> {
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, total_len) };
            let mut items: Vec<(f32, f32, f32, f32, String, i32, i32)> =
                Vec::with_capacity(n as usize);
            let mut off = 0usize;
            for _ in 0..n {
                if off + 28 > bytes.len() {
                    break;
                }
                let x0 = f32::from_ne_bytes(bytes[off..off + 4].try_into().unwrap());
                let y0 = f32::from_ne_bytes(bytes[off + 4..off + 8].try_into().unwrap());
                let x1 = f32::from_ne_bytes(bytes[off + 8..off + 12].try_into().unwrap());
                let y1 = f32::from_ne_bytes(bytes[off + 12..off + 16].try_into().unwrap());
                let block_no = i32::from_ne_bytes(bytes[off + 16..off + 20].try_into().unwrap());
                let block_type = i32::from_ne_bytes(bytes[off + 20..off + 24].try_into().unwrap());
                let text_len = i32::from_ne_bytes(bytes[off + 24..off + 28].try_into().unwrap()) as usize;
                off += 28;
                if off + text_len > bytes.len() {
                    break;
                }
                let s = std::str::from_utf8(&bytes[off..off + text_len])
                    .unwrap_or("")
                    .to_string();
                off += text_len;
                items.push((x0, y0, x1, y1, s, block_no, block_type));
            }
            Ok(items.into_pyobject(py)?.into_any().unbind())
        })();

        unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        result
    }

    /// dict / rawdict 模式：解析 C 层二进制 buffer，构造 PyMuPDF 兼容嵌套 dict。
    /// include_chars=true 时（rawdict）每个 span 额外包含 chars 列表。
    fn stext_to_dict(
        &self,
        py: Python<'_>,
        stpage: *mut fz_stext_page,
        include_chars: bool,
    ) -> PyResult<Py<PyAny>> {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut total_len: usize = 0;
        let rc = unsafe {
            mupdf_safe_stext_to_dict(
                self.ctx,
                stpage,
                if include_chars { 1 } else { 0 },
                &mut ptr,
                &mut total_len,
            )
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        if ptr.is_null() || total_len < 4 {
            // 空结构：返回 {"blocks": []}
            let d = PyDict::new(py);
            d.set_item("blocks", Vec::<Py<PyAny>>::new().into_pyobject(py)?.into_any().unbind())?;
            return Ok(d.into_any().unbind());
        }

        let result = (|| -> PyResult<Py<PyAny>> {
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, total_len) };
            let mut off = 0usize;
            let need = |n: usize, off: usize, bytes: &[u8]| -> PyResult<()> {
                if off + n > bytes.len() {
                    return Err(PyRuntimeError::new_err(format!(
                        "dict buffer underrun at offset {} need {} have {}",
                        off, n, bytes.len()
                    )));
                }
                Ok(())
            };
            let take_i32 = |off: &mut usize, bytes: &[u8]| -> PyResult<i32> {
                need(4, *off, bytes)?;
                let v = i32::from_ne_bytes(bytes[*off..*off + 4].try_into().unwrap());
                *off += 4;
                Ok(v)
            };
            let take_f32 = |off: &mut usize, bytes: &[u8]| -> PyResult<f32> {
                need(4, *off, bytes)?;
                let v = f32::from_ne_bytes(bytes[*off..*off + 4].try_into().unwrap());
                *off += 4;
                Ok(v)
            };
            let take_str = |off: &mut usize, bytes: &[u8]| -> PyResult<String> {
                let len = take_i32(off, bytes)? as usize;
                need(len, *off, bytes)?;
                let s = std::str::from_utf8(&bytes[*off..*off + len])
                    .unwrap_or("")
                    .to_string();
                *off += len;
                Ok(s)
            };

            let block_count = take_i32(&mut off, bytes)? as usize;
            let mut blocks: Vec<Py<PyAny>> = Vec::with_capacity(block_count);
            for _ in 0..block_count {
                let btype = take_i32(&mut off, bytes)?;
                let x0 = take_f32(&mut off, bytes)?;
                let y0 = take_f32(&mut off, bytes)?;
                let x1 = take_f32(&mut off, bytes)?;
                let y1 = take_f32(&mut off, bytes)?;
                let line_count = take_i32(&mut off, bytes)? as usize;

                let block_dict = PyDict::new(py);
                block_dict.set_item("type", btype)?;
                block_dict.set_item("bbox", (x0, y0, x1, y1))?;

                if btype == 1 {
                    // image 块：无 lines。简化，不输出 image 数据。
                    block_dict.set_item("lines", Vec::<Py<PyAny>>::new().into_pyobject(py)?.into_any().unbind())?;
                } else {
                    let mut lines: Vec<Py<PyAny>> = Vec::with_capacity(line_count);
                    for _ in 0..line_count {
                        let lx0 = take_f32(&mut off, bytes)?;
                        let ly0 = take_f32(&mut off, bytes)?;
                        let lx1 = take_f32(&mut off, bytes)?;
                        let ly1 = take_f32(&mut off, bytes)?;
                        let wmode = take_i32(&mut off, bytes)?;
                        let dir_x = take_f32(&mut off, bytes)?;
                        let dir_y = take_f32(&mut off, bytes)?;
                        let span_count = take_i32(&mut off, bytes)? as usize;

                        let line_dict = PyDict::new(py);
                        line_dict.set_item("bbox", (lx0, ly0, lx1, ly1))?;
                        line_dict.set_item("wmode", wmode)?;
                        line_dict.set_item("dir", (dir_x, dir_y))?;

                        let mut spans: Vec<Py<PyAny>> = Vec::with_capacity(span_count);
                        for _ in 0..span_count {
                            let sx0 = take_f32(&mut off, bytes)?;
                            let sy0 = take_f32(&mut off, bytes)?;
                            let sx1 = take_f32(&mut off, bytes)?;
                            let sy1 = take_f32(&mut off, bytes)?;
                            let color = take_i32(&mut off, bytes)?;
                            let flags = take_i32(&mut off, bytes)?;
                            let size = take_f32(&mut off, bytes)?;
                            let font = take_str(&mut off, bytes)?;
                            let text = take_str(&mut off, bytes)?;

                            let span_dict = PyDict::new(py);
                            span_dict.set_item("bbox", (sx0, sy0, sx1, sy1))?;
                            span_dict.set_item("color", color)?;
                            span_dict.set_item("flags", flags)?;
                            span_dict.set_item("size", size)?;
                            span_dict.set_item("font", font)?;
                            span_dict.set_item("text", text)?;

                            if include_chars {
                                let char_count = take_i32(&mut off, bytes)? as usize;
                                let mut chars: Vec<Py<PyAny>> = Vec::with_capacity(char_count);
                                for _ in 0..char_count {
                                    let cx0 = take_f32(&mut off, bytes)?;
                                    let cy0 = take_f32(&mut off, bytes)?;
                                    let cx1 = take_f32(&mut off, bytes)?;
                                    let cy1 = take_f32(&mut off, bytes)?;
                                    let ox = take_f32(&mut off, bytes)?;
                                    let oy = take_f32(&mut off, bytes)?;
                                    let c_utf8 = take_str(&mut off, bytes)?;
                                    let c_dict = PyDict::new(py);
                                    c_dict.set_item("bbox", (cx0, cy0, cx1, cy1))?;
                                    c_dict.set_item("origin", (ox, oy))?;
                                    c_dict.set_item("c", c_utf8)?;
                                    chars.push(c_dict.into_any().unbind());
                                }
                                span_dict.set_item(
                                    "chars",
                                    chars.into_pyobject(py)?.into_any().unbind(),
                                )?;
                            }

                            spans.push(span_dict.into_any().unbind());
                        }
                        line_dict.set_item("spans", spans.into_pyobject(py)?.into_any().unbind())?;
                        lines.push(line_dict.into_any().unbind());
                    }
                    block_dict.set_item("lines", lines.into_pyobject(py)?.into_any().unbind())?;
                }
                blocks.push(block_dict.into_any().unbind());
            }

            let page_dict = PyDict::new(py);
            page_dict.set_item("blocks", blocks.into_pyobject(py)?.into_any().unbind())?;
            Ok(page_dict.into_any().unbind())
        })();

        unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        result
    }
}

/// 字节 slice → PyString 的共享快速路径。
///
/// Phase 6 #3：release build 跳过 UTF-8 校验（MuPDF fz_print_stext_page_* 输出保证 UTF-8）。
/// debug build 保留校验 + lossy fallback，便于 corrupt PDF 触发 bug 时定位。
fn slice_to_pystring<'py>(
    py: Python<'py>,
    slice: &[u8],
) -> PyResult<Bound<'py, PyString>> {
    if slice.is_empty() {
        return Ok(PyString::new(py, ""));
    }
    #[cfg(not(debug_assertions))]
    {
        // SAFETY: MuPDF stext 输出保证 UTF-8。
        let s = unsafe { std::str::from_utf8_unchecked(slice) };
        Ok(PyString::new(py, s))
    }
    #[cfg(debug_assertions)]
    {
        match std::str::from_utf8(slice) {
            Ok(s) => Ok(PyString::new(py, s)),
            Err(_) => {
                let lossy = String::from_utf8_lossy(slice);
                Ok(PyString::new(py, &lossy))
            }
        }
    }
}

/// C 字节缓冲 → PyString 的快速路径。
///
/// Tier 2 #8 优化：替换原先的 `cbuf_to_string`（`String::from_utf8_lossy + into_owned`）
/// + `into_pyobject`，省一次堆分配 + 一次拷贝。
fn cbuf_to_pystring<'py>(
    py: Python<'py>,
    ptr: *const c_char,
    len: usize,
) -> PyResult<Bound<'py, PyString>> {
    if ptr.is_null() || len == 0 {
        return Ok(PyString::new(py, ""));
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    slice_to_pystring(py, slice)
}

/// Phase 6 #2：借用版 cbuf → PyString。与 cbuf_to_pystring 共用 slice_to_pystring，
/// 区别是这里**不调用 mupdf_free**（buffer 由 mupdf_safe_release_buffer 显式释放）。
///
/// # Safety
/// data 必须在 caller 调用 mupdf_safe_release_buffer 之前消费完。
unsafe fn cbuf_borrowed_to_pystring<'py>(
    py: Python<'py>,
    ptr: *const c_char,
    len: usize,
) -> PyResult<Bound<'py, PyString>> {
    if ptr.is_null() || len == 0 {
        return Ok(PyString::new(py, ""));
    }
    let slice = std::slice::from_raw_parts(ptr as *const u8, len);
    slice_to_pystring(py, slice)
}

impl PyPage {
    /// 创建注释，返回新注释在页面注释列表中的 index。
    fn create_annot_impl(
        &self,
        annot_type: i32,
        quads: &[(f32, f32, f32, f32, f32, f32, f32, f32)],
        color: &[f32],
        color_n: i32,
        contents: &str,
        _author: &str,
    ) -> PyResult<i32> {
        let flat_quads: Vec<f32> = quads.iter().flat_map(|q| {
            [q.0, q.1, q.2, q.3, q.4, q.5, q.6, q.7]
        }).collect();
        let quad_count = quads.len() as i32;
        let contents_c = std::ffi::CString::new(contents).unwrap_or_default();
        let mut out_index: c_int = -1;
        let rc = unsafe {
            mupdf_safe_create_annot(
                self.ctx, self.raw_doc, self.raw,
                annot_type,
                if flat_quads.is_empty() { ptr::null() } else { flat_quads.as_ptr() },
                quad_count,
                color_n, color.as_ptr(),
                contents_c.as_ptr(),
                &mut out_index,
            )
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        Ok(out_index)
    }
}

impl Drop for PyPage {
    fn drop(&mut self) {
        // Phase 6：释放 stext 缓存（如果已构建）。drop 顺序：stext → page。
        if let Some(stpage) = self.stext_cache.lock().unwrap().take() {
            unsafe { mupdf_safe_drop_stext_page(self.ctx, stpage); }
        }
        unsafe {
            if !self.raw.is_null() {
                mupdf_safe_drop_page(self.ctx, self.raw);
                self.raw = ptr::null_mut();
            }
        }
    }
}

unsafe impl Send for PyPage {}
unsafe impl Sync for PyPage {}
