//! Job orchestration (ARCHITECTURE.md §3/§5, spec §16/§21-24/§42/§45).
//!
//! Phase 2 scope: drives one document (PDF or image) through
//! discover -> native-text-or-OCR -> markdown/metadata export, with per-page
//! error isolation so one bad page doesn't abort the job (spec §45). Chunked
//! processing, checkpoint-based resume across app restarts, and multi-document
//! batching are Phase 5 work (MVP_PLAN.md) — this already writes state in the
//! shape Phase 5's resume logic needs (`completed_pages`/`failed_pages`), it
//! just doesn't yet reload and continue from it after a restart.

use crate::core::document::{DocumentKind, DocumentSource};
use crate::core::preprocess;
use crate::core::traits::{CoreResult, OCRProvider};
use crate::core::{pdf, traits::JobId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageSource {
    Native,
    Ocr,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageStatus {
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRecord {
    pub page: u32,
    pub source: PageSource,
    pub confidence: f32,
    pub status: PageStatus,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobStatus {
    Completed,
    CompletedWithWarnings,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobState {
    pub document_id: JobId,
    pub document_path: PathBuf,
    pub total_pages: u32,
    pub completed_pages: u32,
    pub failed_pages: Vec<u32>,
    pub status: JobStatus,
    pub pages: Vec<PageRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum JobEvent {
    PageStarted { page: u32, total: u32 },
    PageCompleted { page: u32, total: u32, status: PageStatus },
}

const RENDER_DPI: u32 = 200;

fn process_page(
    source: &DocumentSource,
    page: u32,
    tmp_dir: &Path,
    raw_dir: &Path,
    ocr: &dyn OCRProvider,
) -> CoreResult<PageRecord> {
    let raw_path = raw_dir.join(format!("page-{page:03}.txt"));

    if source.kind == DocumentKind::Pdf {
        if let Some(native) = pdf::extract_native_text(&source.path, page)? {
            std::fs::write(&raw_path, &native)?;
            return Ok(PageRecord {
                page,
                source: PageSource::Native,
                confidence: 1.0,
                status: PageStatus::Completed,
                warnings: vec![],
            });
        }
    }

    let render_stem = tmp_dir.join(format!("page-{page:03}"));
    let rendered = if source.kind == DocumentKind::Pdf {
        pdf::render_page(&source.path, page, RENDER_DPI, &render_stem)?
    } else {
        source.path.clone()
    };

    let preprocessed = tmp_dir.join(format!("page-{page:03}-bin.png"));
    preprocess::preprocess(&rendered, &preprocessed)?;

    let result = ocr.recognize(&preprocessed)?;
    std::fs::write(&raw_path, &result.text)?;

    Ok(PageRecord {
        page,
        source: PageSource::Ocr,
        confidence: result.confidence,
        status: PageStatus::Completed,
        warnings: result.warnings,
    })
}

/// Processes one document end to end, writing `output_dir/{raw,metadata.json,
/// full_document.md}` (spec §24). Per-page failures are isolated (spec §45):
/// caught, recorded with `PageStatus::Failed`, and the job continues.
pub fn run_document<F: FnMut(JobEvent)>(
    source: &DocumentSource,
    output_dir: &Path,
    ocr: &dyn OCRProvider,
    mut on_progress: F,
) -> CoreResult<JobState> {
    let raw_dir = output_dir.join("raw");
    let tmp_dir = output_dir.join(".tmp");
    std::fs::create_dir_all(&raw_dir)?;
    std::fs::create_dir_all(&tmp_dir)?;

    if source.kind == DocumentKind::Pdf {
        pdf::require_poppler()?;
    }

    let total_pages = match source.kind {
        DocumentKind::Pdf => pdf::page_count(&source.path)?,
        DocumentKind::Image => 1,
    };

    let mut pages = Vec::with_capacity(total_pages as usize);
    let mut failed_pages = Vec::new();
    let mut completed_pages = 0u32;

    for page in 1..=total_pages {
        on_progress(JobEvent::PageStarted { page, total: total_pages });
        let record = match process_page(source, page, &tmp_dir, &raw_dir, ocr) {
            Ok(record) => {
                completed_pages += 1;
                record
            }
            Err(e) => {
                failed_pages.push(page);
                PageRecord {
                    page,
                    source: PageSource::Ocr,
                    confidence: 0.0,
                    status: PageStatus::Failed,
                    warnings: vec![e.to_string()],
                }
            }
        };
        on_progress(JobEvent::PageCompleted {
            page,
            total: total_pages,
            status: record.status,
        });
        pages.push(record);
    }

    std::fs::remove_dir_all(&tmp_dir).ok();

    let status = if failed_pages.is_empty() {
        JobStatus::Completed
    } else {
        JobStatus::CompletedWithWarnings
    };

    let state = JobState {
        document_id: Uuid::new_v4(),
        document_path: source.path.clone(),
        total_pages,
        completed_pages,
        failed_pages,
        status,
        pages,
    };

    write_outputs(&state, output_dir, &raw_dir)?;
    Ok(state)
}

fn write_outputs(state: &JobState, output_dir: &Path, raw_dir: &Path) -> CoreResult<()> {
    let metadata_path = output_dir.join("metadata.json");
    std::fs::write(&metadata_path, serde_json::to_string_pretty(state).unwrap())?;

    let mut full_doc = String::new();
    for page in &state.pages {
        let raw_path = raw_dir.join(format!("page-{:03}.txt", page.page));
        let text = std::fs::read_to_string(&raw_path).unwrap_or_default();
        full_doc.push_str(&format!("<!-- page {} -->\n\n", page.page));
        full_doc.push_str(&text);
        full_doc.push_str("\n\n");
    }
    std::fs::write(output_dir.join("full_document.md"), full_doc)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::{CoreError, OCRPageResult};

    struct StubOcr;
    impl OCRProvider for StubOcr {
        fn recognize(&self, _image_path: &Path) -> CoreResult<OCRPageResult> {
            Ok(OCRPageResult {
                text: "stub text".into(),
                confidence: 0.9,
                warnings: vec![],
            })
        }
        fn engine_info(&self) -> &'static str {
            "stub"
        }
    }

    struct FailingOcr;
    impl OCRProvider for FailingOcr {
        fn recognize(&self, _image_path: &Path) -> CoreResult<OCRPageResult> {
            Err(CoreError::Engine("boom".into()))
        }
        fn engine_info(&self) -> &'static str {
            "failing"
        }
    }

    fn make_test_image(dir: &Path) -> PathBuf {
        let path = dir.join("input.png");
        let img = image::ImageBuffer::from_fn(50, 50, |_, _| image::Luma([128u8]));
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn image_document_runs_through_stub_ocr() {
        let base = std::env::temp_dir().join(format!("documind-job-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let image_path = make_test_image(&base);
        let output_dir = base.join("output");

        let source = DocumentSource {
            path: image_path,
            kind: DocumentKind::Image,
        };
        let state = run_document(&source, &output_dir, &StubOcr, |_| {}).unwrap();

        assert_eq!(state.status, JobStatus::Completed);
        assert_eq!(state.completed_pages, 1);
        assert!(state.failed_pages.is_empty());
        let full_doc = std::fs::read_to_string(output_dir.join("full_document.md")).unwrap();
        assert!(full_doc.contains("stub text"));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_failing_page_is_isolated_not_fatal() {
        let base = std::env::temp_dir().join(format!("documind-job-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let image_path = make_test_image(&base);
        let output_dir = base.join("output");

        let source = DocumentSource {
            path: image_path,
            kind: DocumentKind::Image,
        };
        let state = run_document(&source, &output_dir, &FailingOcr, |_| {}).unwrap();

        assert_eq!(state.status, JobStatus::CompletedWithWarnings);
        assert_eq!(state.failed_pages, vec![1]);
        assert!(output_dir.join("metadata.json").exists());

        std::fs::remove_dir_all(&base).ok();
    }
}
