# MVP & Phased Delivery Plan

Concretizes the spec's §47 phase list against the decisions in `TECH_DECISION.md`, `ARCHITECTURE.md`, `DEPENDENCY_STRATEGY.md`, `MODEL_STRATEGY.md`, `PACKAGING.md`.

## Phase 1 — Foundation

- Tauri v2 + React/TS project scaffold; CI skeleton building an empty app on all three OS.
- File import (drag/drop, file picker, folder picker) → `DocumentSource` list in the UI, no processing yet.
- PDF/image type detection (`DocumentParser::discover`).
- Core project structure: `ARCHITECTURE.md`'s module boundaries stood up as empty Rust modules/traits (`OCRProvider`, `LocalAIProvider`, `DocumentParser`, `ModelManager`, `DependencyManager`, `DocumentStorage`) so later phases fill in, not redesign.
- `DependencyManager`/`HardwareManager` skeleton: platform/arch/RAM/CPU/disk detection working and shown in a debug panel.
- OCR installation: bundled English ONNX runtime+model wired up end-to-end (download-free path first, since it ships in the installer) — proves the sidecar/library-loading mechanism before building the download manager.

**Exit criterion:** app launches on all 3 OSes in CI, detects hardware correctly, loads the bundled ONNX runtime and reports "OCR ready" with zero network access.

## Phase 2 — OCR MVP

- `OCRService` wired to RapidOCR-ONNX via `ort`; image preprocessing (deskew, binarization, denoise) as a discrete pipeline stage.
- PDF native-text extraction + per-page native-vs-OCR decision (§17).
- Full pipeline stub through to Markdown export (flat, no chapter detection yet — one `full_document.md` + `raw/page-NNN.txt`).
- Progress UI, per-page error handling and isolation (§45) — a bad page doesn't kill the job.
- **Benchmark gate** (per `MODEL_STRATEGY.md` §1): run the §43 fixture set through RapidOCR-ONNX and Tesseract, publish `docs/benchmarks/ocr-engine-comparison.md`, confirm or revise the default engine choice before Phase 3 builds structure detection on top of it.
- Download manager (§27) built out fully here (resumable, checksummed, disk-space-checked), since it's needed for any OCR language pack beyond the bundled English set.

**Exit criterion:** drop a real scanned book (English, 50–100 pages) → get a readable `full_document.md` with per-page confidence in `metadata.json`, entirely offline after the initial (bundled) OCR is ready.

## Phase 3 — Document Intelligence

- Layout detection via the PP-Structure ONNX model (headings, paragraphs, tables, figures, captions, footnotes, headers/footers, page numbers).
- Text-signal heading/chapter regex library (EN/VI/JA/ZH patterns from §19).
- Signal combination logic (`StructureClassifier`, no AI yet — rule+layout only).
- Chapter/section-based Markdown hierarchy and the `chapters/NN_Title/{chapter.md, pages/}` export layout (§24).

**Exit criterion:** the same 100-page book now exports with correct chapter boundaries and heading hierarchy using layout+regex signals alone (no AI model installed) — this is the point that proves AI is genuinely optional per §2/§3.

## Phase 4 — Local AI

- llama.cpp sidecar integration (`AIService`, localhost-only HTTP, process lifecycle owned by the app).
- `ModelManager` tiering (`MODEL_STRATEGY.md` §2) + hardware-based recommendation.
- `LocalAIProvider::classify_structure` wired into the `StructureClassifier` as the tiebreaker signal for ambiguous cases (§18's "combine signals" requirement fully realized).
- First-run Setup Wizard Step 3 (AI install screen, §4/§5) with real size/disk numbers from the manifest.
- Validate the "classification only, never rewrite" boundary (`MODEL_STRATEGY.md` §5) with a test that asserts OCR body text is byte-identical pre/post AI pass.

**Exit criterion:** same fixture book processed with AI enabled shows measurably better chapter/heading recall on the ambiguous-case fixtures than the Phase 3 rule-only baseline, with zero body-text drift.

## Phase 5 — Reliability

- Full resume/retry per §22–23: kill the process mid-job in a test harness, relaunch, verify exact resume point and that completed pages aren't reprocessed.
- Checkpointing granularity tuned (per-page write cost vs. resume precision).
- Large-document stress tests: 500, 1000, 2000+ page synthetic/real fixtures, memory-bounded via the chunked pipeline (§22), profiled for peak RSS.
- Batch/multi-document queue in the Job Manager.

**Exit criterion:** a 2000-page document processes to completion (or is interrupted and resumed) without peak memory exceeding a fixed budget independent of document length.

## Phase 6 — Production Packaging

- Full GitHub Actions build matrix (`PACKAGING.md` §4) producing signed(-pending-certs)/unsigned installers for all targets.
- Full Offline Bundle build variant.
- Settings page (§26): reinstall OCR, change/remove AI model, check for updates, storage usage.
- Code signing setup (Authenticode + notarization) as a checklist item, not a blocker for earlier internal builds.
- Release process: tag → CI → manifest publish → GitHub Release with all installers attached.

**Exit criterion:** a clean machine (VM, no dev tools) can install from each platform's artifact, complete setup, and process a document — validated manually per OS before the first public release.

## Cross-cutting, present from Phase 1 onward

- **Offline enforcement** (`ARCHITECTURE.md` §6): the `NetworkGuard` and Offline Mode toggle exist from Phase 1, even before there's much to guard, so no later phase can add a network call without going through the reviewed choke point.
- **Testing** (§43): fixture corpus (EN/VI/JA books, technical books, scanned/native/mixed PDFs, low-quality/rotated scans, multi-column, tables, code blocks) is built incrementally alongside the phase that first needs it (Phase 2 needs OCR fixtures, Phase 3 needs layout/structure fixtures, etc.), not deferred to a single QA phase.
- **No silent data loss** (§46): every phase's exit criteria include an explicit check that failures are surfaced, not swallowed.

## Explicit non-goals for the MVP (revisit post-Phase 6)

- Cloud OCR/AI mode (§15 keeps this possible via the same provider interfaces, but no cloud provider implementation ships in the MVP).
- `.rpm` packaging (stretch, after `.deb`/`.AppImage` are proven).
- macOS universal binaries (ship arch-specific first; revisit if native dep universal builds prove clean).
- OCR error auto-correction beyond confidence flagging (§2 lists it as a possible AI use; not required for the core promise in §49 and deferred past Phase 6).
