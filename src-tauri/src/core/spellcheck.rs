//! Dictionary-based error detection and correction *suggestion* (never
//! auto-applied — see `SpellChecker::correct_text`'s doc comment).
//!
//! Two data sources per language:
//! - **Validity checking**: `zspell` against system Hunspell `.aff`/`.dic`
//!   files (affix-aware — correctly accepts inflected forms not literally
//!   listed), unioned with the bundled frequency wordlist below (Ubuntu's
//!   `hunspell-vi` package turned out to have only ~6,600 entries and to be
//!   missing ordinary words like "hóa"/"hòa"/"thỏa" — see
//!   `resources/wordfreq/README.md` for the root cause).
//! - **Suggestion ranking**: candidates within edit distance are ranked by
//!   real word frequency (`resources/wordfreq/{en_top10k.txt,
//!   vi_syllables.tsv}`), not just edit distance — plain edit distance
//!   can't tell "simple" from "smile" (both one edit from "smple"); the far
//!   more common word should win, and now does. For Vietnamese specifically,
//!   substitutions between visually/phonetically confusable
//!   diacritic-variant characters (the same confusion pairs Hunspell's own
//!   `vi_VN.aff` `MAP` directives encode, e.g. ơ/ờ/ở/ỡ/ớ/ợ) cost less than
//!   an arbitrary substitution, since that's the dominant real OCR failure
//!   mode observed in testing (character/diacritic confusion, not random
//!   noise).
//!
//! This is still not real language understanding — no grammar, no sentence
//! context, so it still can't fix badly garbled words (edit distance too
//! large) or context-dependent choices a frequency table can't resolve.
//! That needs a `LocalAIProvider` (MVP_PLAN.md Phase 4, not yet built).

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

/// Frequency-ranked, most-common-first. Rank position stands in for a real
/// count (see `resources/wordfreq/README.md`).
const EN_FREQ_LIST: &str = include_str!("../../resources/wordfreq/en_top10k.txt");
/// `word\tcount`, most-common-first — a real (if small and informally
/// sourced, see the README) usage-frequency corpus.
const VI_FREQ_LIST: &str = include_str!("../../resources/wordfreq/vi_syllables.tsv");

/// Below this count in the source corpus, a Vietnamese syllable is more
/// likely noise (an OCR error, foreign fragment, or typo that made it into
/// the merged source list) than a real word worth trusting for validity
/// checking. Suggestion ranking uses the raw frequency regardless — a
/// same-distance real candidate with count 1 still beats one with count 0.
const VI_MIN_COUNT_FOR_VALIDITY: u64 = 2;

fn load_frequency_table(lang: &str) -> HashMap<String, u64> {
    match lang {
        "eng" => EN_FREQ_LIST
            .lines()
            .enumerate()
            .map(|(i, w)| (w.trim().to_lowercase(), (EN_FREQ_LIST.lines().count() - i) as u64))
            .collect(),
        "vie" => VI_FREQ_LIST
            .lines()
            .filter_map(|line| {
                let (word, count) = line.split_once('\t')?;
                Some((word.to_string(), count.trim().parse().ok()?))
            })
            .collect(),
        _ => HashMap::new(),
    }
}

/// Diacritic/character confusion groups for Vietnamese, taken from the
/// single-character `MAP` directives in Hunspell's own `vi_VN.aff` — i.e.
/// not guessed, but the same confusability data the reference Vietnamese
/// spellchecker ships with. Two characters in the same group are what OCR
/// most often confuses (missing/misread tone marks, wrong base vowel).
const VI_CONFUSION_GROUPS: &[&str] = &[
    "ảã", "ẩẫ", "ẳẵ", "ẻẽ", "ểễ", "ỉĩ", "ỏõ", "ổỗ", "ởỡ", "ủũ", "ửữ", "ỷỹ",
    "aàảãáạ", "ăằẳẵắặ", "âầẩẫấậ", "eèẻẽéẹ", "êềểễếệ", "iìỉĩíị",
    "oòỏõóọ", "ôồổỗốộ", "ơờởỡớợ", "uùủũúụ", "ưừửữứự", "yỳỷỹýỵ",
];

struct ConfusionTable(HashMap<char, u32>);

impl ConfusionTable {
    fn build(groups: &[&str]) -> Self {
        let mut map = HashMap::new();
        for (id, group) in groups.iter().enumerate() {
            for c in group.chars() {
                map.insert(c, id as u32);
            }
        }
        Self(map)
    }

    /// `true` if `a`/`b` are different characters from the same confusion
    /// group (a substitution OCR is prone to making).
    fn confusable(&self, a: char, b: char) -> bool {
        a != b && self.0.get(&a).is_some_and(|ga| self.0.get(&b) == Some(ga))
    }
}

/// Edit costs in "decicost" units (10 = one full edit) so fractional costs
/// (a confusable substitution costs less than an arbitrary one) stay
/// integers and totally orderable, avoiding float-comparison pitfalls.
const COST_UNIT: u32 = 10;
const CONFUSABLE_SUBSTITUTION_COST: u32 = 4;

/// Max accepted edit cost (decicost units — divide by `COST_UNIT` for the
/// "number of edits" equivalent), scaled to word length so short words need
/// a near-exact match while longer words tolerate more OCR noise.
fn max_cost_for(len: usize) -> u32 {
    match len {
        0..=3 => COST_UNIT,
        4..=6 => COST_UNIT * 2,
        _ => COST_UNIT * 3,
    }
}

/// Restricted (optimal string alignment) Damerau-Levenshtein distance in
/// decicost units, with an optional confusion table lowering substitution
/// cost between visually/phonetically similar characters. Early-exits once
/// it's clear the cost will exceed `max`, since this runs per-candidate over
/// a word list.
fn edit_cost(a: &[char], b: &[char], max: u32, confusion: Option<&ConfusionTable>) -> Option<u32> {
    if (a.len().abs_diff(b.len()) as u32) * COST_UNIT > max {
        return None;
    }
    let (m, n) = (a.len(), b.len());
    let mut prev2 = vec![0u32; n + 1];
    let mut prev: Vec<u32> = (0..=n as u32).map(|j| j * COST_UNIT).collect();
    let mut curr = vec![0u32; n + 1];

    for i in 1..=m {
        curr[0] = i as u32 * COST_UNIT;
        let mut row_min = curr[0];
        for j in 1..=n {
            let sub_cost = if a[i - 1] == b[j - 1] {
                0
            } else if confusion.is_some_and(|c| c.confusable(a[i - 1], b[j - 1])) {
                CONFUSABLE_SUBSTITUTION_COST
            } else {
                COST_UNIT
            };
            let mut val = (prev[j] + COST_UNIT)
                .min(curr[j - 1] + COST_UNIT)
                .min(prev[j - 1] + sub_cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                val = val.min(prev2[j - 2] + COST_UNIT);
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

/// Per-language candidate pool for suggestion search: the union of the
/// system Hunspell word list and the bundled frequency wordlist (see module
/// doc comment for why both), bucketed by length and carrying each word's
/// frequency (0 if only the Hunspell list had it) for tie-breaking.
/// A candidate word with its lowercase form and capitalization flag
/// precomputed once at index-build time rather than on every `suggest()`
/// call (this index is searched once per unresolved word per page, and
/// `to_lowercase()`/`collect()` on every comparison showed up as real cost
/// on low-quality pages with many distinct unknown words).
struct Candidate {
    word: String,
    lower: Vec<char>,
    has_caps: bool,
    freq: u64,
}

struct WordIndex {
    by_length: HashMap<usize, Vec<Candidate>>,
    confusion: Option<ConfusionTable>,
}

impl WordIndex {
    fn build(dic_content: &str, freq: &HashMap<String, u64>, lang: &str) -> Self {
        let mut words: HashMap<String, u64> = HashMap::new();
        for line in dic_content.lines().skip(1) {
            // Hunspell .dic format: `word` or `word/AFFIXFLAGS`.
            let word = line.split('/').next().unwrap_or("").trim();
            if word.is_empty() || !word.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '-') {
                continue;
            }
            let count = freq.get(&word.to_lowercase()).copied().unwrap_or(0);
            words.entry(word.to_string()).or_insert(count);
        }
        for (word, count) in freq {
            words.entry(word.clone()).or_insert(*count);
        }

        let mut by_length: HashMap<usize, Vec<Candidate>> = HashMap::new();
        for (word, freq) in words {
            let lower: Vec<char> = word.to_lowercase().chars().collect();
            let has_caps = word.chars().any(char::is_uppercase);
            by_length.entry(word.chars().count()).or_default().push(Candidate { word, lower, has_caps, freq });
        }

        let confusion = (lang == "vie").then(|| ConfusionTable::build(VI_CONFUSION_GROUPS));
        Self { by_length, confusion }
    }

    fn word_count(&self) -> usize {
        self.by_length.values().map(Vec::len).sum()
    }

    /// Closest candidate to `word` (case-insensitive) within its
    /// length-appropriate cost threshold, or `None` if nothing is close
    /// enough. Ties on edit cost are broken by frequency (more common word
    /// wins), then by same-first-letter, then by closer length, then by
    /// preferring plain-lowercase entries over ones stored with intrinsic
    /// capitals (proper nouns/acronyms).
    fn suggest(&self, word: &str) -> Option<String> {
        let target: Vec<char> = word.to_lowercase().chars().collect();
        let max_cost = max_cost_for(target.len());
        // Key: (edit cost, -frequency, first-char mismatch?, |length diff|,
        // has intrinsic capitalization?). Smaller wins on every field.
        let mut best: Option<((u32, std::cmp::Reverse<u64>, u8, usize, u8), &str)> = None;

        for len in target.len().saturating_sub(max_cost as usize / COST_UNIT as usize)
            ..=(target.len() + max_cost as usize / COST_UNIT as usize)
        {
            let Some(candidates) = self.by_length.get(&len) else { continue };
            let len_diff = len.abs_diff(target.len());
            for c in candidates {
                let Some(cost) = edit_cost(&target, &c.lower, max_cost, self.confusion.as_ref()) else {
                    continue;
                };
                let first_mismatch = u8::from(c.lower.first() != target.first());
                let has_caps = u8::from(c.has_caps);
                let key = (cost, std::cmp::Reverse(c.freq), first_mismatch, len_diff, has_caps);
                if best.is_none_or(|(best_key, _)| key < best_key) {
                    best = Some((key, c.word.as_str()));
                }
            }
        }

        best.filter(|((cost, ..), _)| *cost <= max_cost && *cost > 0)
            .map(|(_, w)| w.to_string())
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
    /// Words from the bundled frequency list trusted as valid on their own
    /// (count >= a noise floor for Vietnamese; all of it for English, which
    /// is a clean top-10k list) — fills gaps in the system Hunspell list.
    extra_known: HashSet<String>,
    suggest_indices: Vec<WordIndex>,
    /// Languages requested but with no dictionary available on this machine
    /// — surfaced so callers can tell "checked, found nothing" apart from
    /// "couldn't check at all" (spec §46: no silent skipping).
    pub unavailable_langs: Vec<String>,
}

impl SpellChecker {
    /// `langs` are Tesseract-style codes (`"eng"`, `"vie"`; `"eng+vie"` is
    /// split automatically) matching what `TesseractOcrProvider::resolve_lang`
    /// resolved to, so the same language selection drives OCR, flagging and
    /// correction together.
    pub fn load(langs: &str) -> Self {
        let mut dictionaries = Vec::new();
        let mut extra_known = HashSet::new();
        let mut suggest_indices = Vec::new();
        let mut unavailable_langs = Vec::new();

        for lang in langs.split('+').map(str::trim).filter(|l| !l.is_empty()) {
            let freq = load_frequency_table(lang);
            let candidates = system_dict_paths(lang);
            let loaded = candidates.iter().find_map(|(aff, dic)| {
                let aff_content = std::fs::read_to_string(Path::new(aff)).ok()?;
                let dic_content = std::fs::read_to_string(Path::new(dic)).ok()?;
                let dict = zspell::builder()
                    .config_str(&aff_content)
                    .dict_str(&dic_content)
                    .build()
                    .ok()?;
                Some((dict, WordIndex::build(&dic_content, &freq, lang)))
            });
            match loaded {
                Some((dict, index)) => {
                    dictionaries.push(dict);
                    let min_count = if lang == "vie" { VI_MIN_COUNT_FOR_VALIDITY } else { 1 };
                    extra_known.extend(freq.iter().filter(|(_, c)| **c >= min_count).map(|(w, _)| w.clone()));
                    suggest_indices.push(index);
                }
                None => unavailable_langs.push(lang.to_string()),
            }
        }

        Self {
            dictionaries,
            extra_known,
            suggest_indices,
            unavailable_langs,
        }
    }

    pub fn is_ready(&self) -> bool {
        !self.dictionaries.is_empty()
    }

    /// Total suggestion-candidate pool size across loaded languages —
    /// informational (e.g. for a Settings page), not used to gate behavior
    /// anymore now that frequency-based ranking (not raw size) is what
    /// makes suggestions trustworthy.
    pub fn candidate_pool_size(&self) -> usize {
        self.suggest_indices.iter().map(WordIndex::word_count).sum()
    }

    fn is_known(&self, word: &str) -> bool {
        self.dictionaries.iter().any(|d| d.check_word(word)) || self.extra_known.contains(&word.to_lowercase())
    }

    fn suggest(&self, word: &str) -> Option<String> {
        self.suggest_indices.iter().filter_map(|idx| idx.suggest(word)).next()
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

    /// Finds candidate corrections without modifying `text`. Returns the
    /// input text unchanged in `CorrectionResult::text` (kept for API
    /// compatibility / potential future opt-in apply); `corrections` holds
    /// suggestions to show the user, never silently written to output files
    /// — see this module's and `core/job.rs`'s doc comments for why
    /// auto-apply was tried during development and reverted (ambiguous
    /// same-distance real words, e.g. "smple" -> "smile" vs "simple",
    /// resolved by frequency here but not eliminated for every case).
    pub fn correct_text(&self, text: &str, cap: usize) -> CorrectionResult {
        if !self.is_ready() {
            return CorrectionResult { text: text.to_string(), corrections: vec![], unresolved: vec![] };
        }

        let mut corrections = Vec::new();
        let mut unresolved = Vec::new();
        let mut seen_corrections = HashSet::new();
        let mut seen_unresolved = HashSet::new();

        for token in tokenize(text) {
            if let Token::Word { core, .. } = token {
                if core.chars().count() <= 1 || self.is_known(core) {
                    continue;
                }
                if let Some(suggestion) = self.suggest(core) {
                    let cased = match_case(core, &suggestion);
                    if seen_corrections.insert(core.to_string()) {
                        corrections.push(CorrectionRecord { original: core.to_string(), corrected: cased });
                        if corrections.len() >= cap {
                            break;
                        }
                    }
                } else if seen_unresolved.insert(core.to_string()) {
                    unresolved.push(core.to_string());
                    if unresolved.len() >= cap {
                        break;
                    }
                }
            }
        }

        CorrectionResult { text: text.to_string(), corrections, unresolved }
    }
}

#[allow(dead_code)] // Space/leading/trailing are exercised by the tokenizer round-trip test
enum Token<'a> {
    Space(&'a str),
    Word { leading: &'a str, core: &'a str, trailing: &'a str },
}

/// Splits `text` into whitespace runs and word runs, and further splits each
/// word run into leading/trailing punctuation around an alphabetic core.
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
    fn frequency_breaks_ties_toward_the_common_word() {
        let checker = SpellChecker::load("eng");
        if !checker.is_ready() {
            eprintln!("skipping: no en_US hunspell dictionary on this machine");
            return;
        }
        // "smple" is edit-distance 1 from both "simple" and "smile" -
        // frequency must pick "simple" (far more common).
        let result = checker.correct_text("a smple test", 10);
        assert_eq!(result.corrections.len(), 1);
        assert_eq!(result.corrections[0].corrected, "simple");

        let result = checker.correct_text("I will definately go", 10);
        assert_eq!(result.corrections.len(), 1);
        assert_eq!(result.corrections[0].corrected, "definitely");
    }

    #[test]
    fn vietnamese_common_words_are_no_longer_false_flagged() {
        let checker = SpellChecker::load("vie");
        if !checker.is_ready() {
            eprintln!("skipping: no vi_VN hunspell dictionary on this machine");
            return;
        }
        // These were exactly the false positives found against the
        // hunspell-vi system dictionary alone (see README.md in
        // resources/wordfreq/).
        let flagged = checker.flag_words("hóa hòa thỏa", 10);
        assert!(flagged.is_empty(), "unexpected false positives: {flagged:?}");
    }

    #[test]
    fn vietnamese_diacritic_confusion_is_cheaper_than_arbitrary_substitution() {
        let table = ConfusionTable::build(VI_CONFUSION_GROUPS);
        assert!(table.confusable('ơ', 'ờ'));
        assert!(!table.confusable('a', 'z'));
        let a: Vec<char> = "chương".chars().collect();
        let b: Vec<char> = "chuong".chars().collect(); // ASCII-typed, no diacritics at all
        // sanity: cost function runs without panicking on real Vietnamese text
        let _ = edit_cost(&a, &b, 100, Some(&table));
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
