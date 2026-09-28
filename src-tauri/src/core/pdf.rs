//! PDF handling (spec §16-17, ARCHITECTURE.md §3): page count, per-page native
//! text extraction, and page rendering to an image for the OCR fallback path.
//!
//! Implemented by shelling out to `poppler-utils` (`pdfinfo`/`pdftotext`/
//! `pdftoppm`) rather than a Rust PDF binding. Poppler is a mature, widely
//! packaged PDF engine; invoking it as an external tool mirrors how the
//! shipped app will eventually invoke a bundled sidecar binary
//! (DEPENDENCY_STRATEGY.md's sidecar model), so this is the right shape even
//! though these three tools are system-installed for now rather than bundled.

use crate::core::traits::{CoreError, CoreResult};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A PDF page is treated as "native-reliable" (skip OCR, spec §17) only if its
/// extracted text layer is non-trivial. A handful of stray characters (a
/// watermark, a page number) should not be mistaken for a real text layer on
/// an otherwise-scanned page.
const NATIVE_TEXT_MIN_CHARS: usize = 20;

pub fn require_poppler() -> CoreResult<()> {
    for tool in ["pdfinfo", "pdftotext", "pdftoppm"] {
        if which(tool).is_none() {
            return Err(CoreError::ToolMissing(
                "poppler-utils (pdfinfo/pdftotext/pdftoppm)",
            ));
        }
    }
    Ok(())
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(bin))
            .find(|p| p.is_file())
    })
}

pub fn page_count(pdf_path: &Path) -> CoreResult<u32> {
    let output = Command::new("pdfinfo")
        .arg(pdf_path)
        .output()
        .map_err(|e| CoreError::Engine(format!("failed to run pdfinfo: {e}")))?;
    if !output.status.success() {
        return Err(CoreError::Engine(format!(
            "pdfinfo failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Pages:") {
            return rest
                .trim()
                .parse()
                .map_err(|_| CoreError::Engine("could not parse page count".into()));
        }
    }
    Err(CoreError::Engine(
        "pdfinfo output did not contain a page count".into(),
    ))
}

/// Extracts the native text layer for a single 1-indexed page, `None` if the
/// page has no meaningful text layer (see `NATIVE_TEXT_MIN_CHARS`).
pub fn extract_native_text(pdf_path: &Path, page: u32) -> CoreResult<Option<String>> {
    let output = Command::new("pdftotext")
        .args(["-f", &page.to_string(), "-l", &page.to_string(), "-layout"])
        .arg(pdf_path)
        .arg("-") // stdout
        .output()
        .map_err(|e| CoreError::Engine(format!("failed to run pdftotext: {e}")))?;
    if !output.status.success() {
        return Err(CoreError::Engine(format!(
            "pdftotext failed on page {page}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.chars().filter(|c| !c.is_whitespace()).count() >= NATIVE_TEXT_MIN_CHARS {
        Ok(Some(text))
    } else {
        Ok(None)
    }
}

/// Renders a single 1-indexed page to a PNG at `dpi`, returning the output
/// path. `out_stem` is the destination path without extension (poppler
/// appends `.png` itself).
pub fn render_page(pdf_path: &Path, page: u32, dpi: u32, out_stem: &Path) -> CoreResult<PathBuf> {
    let status = Command::new("pdftoppm")
        .args([
            "-f",
            &page.to_string(),
            "-l",
            &page.to_string(),
            "-r",
            &dpi.to_string(),
            "-png",
            "-singlefile",
        ])
        .arg(pdf_path)
        .arg(out_stem)
        .status()
        .map_err(|e| CoreError::Engine(format!("failed to run pdftoppm: {e}")))?;
    if !status.success() {
        return Err(CoreError::Engine(format!(
            "pdftoppm failed on page {page}"
        )));
    }
    Ok(out_stem.with_extension("png"))
}
