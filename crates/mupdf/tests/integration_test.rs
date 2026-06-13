//! 阶段一集成测试：打开 PDF → 取页数 → 加载页 → 取尺寸。
//!
//! 运行：cargo test --test integration_test
//!
//! 默认使用 tests/samples/sample.pdf，可通过 MUPDF_TEST_PDF 环境变量指定。

use std::path::PathBuf;

fn sample_pdf() -> PathBuf {
    if let Ok(p) = std::env::var("MUPDF_TEST_PDF") {
        return PathBuf::from(p);
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/samples/sample.pdf");
    p
}

#[test]
fn open_and_count_pages() {
    let path = sample_pdf();
    if !path.exists() {
        eprintln!("跳过：测试 PDF 不存在 ({})。请放置测试 PDF 或设置 MUPDF_TEST_PDF。", path.display());
        return;
    }

    let ctx = mupdf::Context::new().expect("创建上下文失败");
    let doc = mupdf::Document::open(&ctx, path.to_str().unwrap()).expect("打开文档失败");

    let count = doc.page_count().expect("取页数失败");
    println!("页数: {}", count);
    assert!(count > 0, "页数应大于 0");
}

#[test]
fn load_page_and_rect() {
    let path = sample_pdf();
    if !path.exists() {
        eprintln!("跳过：测试 PDF 不存在");
        return;
    }

    let ctx = mupdf::Context::new().expect("创建上下文失败");
    let doc = mupdf::Document::open(&ctx, path.to_str().unwrap()).expect("打开文档失败");

    let page = doc.page(0).expect("加载第 0 页失败");
    let rect = page.rect();
    println!("页面尺寸: {:.1} x {:.1} (rect={:?})", rect.width(), rect.height(), rect);
    assert!(rect.width() > 0.0, "宽度应大于 0");
    assert!(rect.height() > 0.0, "高度应大于 0");
}

#[test]
fn metadata() {
    let path = sample_pdf();
    if !path.exists() {
        eprintln!("跳过：测试 PDF 不存在");
        return;
    }

    let ctx = mupdf::Context::new().expect("创建上下文失败");
    let doc = mupdf::Document::open(&ctx, path.to_str().unwrap()).expect("打开文档失败");

    // format 字段几乎总有值
    let format = doc.metadata("format");
    println!("format: {:?}", format);
}

#[test]
fn context_clone_for_thread() {
    // fz_clone_context 需要锁函数（阶段四实现）。
    // 阶段一仅验证它不会 crash——可能返回 Err 是预期行为。
    let ctx = mupdf::Context::new().expect("创建上下文失败");
    match ctx.clone_for_thread() {
        Ok(clone) => {
            println!("克隆成功");
            drop(clone);
        }
        Err(e) => {
            println!("克隆返回错误（阶段四提供锁函数后修复）: {}", e);
        }
    }
    drop(ctx);
}

#[test]
fn open_nonexistent_file_errors() {
    let ctx = mupdf::Context::new().expect("创建上下文失败");
    let result = mupdf::Document::open(&ctx, "/nonexistent/file.pdf");
    assert!(result.is_err(), "打开不存在的文件应返回错误");
    if let Err(e) = result {
        println!("错误消息: {}", e);
    }
}
