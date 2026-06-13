//! 并行批量处理：rayon + 每文档独立 fz_context。
//!
//! 关键设计：
//! - 每个文档任务创建自己的 fz_context（无锁函数，单线程内使用）
//! - 不用 fz_clone_context：clone 要求源 ctx 有锁函数；这里完全独立更简单
//! - 调用方用 py.allow_threads() 释放 GIL，rayon 内部纯 Rust，结束后再回 Python

use mupdf_sys::{
    self, fz_context, fz_document, mupdf_ext_extract_pages_text, mupdf_safe_drop_context,
    mupdf_safe_drop_document, mupdf_safe_new_context, mupdf_safe_open_document,
};
use pyo3::prelude::*;
use pyo3::types::PyList;
use rayon::prelude::*;
use std::ffi::CString;
use std::ptr;

/// 单文档处理结果。
pub enum DocResult {
    /// 成功，每页一个字符串。
    Ok(Vec<String>),
    /// 失败，错误消息。
    Err(String),
}

/// 处理一个 PDF：开 ctx → 开 doc → 批量提文本 → 清理。
///
/// 内部不持有 Python 对象，纯 Rust，可在 rayon worker 里调用。
fn process_one(path: &str) -> DocResult {
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

    // 3. 批量提取
    let mut text_ptr: *mut std::os::raw::c_char = ptr::null_mut();
    let mut text_len: usize = 0;
    let mut offsets_ptr: *mut usize = ptr::null_mut();
    let mut page_count: i32 = 0;
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

    let result = if rc != 0 {
        DocResult::Err(last_error_str())
    } else if page_count == 0 || text_ptr.is_null() || offsets_ptr.is_null() {
        DocResult::Ok(Vec::new())
    } else {
        // 切片成 Vec<String>
        let bytes = unsafe { std::slice::from_raw_parts(text_ptr as *const u8, text_len) };
        let offsets = unsafe { std::slice::from_raw_parts(offsets_ptr, page_count as usize + 1) };
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

    // 4. 清理
    unsafe {
        if !text_ptr.is_null() {
            mupdf_sys::mupdf_free(text_ptr as *mut _);
        }
        if !offsets_ptr.is_null() {
            mupdf_sys::mupdf_free(offsets_ptr as *mut _);
        }
        mupdf_safe_drop_document(ctx, doc);
        mupdf_safe_drop_context(ctx);
    }

    result
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

/// Python 入口：process_documents(paths, mode="text") -> list
///
/// 并行处理多个文档。每个文档在 rayon worker 中独立 ctx 跑，
/// 失败的文档对应 None，成功的对应 list[str]。
#[pyfunction]
#[pyo3(signature = (paths, mode = "text"))]
pub fn process_documents(
    py: Python<'_>,
    paths: Vec<String>,
    mode: &str,
) -> PyResult<Vec<Py<PyAny>>> {
    if mode != "text" {
        return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "process_documents currently only supports mode='text', got '{}'",
            mode
        )));
    }

    // 释放 GIL，让 rayon worker 并行跑（无 Python 调用）
    // PyO3 0.29: allow_threads 改名为 detach
    let results: Vec<DocResult> = py.detach(|| paths.par_iter().map(|p| process_one(p)).collect());

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
