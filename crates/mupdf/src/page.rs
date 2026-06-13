//! fz_page 的 RAII 封装，借用 Document 的生命周期。

use crate::geometry::Rect;
use std::marker::PhantomData;

/// 单页。借用 Document（不可比 Document 活得长）。
pub struct Page<'doc, 'ctx> {
    raw: *mut mupdf_sys::fz_page,
    ctx: *mut mupdf_sys::fz_context,
    _doc: PhantomData<&'doc ()>,
    _ctx: PhantomData<&'ctx ()>,
}

impl<'doc, 'ctx> Page<'doc, 'ctx> {
    pub(crate) fn from_raw(raw: *mut mupdf_sys::fz_page, ctx: *mut mupdf_sys::fz_context) -> Self {
        Page {
            raw,
            ctx,
            _doc: PhantomData,
            _ctx: PhantomData,
        }
    }

    /// 页面矩形（MuPDF 内部左下原点坐标系）。
    pub fn rect(&self) -> Rect {
        let r = unsafe { mupdf_sys::mupdf_safe_bound_page(self.ctx, self.raw) };
        Rect::from(r)
    }

    /// 页面宽度。
    pub fn width(&self) -> f32 {
        self.rect().width()
    }

    /// 页面高度。
    pub fn height(&self) -> f32 {
        self.rect().height()
    }

    /// 页面旋转角度（0/90/180/270）。
    pub fn rotation(&self) -> i32 {
        unsafe { mupdf_sys::mupdf_safe_page_rotation(self.ctx, self.raw) as i32 }
    }
}

impl Drop for Page<'_, '_> {
    fn drop(&mut self) {
        unsafe {
            mupdf_sys::mupdf_safe_drop_page(self.ctx, self.raw);
        }
    }
}
