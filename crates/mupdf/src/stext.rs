//! fz_stext_page 的 RAII 封装。从某页提取的结构化文本，可导出为 text/html/xml。

use crate::error::{MuPdfError, Result};
use crate::page::Page;
use std::marker::PhantomData;
use std::os::raw::c_char;

/// malloc 出来的 C buffer，由 mupdf_free 释放。
pub(crate) struct CBuffer {
    ptr: *mut c_char,
    len: usize,
}

impl CBuffer {
    pub(crate) fn new(ptr: *mut c_char, len: usize) -> Self {
        CBuffer { ptr, len }
    }

    /// 取出 buffer 内容作为 Vec<u8>，并消费 self（避免 Drop 双重释放）。
    pub(crate) fn to_vec(mut self) -> Vec<u8> {
        let v = if self.ptr.is_null() || self.len == 0 {
            Vec::new()
        } else {
            unsafe {
                let slice = std::slice::from_raw_parts(self.ptr as *const u8, self.len);
                slice.to_vec()
            }
        };
        if !self.ptr.is_null() {
            unsafe { mupdf_sys::mupdf_free(self.ptr as *mut _) };
            self.ptr = std::ptr::null_mut();
        }
        v
    }
}

impl Drop for CBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { mupdf_sys::mupdf_free(self.ptr as *mut _) };
            self.ptr = std::ptr::null_mut();
        }
    }
}

/// 单页的结构化文本（借用 Page 期间有效）。
pub struct STextPage<'a, 'doc, 'ctx> {
    raw: *mut mupdf_sys::fz_stext_page,
    ctx: *mut mupdf_sys::fz_context,
    _page: PhantomData<&'a Page<'doc, 'ctx>>,
}

impl<'a, 'doc, 'ctx> STextPage<'a, 'doc, 'ctx> {
    pub(crate) fn from_raw(
        raw: *mut mupdf_sys::fz_stext_page,
        ctx: *mut mupdf_sys::fz_context,
    ) -> Self {
        STextPage {
            raw,
            ctx,
            _page: PhantomData,
        }
    }

    fn to_buffer(&self, mode: TextMode) -> Result<Vec<u8>> {
        let mut ptr: *mut c_char = std::ptr::null_mut();
        let mut len: usize = 0;
        let rc = unsafe {
            match mode {
                TextMode::Text => mupdf_sys::mupdf_safe_stext_to_text(
                    self.ctx,
                    self.raw,
                    &mut ptr,
                    &mut len,
                ),
                TextMode::Html => mupdf_sys::mupdf_safe_stext_to_html(
                    self.ctx,
                    self.raw,
                    &mut ptr,
                    &mut len,
                ),
                TextMode::Xml => mupdf_sys::mupdf_safe_stext_to_xml(
                    self.ctx,
                    self.raw,
                    &mut ptr,
                    &mut len,
                ),
            }
        };
        if rc != 0 {
            return Err(MuPdfError::STextPage(
                MuPdfError::from_last_error().to_string(),
            ));
        }
        Ok(CBuffer::new(ptr, len).to_vec())
    }

    /// 纯文本输出（与 PyMuPDF 的 get_text("text") 一致）。
    pub fn to_text(&self) -> Result<String> {
        let bytes = self.to_buffer(TextMode::Text)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// HTML 输出（与 get_text("html") 一致）。
    pub fn to_html(&self) -> Result<String> {
        let bytes = self.to_buffer(TextMode::Html)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// XML 输出（与 get_text("xml") 一致）。
    pub fn to_xml(&self) -> Result<String> {
        let bytes = self.to_buffer(TextMode::Xml)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl Drop for STextPage<'_, '_, '_> {
    fn drop(&mut self) {
        unsafe {
            mupdf_sys::mupdf_safe_drop_stext_page(self.ctx, self.raw);
        }
    }
}

#[derive(Clone, Copy)]
enum TextMode {
    Text,
    Html,
    Xml,
}
