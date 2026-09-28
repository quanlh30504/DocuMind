//! Full end-to-end smoke check: a scanned (image-only) PDF through
//! job::run_document, exercising render -> preprocess -> OCR, not just the
//! native-text path. Run: `cargo run --example pdf_ocr_smoke -- <pdf-path>`

use documind_lib::core::document::{DocumentKind, DocumentSource};
use documind_lib::core::job;
use documind_lib::core::ocr::TesseractOcrProvider;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pdf_path = std::path::PathBuf::from(&args[1]);
    let output_dir = std::env::temp_dir().join("documind-pdf-ocr-smoke");
    std::fs::remove_dir_all(&output_dir).ok();

    let ocr = TesseractOcrProvider::default();
    let source = DocumentSource {
        path: pdf_path,
        kind: DocumentKind::Pdf,
    };

    let state = job::run_document(&source, &output_dir, &ocr, |event| {
        println!("event: {event:?}");
    })
    .expect("run_document failed");

    println!("status = {:?}", state.status);
    for page in &state.pages {
        println!(
            "page {} source={:?} confidence={:.2}",
            page.page, page.source, page.confidence
        );
        assert_eq!(page.source, documind_lib::core::job::PageSource::Ocr, "expected OCR path for image-only PDF");
    }

    let full_doc = std::fs::read_to_string(output_dir.join("full_document.md")).unwrap();
    println!("--- full_document.md ---\n{full_doc}\n--- end ---");
    assert!(full_doc.to_lowercase().contains("documind"));

    println!("OK");
}
