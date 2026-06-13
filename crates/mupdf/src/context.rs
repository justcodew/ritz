//! fz_context 的 RAII 封装。
//!
//! 每个 fz_context 非线程安全。多文档并行时每线程 clone 一个。

use crate::error::{MuPdfError, Result};

/// MuPDF 上下文。拥有底层 fz_context 的所有权。
pub struct Context {
    raw: *mut mupdf_sys::fz_context,
}

impl Context {
    /// 创建新上下文并注册文档处理器。
    pub fn new() -> Result<Self> {
        let raw = unsafe { mupdf_sys::mupdf_safe_new_context() };
        if raw.is_null() {
            return Err(MuPdfError::AllocContext(
                MuPdfError::from_last_error().to_string(),
            ));
        }
        Ok(Context { raw })
    }

    /// 为多文档并行克隆上下文（独立锁状态，可跨线程使用）。
    pub fn clone_for_thread(&self) -> Result<Context> {
        let raw = unsafe { mupdf_sys::mupdf_safe_clone_context(self.raw) };
        if raw.is_null() {
            return Err(MuPdfError::from_last_error());
        }
        Ok(Context { raw })
    }

    /// 返回原始指针（供下层 Document/Page 使用）。
    pub(crate) fn as_ptr(&self) -> *mut mupdf_sys::fz_context {
        self.raw
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            mupdf_sys::mupdf_safe_drop_context(self.raw);
        }
    }
}

// 单个 context 非线程安全，但通过 clone-per-thread 模式可安全跨线程使用各自独立的 clone。
// Context 本身移入线程后不再跨线程共享，故标记 Send。
unsafe impl Send for Context {}
// 不实现 Sync：禁止跨线程共享引用