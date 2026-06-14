//! Rennie `PdfValue` —— PDF 对象模型的 Rust 表示。
//!
//! 命名取自 Rennie（PDF.js 的对象模型传统），目的是为后续 PDF 对象读写、
//! xref 操作、低层修改铺路。当前实现类型定义、最小转换，并通过 PyO3 暴露给 Python。
//!
//! PDF spec（ISO 32000-1）的对象类型只有 8 种：
//!   null / boolean / integer / real / name / string / array / dictionary
//! 加上 stream（dictionary + 字节流）就是 9 种。Rennie 在此基础上额外加 Ref，
//! 表示"对象的间接引用"，便于按 xref 解析后再解引用。
//!
//! 注意：用 `Arc<[u8]>` 而非 `Rc<Vec<u8>>` 持有字节，使 PdfValue 满足 Send/Sync
//! （PyO3 pyclass 要求）。

use std::collections::BTreeMap;
use std::sync::Arc;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString};

/// PDF 对象的 Rust 端表示。
///
/// 设计要点：
/// - 用 `Arc<[u8]>` 持有 string/stream 的字节，避免大对象在嵌套结构里被反复克隆，
///   同时满足 Send/Sync（PyO3 pyclass 要求）。
/// - `Name` 用 `Arc<str>`，频繁出现且通常很短，避免拷贝。
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
    Name(Arc<str>),

    /// PDF string（字面或 hex）。保留原始字节，编码交给上层判断。
    Str(Arc<[u8]>),

    /// PDF array（有序对象序列）。
    Array(Vec<PdfValue>),

    /// PDF dictionary（name → value 映射）。
    Dict(BTreeMap<String, PdfValue>),

    /// PDF stream（dict + 原始字节）。解压/解码由调用方按 /Filter 决定。
    Stream {
        dict: BTreeMap<String, PdfValue>,
        data: Arc<[u8]>,
    },

    /// 间接引用 `(num, gen)`：尚未解引用的指针。
    /// 调用方需通过 document xref 表解引用得到实际对象。
    Ref { num: i32, gen: i32 },
}

impl PdfValue {
    /// 构造便捷方法。
    pub fn name<S: Into<String>>(s: S) -> Self {
        PdfValue::Name(Arc::from(s.into().as_str()))
    }

    pub fn str_value(b: Vec<u8>) -> Self {
        PdfValue::Str(Arc::from(b))
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

// =========================================================================
// PyO3 暴露层：PdfValue → Python 友好的封装
// =========================================================================

/// PyPdfValue —— PdfValue 的 Python 包装类。
///
/// 在 Python 侧，`ritz.PdfValue` 让用户构造和检查 PDF 对象模型，
/// 为后续的 PDF 编辑/xref 操作铺路。
///
/// 构造方式：
///   PdfValue.null()
///   PdfValue.bool(True)
///   PdfValue.int(42)
///   PdfValue.float(3.14)
///   PdfValue.name("Page")  # 不含前导 /
///   PdfValue.str("hello")
///   PdfValue.array([v1, v2, ...])
///   PdfValue.dict({"Type": v1, "Count": v2})
///   PdfValue.stream({"Length": 10}, b"...")
///   PdfValue.ref(num, gen)
#[pyclass(name = "PdfValue", module = "_ritz")]
pub struct PyPdfValue {
    pub inner: PdfValue,
}

impl PyPdfValue {
    pub fn new(v: PdfValue) -> Self {
        PyPdfValue { inner: v }
    }
}

#[pymethods]
impl PyPdfValue {
    /// null 对象。
    #[staticmethod]
    fn null() -> Self {
        PyPdfValue::new(PdfValue::Null)
    }

    /// boolean。
    #[staticmethod]
    #[pyo3(name = "bool")]
    fn bool_value(b: bool) -> Self {
        PyPdfValue::new(PdfValue::Bool(b))
    }

    /// integer。
    #[staticmethod]
    fn int(n: i64) -> Self {
        PyPdfValue::new(PdfValue::Int(n))
    }

    /// real number。
    #[staticmethod]
    fn float(n: f64) -> Self {
        PyPdfValue::new(PdfValue::Float(n))
    }

    /// name object（不含前导 /）。
    #[staticmethod]
    fn name(s: &str) -> Self {
        PyPdfValue::new(PdfValue::name(s))
    }

    /// string。
    #[staticmethod]
    #[pyo3(name = "str")]
    fn str_value(s: &str) -> Self {
        PyPdfValue::new(PdfValue::str_value(s.as_bytes().to_vec()))
    }

    /// array。
    #[staticmethod]
    fn array(items: Vec<Py<PyPdfValue>>) -> Self {
        let v: Vec<PdfValue> = Python::attach(|py| {
            items
                .into_iter()
                .map(|r| r.borrow(py).inner.clone())
                .collect()
        });
        PyPdfValue::new(PdfValue::Array(v))
    }

    /// dict（key 顺序会按 ASCII 排序）。
    #[staticmethod]
    fn dict(d: &Bound<'_, PyDict>) -> PyResult<Self> {
        let py = d.py();
        let mut m = BTreeMap::new();
        for (k, v) in d.iter() {
            let key: String = k.extract()?;
            let val: Py<PyPdfValue> = v.extract()?;
            m.insert(key, val.borrow(py).inner.clone());
        }
        Ok(PyPdfValue::new(PdfValue::Dict(m)))
    }

    /// stream：dict + 原始字节。
    #[staticmethod]
    #[pyo3(signature = (dict, data))]
    fn stream(dict: &Bound<'_, PyDict>, data: &Bound<'_, PyBytes>) -> PyResult<Self> {
        let py = dict.py();
        let mut m = BTreeMap::new();
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            let val: Py<PyPdfValue> = v.extract()?;
            m.insert(key, val.borrow(py).inner.clone());
        }
        Ok(PyPdfValue::new(PdfValue::Stream {
            dict: m,
            data: Arc::from(data.as_bytes()),
        }))
    }

    /// 间接引用 (num, gen)。
    #[staticmethod]
    #[pyo3(name = "ref", signature = (num, gen))]
    fn ref_value(num: i32, gen: i32) -> Self {
        PyPdfValue::new(PdfValue::Ref { num, gen })
    }

    /// PDF 类型名："null"/"boolean"/"integer"/"real"/"name"/"string"/"array"/"dict"/"stream"/"ref"
    #[getter]
    fn type_name(&self) -> &'static str {
        self.inner.type_name()
    }

    /// 是否 null。
    fn is_null(&self) -> bool {
        matches!(self.inner, PdfValue::Null)
    }

    /// 是否 boolean。
    fn is_bool(&self) -> bool {
        matches!(self.inner, PdfValue::Bool(_))
    }

    /// 是否 integer 或 real。
    fn is_number(&self) -> bool {
        matches!(self.inner, PdfValue::Int(_) | PdfValue::Float(_))
    }

    /// 是否 name。
    fn is_name(&self) -> bool {
        matches!(self.inner, PdfValue::Name(_))
    }

    /// 是否 string。
    fn is_string(&self) -> bool {
        matches!(self.inner, PdfValue::Str(_))
    }

    /// 是否 array。
    fn is_array(&self) -> bool {
        matches!(self.inner, PdfValue::Array(_))
    }

    /// 是否 dict 或 stream。
    fn is_dict(&self) -> bool {
        matches!(self.inner, PdfValue::Dict(_) | PdfValue::Stream { .. })
    }

    /// 转 Python 原生对象：
    ///   null → None
    ///   bool → bool
    ///   int → int
    ///   float → float
    ///   name → str
    ///   string → str（utf-8 lossy）
    ///   array → list
    ///   dict → dict
    ///   stream → {"dict": dict, "data": bytes}
    ///   ref → (num, gen)
    fn to_python<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.inner.to_pyobject(py)
    }

    /// dict 模式：按 key 取值，找不到返回 None。
    /// 非 dict/stream 类型返回 None。
    fn get(&self, py: Python<'_>, key: &str) -> PyResult<Option<Py<PyPdfValue>>> {
        match &self.inner {
            PdfValue::Dict(m) | PdfValue::Stream { dict: m, .. } => {
                if let Some(v) = m.get(key) {
                    let obj = Py::new(py, PyPdfValue::new(v.clone()))?;
                    Ok(Some(obj))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            PdfValue::Null => "PdfValue.null()".into(),
            PdfValue::Bool(b) => format!("PdfValue.bool({})", b),
            PdfValue::Int(n) => format!("PdfValue.int({})", n),
            PdfValue::Float(n) => format!("PdfValue.float({})", n),
            PdfValue::Name(s) => format!("PdfValue.name({:?})", s.as_ref()),
            PdfValue::Str(b) => format!(
                "PdfValue.str({:?})",
                String::from_utf8_lossy(b.as_ref())
            ),
            PdfValue::Array(a) => format!("PdfValue.array(len={})", a.len()),
            PdfValue::Dict(d) => format!("PdfValue.dict(keys={})", d.len()),
            PdfValue::Stream { dict, data } => format!(
                "PdfValue.stream(keys={}, {} bytes)",
                dict.len(),
                data.len()
            ),
            PdfValue::Ref { num, gen } => format!("PdfValue.ref({}, {})", num, gen),
        }
    }
}

impl PdfValue {
    /// 递归转换为 Python 原生对象。
    pub fn to_pyobject<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match self {
            PdfValue::Null => Ok(py.None().into_bound(py).into_any()),
            PdfValue::Bool(b) => Ok(PyBool::new(py, *b).to_owned().into_any()),
            PdfValue::Int(n) => Ok(n.into_pyobject(py)?.into_any()),
            PdfValue::Float(n) => Ok(PyFloat::new(py, *n).to_owned().into_any()),
            PdfValue::Name(s) => Ok(PyString::new(py, s.as_ref()).into_any()),
            PdfValue::Str(b) => {
                let s = String::from_utf8_lossy(b.as_ref());
                Ok(PyString::new(py, &s).into_any())
            }
            PdfValue::Array(arr) => {
                let list = PyList::empty(py);
                for v in arr {
                    list.append(v.to_pyobject(py)?)?;
                }
                Ok(list.into_any())
            }
            PdfValue::Dict(m) => {
                let dict = PyDict::new(py);
                for (k, v) in m {
                    dict.set_item(k, v.to_pyobject(py)?)?;
                }
                Ok(dict.into_any())
            }
            PdfValue::Stream { dict, data } => {
                let d = PyDict::new(py);
                let inner = PyDict::new(py);
                for (k, v) in dict {
                    inner.set_item(k, v.to_pyobject(py)?)?;
                }
                d.set_item("dict", inner)?;
                d.set_item("data", PyBytes::new(py, data))?;
                Ok(d.into_any())
            }
            PdfValue::Ref { num, gen } => {
                let t = (num, gen).into_pyobject(py)?;
                Ok(t.into_any())
            }
        }
    }

    /// 从 Python 原生对象构造 PdfValue。
    pub fn from_pyobject(obj: &Bound<'_, PyAny>) -> PyResult<PdfValue> {
        // None
        if obj.is_none() {
            return Ok(PdfValue::Null);
        }
        // bool（要先于 int 检查，因为 Python bool 是 int 子类）
        if obj.is_instance_of::<PyBool>() {
            let b: bool = obj.extract()?;
            return Ok(PdfValue::Bool(b));
        }
        // int
        if obj.is_instance_of::<PyInt>() {
            let n: i64 = obj.extract()?;
            return Ok(PdfValue::Int(n));
        }
        // float
        if obj.is_instance_of::<PyFloat>() {
            let n: f64 = obj.extract()?;
            return Ok(PdfValue::Float(n));
        }
        // str
        if obj.is_instance_of::<PyString>() {
            let s: String = obj.extract()?;
            return Ok(PdfValue::str_value(s.into_bytes()));
        }
        // bytes
        if let Ok(b) = obj.cast::<PyBytes>() {
            return Ok(PdfValue::str_value(b.as_bytes().to_vec()));
        }
        // list
        if let Ok(l) = obj.cast::<PyList>() {
            let mut out = Vec::with_capacity(l.len());
            for item in l.iter() {
                out.push(PdfValue::from_pyobject(&item)?);
            }
            return Ok(PdfValue::Array(out));
        }
        // dict
        if let Ok(d) = obj.cast::<PyDict>() {
            let mut m = BTreeMap::new();
            for (k, v) in d.iter() {
                let key: String = k.extract()?;
                m.insert(key, PdfValue::from_pyobject(&v)?);
            }
            return Ok(PdfValue::Dict(m));
        }
        Err(PyTypeError::new_err(format!(
            "cannot convert {} to PdfValue (supported: None/bool/int/float/str/bytes/list/dict)",
            obj.get_type().name()?
        )))
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

    #[test]
    fn type_names() {
        assert_eq!(PdfValue::Null.type_name(), "null");
        assert_eq!(PdfValue::Bool(false).type_name(), "boolean");
        assert_eq!(PdfValue::Int(0).type_name(), "integer");
        assert_eq!(PdfValue::Float(0.0).type_name(), "real");
        assert_eq!(PdfValue::name("X").type_name(), "name");
        assert_eq!(PdfValue::str_value(b"X".to_vec()).type_name(), "string");
        assert_eq!(PdfValue::Array(vec![]).type_name(), "array");
        assert_eq!(PdfValue::Dict(BTreeMap::new()).type_name(), "dict");
        assert_eq!(
            PdfValue::Stream {
                dict: BTreeMap::new(),
                data: Arc::from(Vec::<u8>::new())
            }
            .type_name(),
            "stream"
        );
        assert_eq!(
            PdfValue::Ref { num: 1, gen: 0 }.type_name(),
            "ref"
        );
    }
}
