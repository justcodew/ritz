//! fz_page 的 RAII 封装，借用 Document 的生命周期。

use crate::error::{MuPdfError, Result};
use crate::geometry::Rect;
use crate::pixmap::Pixmap;
use crate::stext::STextPage;
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

    /// 指定 box 类型：0=media 1=crop 2=bleed 3=trim 4=art。
    pub fn bound_box(&self, box_type: i32) -> Rect {
        let r = unsafe { mupdf_sys::mupdf_safe_bound_page_box(self.ctx, self.raw, box_type) };
        Rect::from(r)
    }

    /// mediabox（与 PyMuPDF Page.mediabox 对应）。
    pub fn mediabox(&self) -> Rect {
        self.bound_box(0)
    }

    /// cropbox（与 PyMuPDF Page.cropbox 对应；fz_bound_page 默认就是 cropbox）。
    pub fn cropbox(&self) -> Rect {
        self.bound_box(1)
    }

    /// 构造本页的结构化文本，可导出为 text/html/xml。
    pub fn new_stext_page(&self) -> Result<STextPage<'_, 'doc, 'ctx>> {
        let mut raw: *mut mupdf_sys::fz_stext_page = std::ptr::null_mut();
        let rc = unsafe { mupdf_sys::mupdf_safe_new_stext_page(self.ctx, self.raw, &mut raw) };
        if rc != 0 || raw.is_null() {
            return Err(MuPdfError::STextPage(
                MuPdfError::from_last_error().to_string(),
            ));
        }
        Ok(STextPage::from_raw(raw, self.ctx))
    }

    /// 便捷：直接获取本页文本（默认纯文本）。
    pub fn text(&self) -> Result<String> {
        self.new_stext_page()?.to_text()
    }

    /// 渲染本页为像素图。
    ///
    /// * `zoom` - 1.0 = 72 DPI 原始尺寸
    /// * `alpha` - 是否带 alpha 通道
    pub fn new_pixmap(&self, zoom: f32, alpha: bool) -> Result<Pixmap<'_, 'doc, 'ctx>> {
        let mut raw: *mut mupdf_sys::fz_pixmap = std::ptr::null_mut();
        let rc = unsafe {
            mupdf_sys::mupdf_safe_render_pixmap(
                self.ctx,
                self.raw,
                zoom,
                alpha as std::os::raw::c_int,
                &mut raw,
            )
        };
        if rc != 0 || raw.is_null() {
            return Err(MuPdfError::Render(
                MuPdfError::from_last_error().to_string(),
            ));
        }
        Ok(Pixmap::from_raw(raw, self.ctx))
    }

    /// 便捷：渲染并直接编码为 PNG。
    pub fn png(&self, zoom: f32, alpha: bool) -> Result<Vec<u8>> {
        self.new_pixmap(zoom, alpha)?.to_png()
    }
}

impl Drop for Page<'_, '_> {
    fn drop(&mut self) {
        unsafe {
            mupdf_sys::mupdf_safe_drop_page(self.ctx, self.raw);
        }
    }
}
