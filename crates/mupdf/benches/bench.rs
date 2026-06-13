//! 性能基准：核心 mupdf 操作的微基准。
//!
//! 运行：cargo bench -p mupdf
//! 报告告：target/criterion/report/index.html
//!
//! 基准场景：
//!   1. open + page_count
//!   2. load_page + bound_page（page 0）
//!   3. new_stext_page + text（提取文本）
//!   4. new_pixmap（渲染 1.0 zoom）
//!
//! 注：criterion 默认跑 100 个样本 + 统计分析，单 bench 约 5-10 秒。

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use mupdf::{Context, Document};
use std::path::PathBuf;

fn sample_pdf() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("benches/samples/sample.pdf");
    p
}

fn bigger_pdf() -> Option<PathBuf> {
    // 尝试找一个多页 PDF（用于端到端 benchmark）
    let candidates = [
        "../../vendor/mupdf/thirdparty/extract/test/agstat.pdf",
        "../../vendor/mupdf/thirdparty/brotli/docs/brotli-comparison-study-2015-09-22.pdf",
        "../../vendor/mupdf/thirdparty/extract/test/column_span_2.pdf",
        "../../vendor/mupdf/thirdparty/extract/test/electoral_roll.pdf",
        "../../vendor/mupdf/thirdparty/extract/test/twotables_2.pdf",
        "../../vendor/mupdf/thirdparty/extract/test/row_span.pdf",
        "../../vendor/mupdf/thirdparty/extract/test/text_graphic_image.pdf",
        "../../vendor/mupdf/thirdparty/extract/test/Python2clipped.pdf",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn bench_open(c: &mut Criterion) {
    let path = sample_pdf();
    let mut group = c.benchmark_group("open");
    group.bench_function("open+count", |b| {
        b.iter(|| {
            let ctx = Context::new().unwrap();
            let doc = Document::open(&ctx, path.to_str().unwrap()).unwrap();
            let _ = doc.page_count().unwrap();
            drop(doc);
            drop(ctx);
        })
    });
    group.finish();
}

fn bench_text(c: &mut Criterion) {
    let path = sample_pdf();
    let ctx = Context::new().unwrap();
    let doc = Document::open(&ctx, path.to_str().unwrap()).unwrap();
    let page = doc.page(0).unwrap();

    let mut group = c.benchmark_group("extract_text");
    group.bench_function("text", |b| {
        b.iter(|| {
            let _ = page.text().unwrap();
        })
    });
    group.finish();

    drop(page);
    drop(doc);
    drop(ctx);
}

fn bench_pixmap(c: &mut Criterion) {
    let path = sample_pdf();
    let ctx = Context::new().unwrap();
    let doc = Document::open(&ctx, path.to_str().unwrap()).unwrap();
    let page = doc.page(0).unwrap();

    let mut group = c.benchmark_group("render_pixmap");

    for zoom in [0.5f32, 1.0f32, 2.0f32] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("zoom={}", zoom)),
            &zoom,
            |b, &zoom| {
                b.iter(|| {
                    let _pix = page.new_pixmap(zoom, false).unwrap();
                })
            },
        );
    }
    group.finish();

    drop(page);
    drop(doc);
    drop(ctx);
}

/// 多页 PDF 上的端到端 benchmark：open + extract all pages text。
fn bench_e2e_multi_page(c: &mut Criterion) {
    let path = match bigger_pdf() {
        Some(p) => p,
        None => {
            return;
        }
    };

    let mut group = c.benchmark_group("e2e_multi_page");
    group.bench_function("extract_all_text", |b| {
        b.iter(|| {
            let ctx = Context::new().unwrap();
            let doc = Document::open(&ctx, path.to_str().unwrap()).unwrap();
            let n = doc.page_count().unwrap();
            for i in 0..n {
                let page = doc.page(i).unwrap();
                let _ = page.text().unwrap();
            }
            drop(doc);
            drop(ctx);
        })
    });
    group.finish();
}

criterion_group!(benches, bench_open, bench_text, bench_pixmap, bench_e2e_multi_page);
criterion_main!(benches);
