//! Dictionary-based error detection *and* correction.
//!
//! Validity checking uses `zspell` (pure Rust, affix-aware — correctly
//! accepts inflected forms not literally present in the flat word list).
//! Suggestion/correction does not: `zspell` 0.5.5 does not expose a public
//! suggestion API yet (its own docs call it "currently unstable"), so this
//! module parses the same Hunspell `.dic` word list directly and finds the
//! closest match by edit distance itself (see `suggest_within`).
//!
//! This is still not real language understanding — it is nearest-neighbor
//! string matching against a flat word list, with no grammar, context, or
//! semantics. It will confidently "fix" a rare-but-correct word into a
//! common-but-wrong one, and it will fail to fix badly garbled OCR whose
//! edit distance from the intended word is large (spec §2's "OCR error
//! correction suggestions" ultimately wants a `LocalAIProvider` for that —
//! MVP_PLAN.md Phase 4, not yet implemented). Every correction this module
//! makes is recorded (original -> corrected) rather than applied silently
//! (spec §46), and words it can't confidently fix are surfaced as
//! `flagged_words` instead of being dropped.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use zspell::Dictionary;

/// (aff_path, dic_path) candidates per language code, in priority order.
/// System-installed Hunspell dictionaries (`hunspell-en-us`, `hunspell-vi`)
/// for now — see DEPENDENCY_STRATEGY.md §3 for how this should eventually be
/// bundled/downloaded like OCR language packs instead.
fn system_dict_paths(lang: &str) -> Vec<(&'static str, &'static str)> {
    match lang {
        "eng" => vec![("/usr/share/hunspell/en_US.aff", "/usr/share/hunspell/en_US.dic")],
        "vie" => vec![("/usr/share/hunspell/vi_VN.aff", "/usr/share/hunspell/vi_VN.dic")],
        _ => vec![],
    }
}

/// Max edit distance (Damerau-Levenshtein, restricted/OSA variant — handles
/// adjacent-character transpositions, a common OCR failure mode) accepted as
/// a confident correction, scaled to word length so short words need a near-
/// exact match (avoids "a" -> "at"-style overreach) while longer words tolerate
/// more OCR noise.
fn max_distance_for(len: usize) -> usize {
    match len {
        0..=3 => 1,
        4..=6 => 2,
        _ => 3,
    }
}

/// Per-language word list used only for suggestion search (see module doc —
/// `zspell` handles validity checking instead, which is affix-aware).
struct WordIndex {
    /// Words bucketed by char length, so a candidate search only compares
    /// against words of plausibly similar length instead of the whole list.
    by_length: HashMap<usize, Vec<String>>,
    word_count: usize,
}

/// Below this many entries, a dictionary's *validity checking* still works
/// (a word it does contain really is valid), but its *absence* stops being
/// meaningful evidence a word is wrong — too many genuinely correct words
/// are simply missing. Found empirically: Ubuntu's `hunspell-vi` package
/// ships only ~6,600 words (vs ~79,000 for `hunspell-en-us`) and doesn't even
/// contain common words like "hóa"/"hòa"/"thỏa", which were being
/// "auto-corrected" into wrong words as a result. Below this threshold,
/// `SpellChecker` still flags (spec §46: better to under-claim than corrupt
/// correct text), but does not auto-apply a correction from that language's
/// word list — see `SpellChecker::load`'s `low_coverage_langs`.
const MIN_WORDS_FOR_AUTOCORRECT: usize = 20_000;

impl WordIndex {
    fn parse(dic_content: &str) -> Self {
        let mut by_length: HashMap<usize, Vec<String>> = HashMap::new();
        let mut word_count = 0;
        for line in dic_content.lines().skip(1) {
            // Hunspell .dic format: `word` or `word/AFFIXFLAGS`.
            let word = line.split('/').next().unwrap_or("").trim();
            if word.is_empty() || !word.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '-') {
                continue;
            }
            word_count += 1;
            by_length.entry(word.chars().count()).or_default().push(word.to_string());
        }
        Self { by_length, word_count }
    }

    /// Closest candidate to `word` (compared case-insensitively) within its
    /// length-appropriate distance threshold, or `None` if nothing is close
    /// enough to be a confident correction.
    fn suggest(&self, word: &str) -> Option<String> {
        let target: Vec<char> = word.to_lowercase().chars().collect();
        let max_dist = max_distance_for(target.len());
        // Tie-break key: (distance, first-char mismatch?, |length diff|,
        // candidate has intrinsic capitalization?). OCR/typo errors rarely
        // land on the first letter, so on a distance tie prefer
        // same-first-letter candidates, then closer length; a final tiebreak
        // prefers plain lowercase dictionary entries over ones stored with
        // capitals (proper nouns/acronyms like "TeX"), since a garbled common
        // word is statistically more likely to have been a common word.
        let mut best: Option<((usize, u8, usize, u8), &str)> = None;

        for len in target.len().saturating_sub(max_dist)..=(target.len() + max_dist) {
            let Some(candidates) = self.by_length.get(&len) else { continue };
            let len_diff = len.abs_diff(target.len());
            for candidate in candidates {
                let candidate_lower: Vec<char> = candidate.to_lowercase().chars().collect();
                let dist = damerau_levenshtein(&target, &candidate_lower, max_dist);
                let Some(dist) = dist else { continue };
                let first_mismatch = u8::from(candidate_lower.first() != target.first());
                let has_caps = u8::from(candidate.chars().any(char::is_uppercase));
                let key = (dist, first_mismatch, len_diff, has_caps);
                if best.is_none_or(|(best_key, _)| key < best_key) {
                    best = Some((key, candidate.as_str()));
                }
            }
        }

        best.filter(|((dist, ..), _)| *dist <= max_dist && *dist > 0)
            .map(|(_, w)| w.to_string())
    }
}

/// Restricted (optimal string alignment) Damerau-Levenshtein distance,
/// early-exiting once it's clear the distance will exceed `max`, since this
/// runs per-candidate over a large word list.
fn damerau_levenshtein(a: &[char], b: &[char], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let (m, n) = (a.len(), b.len());
    let mut prev2 = vec![0usize; n + 1];
    let mut prev = (0..=n).collect::<Vec<_>>();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        let mut row_min = curr[0];
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let mut val = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                val = val.min(prev2[j - 2] + 1);
            }
            curr[j] = val;
            row_min = row_min.min(val);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut curr);
    }
    Some(prev[n])
}

/// Applies `sample`'s capitalization pattern (all-caps / capitalized /
/// lowercase) to `word`, so correcting "TRIÉN" yields "TRIỂN", not "triển".
fn match_case(sample: &str, word: &str) -> String {
    let letters: Vec<char> = sample.chars().filter(|c| c.is_alphabetic()).collect();
    if !letters.is_empty() && letters.iter().all(|c| c.is_uppercase()) {
        word.to_uppercase()
    } else if sample.chars().next().is_some_and(char::is_uppercase) {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => word.to_string(),
        }
    } else {
        word.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionRecord {
    pub original: String,
    pub corrected: String,
}

pub struct CorrectionResult {
    pub text: String,
    pub corrections: Vec<CorrectionRecord>,
    /// Flagged as probably wrong but no confident correction was found.
    pub unresolved: Vec<String>,
}

pub struct SpellChecker {
    dictionaries: Vec<Dictionary>,
    /// Only word lists with >= `MIN_WORDS_FOR_AUTOCORRECT` entries — used for
    /// suggestion search. A sparse dictionary still contributes to
    /// `dictionaries` (validity checking / flagging), just not to
    /// suggestions, since its "not found" verdict isn't reliable enough to
    /// act on (see `MIN_WORDS_FOR_AUTOCORRECT`'s doc comment).
    suggest_indices: Vec<WordIndex>,
    /// Languages requested but with no dictionary available on this machine
    /// — surfaced so callers can tell "checked, found nothing" apart from
    /// "couldn't check at all" (spec §46: no silent skipping).
    pub unavailable_langs: Vec<String>,
    /// Languages whose dictionary loaded but was too sparse to trust for
    /// auto-correction (see `MIN_WORDS_FOR_AUTOCORRECT`) — words in these
    /// languages are still flagged, never auto-corrected.
    pub low_coverage_langs: Vec<String>,
}

impl SpellChecker {
    /// `langs` are Tesseract-style codes (`"eng"`, `"vie"`; `"eng+vie"` is
    /// split automatically) matching what `TesseractOcrProvider::resolve_lang`
    /// resolved to, so the same language selection drives OCR, flagging and
    /// correction together.
    pub fn load(langs: &str) -> Self {
        let mut dictionaries = Vec::new();
        let mut suggest_indices = Vec::new();
        let mut unavailable_langs = Vec::new();
        let mut low_coverage_langs = Vec::new();

        for lang in langs.split('+').map(str::trim).filter(|l| !l.is_empty()) {
            let candidates = system_dict_paths(lang);
            let loaded = candidates.iter().find_map(|(aff, dic)| {
                let aff_content = std::fs::read_to_string(Path::new(aff)).ok()?;
                let dic_content = std::fs::read_to_string(Path::new(dic)).ok()?;
                let dict = zspell::builder()
                    .config_str(&aff_content)
                    .dict_str(&dic_content)
                    .build()
                    .ok()?;
                Some((dict, WordIndex::parse(&dic_content)))
            });
            match loaded {
                Some((dict, index)) => {
                    dictionaries.push(dict);
                    if index.word_count >= MIN_WORDS_FOR_AUTOCORRECT {
                        suggest_indices.push(index);
                    } else {
                        low_coverage_langs.push(lang.to_string());
                    }
                }
                None => unavailable_langs.push(lang.to_string()),
            }
        }

        Self {
            dictionaries,
            suggest_indices,
            unavailable_langs,
            low_coverage_langs,
        }
    }

    pub fn is_ready(&self) -> bool {
        !self.dictionaries.is_empty()
    }

    fn is_known(&self, word: &str) -> bool {
        self.dictionaries.iter().any(|d| d.check_word(word))
    }

    fn suggest(&self, word: &str) -> Option<String> {
        self.suggest_indices
            .iter()
            .filter_map(|idx| idx.suggest(word))
            .min_by_key(|s| damerau_levenshtein(
                &word.to_lowercase().chars().collect::<Vec<_>>(),
                &s.to_lowercase().chars().collect::<Vec<_>>(),
                usize::MAX,
            ).unwrap_or(usize::MAX))
    }

    /// Distinct words in `text` not found in any loaded dictionary — a
    /// heuristic signal, not a verdict (real names/terms of art will also
    /// appear here). Used where only detection, not correction, is wanted.
    pub fn flag_words(&self, text: &str, cap: usize) -> Vec<String> {
        if !self.is_ready() {
            return Vec::new();
        }
        let mut seen = HashSet::new();
        let mut flagged = Vec::new();
        for raw in text.split_whitespace() {
            let word: String = raw.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
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

    /// Applies confident corrections in place (preserving original spacing
    /// and capitalization pattern) and returns the corrected text alongside
    /// a full record of what changed, plus words that were flagged but left
    /// untouched because no confident correction was found.
    pub fn correct_text(&self, text: &str, cap: usize) -> CorrectionResult {
        if !self.is_ready() {
            return CorrectionResult { text: text.to_string(), corrections: vec![], unresolved: vec![] };
        }

        let mut out = String::with_capacity(text.len());
        let mut corrections = Vec::new();
        let mut unresolved = Vec::new();
        let mut seen_unresolved = HashSet::new();

        for token in tokenize(text) {
            match token {
                Token::Space(s) => out.push_str(s),
                Token::Word { leading, core, trailing } => {
                    out.push_str(leading);
                    if core.chars().count() <= 1 || self.is_known(core) {
                        out.push_str(core);
                    } else if let Some(suggestion) = self.suggest(core) {
                        let cased = match_case(core, &suggestion);
                        corrections.push(CorrectionRecord {
                            original: core.to_string(),
                            corrected: cased.clone(),
                        });
                        out.push_str(&cased);
                    } else {
                        out.push_str(core);
                        if unresolved.len() < cap && seen_unresolved.insert(core.to_string()) {
                            unresolved.push(core.to_string());
                        }
                    }
                    out.push_str(trailing);
                }
            }
        }

        CorrectionResult { text: out, corrections, unresolved }
    }
}

enum Token<'a> {
    Space(&'a str),
    Word { leading: &'a str, core: &'a str, trailing: &'a str },
}

/// Splits `text` into whitespace runs and word runs, and further splits each
/// word run into leading/trailing punctuation around an alphabetic core, so
/// correction can replace just the core and reassemble the token exactly.
fn tokenize(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut i = 0;
    let bytes_len = text.len();
    let is_ws = |c: char| c.is_whitespace();

    while i < bytes_len {
        let rest = &text[i..];
        let ch = rest.chars().next().unwrap();
        if is_ws(ch) {
            let end = rest.find(|c: char| !is_ws(c)).unwrap_or(rest.len());
            tokens.push(Token::Space(&rest[..end]));
            i += end;
        } else {
            let end = rest.find(is_ws).unwrap_or(rest.len());
            let word = &rest[..end];
            let core_start = word.find(char::is_alphabetic).unwrap_or(word.len());
            let core_end = word.rfind(char::is_alphabetic).map(|p| p + word[p..].chars().next().unwrap().len_utf8()).unwrap_or(core_start);
            tokens.push(Token::Word {
                leading: &word[..core_start],
                core: &word[core_start..core_end],
                trailing: &word[core_end..],
            });
            i += end;
        }
    }
    tokens
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
        let result = checker.correct_text("anything here", 10);
        assert_eq!(result.text, "anything here");
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

    #[test]
    fn corrects_a_transposition_typo_and_preserves_case_and_punctuation() {
        let checker = SpellChecker::load("eng");
        if !checker.is_ready() {
            eprintln!("skipping: no en_US hunspell dictionary on this machine");
            return;
        }
        // Deliberately not "teh" -> "the": "tea" is an equally-close real
        // word with no frequency data to break the tie (documented
        // limitation). "recieve" has no such close neighbor.
        let result = checker.correct_text("Please recieve, this.", 10);
        assert_eq!(result.text, "Please receive, this.");
        assert_eq!(result.corrections.len(), 1);
        assert_eq!(result.corrections[0].original, "recieve");
        assert_eq!(result.corrections[0].corrected, "receive");
    }

    #[test]
    fn tokenize_preserves_whitespace_and_punctuation_exactly() {
        let text = "  Hello,   world!\nNext line.";
        let mut rebuilt = String::new();
        for tok in tokenize(text) {
            match tok {
                Token::Space(s) => rebuilt.push_str(s),
                Token::Word { leading, core, trailing } => {
                    rebuilt.push_str(leading);
                    rebuilt.push_str(core);
                    rebuilt.push_str(trailing);
                }
            }
        }
        assert_eq!(rebuilt, text);
    }
}
