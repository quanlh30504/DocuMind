//! Smoke check for language selection/auto-detect (core/ocr/tesseract.rs).
//! Run: `cargo run --example lang_smoke -- <image-path> [lang-or-auto]`

use documind_lib::core::ocr::TesseractOcrProvider;
use documind_lib::core::traits::OCRProvider;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let image_path = std::path::PathBuf::from(&args[1]);
    let selection = args.get(2).map(|s| s.as_str());

    println!("installed_langs = {:?}", TesseractOcrProvider::installed_langs());
    let resolved = TesseractOcrProvider::resolve_lang(selection);
    println!("selection={selection:?} -> resolved lang={resolved}");

    let ocr = TesseractOcrProvider::new(resolved);
    let result = ocr.recognize(&image_path).expect("recognize failed");
    println!("confidence = {:.2}", result.confidence);
    println!("--- text ---\n{}\n--- end ---", result.text);
}
