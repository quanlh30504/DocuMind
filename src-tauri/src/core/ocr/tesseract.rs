//! Tesseract OCR provider (TECH_DECISION.md §2's documented fallback engine).
//!
//! Used as the concrete `OCRProvider` for the Phase 2 MVP: it needs no
//! bundled ONNX runtime or downloaded model set to produce a real, working
//! pipeline end-to-end (MVP_PLAN.md Phase 2 exit criterion), unlike the
//! documented-default RapidOCR-ONNX engine, which depends on Phase 2's
//! download-manager work landing first. Swapping in RapidOCR-ONNX later is a
//! second `OCRProvider` implementation, not a pipeline change — see
//! `core/traits.rs`.
//!
//! Invoked via CLI (`tesseract ... tsv`) rather than a native binding, which
//! both sidesteps binding/build complexity and mirrors how the shipped app
//! will eventually invoke a bundled OCR sidecar binary.

use crate::core::traits::{CoreError, CoreResult, OCRPageResult, OCRProvider};
use std::path::Path;
use std::process::Command;

pub struct TesseractOcrProvider {
    lang: String,
}

impl TesseractOcrProvider {
    pub fn new(lang: impl Into<String>) -> Self {
        Self { lang: lang.into() }
    }

    pub fn is_available() -> bool {
        Command::new("tesseract")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

impl Default for TesseractOcrProvider {
    fn default() -> Self {
        Self::new("eng")
    }
}

/// One data row of Tesseract's TSV output
/// (level, page_num, block_num, par_num, line_num, word_num, left, top,
/// width, height, conf, text).
struct TsvWord {
    line_key: (i32, i32, i32),
    conf: f32,
    text: String,
}

fn parse_tsv(tsv: &str) -> Vec<TsvWord> {
    let mut words = Vec::new();
    for line in tsv.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 12 {
            continue;
        }
        let conf: f32 = cols[10].parse().unwrap_or(-1.0);
        let text = cols[11].trim();
        if conf < 0.0 || text.is_empty() {
            continue; // non-word aggregate rows (block/par/line) report conf = -1
        }
        let block: i32 = cols[2].parse().unwrap_or(0);
        let par: i32 = cols[3].parse().unwrap_or(0);
        let line_num: i32 = cols[4].parse().unwrap_or(0);
        words.push(TsvWord {
            line_key: (block, par, line_num),
            conf,
            text: text.to_string(),
        });
    }
    words
}

fn reconstruct(words: &[TsvWord]) -> (String, f32) {
    if words.is_empty() {
        return (String::new(), 0.0);
    }
    let mut lines: Vec<(i32, i32, i32, String)> = Vec::new();
    let mut conf_sum = 0f32;
    for w in words {
        conf_sum += w.conf;
        match lines.last_mut() {
            Some((b, p, l, text)) if (*b, *p, *l) == w.line_key => {
                text.push(' ');
                text.push_str(&w.text);
            }
            _ => lines.push((w.line_key.0, w.line_key.1, w.line_key.2, w.text.clone())),
        }
    }
    let text = lines
        .into_iter()
        .map(|(.., t)| t)
        .collect::<Vec<_>>()
        .join("\n");
    let mean_conf = conf_sum / words.len() as f32 / 100.0;
    (text, mean_conf.clamp(0.0, 1.0))
}

impl OCRProvider for TesseractOcrProvider {
    fn recognize(&self, image_path: &Path) -> CoreResult<OCRPageResult> {
        let output = Command::new("tesseract")
            .arg(image_path)
            .arg("stdout")
            .args(["-l", &self.lang])
            .arg("tsv")
            .output()
            .map_err(|_| CoreError::ToolMissing("tesseract"))?;

        if !output.status.success() {
            return Err(CoreError::Engine(format!(
                "tesseract failed on {}: {}",
                image_path.display(),
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let tsv = String::from_utf8_lossy(&output.stdout);
        let words = parse_tsv(&tsv);
        let (text, confidence) = reconstruct(&words);

        let mut warnings = Vec::new();
        if confidence < 0.5 {
            warnings.push("low_confidence".to_string());
        }
        if text.trim().is_empty() {
            warnings.push("no_text_detected".to_string());
        }

        Ok(OCRPageResult {
            text,
            confidence,
            warnings,
        })
    }

    fn engine_info(&self) -> &'static str {
        "tesseract"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstruct_joins_words_on_same_line_and_averages_confidence() {
        let words = vec![
            TsvWord {
                line_key: (1, 1, 1),
                conf: 90.0,
                text: "Hello".into(),
            },
            TsvWord {
                line_key: (1, 1, 1),
                conf: 80.0,
                text: "world".into(),
            },
            TsvWord {
                line_key: (1, 1, 2),
                conf: 70.0,
                text: "Next".into(),
            },
        ];
        let (text, conf) = reconstruct(&words);
        assert_eq!(text, "Hello world\nNext");
        assert!((conf - 0.8).abs() < 0.001);
    }

    #[test]
    fn reconstruct_handles_empty_input() {
        let (text, conf) = reconstruct(&[]);
        assert_eq!(text, "");
        assert_eq!(conf, 0.0);
    }
}
