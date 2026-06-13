//! fz_pixmap 的 RAII 封装。渲染后的像素图，可导出为 PNG。

use crate::error::{MuPdfError, Result};
use crate::page::Page;
use crate::stext::CBuffer;
use std::marker::PhantomData;
use std::os::raw::c_uchar;

/// 渲染后的像素图（借用 Page 期间有效）。
pub struct Pixmap<'a, 'doc, 'ctx> {
    raw: *mut mupdf_sys::fz_pixmap,
    ctx: *mut mupdf_sys::fz_context,
    _page: PhantomData<&'a Page<'doc, 'ctx>>,
}

impl<'a, 'doc, 'ctx> Pixmap<'a, 'doc, 'ctx> {
    pub(crate) fn from_raw(
        raw: *mut mupdf_sys::fz_pixmap,
        ctx: *mut mupdf_sys::fz_context,
    ) -> Self {
        Pixmap {
            raw,
            ctx,
            _page: PhantomData,
        }
    }

    pub fn width(&self) -> i32 {
        unsafe { mupdf_sys::mupdf_safe_pixmap_width(self.ctx, self.raw) }
    }

    pub fn height(&self) -> i32 {
        unsafe { mupdf_sys::mupdf_safe_pixmap_height(self.ctx, self.raw) }
    }

    pub fn stride(&self) -> i32 {
        unsafe { mupdf_sys::mupdf_safe_pixmap_stride(self.ctx, self.raw) }
    }

    pub fn components(&self) -> i32 {
        unsafe { mupdf_sys::mupdf_safe_pixmap_components(self.ctx, self.raw) }
    }

    /// 像素数据（借用，长度 = stride * height）。
    pub fn samples(&self) -> &[u8] {
        let p = unsafe { mupdf_sys::mupdf_safe_pixmap_samples(self.ctx, self.raw) };
        if p.is_null() {
            return &[];
        }
        let len = (self.stride() as usize) * (self.height() as usize);
        unsafe { std::slice::from_raw_parts(p, len) }
    }

    /// 编码为 PNG，返回字节。
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let mut ptr: *mut c_uchar = std::ptr::null_mut();
        let mut len: usize = 0;
        let rc = unsafe {
            mupdf_sys::mupdf_safe_pixmap_to_png(self.ctx, self.raw, &mut ptr, &mut len)
        };
        if rc != 0 {
            return Err(MuPdfError::Pixmap(
                MuPdfError::from_last_error().to_string(),
            ));
        }
        Ok(CBuffer::new(ptr as *mut std::os::raw::c_char, len).to_vec())
    }
}

impl Drop for Pixmap<'_, '_, '_> {
    fn drop(&mut self) {
        unsafe {
            mupdf_sys::mupdf_safe_drop_pixmap(self.ctx, self.raw);
        }
    }
}
