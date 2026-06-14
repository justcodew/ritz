//! PyPage — Python 侧 Page 类。

use crate::document::PyDocument;
use crate::pixmap::PyPixmap;
use mupdf_sys::{
    self, fz_context, fz_page, mupdf_safe_bound_page, mupdf_safe_bound_page_box,
    mupdf_safe_drop_page, mupdf_safe_drop_stext_page, mupdf_safe_get_images,
    mupdf_safe_load_links, mupdf_safe_new_stext_page, mupdf_safe_page_rotation,
    mupdf_safe_render_pixmap, mupdf_safe_stext_to_blocks, mupdf_safe_stext_to_dict,
    mupdf_safe_stext_to_html, mupdf_safe_stext_to_json, mupdf_safe_stext_to_text,
    mupdf_safe_stext_to_words, mupdf_safe_stext_to_xml, mupdf_safe_stext_to_xhtml,
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
/// 加速缓存（plan_v1 后 Tier 1 优化）：
/// - `raw_doc`：load_page 时一次性取，省去每次方法调用时 `doc_handle.bind(py).borrow()` 的开销
/// - `rect_cache`：rect/width/height 三个 getter 共享一次 `fz_bound_page` 调用结果
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
        }
    }

    /// 返回缓存的 rect，首次访问时计算。
    fn cached_rect(&self) -> (f32, f32, f32, f32) {
        *self.rect_cache.get_or_init(|| {
            let r = unsafe { mupdf_safe_bound_page(self.ctx, self.raw) };
            (r.x0, r.y0, r.x1, r.y1)
        })
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
        let mut stpage: *mut fz_stext_page = ptr::null_mut();
        let rc = unsafe { mupdf_safe_new_stext_page(self.ctx, self.raw, &mut stpage) };
        if rc != 0 || stpage.is_null() {
            return Err(PyDocument::last_error_pub());
        }

        let result = match mode {
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
        };

        unsafe { mupdf_safe_drop_stext_page(self.ctx, stpage) };
        result
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
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut len: usize = 0;
        let rc = unsafe {
            match mode {
                "text" => mupdf_safe_stext_to_text(self.ctx, stpage, &mut ptr, &mut len),
                "html" => mupdf_safe_stext_to_html(self.ctx, stpage, &mut ptr, &mut len),
                "xhtml" => mupdf_safe_stext_to_xhtml(self.ctx, stpage, &mut ptr, &mut len),
                "xml" => mupdf_safe_stext_to_xml(self.ctx, stpage, &mut ptr, &mut len),
                _ => unreachable!(),
            }
        };
        if rc != 0 {
            return Err(PyDocument::last_error_pub());
        }
        let py_str = cbuf_to_pystring(py, ptr, len)?;
        if !ptr.is_null() {
            unsafe { mupdf_sys::mupdf_free(ptr as *mut _) };
        }
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

/// C 字节缓冲 → PyString 的快速路径。
///
/// Tier 2 #8 优化：替换原先的 `cbuf_to_string`（`String::from_utf8_lossy + into_owned`）
/// + `into_pyobject`，省一次堆分配 + 一次拷贝。
///
/// 安全：MuPDF 输出本就是 UTF-8。但为防 corrupt PDF，先 `std::str::from_utf8` 校验
/// （零拷贝、返回 Result），失败时回退到 lossy（与旧行为一致）。
fn cbuf_to_pystring<'py>(
    py: Python<'py>,
    ptr: *const c_char,
    len: usize,
) -> PyResult<Bound<'py, PyString>> {
    if ptr.is_null() || len == 0 {
        return Ok(PyString::new(py, ""));
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    // Fast path：合法 UTF-8 → 单次拷贝构造 PyString
    match std::str::from_utf8(slice) {
        Ok(s) => Ok(PyString::new(py, s)),
        Err(_) => {
            // Slow path：corrupt PDF 触发，lossy 替换。仍只构造一次 PyString。
            let lossy = String::from_utf8_lossy(slice);
            Ok(PyString::new(py, &lossy))
        }
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
    }
}

unsafe impl Send for PyPage {}
unsafe impl Sync for PyPage {}
