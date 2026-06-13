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

#[test]
fn extract_text() {
    let path = sample_pdf();
    if !path.exists() {
        eprintln!("跳过：测试 PDF 不存在");
        return;
    }

    let ctx = mupdf::Context::new().expect("创建上下文失败");
    let doc = mupdf::Document::open(&ctx, path.to_str().unwrap()).expect("打开文档失败");
    let page = doc.page(0).expect("加载第 0 页失败");

    let text = page.text().expect("提取文本失败");
    println!("文本长度: {}", text.len());
    println!("文本前 200 字符: {}", text.chars().take(200).collect::<String>());
    assert!(!text.is_empty(), "文本不应为空");

    // 测试 html / xml 模式
    let st = page.new_stext_page().expect("构造 stext 失败");
    let html = st.to_html().expect("转 html 失败");
    let xml = st.to_xml().expect("转 xml 失败");
    println!("html 长度: {}, xml 长度: {}", html.len(), xml.len());
    assert!(html.contains("<"), "html 应包含 <");
    assert!(xml.contains("<"), "xml 应包含 <");
}

#[test]
fn render_pixmap_and_png() {
    let path = sample_pdf();
    if !path.exists() {
        eprintln!("跳过：测试 PDF 不存在");
        return;
    }

    let ctx = mupdf::Context::new().expect("创建上下文失败");
    let doc = mupdf::Document::open(&ctx, path.to_str().unwrap()).expect("打开文档失败");
    let page = doc.page(0).expect("加载第 0 页失败");

    let pix = page.new_pixmap(1.0, false).expect("渲染失败");
    println!(
        "像素图: {}x{} stride={} comps={}",
        pix.width(),
        pix.height(),
        pix.stride(),
        pix.components()
    );
    assert!(pix.width() > 0 && pix.height() > 0);

    let samples = pix.samples();
    println!("samples 字节数: {}", samples.len());
    assert!(!samples.is_empty());

    let png = page.png(1.0, false).expect("转 PNG 失败");
    println!("PNG 字节数: {}", png.len());
    assert_eq!(&png[0..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A], "PNG 头部");
}

#[test]
fn json_and_box_types() {
    let path = sample_pdf();
    if !path.exists() {
        eprintln!("跳过：测试 PDF 不存在");
        return;
    }

    let ctx = mupdf::Context::new().expect("创建上下文失败");
    let doc = mupdf::Document::open(&ctx, path.to_str().unwrap()).expect("打开文档失败");
    let page = doc.page(0).expect("加载第 0 页失败");

    let st = page.new_stext_page().expect("构造 stext 失败");
    let json = st.to_json(1.0).expect("转 json 失败");
    println!("json 长度: {}", json.len());
    assert!(json.trim_start().starts_with('{'));

    let mb = page.mediabox();
    let cb = page.cropbox();
    println!("mediabox={:?} cropbox={:?}", mb, cb);
    assert!(mb.width() > 0.0 && mb.height() > 0.0);
    assert!(cb.width() > 0.0 && cb.height() > 0.0);
}
