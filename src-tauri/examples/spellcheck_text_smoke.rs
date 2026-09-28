//! Runs spellcheck flagging directly over arbitrary text (not through OCR),
//! to check flagging quality against known-garbled real-world text.
//! Run: `cargo run --example spellcheck_text_smoke -- <lang> <text-file>`

use documind_lib::core::spellcheck::SpellChecker;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lang = &args[1];
    let text = std::fs::read_to_string(&args[2]).unwrap();

    let checker = SpellChecker::load(lang);
    println!("ready={} unavailable={:?}", checker.is_ready(), checker.unavailable_langs);

    let start = std::time::Instant::now();
    let result = checker.correct_text(&text, 200);
    println!("correct_text took {:?}", start.elapsed());

    println!("{} corrections:", result.corrections.len());
    for c in &result.corrections {
        println!("  {} -> {}", c.original, c.corrected);
    }
    println!("{} unresolved (flagged, not fixed): {:?}", result.unresolved.len(), result.unresolved);
    println!("--- corrected text ---\n{}\n--- end ---", result.text);
}
