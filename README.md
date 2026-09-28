# DocuMind

Local-first, offline-capable OCR & document intelligence desktop app. Drop a PDF, book, or folder of scanned images in and get a structured, chapter-separated Markdown document — without uploading anything to the cloud.

See the planning docs before touching the pipeline code:

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — system design, module/provider boundaries
- [`TECH_DECISION.md`](TECH_DECISION.md) — why Tauri, RapidOCR/ONNX Runtime, llama.cpp
- [`DEPENDENCY_STRATEGY.md`](DEPENDENCY_STRATEGY.md) — no-system-install runtime/model management
- [`MODEL_STRATEGY.md`](MODEL_STRATEGY.md) — OCR language packs, AI model tiering
- [`PACKAGING.md`](PACKAGING.md) — per-OS installers, CI matrix
- [`MVP_PLAN.md`](MVP_PLAN.md) — phased delivery plan and current status

## Stack

- Shell: [Tauri v2](https://tauri.app/) (Rust core + React/TypeScript UI via Vite)
- OCR: system Tesseract via CLI for now (`core/ocr/tesseract.rs`) — RapidOCR-ONNX is the documented target default (`TECH_DECISION.md` §2) but not yet implemented; see `MVP_PLAN.md`'s Phase 2 engine-sequencing note for why
- Spelling: dictionary-based error flagging/suggestion, not auto-applied (`core/spellcheck.rs`) — see `MODEL_STRATEGY.md` §5 for the AI-vs-heuristic boundary
- Local AI: llama.cpp sidecar (Phase 4, not yet implemented)

## Development

Prerequisites: Rust toolchain, Node.js 22+, and on Linux:
- Build headers: `libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev patchelf` (see [Tauri's Linux prerequisites](https://tauri.app/start/prerequisites/); note `libayatana-appindicator3-dev`, not `libappindicator3-dev`, on Ubuntu 24.04+ — they conflict)
- Runtime tools the app currently shells out to: `tesseract-ocr tesseract-ocr-eng tesseract-ocr-vie poppler-utils`, and `hunspell-vi` for Vietnamese spell-suggestions (English uses the pre-installed `hunspell-en-us`)

```bash
npm install
npm run tauri dev
```

Run the Rust test suite:

```bash
cd src-tauri && cargo test
```

## Building installers locally

```bash
npm run tauri build                          # all bundle targets for this OS
npm run tauri build -- --bundles deb,appimage # just these two (skips e.g. rpm if rpmbuild isn't installed)
```

Output lands in `src-tauri/target/release/bundle/<format>/`. Verified locally on Linux: `.deb` (installable, correct icons/desktop entry) and `.AppImage` (self-contained, runs standalone — confirmed by launching it directly).

## CI/CD

- `.github/workflows/ci.yml` — runs on every push/PR to `main`: Rust + TypeScript checks and the full test suite (Linux only, for fast feedback).
- `.github/workflows/release.yml` — the full cross-platform build matrix (Windows, macOS arm64 + Intel, Linux) from `PACKAGING.md` §4, triggered by pushing a `v*.*.*` tag or manual dispatch. Publishes a **draft** GitHub Release with all installers attached — a human reviews and publishes it, nothing goes out automatically. Builds are currently unsigned (`PACKAGING.md` §5); Windows/macOS installers will show an OS security warning until code-signing certificates are set up.

To cut a release: bump `version` in `src-tauri/tauri.conf.json` and `package.json`, commit, tag (`git tag v0.1.0 && git push --tags`), and check the draft release once CI finishes.

## Status

Phase 2 (OCR MVP) complete and verified against real documents; Phase 6 (Packaging) groundwork — CI + cross-platform release workflow, local Linux packaging verified — is in place ahead of Phases 3-5. See [`MVP_PLAN.md`](MVP_PLAN.md) for the full phase breakdown, exit criteria, and what's still open.
