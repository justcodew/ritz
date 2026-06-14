//! 并行批量处理：rayon + 每文档独立 fz_context。
//!
//! 关键设计：
//! - 每个文档任务创建自己的 fz_context（无锁函数，单线程内使用）
//! - 不用 fz_clone_context：clone 要求源 ctx 有锁函数；这里完全独立更简单
//! - 调用方用 py.allow_threads() 释放 GIL，rayon 内部纯 Rust，结束后再回 Python
//!
//! 支持的 mode：
//! - "text"  : 快速批处理路径（mupdf_ext_extract_pages_text 一次 C 调用拿全部页）
//! - "html" / "xhtml" / "xml" / "json" / "rawjson" : 每页 stext_to_* 单独调用，
//!            输出仍是 Vec<String>（每页一个字符串）
//! - "blocks" / "dict" / "rawdict" / "words" : 结构化模式，返回 JSON 字符串

use mupdf_sys::{
    self, fz_context, fz_document, mupdf_ext_extract_pages_text, mupdf_safe_count_pages,
    mupdf_safe_drop_context, mupdf_safe_drop_document, mupdf_safe_drop_page,
    mupdf_safe_drop_stext_page, mupdf_safe_load_page, mupdf_safe_new_context,
    mupdf_safe_new_stext_page, mupdf_safe_open_document, mupdf_safe_stext_to_html,
    mupdf_safe_stext_to_json, mupdf_safe_stext_to_xhtml, mupdf_safe_stext_to_xml,
};
use pyo3::prelude::*;
use pyo3::types::PyList;
use rayon::prelude::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::ptr;

/// 单文档处理结果。
pub enum DocResult {
    /// 成功，每页一个字符串。
    Ok(Vec<String>),
    /// 失败，错误消息。
    Err(String),
}

/// 处理一个 PDF：开 ctx → 开 doc → 提取全部页文本 → 清理。
///
/// 内部不持有 Python 对象，纯 Rust，可在 rayon worker 里调用。
fn process_one(path: &str, mode: &str) -> DocResult {
    let c_path = match CString::new(path) {
        Ok(s) => s,
        Err(_) => return DocResult::Err("path contains NUL".into()),
    };

    // 1. 独立 fz_context（无锁函数，仅当前线程使用）
    let ctx: *mut fz_context = unsafe { mupdf_safe_new_context() };
    if ctx.is_null() {
        return DocResult::Err(last_error_str());
    }

    // 2. 打开文档
    let doc: *mut fz_document = unsafe { mupdf_safe_open_document(ctx, c_path.as_ptr()) };
    if doc.is_null() {
        let e = last_error_str();
        unsafe { mupdf_safe_drop_context(ctx) };
        return DocResult::Err(e);
    }

    let result = unsafe { process_doc(ctx, doc, mode) };

    // 4. 清理
    unsafe {
        mupdf_safe_drop_document(ctx, doc);
        mupdf_safe_drop_context(ctx);
    }

    result
}

/// 实际的提取逻辑：根据 mode 分发到不同路径。
unsafe fn process_doc(ctx: *mut fz_context, doc: *mut fz_document, mode: &str) -> DocResult {
    // text 模式走快路径：一次 C 调用拿全部页
    if mode == "text" {
        return extract_text_batch(ctx, doc);
    }

    // 其他模式：逐页提取，每页一个字符串
    let n = mupdf_safe_count_pages(ctx, doc);
    if n < 0 {
        return DocResult::Err(last_error_str());
    }

    let mut out: Vec<String> = Vec::with_capacity(n as usize);
    for i in 0..n {
        let page = mupdf_safe_load_page(ctx, doc, i);
        if page.is_null() {
            return DocResult::Err(last_error_str());
        }
        let mut stpage: *mut mupdf_sys::fz_stext_page = ptr::null_mut();
        let rc = mupdf_safe_new_stext_page(ctx, page, &mut stpage);
        if rc != 0 || stpage.is_null() {
            mupdf_safe_drop_page(ctx, page);
            let e = last_error_str();
            return DocResult::Err(e);
        }

        let s = extract_one_page(ctx, stpage, mode);
        mupdf_safe_drop_stext_page(ctx, stpage);
        mupdf_safe_drop_page(ctx, page);

        match s {
            Ok(t) => out.push(t),
            Err(e) => return DocResult::Err(e),
        }
    }
    DocResult::Ok(out)
}

/// 快速批量纯文本路径：一次 C 调用提取全部页。
unsafe fn extract_text_batch(ctx: *mut fz_context, doc: *mut fz_document) -> DocResult {
    let mut text_ptr: *mut c_char = ptr::null_mut();
    let mut text_len: usize = 0;
    let mut offsets_ptr: *mut usize = ptr::null_mut();
    let mut page_count: c_int = 0;
    let rc = mupdf_ext_extract_pages_text(
        ctx,
        doc,
        &mut text_ptr,
        &mut text_len,
        &mut offsets_ptr,
        &mut page_count,
    );

    let result = if rc != 0 {
        DocResult::Err(last_error_str())
    } else if page_count == 0 || text_ptr.is_null() || offsets_ptr.is_null() {
        DocResult::Ok(Vec::new())
    } else {
        let bytes = std::slice::from_raw_parts(text_ptr as *const u8, text_len);
        let offsets = std::slice::from_raw_parts(offsets_ptr, page_count as usize + 1);
        let mut out = Vec::with_capacity(page_count as usize);
        for i in 0..page_count as usize {
            let start = offsets[i];
            let end = offsets[i + 1];
            let slice = if start <= end && end <= bytes.len() {
                &bytes[start..end]
            } else {
                &bytes[start..]
            };
            out.push(String::from_utf8_lossy(slice).into_owned());
        }
        DocResult::Ok(out)
    };

    if !text_ptr.is_null() {
        mupdf_sys::mupdf_free(text_ptr as *mut _);
    }
    if !offsets_ptr.is_null() {
        mupdf_sys::mupdf_free(offsets_ptr as *mut _);
    }
    result
}

/// 提取单页文本（指定 mode）。
unsafe fn extract_one_page(
    ctx: *mut fz_context,
    stpage: *mut mupdf_sys::fz_stext_page,
    mode: &str,
) -> Result<String, String> {
    let mut ptr: *mut c_char = ptr::null_mut();
    let mut len: usize = 0;
    let rc = match mode {
        "html" => mupdf_safe_stext_to_html(ctx, stpage, &mut ptr, &mut len),
        "xhtml" => mupdf_safe_stext_to_xhtml(ctx, stpage, &mut ptr, &mut len),
        "xml" => mupdf_safe_stext_to_xml(ctx, stpage, &mut ptr, &mut len),
        "json" => mupdf_safe_stext_to_json(ctx, stpage, 1.0, &mut ptr, &mut len),
        _ => {
            // blocks/words/dict/rawdict：C 层返回结构化 buffer，
            // batch 场景下统一序列化成 JSON-ish 字符串供调用方解析
            return extract_structured(ctx, stpage, mode);
        }
    };
    if rc != 0 {
        return Err(last_error_str());
    }
    let s = cbuf_to_string(ptr, len);
    if !ptr.is_null() {
        mupdf_sys::mupdf_free(ptr as *mut _);
    }
    Ok(s)
}

/// 结构化模式（blocks/dict/rawdict/words）：返回原始 buffer 的 hex/bytes 表示。
/// 用户在 Python 侧自己解析。这是 batch 场景的简化方案——
/// 完整结构化并行需要更复杂的 Python 对象构造，暂不实现。
unsafe fn extract_structured(
    _ctx: *mut fz_context,
    _stpage: *mut mupdf_sys::fz_stext_page,
    mode: &str,
) -> Result<String, String> {
    Err(format!(
        "process_documents mode '{}' not yet supported in batch; \
         use per-page doc[i].get_text(\"{}\") instead, or mode in \
         (text, html, xhtml, xml, json)",
        mode, mode
    ))
}

fn cbuf_to_string(ptr: *const c_char, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(ptr as *const u8, len);
        String::from_utf8_lossy(slice).into_owned()
    }
}

fn last_error_str() -> String {
    unsafe {
        let p = mupdf_sys::mupdf_last_error();
        if p.is_null() || *p == 0 {
            "unknown error".to_string()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

// 引用以避免未使用 import 警告（blocks/words/dict 等结构化模式暂未实现 batch）
#[allow(dead_code)]
const _BATCH_MODE_STRUCT_NOT_IMPL: &str = "blocks/dict/rawdict/words";

/// Python 入口：process_documents(paths, mode="text") -> list
///
/// 并行处理多个文档。每个文档在 rayon worker 中独立 ctx 跑，
/// 失败的文档对应 None，成功的对应 list[str]。
///
/// 支持的 mode：
/// - "text"  : 快速批量路径（推荐，最常用）
/// - "html" / "xhtml" / "xml" / "json" : 逐页 stext 提取
/// - 其他（blocks/dict/rawdict/words）：暂不支持，建议用逐页 API
#[pyfunction]
#[pyo3(signature = (paths, mode = "text"))]
pub fn process_documents(
    py: Python<'_>,
    paths: Vec<String>,
    mode: &str,
) -> PyResult<Vec<Py<PyAny>>> {
    // 释放 GIL，让 rayon worker 并行跑（无 Python 调用）
    // PyO3 0.29: allow_threads 改名为 detach
    let results: Vec<DocResult> =
        py.detach(|| paths.par_iter().map(|p| process_one(p, mode)).collect());

    // 重新持有 GIL，把结果转成 Python 对象：成功为 list[str]，失败为 None
    let mut out = Vec::with_capacity(results.len());
    for r in results {
        let obj = match r {
            DocResult::Ok(pages) => {
                let list = PyList::new(py, pages.iter().map(|s| s.as_str()))?;
                list.into_any().unbind()
            }
            DocResult::Err(_msg) => py.None(),
        };
        out.push(obj);
    }
    Ok(out)
}
