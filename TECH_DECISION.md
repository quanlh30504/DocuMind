# Technology Decisions

This document records the comparative analysis behind DocuMind's core technology choices, as required before any implementation begins. Each section compares real alternatives against the product's non-negotiable constraints: **local-first, offline-capable, cross-platform (Windows/macOS/Linux), no mandatory system-wide dependencies, production-shippable installer size.**

---

## 1. Desktop Application Framework

| Criterion | Electron | Tauri v2 | Python desktop (PyQt/BeeWare/Briefcase) |
|---|---|---|---|
| Installer/runtime size | 120–200 MB (bundles Chromium+Node) | 5–15 MB (uses OS webview) | 60–150 MB (bundles CPython + libs) |
| Native perf / memory | Heavy (full Chromium per instance) | Light (native Rust host, OS webview) | Medium, GIL limits concurrency |
| Binding to native OCR/AI (Rust/C++/ONNX) | Via Node native addons (N-API), workable but adds build complexity | Direct — app core **is** Rust, first-class FFI to ONNX Runtime / llama.cpp | Via ctypes/pybind11, workable but ties everything to CPython's packaging story |
| Cross-platform packaging | Mature (electron-builder) | Mature, built into Tauri CLI (`tauri build`) for msi/exe, dmg/app, deb/AppImage/rpm | Fragmented (PyInstaller/Briefcase/Nuitka), less reliable, GPU-accelerated UI libs are weak |
| Security model | Node has full OS access unless sandboxed carefully | Explicit capability/permission allowlist by default | Full OS access, no built-in sandboxing |
| Sidecar binaries (bundling OCR/AI engines per platform) | Supported, manual wiring | First-class (`externalBin`, `tauri.conf.json`) with target-triple naming convention | Manual, must wire per-OS |
| Ecosystem maturity | Very mature, huge community | Mature since v2 GA (Oct 2024), growing fast, used in production by many privacy-focused apps | Mature language, weak *desktop packaging* ecosystem specifically |

**Decision: Tauri v2.**

Rationale:
- The product's core value (OCR + local AI + heavy file I/O over 1000+ page documents) is CPU/memory sensitive; Electron's Chromium-per-instance overhead works against the "process a 2000-page book without exploding RAM" requirement.
- Tauri's Rust core lets the document pipeline (PDF parsing, image preprocessing, OCR orchestration, AI inference orchestration) be written in the same language as the shell, with no IPC serialization tax to a separate Python subprocess for the hot path.
- `externalBin` sidecars map exactly onto the spec's `platform/{windows-x64,macos-arm64,...}` binary-selection requirement (§8).
- Installer stays small (§35, §36 explicitly want a lightweight base installer with a separate full offline bundle) — only Tauri meets this without extra engineering.
- Pure-Python desktop packaging (PyInstaller etc.) is explicitly what the spec tells us to avoid (§6: don't require the user to install Python).

Trade-off accepted: Tauri's UI layer depends on the OS webview (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux) rather than a bundled, version-pinned Chromium. We mitigate this by targeting broadly-supported web APIs and testing on the CI matrix (§34), and by requiring WebView2 (auto-installed on Windows 11, redistributable bundled in the installer on Windows 10).

Frontend stack inside Tauri: React + TypeScript + Vite (standard, well-documented Tauri template, not a novel decision worth its own doc).

---

## 2. OCR Engine

Evaluated against: English/Vietnamese/Japanese/Chinese accuracy, layout/table detection, CPU performance, GPU optionality, offline operation, packaging complexity, licensing.

| Engine | Accuracy (Latin) | Accuracy (CJK) | Vietnamese | Layout/table | Packaging | License |
|---|---|---|---|---|---|---|
| **Tesseract** | Good on clean scans, weak on complex layouts | Weak-to-medium | Medium (needs `vie` traineddata, diacritics are error-prone) | None built-in (needs separate layout model) | Excellent — small (~10MB), static binary, no ML runtime needed | Apache-2.0 |
| **PaddleOCR (native, PaddlePaddle inference)** | Strong | Strongest of the group (PP-OCRv4/v5 trained heavily on CJK corpora) | Good with community fine-tunes | Best-in-class (PP-Structure gives tables, layout, reading order) | Poor for our case — requires the PaddlePaddle inference runtime (large, historically awkward to statically link, thin Windows/ARM64 desktop story) | Apache-2.0 |
| **RapidOCR** (PP-OCR detection/recognition models re-exported to **ONNX**, run via ONNX Runtime) | Strong (inherits PP-OCRv4 weights) | Strong (inherits PaddleOCR CJK weights) | Good with the multilingual/Vietnamese recognition model | Good (layout/table models also available as ONNX) | **Excellent for us** — ONNX Runtime has official static/dynamic builds for Win/macOS(arm64+x64)/Linux(x64/arm64), with a mature Rust binding (`ort` crate), no Python required at runtime | Apache-2.0 |
| **EasyOCR** | Medium | Medium | Weak | None | PyTorch runtime required — heavy, same packaging problem as native Paddle, worse | Apache-2.0 |

**Decision: RapidOCR's ONNX model set, executed through ONNX Runtime via the Rust `ort` crate, as the default engine. Tesseract bundled as a lightweight fallback/offline-recovery path** (e.g., constrained hardware, or as a fast pre-pass for native-text confidence checks) but not the default.

Rationale:
- RapidOCR gives us PaddleOCR-grade accuracy (best of the group for CJK, competitive for Latin/Vietnamese with the multilingual recognition model) **without** dragging in the PaddlePaddle framework, whose desktop cross-platform story is the weakest part of the native option.
- ONNX Runtime is exactly the kind of dependency §6/§8 wants: a single native library per platform/arch, no interpreter, statically selectable from `platform/<target>/`, with official GPU execution providers (DirectML on Windows, CoreML on macOS, CUDA/ROCm on Linux) for later performance work — satisfying §9/§11's "GPU where available, CPU fallback" requirement.
- PP-Structure-derived ONNX layout/table models give us table/figure/reading-order detection (§18) for free from the same model family, instead of a second vendor.
- This is a starting hypothesis, not a final commitment: §12 explicitly requires benchmarking real documents before locking the default. Phase 2 (§47) includes a benchmark pass across the fixture set in §43 (EN/VI/JA scanned + native PDFs, low-quality scans, rotated pages, tables) comparing RapidOCR-ONNX vs. Tesseract vs. (if time permits) a PaddleOCR-native spike, before the default is frozen. This document records the *starting* engine, `MODEL_STRATEGY.md` records how the benchmark gate works.

---

## 3. Local AI Runtime

| Runtime | Cross-platform | Bundling into an app | GPU support | Maturity/API stability | Notes |
|---|---|---|---|---|---|
| **llama.cpp** | Yes — CPU-first, portable C/C++, official Metal/CUDA/Vulkan/DirectML/ROCm backends | Yes — compiles to a single binary per platform+arch (`llama-server`), or linked directly via Rust bindings (`llama-cpp-rs` / raw FFI) | Yes, via backend flags at build time; degrades cleanly to CPU | Very mature, huge model ecosystem via GGUF | No external daemon required; we own the process lifecycle |
| **ONNX Runtime GenAI** | Yes | Yes | Yes (same EPs as OCR runtime — reuse) | Improving fast but LLM tooling/quantization ecosystem is thinner than GGUF | Would let OCR and AI share one runtime dependency, which is attractive, but GGUF's quantization tooling and model availability (esp. small instruct models tuned for classification-style tasks) is more mature today |
| **MLX** | Apple Silicon only | N/A cross-platform | Apple GPU only | Mature on macOS, zero coverage elsewhere | Would require a *second* runtime just for macOS, doubling maintenance for one platform's marginal perf gain over llama.cpp's Metal backend |
| **Ollama** | Yes | Poor fit — it's designed as a user-installed background daemon with its own model store/registry, not an embeddable library | Yes | Very mature UX for end users running it themselves | Violates §11's explicit instruction: don't require the user to separately install/manage a runtime; we'd be bundling and silently managing someone else's daemon, its port, and its model cache format — more moving parts than owning a `llama-server` sidecar directly |

**Decision: llama.cpp**, invoked as a Tauri sidecar (`llama-server`, OpenAI-compatible HTTP API on localhost, loopback-only) or via direct Rust FFI for tighter control — sidecar chosen first for implementation simplicity and process isolation (a model crash doesn't take down the UI).

Rationale:
- One runtime, every platform, via backend compile flags — satisfies §11 directly ("prefer the simplest reliable option," avoid a second runtime for Apple Silicon).
- GGUF quantization gives predictable memory footprints we can map directly to the tiered model strategy in §9/§10 (see `MODEL_STRATEGY.md`).
- Runs entirely offline once the model file is on disk — no daemon, no telemetry, no registry call, which is not guaranteed with Ollama's default behavior.
- localhost-only binding + no auth token needed since it's loopback and single-user; still gated behind the app's Offline Mode network guard (§14) for defense in depth.

---

## 4. Model Distribution Strategy

See `MODEL_STRATEGY.md` for the full design. Summary: models (OCR ONNX weights, AI GGUF weights) are **not** embedded in the installer; they are downloaded on first run / on demand, per §35–36, verified by SHA-256 manifest (§28), and stored in the app's data directory, not a system path.

## 5. Dependency Management Strategy

See `DEPENDENCY_STRATEGY.md`. Summary: no system Python/Node/CUDA required. ONNX Runtime and the llama.cpp sidecar are prebuilt, platform+arch-specific binaries shipped either inside the installer (small, <20MB each) or downloaded on first run, selected by the Rust `DependencyManager`/`HardwareManager` using the `sysinfo` crate for CPU/RAM/disk and platform-specific GPU queries (NVML for NVIDIA, Metal API probing on macOS, DXGI on Windows) for VRAM.

## 6. Packaging Strategy

See `PACKAGING.md`. Summary: Tauri's built-in bundler targets, driven by a GitHub Actions build matrix (§34), producing `.msi`/`.exe` (Windows), `.dmg`/`.app` for arm64 and x86_64 (macOS, universal binary where the linked native deps allow it), and `.AppImage`/`.deb`/optionally `.rpm` (Linux).

## 7. Offline & Security Strategy

See `ARCHITECTURE.md` §"Offline & Network Boundary" and `DEPENDENCY_STRATEGY.md` §"Integrity Verification." Summary: a single network-egress choke point in the Rust core; Offline Mode flips it to deny-all except explicit, user-initiated downloads; every downloaded artifact is checksum-verified against a signed manifest before it is ever executed or loaded.
