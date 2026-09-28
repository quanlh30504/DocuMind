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
- OCR: RapidOCR's ONNX models via ONNX Runtime (Phase 2)
- Local AI: llama.cpp sidecar (Phase 4)

## Development

Prerequisites: Rust toolchain, Node.js 22+, and the [Tauri Linux prerequisites](https://tauri.app/start/prerequisites/) (`webkit2gtk`, `librsvg2`) if developing on Linux.

```bash
npm install
npm run tauri dev
```

Run the Rust test suite:

```bash
cd src-tauri && cargo test
```

## Status

Phase 1 (Foundation) — see [`MVP_PLAN.md`](MVP_PLAN.md) for the full phase breakdown and exit criteria.
