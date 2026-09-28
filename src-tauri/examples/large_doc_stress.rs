//! Stress test for Phase 5 (MVP_PLAN.md): processes a synthetic N-page
//! image-only PDF (no native text, forces the render+OCR path every page)
//! and reports peak memory and per-page timing, to check the "bounded
//! memory regardless of document length" claim in core/job.rs's doc comment
//! against something bigger than the small fixtures used in `cargo test`.
//!
//! Run: `cargo run --release --example large_doc_stress -- <page-count>`
//! Interrupt with Ctrl+C partway through, then run again with the same
//! output dir (printed at start) to see the resume behavior on a real
//! larger document.

use documind_lib::core::document::{DocumentKind, DocumentSource};
use documind_lib::core::job;
use documind_lib::core::ocr::TesseractOcrProvider;
use std::io::Write;

/// Builds a multi-page PDF where every page is a filled rectangle (no text
/// object), so every page has no native text layer and must be rendered +
/// OCR'd — matching a real scanned book rather than a text-layer PDF.
fn make_multi_page_pdf(path: &std::path::Path, pages: u32) {
    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: String = (0..pages).map(|i| format!("{} 0 R ", 3 + i * 2)).collect();
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.trim(), pages).into_bytes());
    for _ in 0..pages {
        let page_obj_index = objects.len() as u32 + 1;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 500] /Contents {} 0 R >>",
                page_obj_index + 1
            )
            .into_bytes(),
        );
        let content = b"1 0 0 rg 40 40 200 200 re f".to_vec();
        objects.push(
            format!("<< /Length {} >>\nstream\n", content.len())
                .into_bytes()
                .into_iter()
                .chain(content)
                .chain(b"\nendstream".to_vec())
                .collect(),
        );
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets = vec![0usize];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_offset = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets[1..] {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF", objects.len() + 1, xref_offset).as_bytes(),
    );
    std::fs::write(path, out).unwrap();
}

fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
}

fn main() {
    let page_count: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(200);

    // Fixed (not pid-based) path so a second invocation with the same page
    // count reuses the same PDF/output_dir — lets this double as a manual
    // interrupt+resume check: run, Ctrl+C partway through, run again.
    let base = std::env::temp_dir().join(format!("documind-stress-{page_count}"));
    std::fs::create_dir_all(&base).unwrap();
    let pdf_path = base.join("stress.pdf");
    let output_dir = base.join("output");

    if pdf_path.exists() {
        println!("Reusing existing {page_count}-page PDF at {}", pdf_path.display());
    } else {
        println!("Generating {page_count}-page synthetic PDF...");
        make_multi_page_pdf(&pdf_path, page_count);
    }
    println!("output_dir = {}", output_dir.display());

    let ocr = TesseractOcrProvider::default();
    let source = DocumentSource { path: pdf_path, kind: DocumentKind::Pdf };

    let start = std::time::Instant::now();
    let mut last_report = std::time::Instant::now();
    let state = job::run_document(&source, &output_dir, &ocr, None, |event| {
        if last_report.elapsed().as_secs() >= 5 {
            if let job::JobEvent::PageCompleted { page, total, .. } = &event {
                let rss = peak_rss_kb().unwrap_or(0);
                println!(
                    "page {page}/{total} — elapsed {:?} — peak RSS {:.1} MB",
                    start.elapsed(),
                    rss as f64 / 1024.0
                );
                std::io::stdout().flush().ok();
            }
            last_report = std::time::Instant::now();
        }
    })
    .expect("run_document failed");

    let elapsed = start.elapsed();
    let peak = peak_rss_kb().unwrap_or(0);
    println!("\n=== done ===");
    println!("status = {:?}", state.status);
    println!("completed = {}/{}", state.completed_pages, state.total_pages);
    println!("failed = {:?}", state.failed_pages);
    println!("total elapsed = {elapsed:?} ({:.2}s/page)", elapsed.as_secs_f64() / page_count as f64);
    println!("peak RSS = {:.1} MB ({:.1} KB/page)", peak as f64 / 1024.0, peak as f64 / page_count as f64);
}
