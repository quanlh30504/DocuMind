//! Smoke check for the dictionary flagging feature wired into the real
//! pipeline (core/job.rs + core/spellcheck.rs), matching what
//! process_document does. Run:
//! `cargo run --example spellcheck_smoke -- <image-path> <lang>`

use documind_lib::core::document::{DocumentKind, DocumentSource};
use documind_lib::core::job;
use documind_lib::core::ocr::TesseractOcrProvider;
use documind_lib::core::spellcheck::SpellChecker;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let image_path = std::path::PathBuf::from(&args[1]);
    let lang = args.get(2).map(|s| s.as_str()).unwrap_or("auto");
    let output_dir = std::env::temp_dir().join("documind-spellcheck-smoke");
    std::fs::remove_dir_all(&output_dir).ok();

    let resolved_lang = TesseractOcrProvider::resolve_lang(Some(lang));
    println!("resolved lang = {resolved_lang}");
    let ocr = TesseractOcrProvider::new(resolved_lang.clone());
    let checker = SpellChecker::load(&resolved_lang);
    println!(
        "spellcheck ready={} unavailable={:?}",
        checker.is_ready(),
        checker.unavailable_langs
    );

    let source = DocumentSource {
        path: image_path,
        kind: DocumentKind::Image,
    };

    let state = job::run_document(&source, &output_dir, &ocr, Some(&checker), |_| {})
        .expect("run_document failed");

    for page in &state.pages {
        println!("page {} warnings={:?}", page.page, page.warnings);
        println!("page {} flagged_words={:?}", page.page, page.flagged_words);
    }
}
