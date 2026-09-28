//! Provider interfaces (ARCHITECTURE.md §2, spec §38-41).
//!
//! These traits are the seam between the app core and swappable engines
//! (OCR/AI runtimes, storage backend). Phase 1 stands up the types and
//! signatures only; concrete implementations land in Phases 2-5 per
//! MVP_PLAN.md. Keeping them here from the start means later phases fill in
//! bodies rather than redesign the boundary.
#![allow(dead_code)] // exercised starting Phase 2; see MVP_PLAN.md

use crate::core::document::DocumentSource;
use crate::core::hardware::{HardwareProfile, PlatformTarget};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

pub type JobId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeId(pub String);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    Installed,
    Missing,
    Corrupt,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type CoreResult<T> = Result<T, CoreError>;

/// OCR engine boundary (spec §12, TECH_DECISION.md §2). Phase 2 implements this
/// against RapidOCR's ONNX models via the `ort` crate.
pub trait OCRProvider {
    fn recognize(&self, image_path: &Path) -> CoreResult<String>;
    fn engine_info(&self) -> &'static str;
}

/// Local AI boundary (spec §10, MODEL_STRATEGY.md §5). Deliberately returns
/// typed classification output, never free-text body content, so "AI rewrote
/// the book" is structurally excluded rather than merely discouraged by
/// prompting. Phase 4 implements this against a llama.cpp sidecar.
pub trait LocalAIProvider {
    fn classify_structure(&self, line_text: &str) -> CoreResult<StructureHint>;
    fn model_info(&self) -> &'static str;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureLabel {
    Heading,
    ChapterTitle,
    Body,
    Caption,
    Footnote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureHint {
    pub label: StructureLabel,
    pub confidence: f32,
}

/// PDF/image ingestion boundary (spec §16-17). Phase 2 implements page
/// rendering and native-text extraction.
pub trait DocumentParser {
    fn discover(&self, root: &Path) -> CoreResult<Vec<DocumentSource>>;
}

/// Model download/verify/install lifecycle (spec §39). Phase 2 implements the
/// download manager this depends on (DEPENDENCY_STRATEGY.md §4).
pub trait ModelManager {
    fn check_installed(&self, id: &ModelId) -> InstallState;
    fn verify_checksum(&self, id: &ModelId) -> CoreResult<()>;
    fn install(&self, id: &ModelId) -> CoreResult<()>;
    fn remove(&self, id: &ModelId) -> CoreResult<()>;
    fn recommended(&self, hw: &HardwareProfile) -> ModelId;
}

/// Native runtime (ONNX Runtime, llama.cpp) lifecycle (spec §40,
/// DEPENDENCY_STRATEGY.md §6).
pub trait DependencyManager {
    fn detect_platform(&self) -> PlatformTarget;
    fn check_runtime(&self, id: &RuntimeId) -> InstallState;
    fn install_runtime(&self, id: &RuntimeId) -> CoreResult<()>;
    fn verify_runtime(&self, id: &RuntimeId) -> CoreResult<()>;
    fn repair_runtime(&self, id: &RuntimeId) -> CoreResult<()>;
}

/// Job checkpoint/output persistence (spec §22-24). Phase 5 implements
/// per-page resume against this boundary.
pub trait DocumentStorage {
    fn checkpoint(&self, job_id: &JobId, state: &[u8]) -> CoreResult<()>;
    fn load_checkpoint(&self, job_id: &JobId) -> CoreResult<Option<Vec<u8>>>;
}
