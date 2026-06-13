//! Rennie `PdfValue` —— PDF 对象模型的 Rust 表示。
//!
//! 命名取自 Rennie（PDF.js 的对象模型传统），目的是为后续 PDF 对象读写、
//! xref 操作、低层修改铺路。当前仅做类型定义和最小转换，不绑定到 FFI。
//!
//! PDF spec（ISO 32000-1）的对象类型只有 8 种：
//!   null / boolean / integer / real / name / string / array / dictionary
//! 加上 stream（dictionary + 字节流）就是 9 种。Rennie 在此基础上额外加 Ref，
//! 表示"对象的间接引用"，便于按 xref 解析后再解引用。

use std::collections::BTreeMap;

/// PDF 对象的 Rust 端表示。
///
/// 设计要点：
/// - 用 `Rc<Vec<u8>>` 而不是 `Vec<u8>` 持有 string/stream 的字节，
///   避免大对象在嵌套结构里被反复克隆。
/// - `Name` 单独成 variant，因为 PDF name 在对象图中频繁出现且通常很短，
///   用 `Arc<str>` 持有避免拷贝。
/// - `Dict` 用 `BTreeMap` 而非 `HashMap`：key 顺序按 ASCII 排序，
///   便于稳定测试和确定性输出。PyMuPDF 的 dict 是有序的（按 PDF 文件出现顺序），
///   后续若需要严格保序可换 `IndexMap`。
#[derive(Debug, Clone)]
pub enum PdfValue {
    /// PDF null 对象。
    Null,

    /// PDF boolean（true/false）。
    Bool(bool),

    /// PDF integer（spec 允许 ±2^31，实际多用 i32）。
    Int(i64),

    /// PDF real number（IEEE 754 双精度足够覆盖 spec 精度）。
    Float(f64),

    /// PDF name object，如 `/Type`、`/Page`。存储时不含前导 `/`。
    Name(std::sync::Arc<str>),

    /// PDF string（字面或 hex）。保留原始字节，编码交给上层判断。
    Str(std::rc::Rc<Vec<u8>>),

    /// PDF array（有序对象序列）。
    Array(Vec<PdfValue>),

    /// PDF dictionary（name → value 映射）。
    Dict(BTreeMap<String, PdfValue>),

    /// PDF stream（dict + 原始字节）。解压/解码由调用方按 /Filter 决定。
    Stream {
        dict: BTreeMap<String, PdfValue>,
        data: std::rc::Rc<Vec<u8>>,
    },

    /// 间接引用 `(num, gen)`：尚未解引用的指针。
    /// 调用方需通过 document xref 表解引用得到实际对象。
    Ref { num: i32, gen: i32 },
}

impl PdfValue {
    /// 构造便捷方法。
    pub fn name<S: Into<String>>(s: S) -> Self {
        PdfValue::Name(std::sync::Arc::from(s.into().as_str()))
    }

    pub fn str_value(b: Vec<u8>) -> Self {
        PdfValue::Str(std::rc::Rc::new(b))
    }

    pub fn int(n: i64) -> Self {
        PdfValue::Int(n)
    }

    pub fn float(n: f64) -> Self {
        PdfValue::Float(n)
    }

    pub fn bool_value(b: bool) -> Self {
        PdfValue::Bool(b)
    }

    /// 类型判别。
    pub fn is_null(&self) -> bool {
        matches!(self, PdfValue::Null)
    }

    pub fn is_array(&self) -> bool {
        matches!(self, PdfValue::Array(_))
    }

    /// 按 PDF 类型名返回（用于错误消息和 Python __repr__）。
    pub fn type_name(&self) -> &'static str {
        match self {
            PdfValue::Null => "null",
            PdfValue::Bool(_) => "boolean",
            PdfValue::Int(_) => "integer",
            PdfValue::Float(_) => "real",
            PdfValue::Name(_) => "name",
            PdfValue::Str(_) => "string",
            PdfValue::Array(_) => "array",
            PdfValue::Dict(_) => "dict",
            PdfValue::Stream { .. } => "stream",
            PdfValue::Ref { .. } => "ref",
        }
    }
}

impl From<bool> for PdfValue {
    fn from(b: bool) -> Self {
        PdfValue::Bool(b)
    }
}

impl From<i64> for PdfValue {
    fn from(n: i64) -> Self {
        PdfValue::Int(n)
    }
}

impl From<i32> for PdfValue {
    fn from(n: i32) -> Self {
        PdfValue::Int(n as i64)
    }
}

impl From<f64> for PdfValue {
    fn from(n: f64) -> Self {
        PdfValue::Float(n)
    }
}

impl From<&str> for PdfValue {
    fn from(s: &str) -> Self {
        PdfValue::str_value(s.as_bytes().to_vec())
    }
}

impl From<String> for PdfValue {
    fn from(s: String) -> Self {
        PdfValue::str_value(s.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        let mut d = BTreeMap::new();
        d.insert("Type".into(), PdfValue::name("Page"));
        d.insert("Count".into(), PdfValue::int(3));
        let v = PdfValue::Dict(d);
        assert_eq!(v.type_name(), "dict");
        if let PdfValue::Dict(m) = &v {
            assert_eq!(m.get("Type").map(|x| x.type_name()), Some("name"));
            assert_eq!(m.get("Count").map(|x| x.type_name()), Some("integer"));
        }
    }

    #[test]
    fn conversions() {
        let v: PdfValue = true.into();
        assert!(matches!(v, PdfValue::Bool(true)));
        let v: PdfValue = 42i32.into();
        assert!(matches!(v, PdfValue::Int(42)));
        let v: PdfValue = "hello".into();
        assert!(matches!(v, PdfValue::Str(_)));
    }
}
