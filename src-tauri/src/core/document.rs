//! File discovery + natural sort + PDF/image classification
//! (ARCHITECTURE.md §3 pipeline stages: "File Discovery -> Natural Sort -> PDF/Image Classify").
//!
//! This is intentionally shallow for Phase 1 (spec MVP_PLAN.md): it identifies
//! `DocumentSource`s and orders them, but does not parse PDF content or render
//! pages yet — that's `DocumentParser` (core/traits.rs), filled in during Phase 2.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Root directory for everything DocuMind writes: runtimes, models, job state,
/// output. Never a system path — see ARCHITECTURE.md §7.
pub fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("DocuMind")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Pdf,
    Image,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSource {
    pub path: PathBuf,
    pub kind: DocumentKind,
}

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp"];

fn classify(path: &Path) -> Option<DocumentKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext == "pdf" {
        Some(DocumentKind::Pdf)
    } else if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        Some(DocumentKind::Image)
    } else {
        None
    }
}

/// Discovers PDF/image files under `root` (a file or a folder, per spec §25
/// "Select Files" / "Select Folder"), recursing into subfolders, and returns
/// them in natural sort order (so `page-2.png` sorts before `page-10.png`).
pub fn discover(root: &Path) -> std::io::Result<Vec<DocumentSource>> {
    let mut found = Vec::new();
    collect(root, &mut found)?;
    found.sort_by(|a, b| natural_cmp(&a.path, &b.path));
    Ok(found)
}

fn collect(path: &Path, out: &mut Vec<DocumentSource>) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            collect(&entry?.path(), out)?;
        }
    } else if let Some(kind) = classify(path) {
        out.push(DocumentSource {
            path: path.to_path_buf(),
            kind,
        });
    }
    Ok(())
}

/// Natural sort: splits filenames into alternating text/number runs so
/// numeric page suffixes compare by value, not lexically.
fn natural_cmp(a: &Path, b: &Path) -> std::cmp::Ordering {
    let a = a.to_string_lossy();
    let b = b.to_string_lossy();
    natural_key(&a).cmp(&natural_key(&b))
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Chunk {
    Text(String),
    Number(u64, usize), // (value, original digit count, for stable tie-break)
}

fn natural_key(s: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut digits = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    digits.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            let len = digits.len();
            let value: u64 = digits.parse().unwrap_or(u64::MAX);
            chunks.push(Chunk::Number(value, len));
        } else {
            let mut text = String::new();
            while let Some(&t) = chars.peek() {
                if !t.is_ascii_digit() {
                    text.push(t);
                    chars.next();
                } else {
                    break;
                }
            }
            chunks.push(Chunk::Text(text.to_lowercase()));
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_sort_orders_numeric_suffixes_by_value() {
        let mut names = vec!["page-10.png", "page-2.png", "page-1.png"];
        names.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
        assert_eq!(names, vec!["page-1.png", "page-2.png", "page-10.png"]);
    }

    #[test]
    fn classify_recognizes_pdf_and_images() {
        assert_eq!(classify(Path::new("book.pdf")), Some(DocumentKind::Pdf));
        assert_eq!(classify(Path::new("scan.PNG")), Some(DocumentKind::Image));
        assert_eq!(classify(Path::new("notes.txt")), None);
    }
}
