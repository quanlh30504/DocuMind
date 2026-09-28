//! Manual smoke check for the PDF + preprocessing plumbing (core/pdf.rs,
//! core/preprocess.rs) against real fixtures, independent of the OCR engine.
//! Not part of the test suite — run explicitly:
//! `cargo run --example pdf_smoke -- <pdf-path> <image-path>`

use documind_lib::core::{pdf, preprocess};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pdf_path = Path::new(&args[1]);
    let image_path = Path::new(&args[2]);
    let out_dir = std::env::temp_dir().join("documind-pdf-smoke");
    std::fs::create_dir_all(&out_dir).unwrap();

    pdf::require_poppler().expect("poppler-utils must be installed");

    let pages = pdf::page_count(pdf_path).expect("page_count failed");
    println!("page_count = {pages}");

    let native = pdf::extract_native_text(pdf_path, 1).expect("extract_native_text failed");
    println!("native text page 1 = {native:?}");
    assert!(native.as_deref().unwrap_or("").contains("Hello DocuMind"));

    let render_stem = out_dir.join("rendered");
    let rendered = pdf::render_page(pdf_path, 1, 150, &render_stem).expect("render_page failed");
    println!("rendered page to {}", rendered.display());
    assert!(rendered.exists());

    let preprocessed = out_dir.join("preprocessed.png");
    preprocess::preprocess(image_path, &preprocessed).expect("preprocess failed");
    println!("preprocessed to {}", preprocessed.display());
    assert!(preprocessed.exists());

    println!("OK");
}
