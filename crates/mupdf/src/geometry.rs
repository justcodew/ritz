//! 几何值类型，与 MuPDF C 结构体二进制兼容（#[repr(C)]）。

use mupdf_sys::{fz_matrix, fz_point, fz_rect};

/// 矩形（左下原点坐标系，MuPDF 内部用）
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Rect { x0, y0, x1, y1 }
    }
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }
    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
}

impl From<fz_rect> for Rect {
    fn from(r: fz_rect) -> Self {
        Rect { x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y1 }
    }
}

impl From<Rect> for fz_rect {
    fn from(r: Rect) -> Self {
        fz_rect { x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y1 }
    }
}

/// 2D 变换矩阵
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn new(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Matrix { a, b, c, d, e, f }
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        Matrix { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
    }
}

impl From<fz_matrix> for Matrix {
    fn from(m: fz_matrix) -> Self {
        Matrix { a: m.a, b: m.b, c: m.c, d: m.d, e: m.e, f: m.f }
    }
}

/// 点
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

impl From<fz_point> for Point {
    fn from(p: fz_point) -> Self {
        Point { x: p.x, y: p.y }
    }
}
