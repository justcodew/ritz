//! MuPDF 原始 FFI 绑定。
//!
//! 分两种模式（由 build.rs 决定）：
//! - `bindgen` feature：从 MuPDF 头文件自动生成完整绑定
//! - 默认：使用预生成/手写的 bindings.rs（CI 无需 libclang）

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(deref_nullptr)]
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
