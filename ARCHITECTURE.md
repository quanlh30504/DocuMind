# DocuMind Architecture

Local-first OCR & Document Intelligence desktop app. See `TECH_DECISION.md` for the comparative analysis behind every choice below; this document describes the resulting system.

## 1. High-level shape

```
┌─────────────────────────────────────────────────────────┐
│                     Tauri Shell (Rust)                   │
│  ┌───────────────────────────────────────────────────┐  │
│  │  WebView UI (React + TS)   — presentation only      │  │
│  └───────────────────────────────────────────────────┘  │
│                         │ Tauri IPC (commands + events)  │
│  ┌───────────────────────────────────────────────────┐  │
│  │                Application Core (Rust)              │  │
│  │                                                      │  │
│  │  DocumentPipeline   JobManager     ExportService     │  │
│  │  OCRService         AIService      ModelManager      │  │
│  │  DependencyManager  HardwareManager DocumentStorage  │  │
│  │  NetworkGuard (Offline Mode enforcement)             │  │
│  └───────────────────────────────────────────────────┘  │
│            │                         │                   │
│    ┌───────┴────────┐       ┌────────┴─────────┐        │
│    │ ONNX Runtime    │       │ llama.cpp sidecar │        │
│    │ (OCR, in-proc   │       │ (localhost HTTP,  │        │
│    │  via `ort`)     │       │  loopback only)   │        │
│    └────────────────┘       └────────────────────┘        │
└─────────────────────────────────────────────────────────┘
         │                                    │
   app-data/models/ocr/*.onnx         app-data/models/ai/*.gguf
```

Everything below the IPC line is Rust. The WebView never talks to the filesystem, network, or native processes directly — it only calls Tauri commands and receives progress events, which keeps the capability/permission boundary (Tauri's security model) meaningful and gives us one place (`NetworkGuard`) to enforce Offline Mode.

## 2. Provider interfaces (§38)

Defined as Rust traits so any component is swappable without touching callers:

```rust
trait OCRProvider {
    fn recognize(&self, image: PreparedImage) -> Result<OCRPageResult>;
    fn engine_info(&self) -> EngineInfo;
}

trait LocalAIProvider {
    fn classify_structure(&self, page: &OCRPageResult, context: &DocContext) -> Result<StructureHints>;
    fn model_info(&self) -> ModelInfo;
}

trait DocumentParser {          // PDF/image ingestion
    fn discover(&self, path: &Path) -> Result<Vec<DocumentSource>>;
    fn extract_native_text(&self, page: &PdfPage) -> Option<NativeText>;
    fn render_page(&self, page: &PdfPage, dpi: u32) -> Result<Image>;
}

trait ModelManager { /* §39 */
    fn check_installed(&self, id: &ModelId) -> InstallState;
    fn download(&self, id: &ModelId) -> DownloadHandle;
    fn verify_checksum(&self, id: &ModelId) -> Result<()>;
    fn install(&self, id: &ModelId) -> Result<()>;
    fn remove(&self, id: &ModelId) -> Result<()>;
    fn update(&self, id: &ModelId) -> Result<()>;
    fn available(&self) -> Vec<ModelDescriptor>;
    fn recommended(&self, hw: &HardwareProfile) -> ModelDescriptor;
}

trait DependencyManager { /* §40 */
    fn detect_platform(&self) -> PlatformTarget;
    fn check_runtime(&self, id: &RuntimeId) -> InstallState;
    fn download_runtime(&self, id: &RuntimeId) -> DownloadHandle;
    fn install_runtime(&self, id: &RuntimeId) -> Result<()>;
    fn verify_runtime(&self, id: &RuntimeId) -> Result<()>;
    fn repair_runtime(&self, id: &RuntimeId) -> Result<()>;
}

trait DocumentStorage {
    fn checkpoint(&self, job_id: &JobId, state: &JobState) -> Result<()>;
    fn load_checkpoint(&self, job_id: &JobId) -> Option<JobState>;
    fn write_output(&self, job_id: &JobId, artifact: OutputArtifact) -> Result<()>;
}
```

`HardwareManager` (§41) is a plain service, not swappable — it wraps `sysinfo` + platform GPU probes and exposes `HardwareProfile { cpu_cores, ram_bytes, gpu: Option<GpuInfo>, vram_bytes: Option<u64>, arch, disk_free_bytes }`.

## 3. Document pipeline (§16–17)

```
File Discovery → Natural Sort → PDF/Image Classify
   → [PDF] Per-page: native text present & reliable?
         ├── yes → NativeTextExtraction
         └── no  → RenderPage → ImagePreprocess → OCR
   → [Image] ImagePreprocess → OCR
   → OCR Confidence tagging
   → Layout Detection (PP-Structure ONNX model)
   → Text Normalization
   → Structure Detection (rule/layout signals; + optional Local AI pass)
   → Chapter/Section Detection
   → Markdown Generation
   → Folder Export
```

Each arrow is a `JobStage` (§42) with independent `PENDING/RUNNING/COMPLETED/FAILED/CANCELLED` status, persisted per page so a crash mid-book resumes at the last completed page (§23), and a single page's failure is isolated (§45) — the job continues, the failure is recorded in `metadata.json`, and only failed pages are eligible for retry.

Reliability rule for native-vs-OCR per PDF page (§17): a page is treated as "native-reliable" only if it has an extractable text layer covering a text-area ratio above a threshold (avoids treating a scanned page with a stray text watermark as native). Otherwise it's rendered and OCR'd. This is decided page-by-page, so hybrid PDFs are handled naturally without a separate mode.

## 4. Structure & chapter detection (§18–20)

Three signal layers feed a single `StructureClassifier`:
1. **Text signals** — regex/pattern library for common chapter/heading idioms across EN/VI/JA/ZH (`CHAPTER 1`, `Chương 1`, `第1章`, `1. Introduction`, `Part I`, …), versioned as data, not hardcoded logic, so new locales are additive.
2. **Layout signals** — from the PP-Structure ONNX pass: font-size-relative-to-body, position on page, whitespace before/after, indentation, page-break alignment.
3. **Semantic signal (optional)** — `LocalAIProvider.classify_structure` takes the OCR'd line + its layout features and returns a structural label (`heading|chapter_title|body|caption|footnote|...`) with a confidence score. This is a *classification* call (small, fast, low-context), never a rewrite — the model receives the page's structural context, not an instruction to generate prose, enforcing §20 (no content rewriting) at the API boundary: `classify_structure` cannot return modified body text, only labels.

Signals are combined with a weighted-vote/heuristic-priority scheme: strong text-signal + layout agreement = high confidence without AI; ambiguous cases (no text-signal match but layout looks heading-like) are the ones routed to the AI pass, which is why AI is "recommended," not mandatory (§2, §3) — the minimum pipeline (Image/PDF → OCR → Markdown) never calls it, and structure detection still functions (regex+layout only, lower recall on ambiguous documents) when the user skips the AI install.

## 5. Job Manager & resume (§22–23, §42)

Documents are processed in bounded chunks (default 20 pages) so RAM is bounded regardless of document length (§22): a chunk is loaded, processed, persisted (raw OCR + intermediate `metadata.json` state), and released before the next chunk starts. `DocumentStorage.checkpoint` writes progress after every page, not just every chunk, so resume granularity is per-page:

```json
{
  "documentId": "...",
  "totalPages": 500,
  "completedPages": 349,
  "failedPages": [112, 217],
  "status": "INTERRUPTED"
}
```

On relaunch, any job found in `INTERRUPTED`/`RUNNING` state (RUNNING implies an unclean shutdown) is offered to the user as "Resume" in the UI, continuing from `completedPages + 1` and re-queuing `failedPages` separately.

OCR/AI models are loaded once per job and kept resident for the job's duration (§44) — `OCRService`/`AIService` own a long-lived engine handle, not a per-page init.

## 6. Offline & network boundary (§13–15, §46)

A single `NetworkGuard` in the Rust core is the only code path permitted to open outbound connections. It has two states:

- **Offline Mode: ON** (can be user-forced) — every call through the guard is rejected except none; the document pipeline never needs network regardless of mode.
- **Offline Mode: OFF (default operating state after setup)** — the guard still only allows requests tagged with an explicit, enumerated purpose: `model_download`, `checksum_manifest_fetch`, `app_update_check`. Anything else (there is nothing else planned) is a compile-time-unreachable path, not a runtime policy decision, so there is no way for a future feature to "accidentally" phone home — adding a new network call requires adding a new tagged purpose here, which is a deliberate, reviewable diff.

The UI's persistent status indicator (§25, §14) reads the guard's current mode directly, not a cached flag, so it can't drift from reality.

## 7. Storage layout

```
<app-data-dir>/
├── runtime/
│   ├── onnxruntime/<platform-target>/...
│   └── llama-server/<platform-target>/...
├── models/
│   ├── ocr/{detection,recognition,layout}/<lang>/*.onnx
│   └── ai/<model-id>/*.gguf
├── jobs/<job-id>/
│   ├── state.json                # resume checkpoint
│   ├── raw/page-NNN.txt
│   └── ...
└── config.json                   # offline mode, model choice, etc.
```

Never under a system path (`/usr`, `Program Files`, etc.) except the installed app binary itself — everything mutable lives in the OS-standard per-user app-data directory, so no admin rights are needed post-install and `Reinstall/Remove` (§26) is just directory operations plus `ModelManager`/`DependencyManager` calls.

## 8. Output layout (§24)

```
output/
├── metadata.json          # per-page confidence/status/warnings (§21)
├── full_document.md
├── raw/page-001.txt ...
├── chapters/01_Introduction/{chapter.md, pages/}
└── images/
```

## 9. Development phases

See `MVP_PLAN.md`.
