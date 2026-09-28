//! Manual smoke check for the full OCR pipeline (core/job.rs +
//! core/ocr/tesseract.rs) against a real synthetic scanned-page image.
//! Run: `cargo run --example ocr_smoke -- <image-path>`

use documind_lib::core::document::{DocumentKind, DocumentSource};
use documind_lib::core::job;
use documind_lib::core::ocr::TesseractOcrProvider;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let image_path = std::path::PathBuf::from(&args[1]);
    let output_dir = std::env::temp_dir().join("documind-ocr-smoke");
    std::fs::remove_dir_all(&output_dir).ok();

    assert!(
        TesseractOcrProvider::is_available(),
        "tesseract binary not found on PATH"
    );
    let ocr = TesseractOcrProvider::default();

    let source = DocumentSource {
        path: image_path,
        kind: DocumentKind::Image,
    };

    let state = job::run_document(&source, &output_dir, &ocr, |event| {
        println!("event: {event:?}");
    })
    .expect("run_document failed");

    println!("status = {:?}", state.status);
    for page in &state.pages {
        println!(
            "page {} source={:?} confidence={:.2} status={:?} warnings={:?}",
            page.page, page.source, page.confidence, page.status, page.warnings
        );
    }

    let full_doc = std::fs::read_to_string(output_dir.join("full_document.md")).unwrap();
    println!("--- full_document.md ---\n{full_doc}\n--- end ---");

    let lower = full_doc.to_lowercase();
    assert!(lower.contains("quick"), "expected OCR to recognize 'quick'");
    assert!(lower.contains("documind"), "expected OCR to recognize 'documind'");
    assert!(state.pages[0].confidence > 0.5, "expected reasonable confidence on clean synthetic text");

    println!("OK");
}
