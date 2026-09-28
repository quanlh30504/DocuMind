//! Job orchestration (ARCHITECTURE.md §3/§5, spec §16/§21-24/§42/§45).
//!
//! Per-page error isolation (spec §45): a bad page is caught, recorded as
//! `PageStatus::Failed`, and the job continues rather than aborting.
//!
//! **Resume (spec §22-23, Phase 5):** `metadata.json` in `output_dir` doubles
//! as the resume checkpoint — it's overwritten after every real page of work
//! (not just at the end), tagged `INTERRUPTED` while a job is running. Calling
//! `run_document` again with the same `output_dir` and source reloads it: any
//! page already `Completed` is skipped (its raw text is re-read from
//! `raw/page-NNN.txt`, not reprocessed — no re-running OCR/spellcheck on work
//! already done), any `Failed` page is retried. A checkpoint for a different
//! source path or page count is discarded rather than misapplied. There is no
//! separate "resume" entry point — this makes calling `run_document` again
//! after a crash, an app restart, or simply re-clicking Process on the same
//! output folder the same operation, which is also what makes a multi-document
//! batch "resumable": each document's own `metadata.json` is independently
//! resumable, so re-running a batch after an interruption only reprocesses
//! the documents/pages that didn't finish.
//!
//! Memory: the per-page heavy data (rendered page image, preprocessed
//! bitmap) is local to `process_page` and dropped every iteration — it does
//! not accumulate across a run. `pages`/`page_texts` (the lightweight
//! metadata + extracted text) do accumulate for the whole document, which is
//! an intentional, small cost (text, not images) traded for being able to
//! assemble `full_document.md` in one pass at the end; verified in
//! `examples/large_doc_stress.rs` not to grow unreasonably even at hundreds
//! of pages.

use crate::core::document::{DocumentKind, DocumentSource};
use crate::core::preprocess;
use crate::core::spellcheck::{CorrectionRecord, SpellChecker};
use crate::core::traits::{CoreResult, OCRProvider};
use crate::core::{pdf, traits::JobId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    /// Candidate corrections found by the dictionary checker
    /// (core/spellcheck.rs) — shown for manual review, **never applied to
    /// `raw/page-NNN.txt` or `full_document.md`**. Testing found this
    /// heuristic (edit distance against a flat word list, no
    /// frequency/context data) picks the wrong one of two equally-close real
    /// words often enough — e.g. "smple" -> "smile" instead of "simple" —
    /// that auto-applying it would sometimes make correct text worse. Real
    /// automatic correction needs a `LocalAIProvider` with actual language
    /// understanding (MVP_PLAN.md Phase 4, not yet implemented).
    #[serde(default)]
    pub suggested_corrections: Vec<CorrectionRecord>,
    /// Words flagged as probably wrong with no dictionary candidate close
    /// enough to even suggest — still need manual review. Empty (with no
    /// `spellcheck_unavailable:<lang>` warning) means "checked, nothing
    /// wrong found"; empty *with* that warning means "not checked".
    #[serde(default)]
    pub flagged_words: Vec<String>,
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
    /// Page was already `Completed` in a resumed checkpoint — reused as-is,
    /// not reprocessed. Distinct from `PageCompleted` so the UI can be
    /// honest about what actually happened (spec §46: no silent skipping).
    PageSkipped { page: u32, total: u32 },
}

const RENDER_DPI: u32 = 200;
/// Cap on flagged/corrected words recorded per page (core/spellcheck.rs) so a
/// low-quality page's noise can't blow up metadata.json.
const MAX_SPELLCHECK_ITEMS_PER_PAGE: usize = 50;

struct SpellcheckOutcome {
    suggested_corrections: Vec<CorrectionRecord>,
    flagged_words: Vec<String>,
}

/// Runs dictionary checking over `text` and returns *suggestions only* — the
/// text itself is never modified here (see `PageRecord::suggested_corrections`
/// doc comment for why auto-apply was tried and reverted).
fn check_spelling(text: &str, spell_checker: Option<&SpellChecker>, warnings: &mut Vec<String>) -> SpellcheckOutcome {
    let Some(checker) = spell_checker else {
        return SpellcheckOutcome { suggested_corrections: vec![], flagged_words: vec![] };
    };
    if !checker.is_ready() {
        for lang in &checker.unavailable_langs {
            warnings.push(format!("spellcheck_unavailable:{lang}"));
        }
        return SpellcheckOutcome { suggested_corrections: vec![], flagged_words: vec![] };
    }
    let result = checker.correct_text(text, MAX_SPELLCHECK_ITEMS_PER_PAGE);
    if !result.corrections.is_empty() {
        warnings.push(format!("suggested_corrections:{}", result.corrections.len()));
    }
    if !result.unresolved.is_empty() {
        warnings.push(format!("possible_spelling_errors:{}", result.unresolved.len()));
    }
    SpellcheckOutcome {
        suggested_corrections: result.corrections,
        flagged_words: result.unresolved,
    }
}

fn process_page(
    source: &DocumentSource,
    page: u32,
    tmp_dir: &Path,
    raw_dir: &Path,
    ocr: &dyn OCRProvider,
    spell_checker: Option<&SpellChecker>,
) -> CoreResult<(PageRecord, String)> {
    let raw_path = raw_dir.join(format!("page-{page:03}.txt"));

    if source.kind == DocumentKind::Pdf {
        if let Some(native) = pdf::extract_native_text(&source.path, page)? {
            std::fs::write(&raw_path, &native)?;
            let mut warnings = Vec::new();
            let outcome = check_spelling(&native, spell_checker, &mut warnings);
            let record = PageRecord {
                page,
                source: PageSource::Native,
                confidence: 1.0,
                status: PageStatus::Completed,
                warnings,
                suggested_corrections: outcome.suggested_corrections,
                flagged_words: outcome.flagged_words,
            };
            return Ok((record, native));
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

    let mut warnings = result.warnings;
    let outcome = check_spelling(&result.text, spell_checker, &mut warnings);

    let record = PageRecord {
        page,
        source: PageSource::Ocr,
        confidence: result.confidence,
        status: PageStatus::Completed,
        warnings,
        suggested_corrections: outcome.suggested_corrections,
        flagged_words: outcome.flagged_words,
    };
    Ok((record, result.text))
}

/// Processes one document end to end, writing `output_dir/{raw,metadata.json,
/// full_document.md}` (spec §24). Per-page failures are isolated (spec §45):
/// caught, recorded with `PageStatus::Failed`, and the job continues.
///
/// `raw/page-NNN.txt` and `full_document.md` always hold the untouched
/// OCR/native text — a `spell_checker` (core/spellcheck.rs) only adds
/// *suggested* corrections and flagged words to the returned `JobState`, it
/// never rewrites the output text (see `PageRecord::suggested_corrections`'s
/// doc comment for why auto-apply was tried during development and reverted).
pub fn run_document<F: FnMut(JobEvent)>(
    source: &DocumentSource,
    output_dir: &Path,
    ocr: &dyn OCRProvider,
    spell_checker: Option<&SpellChecker>,
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

    let (document_id, mut resumable) = load_checkpoint(output_dir, source, total_pages);

    let mut pages = Vec::with_capacity(total_pages as usize);
    let mut page_texts = Vec::with_capacity(total_pages as usize);
    let mut failed_pages = Vec::new();
    let mut completed_pages = 0u32;

    for page in 1..=total_pages {
        if let Some(prior) = resumable.remove(&page) {
            let text = std::fs::read_to_string(raw_dir.join(format!("page-{page:03}.txt"))).unwrap_or_default();
            on_progress(JobEvent::PageSkipped { page, total: total_pages });
            completed_pages += 1;
            page_texts.push(text);
            pages.push(prior);
            continue;
        }

        on_progress(JobEvent::PageStarted { page, total: total_pages });
        let (record, text) = match process_page(source, page, &tmp_dir, &raw_dir, ocr, spell_checker) {
            Ok((record, text)) => {
                completed_pages += 1;
                (record, text)
            }
            Err(e) => {
                failed_pages.push(page);
                (
                    PageRecord {
                        page,
                        source: PageSource::Ocr,
                        confidence: 0.0,
                        status: PageStatus::Failed,
                        warnings: vec![e.to_string()],
                        suggested_corrections: vec![],
                        flagged_words: vec![],
                    },
                    String::new(),
                )
            }
        };
        on_progress(JobEvent::PageCompleted {
            page,
            total: total_pages,
            status: record.status,
        });
        pages.push(record);
        page_texts.push(text);

        // Checkpoint after every page of real work (spec §22-23) — cheap
        // (small JSON, no full_document.md rebuild) and means a crash loses
        // at most the one in-flight page, not the whole run.
        write_metadata(output_dir, &checkpoint_state(document_id, source, total_pages, &pages));
    }

    std::fs::remove_dir_all(&tmp_dir).ok();

    let status = if failed_pages.is_empty() {
        JobStatus::Completed
    } else {
        JobStatus::CompletedWithWarnings
    };

    let state = JobState {
        document_id,
        document_path: source.path.clone(),
        total_pages,
        completed_pages,
        failed_pages,
        status,
        pages,
    };

    write_outputs(&state, output_dir, &page_texts)?;
    Ok(state)
}

fn checkpoint_state(document_id: JobId, source: &DocumentSource, total_pages: u32, pages: &[PageRecord]) -> JobState {
    let failed_pages = pages.iter().filter(|p| p.status == PageStatus::Failed).map(|p| p.page).collect();
    let completed_pages = pages.iter().filter(|p| p.status == PageStatus::Completed).count() as u32;
    JobState {
        document_id,
        document_path: source.path.clone(),
        total_pages,
        completed_pages,
        failed_pages,
        status: JobStatus::Interrupted,
        pages: pages.to_vec(),
    }
}

/// Loads a prior `metadata.json` from `output_dir` if it's a valid resume
/// checkpoint for this exact `source`/`total_pages` (a mismatch — different
/// file, different page count — means the checkpoint doesn't apply and is
/// discarded rather than misapplied to the wrong document). Returns the
/// `document_id` to keep (fresh one if not resuming) and a map of
/// previously-`Completed` pages to skip; `Failed` pages are intentionally
/// left out so the caller retries them.
fn load_checkpoint(output_dir: &Path, source: &DocumentSource, total_pages: u32) -> (JobId, HashMap<u32, PageRecord>) {
    let fresh = || (Uuid::new_v4(), HashMap::new());
    let Ok(content) = std::fs::read_to_string(output_dir.join("metadata.json")) else {
        return fresh();
    };
    let Ok(prior) = serde_json::from_str::<JobState>(&content) else {
        return fresh();
    };
    if prior.document_path != source.path || prior.total_pages != total_pages {
        return fresh();
    }
    let completed = prior
        .pages
        .into_iter()
        .filter(|p| p.status == PageStatus::Completed)
        .map(|p| (p.page, p))
        .collect();
    (prior.document_id, completed)
}

fn write_metadata(output_dir: &Path, state: &JobState) {
    // Best-effort: a checkpoint write failing shouldn't abort the job that's
    // actually making progress. If this keeps failing, the final
    // write_outputs call will surface the same error properly.
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(output_dir.join("metadata.json"), json);
    }
}

fn write_outputs(state: &JobState, output_dir: &Path, page_texts: &[String]) -> CoreResult<()> {
    let metadata_path = output_dir.join("metadata.json");
    std::fs::write(&metadata_path, serde_json::to_string_pretty(state).unwrap())?;

    let mut full_doc = String::new();
    for (page, text) in state.pages.iter().zip(page_texts) {
        full_doc.push_str(&format!("<!-- page {} -->\n\n", page.page));
        full_doc.push_str(text);
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

    /// Hand-built minimal multi-page PDF with **no text objects** (each page
    /// is just a filled rectangle) so every page has no native text layer and
    /// forces the render+OCR path — needed to exercise resume against a real
    /// multi-page document without depending on Tesseract being installed in
    /// the test environment (the OCR provider is a test stub instead).
    fn make_multi_page_pdf(path: &Path, pages: u32) {
        let mut objects: Vec<Vec<u8>> = Vec::new();
        objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        let kids: String = (0..pages).map(|i| format!("{} 0 R ", 3 + i * 2)).collect();
        objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.trim(), pages).into_bytes());
        for _ in 0..pages {
            let page_obj_index = objects.len() as u32 + 1;
            objects.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents {} 0 R >>",
                    page_obj_index + 1
                )
                .into_bytes(),
            );
            let content = b"1 0 0 rg 20 20 100 100 re f".to_vec();
            objects.push(format!("<< /Length {} >>\nstream\n", content.len()).into_bytes().into_iter().chain(content).chain(b"\nendstream".to_vec()).collect());
        }

        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n");
        let mut offsets = vec![0usize];
        for (i, obj) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(obj);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref_offset = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for off in &offsets[1..] {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF", objects.len() + 1, xref_offset).as_bytes());

        std::fs::write(path, out).unwrap();
    }

    /// Counts OCR calls per page (parsed from the image path, which encodes
    /// the page number, e.g. `.../page-003-bin.png`) and optionally fails a
    /// fixed set of pages — used to simulate "this run got interrupted after
    /// some pages failed" for the resume test below.
    struct ControllableOcr {
        fail_pages: Vec<u32>,
        calls: std::sync::Mutex<Vec<u32>>,
    }

    impl ControllableOcr {
        fn new(fail_pages: Vec<u32>) -> Self {
            Self { fail_pages, calls: std::sync::Mutex::new(Vec::new()) }
        }

        fn page_from_path(path: &Path) -> u32 {
            let stem = path.file_stem().unwrap().to_str().unwrap(); // "page-003-bin"
            stem.split('-').nth(1).unwrap().parse().unwrap()
        }
    }

    impl OCRProvider for ControllableOcr {
        fn recognize(&self, image_path: &Path) -> CoreResult<OCRPageResult> {
            let page = Self::page_from_path(image_path);
            self.calls.lock().unwrap().push(page);
            if self.fail_pages.contains(&page) {
                return Err(CoreError::Engine(format!("simulated failure on page {page}")));
            }
            Ok(OCRPageResult { text: format!("page {page} text"), confidence: 0.9, warnings: vec![] })
        }
        fn engine_info(&self) -> &'static str {
            "controllable"
        }
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
        let state = run_document(&source, &output_dir, &StubOcr, None, |_| {}).unwrap();

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
        let state = run_document(&source, &output_dir, &FailingOcr, None, |_| {}).unwrap();

        assert_eq!(state.status, JobStatus::CompletedWithWarnings);
        assert_eq!(state.failed_pages, vec![1]);
        assert!(output_dir.join("metadata.json").exists());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn resume_skips_completed_pages_and_retries_only_failed_ones() {
        if pdf::require_poppler().is_err() {
            eprintln!("skipping: poppler-utils not installed in this environment");
            return;
        }
        let base = std::env::temp_dir().join(format!("documind-resume-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let pdf_path = base.join("book.pdf");
        make_multi_page_pdf(&pdf_path, 5);
        let output_dir = base.join("output");
        let source = DocumentSource { path: pdf_path, kind: DocumentKind::Pdf };

        // Run 1: pages 2 and 4 fail (simulating "the app crashed / those
        // pages had a transient failure").
        let first_ocr = ControllableOcr::new(vec![2, 4]);
        let state1 = run_document(&source, &output_dir, &first_ocr, None, |_| {}).unwrap();
        assert_eq!(state1.status, JobStatus::CompletedWithWarnings);
        assert_eq!(state1.completed_pages, 3);
        assert_eq!(state1.failed_pages, vec![2, 4]);
        assert_eq!(*first_ocr.calls.lock().unwrap(), vec![1, 2, 3, 4, 5], "first run must attempt every page");

        // Run 2: same source/output_dir, a fresh OCR provider that always
        // succeeds. This must NOT re-run OCR on pages 1/3/5 (already
        // Completed) — only on the two that previously failed.
        let second_ocr = ControllableOcr::new(vec![]);
        let state2 = run_document(&source, &output_dir, &second_ocr, None, |_| {}).unwrap();
        assert_eq!(state2.status, JobStatus::Completed);
        assert_eq!(state2.completed_pages, 5);
        assert!(state2.failed_pages.is_empty());
        assert_eq!(
            *second_ocr.calls.lock().unwrap(),
            vec![2, 4],
            "resume must only reprocess previously-failed pages, not re-OCR completed ones"
        );
        // document_id is preserved across the resume, not regenerated.
        assert_eq!(state1.document_id, state2.document_id);

        let full_doc = std::fs::read_to_string(output_dir.join("full_document.md")).unwrap();
        for page in 1..=5 {
            assert!(full_doc.contains(&format!("page {page} text")), "missing page {page} in final output");
        }

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn checkpoint_for_a_different_document_is_not_reused() {
        if pdf::require_poppler().is_err() {
            eprintln!("skipping: poppler-utils not installed in this environment");
            return;
        }
        let base = std::env::temp_dir().join(format!("documind-resume-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let output_dir = base.join("output");

        let pdf_a = base.join("a.pdf");
        make_multi_page_pdf(&pdf_a, 2);
        let source_a = DocumentSource { path: pdf_a, kind: DocumentKind::Pdf };
        run_document(&source_a, &output_dir, &ControllableOcr::new(vec![]), None, |_| {}).unwrap();

        // A different document reusing the same output_dir must not have its
        // pages treated as "already completed" just because a checkpoint
        // happens to be sitting there.
        let pdf_b = base.join("b.pdf");
        make_multi_page_pdf(&pdf_b, 2);
        let source_b = DocumentSource { path: pdf_b, kind: DocumentKind::Pdf };
        let ocr_b = ControllableOcr::new(vec![]);
        run_document(&source_b, &output_dir, &ocr_b, None, |_| {}).unwrap();
        assert_eq!(*ocr_b.calls.lock().unwrap(), vec![1, 2], "different source must not reuse the prior document's checkpoint");

        std::fs::remove_dir_all(&base).ok();
    }
}
