use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().and_then(|p| p.parent()).unwrap();
    let mupdf_dir = workspace_root.join("vendor/mupdf");

    // 1. 校验 MuPDF 子模块存在
    if !mupdf_dir.join("include/mupdf/fitz.h").exists() {
        panic!(
            "MuPDF 源码未找到: {}\n\
             请按 README.md 指引添加子模块：\n\
             \x1b[33m  git submodule add https://github.com/ArtifexSoftware/mupdf.git vendor/mupdf\n  cd vendor/mupdf && git checkout 1.27.0\x1b[0m",
            mupdf_dir.display()
        );
    }

    let release_dir = build_mupdf(&mupdf_dir);
    link_mupdf(&release_dir);
    compile_c_wrapper(&manifest_dir, &mupdf_dir);
    generate_bindings(&manifest_dir, &mupdf_dir);

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=native/error_wrapper.c");
    println!("cargo:rerun-if-changed=native/error_wrapper.h");
    println!("cargo:rerun-if-changed=native/mupdf_extensions.c");
    println!("cargo:rerun-if-changed=native/mupdf_extensions.h");
}

/// 编译 MuPDF 静态库。
/// 关闭不需要的特性以减少体积和依赖：GLUT/X11/CURL。
fn build_mupdf(mupdf_dir: &std::path::Path) -> PathBuf {
    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "4".to_string());
    let build_dir = mupdf_dir.join("build/release");

    // 幂等：仅在 libmupdf.a 不存在时编译
    let lib = build_dir.join("libmupdf.a");
    if lib.exists() {
        return build_dir;
    }

    let mut make = Command::new("make");
    make.current_dir(mupdf_dir)
        .arg(format!("-j{}", jobs))
        .arg("build=release")
        .arg("HAVE_GLUT=no")
        .arg("HAVE_X11=no")
        .arg("HAVE_CURL=no")
        .arg("HAVE_LEPTONICA=no")
        .arg("HAVE_TESSERACT=no")
        .arg("libs"); // 只编译库，不编译 mudraw/mutool 等可执行文件

    let status = make
        .status()
        .unwrap_or_else(|e| panic!("执行 make 失败: {}", e));
    if !status.success() {
        panic!("MuPDF 编译失败，请检查工具链和第三方依赖");
    }

    build_dir
}

/// 链接 MuPDF 静态库及其第三方聚合库。
fn link_mupdf(release_dir: &std::path::Path) {
    println!("cargo:rustc-link-search=native={}", release_dir.display());

    // 链接顺序重要：mupdf 依赖 mupdf-third
    println!("cargo:rustc-link-lib=static=mupdf");

    // MuPDF 1.24+ 将第三方库聚合为 libmupdf-third.a
    let third = release_dir.join("libmupdf-third.a");
    if third.exists() {
        println!("cargo:rustc-link-lib=static=mupdf-third");
    } else {
        // 旧版本需单独链接
        println!("cargo:rustc-link-lib=static=freetype");
        println!("cargo:rustc-link-lib=static=harfbuzz");
        println!("cargo:rustc-link-lib=static=jbig2dec");
        println!("cargo:rustc-link-lib=static=openjp2");
        println!("cargo:rustc-link-lib=static=jpeg");
        println!("cargo:rustc-link-lib=static=gumbo");
        println!("cargo:rustc-link-lib=static=z");
    }

    // 平台系统库
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    match target_os.as_str() {
        "linux" => {
            println!("cargo:rustc-link-lib=pthread");
            println!("cargo:rustc-link-lib=dl");
            println!("cargo:rustc-link-lib=m");
        }
        "macos" => {
            println!("cargo:rustc-link-lib=iconv");
            println!("cargo:rustc-link-lib=framework=Foundation");
        }
        "windows" => {
            println!("cargo:rustc-link-lib=ws2_32");
            println!("cargo:rustc-link-lib=user32");
            println!("cargo:rustc-link-lib=gdi32");
            println!("cargo:rustc-link-lib=advapi32");
        }
        _ => {}
    }
}

/// 编译手写 C 包装层。
fn compile_c_wrapper(manifest_dir: &std::path::Path, mupdf_dir: &std::path::Path) {
    let mut build = cc::Build::new();
    build
        .file(manifest_dir.join("native/error_wrapper.c"))
        .file(manifest_dir.join("native/mupdf_extensions.c"))
        .include(manifest_dir.join("native"))
        .include(mupdf_dir.join("include"));

    // MuPDF 内部用大量未使用变量等告警，C 层无需 -Werror
    build.warnings(false);

    build.compile("mupdf_wrapper");
}

/// 生成 Rust FFI 绑定。
/// 默认使用预生成的 bindings.rs（CI 无需 libclang）。
/// `bindgen` feature 重新生成。
#[cfg(feature = "bindgen")]
fn generate_bindings(manifest_dir: &std::path::Path, mupdf_dir: &std::path::Path) {
    use std::env;

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let bindings = bindgen::Builder::default()
        .header(manifest_dir.join("wrapper.h").to_str().unwrap())
        .clang_arg(format!("-I{}", manifest_dir.join("native").display()))
        .clang_arg(format!("-I{}", mupdf_dir.join("include").display()))
        // 只生成我们用到的符号，避免绑定过亿行
        .allowlist_function("mupdf_.*")
        .allowlist_function("fz_.*")
        .allowlist_function("pdf_.*")
        .allowlist_type("fz_.*")
        .allowlist_type("pdf_.*")
        .allowlist_var("FZ_.*")
        .allowlist_var("PDF_.*")
        // fz_try/fz_catch 是宏，不应绑定
        .blocklist_item("fz_try")
        .blocklist_item("fz_catch")
        .blocklist_item("fz_always")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("bindgen 生成失败");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("写入 bindings.rs 失败");
}

#[cfg(not(feature = "bindgen"))]
fn generate_bindings(manifest_dir: &std::path::Path, _mupdf_dir: &std::path::Path) {
    use std::env;

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let pregenerated = manifest_dir.join("bindings.rs");

    if pregenerated.exists() {
        // 使用签入的预生成绑定
        std::fs::copy(&pregenerated, out_dir.join("bindings.rs"))
            .expect("复制预生成 bindings.rs 失败");
    } else {
        panic!(
            "未找到预生成的 bindings.rs，也未启用 bindgen feature。\n\
             请运行：\n\
             \x1b[33m  cargo build -p mupdf-sys --features bindgen\x1b[0m\n\
             生成后将 crates/mupdf-sys/bindings.rs 签入仓库。"
        );
    }
}
