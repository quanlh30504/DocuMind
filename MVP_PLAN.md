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

- `OCRService` wired to a real, working `OCRProvider` end-to-end; image preprocessing (adaptive binarization landed — Sauvola local threshold; deskew/denoise remain accuracy-tuning follow-ups, see note below) as a discrete pipeline stage.
- PDF native-text extraction + per-page native-vs-OCR decision (§17) — implemented via `poppler-utils` (`pdfinfo`/`pdftotext`/`pdftoppm`).
- Full pipeline stub through to Markdown export (flat, no chapter detection yet — one `full_document.md` + `raw/page-NNN.txt`), with per-page confidence/status/warnings in `metadata.json` (§21).
- Progress events + per-page error handling and isolation (§45) — a bad page doesn't kill the job; implemented and unit-tested (`core/job.rs`).
- **Engine sequencing note (deviation from the original plan, recorded here for transparency):** the concrete `OCRProvider` implemented first is **Tesseract** (`core/ocr/tesseract.rs`, CLI shell-out), not RapidOCR-ONNX. Reason: Tesseract is TECH_DECISION.md §2's documented fallback engine and needs no bundled ONNX runtime or downloaded model set, so it gets a real, verifiable, end-to-end pipeline working immediately. RapidOCR-ONNX is a second implementation of the same `OCRProvider` trait — a drop-in, not a pipeline rewrite — and is the remaining Phase 2 work, gated behind the download-manager/`ort`-integration work below.
- **Benchmark gate** (per `MODEL_STRATEGY.md` §1): run the §43 fixture set through RapidOCR-ONNX and Tesseract, publish `docs/benchmarks/ocr-engine-comparison.md`, confirm or revise the default engine choice before Phase 3 builds structure detection on top of it. **Status: not yet run** — blocked on both a real fixture corpus (§43) and the RapidOCR-ONNX provider existing to benchmark against; the placeholder doc records the methodology so this isn't silently dropped.
- Download manager (§27): **not yet built** — deferred until it's needed to fetch the first real downloadable artifact (an OCR language pack or the RapidOCR-ONNX model set), rather than built speculatively against nothing.

**Exit criterion:** drop a real scanned book (English, 50–100 pages) → get a readable `full_document.md` with per-page confidence in `metadata.json`, entirely offline after the initial (bundled) OCR is ready. **Verified:** PDF page count/native-text-extraction/rendering and the preprocessing stage confirmed against real fixtures (`examples/pdf_smoke.rs`); the full render→preprocess→OCR path confirmed end to end on both a real image and a real image-only PDF, correctly routed to Tesseract with 96% confidence and 100%-accurate recognition of known text (`examples/ocr_smoke.rs`, `examples/pdf_ocr_smoke.rs`); native-vs-OCR page routing confirmed correct on both a text-layer PDF (native, confidence 1.0) and an image-only PDF (OCR path). Still outstanding: a real 50-100 page book fixture (only single-page synthetic fixtures exist so far) and the full offline network-guard check (Phase 2 doesn't yet have a network-guard implementation to verify against — that's tracked separately, see ARCHITECTURE.md §6, ticketed for whenever the first real network call — a model download — is added).

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

**Delivered ahead of schedule, non-AI half only:** OCR output on real documents (see below) surfaced genuine recognition errors beyond what a wrong language model explains — dictionary-based *flagging* (`core/spellcheck.rs`, via the pure-Rust `zspell` crate against Hunspell word lists) was added so users can see which words are probably wrong today, before real AI correction exists. This is explicitly the non-AI half of spec §2's "OCR error correction suggestions": it can tell you a word is *probably* wrong (not in any loaded dictionary), it cannot tell you what it should have been — that needs the real `LocalAIProvider::suggest_correction` this phase still owes, gated on the same llama.cpp/model-download work as `classify_structure`. The UI states this boundary explicitly ("flagged, not fixed") rather than implying more than the dictionary check can deliver. Both directions were offered to the user (full LLM now vs. dictionary flagging first); dictionary flagging first was chosen given this dev machine's history of slow/flaky large downloads.

**Verified** against the user's real garbled Vietnamese OCR output (`en_US`/`vi_VN` Hunspell dictionaries, both now installed): correctly flagged 14 genuinely wrong words (`Cihurơng`→Chương, `TRIÉN`/`TRIÊN`→TRIỂN, `Lÿ`→Lý, `ĐỒIH`→gồm, `óục`→mục, `øiá`→giá, ...), alongside a few expected false positives (real words like "hòa"/"thỏa" the dictionary missed in this split) — consistent with this being a heuristic signal, not a verdict, as documented in `core/spellcheck.rs`.

**Auto-apply was attempted and reverted after two findings during testing (not asked, just the responsible default once found):**
1. `hunspell-vi`'s word list is very sparse (~6,600 words vs. ~79,000 for `hunspell-en-us`) and is missing common words like "hóa"/"hòa"/"thỏa" — auto-applying corrections against it was actively turning *correct* Vietnamese words into wrong ones. `core/spellcheck.rs`'s `MIN_WORDS_FOR_AUTOCORRECT` threshold now disables suggestion-search (not validity-checking) for any dictionary below ~20k words; Vietnamese currently falls back to flag-only until a fuller dictionary is sourced.
2. Even on the good English dictionary, a quick real-world sample got 2 of 4 corrections wrong ("smple"→"smile" instead of "simple"; "definately"→"definably" instead of "definitely") — pure edit-distance search with no word-frequency or context data can't break ties between two equally-close real words. This is a hard limit of the approach, not a bug to fix with more tuning.

Given both findings, `PageRecord.suggested_corrections` are shown for user review (UI: "N suggested correction(s) — review before using, not applied automatically") but **never written into `raw/page-NNN.txt` or `full_document.md`** — the output text is always the untouched OCR/native text. Real automatic correction needs `LocalAIProvider::suggest_correction`, i.e. this phase's still-outstanding llama.cpp work.

**"Tier 1" rule-based accuracy pass (user explicitly asked to refactor the rule-based approach further before committing to the LLM path; two tiers were proposed, user chose Tier 1 first):**
1. **Frequency-ranked suggestions.** Added `resources/wordfreq/{en_top10k.txt,vi_syllables.tsv}` (bundled, no network download) and rank ties on real word frequency instead of arbitrary edit-distance-tuple order. This directly fixed both English miscorrections above: "smple"→**"simple"** (was "smile"), "definately"→**"definitely"** (was "definably"), verified via a regression test (`frequency_breaks_ties_toward_the_common_word`).
2. **A much better Vietnamese word source.** The `hunspell-vi` gaps ("hóa"/"hòa"/"thỏa" missing) turned out to be a tone-mark-placement-convention issue, not just small size (see `resources/wordfreq/README.md`) — the bundled frequency corpus (14,492 syllables, derived from a merged Vietnamese wordlist) covers both conventions and is now unioned into validity checking, not just suggestion ranking. `MIN_WORDS_FOR_AUTOCORRECT` was removed entirely — frequency data, not raw dictionary size, is what makes ranking trustworthy now. Verified: `hóa hòa thỏa` no longer false-flagged (`vietnamese_common_words_are_no_longer_false_flagged`).
3. **Diacritic-aware edit costs for Vietnamese.** Substitutions between characters in the same confusion group (taken directly from Hunspell's own `vi_VN.aff` `MAP` directives, e.g. ơ/ờ/ở/ỡ/ớ/ợ) cost less than an arbitrary substitution, matching the dominant real OCR failure mode (diacritic/tone-mark confusion, not random noise).

Re-verified against the user's real garbled text after this pass: several previously-wrong or unresolved corrections improved (e.g. `øiá`→`giá` now exactly correct), though some heavily garbled words (`ĐỒIH`, intended "gồm") remain too far in edit distance to suggest correctly — an expected limit of a no-context approach, left as Tier 2 (local LLM) scope. Release-build performance: ~19.5ms for a full page with 12 unresolved words (debug build was ~20x slower — release timing is what matters for the shipped app).

**Tier 2 (deferred, user's choice):** a local LLM via llama.cpp, prompted with surrounding context to resolve exactly the cases Tier 1 structurally cannot (context-dependent word choice, badly garbled words) — this is genuinely `LocalAIProvider::suggest_correction`, the Phase 4 work already described above, not a new idea.

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

**Started ahead of Phases 3-5, per explicit request, since packaging is independent of pipeline features:**
- `.github/workflows/ci.yml` (fast Linux check on every push/PR) and `release.yml` (the full `PACKAGING.md` §4 matrix — Windows/macOS-arm64/macOS-Intel/Linux — triggered by a `v*.*.*` tag, using `tauri-apps/tauri-action`, publishing a **draft** GitHub Release) are implemented.
- `tauri.conf.json` bundle metadata filled in (publisher/copyright/category/descriptions, `deb.depends`); fixed the `identifier` ending in `.app` (conflicts with macOS's bundle extension, flagged by Tauri's own build warning) to `com.documind.desktop`.
- **Locally verified on Linux** (`npm run tauri build -- --bundles deb,appimage`): both `.deb` (3.5 MiB) and `.AppImage` (80 MiB) build successfully; the AppImage was launched directly and ran without error, confirming it's genuinely self-contained (bundles its own webkit2gtk/gtk library dependencies rather than assuming they're present).
- `deb.depends` currently lists `tesseract-ocr`/`poppler-utils` as **hard package dependencies** — an honest reflection of Phase 2's current CLI-shellout OCR implementation, not the eventual bundled-sidecar model `DEPENDENCY_STRATEGY.md` describes. This should be revisited once RapidOCR-ONNX + `ort` replaces the system Tesseract dependency.
- **Not done**: Windows/macOS builds are untested (no such runner available in this dev environment — they rely on CI, not yet run since no tag has been pushed); code signing; the Full Offline Bundle variant; the Settings page; the manifest-publish step (nothing to publish yet — no downloadable OCR/AI models exist).

## Cross-cutting, present from Phase 1 onward

- **Offline enforcement** (`ARCHITECTURE.md` §6): the `NetworkGuard` and Offline Mode toggle exist from Phase 1, even before there's much to guard, so no later phase can add a network call without going through the reviewed choke point.
- **Testing** (§43): fixture corpus (EN/VI/JA books, technical books, scanned/native/mixed PDFs, low-quality/rotated scans, multi-column, tables, code blocks) is built incrementally alongside the phase that first needs it (Phase 2 needs OCR fixtures, Phase 3 needs layout/structure fixtures, etc.), not deferred to a single QA phase.
- **No silent data loss** (§46): every phase's exit criteria include an explicit check that failures are surfaced, not swallowed.

## Explicit non-goals for the MVP (revisit post-Phase 6)

- Cloud OCR/AI mode (§15 keeps this possible via the same provider interfaces, but no cloud provider implementation ships in the MVP).
- `.rpm` packaging (stretch, after `.deb`/`.AppImage` are proven).
- macOS universal binaries (ship arch-specific first; revisit if native dep universal builds prove clean).
- OCR error auto-correction beyond confidence flagging (§2 lists it as a possible AI use; not required for the core promise in §49 and deferred past Phase 6).
