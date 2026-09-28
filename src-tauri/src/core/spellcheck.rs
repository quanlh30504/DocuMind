//! Dictionary-based error *flagging* (not correction) — spec §2's "OCR error
//! correction suggestions" is a Local AI use case, but §20 forbids silently
//! rewriting recognized text, and MODEL_STRATEGY.md §5 holds that boundary at
//! the API level (`LocalAIProvider` returns labels, never replacement text).
//! This module is the boundary's non-AI half, shipped first: it can tell you
//! a word is *probably* wrong, it cannot tell you what the word should have
//! been. That needs real language understanding — the planned
//! `LocalAIProvider::suggest_correction` (not yet implemented; MVP_PLAN.md
//! Phase 4) — which is why every place this module's output reaches the UI
//! must say "flagged", not "fixed" or "corrected".
//!
//! Uses `zspell` (pure Rust, no native dependency) against system
//! Hunspell-format dictionaries. Dictionary discovery is a stand-in for
//! DEPENDENCY_STRATEGY.md's bundled/downloaded model story — see
//! `SYSTEM_DICT_PATHS` below.

use std::collections::HashSet;
use std::path::Path;
use zspell::Dictionary;

/// (aff_path, dic_path) candidates per language code, in priority order.
/// These are system-installed Hunspell dictionaries (`hunspell-en-us`,
/// `hunspell-vi`), not something DocuMind bundles or downloads yet — a real
/// shipped build would bundle/download word lists the same way OCR language
/// packs are handled (DEPENDENCY_STRATEGY.md §3), tracked as follow-up work.
fn system_dict_paths(lang: &str) -> Vec<(&'static str, &'static str)> {
    match lang {
        "eng" => vec![("/usr/share/hunspell/en_US.aff", "/usr/share/hunspell/en_US.dic")],
        "vie" => vec![("/usr/share/hunspell/vi_VN.aff", "/usr/share/hunspell/vi_VN.dic")],
        _ => vec![],
    }
}

pub struct SpellChecker {
    dictionaries: Vec<Dictionary>,
    /// Languages that were requested but had no dictionary available on this
    /// machine — surfaced so callers can tell "checked, found nothing wrong"
    /// apart from "couldn't check at all" (spec §46: no silent skipping).
    pub unavailable_langs: Vec<String>,
}

impl SpellChecker {
    /// `langs` are Tesseract-style codes (`"eng"`, `"vie"`; `"eng+vie"` is
    /// split automatically) matching what `TesseractOcrProvider::resolve_lang`
    /// resolved to, so the same language selection drives both OCR and
    /// flagging.
    pub fn load(langs: &str) -> Self {
        let mut dictionaries = Vec::new();
        let mut unavailable_langs = Vec::new();

        for lang in langs.split('+').map(str::trim).filter(|l| !l.is_empty()) {
            let candidates = system_dict_paths(lang);
            let loaded = candidates.iter().find_map(|(aff, dic)| {
                let aff_content = std::fs::read_to_string(Path::new(aff)).ok()?;
                let dic_content = std::fs::read_to_string(Path::new(dic)).ok()?;
                zspell::builder()
                    .config_str(&aff_content)
                    .dict_str(&dic_content)
                    .build()
                    .ok()
            });
            match loaded {
                Some(dict) => dictionaries.push(dict),
                None => unavailable_langs.push(lang.to_string()),
            }
        }

        Self {
            dictionaries,
            unavailable_langs,
        }
    }

    pub fn is_ready(&self) -> bool {
        !self.dictionaries.is_empty()
    }

    fn is_known(&self, word: &str) -> bool {
        self.dictionaries.iter().any(|d| d.check_word(word))
    }

    /// Returns distinct words in `text` that matched none of the loaded
    /// dictionaries — a heuristic signal, not a verdict: real names, terms of
    /// art, and correctly-recognized words absent from the dictionary will
    /// also appear here, alongside genuine OCR garbage. Capped so a
    /// low-quality page's flood of noise doesn't blow up `metadata.json`.
    pub fn flag_words(&self, text: &str, cap: usize) -> Vec<String> {
        if !self.is_ready() {
            return Vec::new();
        }
        let mut seen = HashSet::new();
        let mut flagged = Vec::new();
        for raw in text.split_whitespace() {
            let word: String = raw
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string();
            if word.chars().count() <= 1 || word.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            if self.is_known(&word) {
                continue;
            }
            if seen.insert(word.clone()) {
                flagged.push(word);
                if flagged.len() >= cap {
                    break;
                }
            }
        }
        flagged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_dictionary_yields_unavailable_lang_not_panic() {
        let checker = SpellChecker::load("xx");
        assert!(!checker.is_ready());
        assert_eq!(checker.unavailable_langs, vec!["xx".to_string()]);
        assert!(checker.flag_words("anything here", 10).is_empty());
    }

    #[test]
    fn english_dictionary_flags_garbage_not_real_words() {
        let checker = SpellChecker::load("eng");
        if !checker.is_ready() {
            eprintln!("skipping: no en_US hunspell dictionary on this machine");
            return;
        }
        let flagged = checker.flag_words("The quick brown fox xzqvvt", 10);
        assert!(!flagged.contains(&"quick".to_string()));
        assert!(flagged.contains(&"xzqvvt".to_string()));
    }
}
