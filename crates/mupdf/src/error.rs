use std::ffi::CStr;
use std::fmt;

#[derive(Debug, Clone)]
pub enum MuPdfError {
    AllocContext(String),
    OpenDocument(String),
    LoadPage(String),
    Metadata(String),
    Other(String),
}

impl MuPdfError {
    /// 从 C 层线程局部错误缓冲区构造错误。
    pub fn from_last_error() -> Self {
        let msg = unsafe {
            let p = mupdf_sys::mupdf_last_error();
            if p.is_null() {
                return MuPdfError::Other("unknown error (no message)".into());
            }
            CStr::from_ptr(p).to_string_lossy().into_owned()
        };
        MuPdfError::Other(msg)
    }
}

impl fmt::Display for MuPdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MuPdfError::AllocContext(m) => write!(f, "alloc context: {}", m),
            MuPdfError::OpenDocument(m) => write!(f, "open document: {}", m),
            MuPdfError::LoadPage(m) => write!(f, "load page: {}", m),
            MuPdfError::Metadata(m) => write!(f, "metadata: {}", m),
            MuPdfError::Other(m) => write!(f, "{}", m),
        }
    }
}

impl std::error::Error for MuPdfError {}

pub type Result<T> = std::result::Result<T, MuPdfError>;
