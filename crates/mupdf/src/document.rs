//! fz_document 的 RAII 封装，借用 Context 的生命周期。

use crate::context::Context;
use crate::error::{MuPdfError, Result};
use crate::page::Page;
use std::ffi::CString;
use std::marker::PhantomData;
use std::os::raw::c_int;

/// PDF/其他格式文档。借用 Context（不可比 Context 活得长）。
pub struct Document<'ctx> {
    raw: *mut mupdf_sys::fz_document,
    ctx: *mut mupdf_sys::fz_context,
    _marker: PhantomData<&'ctx Context>,
}

impl<'ctx> Document<'ctx> {
    /// 从文件路径打开文档。
    pub fn open(ctx: &'ctx Context, filename: &str) -> Result<Self> {
        let c_path = CString::new(filename)
            .map_err(|e| MuPdfError::OpenDocument(format!("invalid filename: {}", e)))?;
        let raw = unsafe { mupdf_sys::mupdf_safe_open_document(ctx.as_ptr(), c_path.as_ptr()) };
        if raw.is_null() {
            return Err(MuPdfError::OpenDocument(
                MuPdfError::from_last_error().to_string(),
            ));
        }
        Ok(Document {
            raw,
            ctx: ctx.as_ptr(),
            _marker: PhantomData,
        })
    }

    /// 从内存字节打开文档。
    /// 注意：MuPDF 的 fz_open_memory 不复制数据，调用方须保证 data 在 Document 存活期间有效。
    pub fn open_from_bytes(ctx: &'ctx Context, data: &[u8], mime: &str) -> Result<Self> {
        let c_mime = CString::new(mime)
            .map_err(|e| MuPdfError::OpenDocument(format!("invalid mime: {}", e)))?;
        let mut raw: *mut mupdf_sys::fz_document = std::ptr::null_mut();
        let rc = unsafe {
            mupdf_sys::mupdf_safe_open_document_with_stream(
                ctx.as_ptr(),
                data.as_ptr(),
                data.len(),
                c_mime.as_ptr(),
                &mut raw,
            )
        };
        if rc != 0 || raw.is_null() {
            return Err(MuPdfError::OpenDocument(
                MuPdfError::from_last_error().to_string(),
            ));
        }
        Ok(Document {
            raw,
            ctx: ctx.as_ptr(),
            _marker: PhantomData,
        })
    }

    /// 总页数。
    pub fn page_count(&self) -> Result<usize> {
        let n = unsafe { mupdf_sys::mupdf_safe_count_pages(self.ctx, self.raw) };
        if n < 0 {
            return Err(MuPdfError::from_last_error());
        }
        Ok(n as usize)
    }

    /// 是否需要密码。
    pub fn needs_password(&self) -> bool {
        unsafe { mupdf_sys::mupdf_safe_needs_password(self.ctx, self.raw) != 0 }
    }

    /// 尝试用密码解锁，返回是否成功。
    pub fn authenticate(&self, password: &str) -> Result<bool> {
        let c_pw = CString::new(password).unwrap_or_default();
        let ok = unsafe {
            mupdf_sys::mupdf_safe_authenticate_password(self.ctx, self.raw, c_pw.as_ptr())
        };
        Ok(ok != 0)
    }

    /// 读取元数据字段（"info:Title"、"info:Author" 等）。
    pub fn metadata(&self, key: &str) -> Option<String> {
        let c_key = CString::new(key).ok()?;
        let mut buf = [0u8; 512];
        let n = unsafe {
            mupdf_sys::mupdf_safe_lookup_metadata(
                self.ctx,
                self.raw,
                c_key.as_ptr(),
                buf.as_mut_ptr() as *mut i8,
                buf.len() as c_int,
            )
        };
        if n <= 0 {
            return None;
        }
        let len = n as usize;
        Some(String::from_utf8_lossy(&buf[..len]).into_owned())
    }

    /// 加载指定页（0-based）。
    pub fn page(&self, index: usize) -> Result<Page<'_, 'ctx>> {
        let raw = unsafe { mupdf_sys::mupdf_safe_load_page(self.ctx, self.raw, index as c_int) };
        if raw.is_null() {
            return Err(MuPdfError::LoadPage(
                MuPdfError::from_last_error().to_string(),
            ));
        }
        Ok(Page::from_raw(raw, self.ctx))
    }

    /// 迭代所有页。
    pub fn pages(&self) -> Result<Vec<Page<'_, 'ctx>>> {
        let count = self.page_count()?;
        (0..count).map(|i| self.page(i)).collect()
    }

    pub(crate) fn ctx_ptr(&self) -> *mut mupdf_sys::fz_context {
        self.ctx
    }
}

impl Drop for Document<'_> {
    fn drop(&mut self) {
        unsafe {
            mupdf_sys::mupdf_safe_drop_document(self.ctx, self.raw);
        }
    }
}