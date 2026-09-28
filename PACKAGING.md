# Packaging & CI/CD Strategy

Covers §7 (platform support), §33 (cross-platform packaging), §34 (CI/CD), §35–36 (installer strategy / full offline bundle).

## 1. Targets

| OS | Formats | Architectures |
|---|---|---|
| Windows 10+/11+ | `.msi` (primary), `.exe` (NSIS, secondary) | x64 |
| macOS | `.dmg` containing `.app` | arm64 (Apple Silicon), x86_64 (Intel) — universal binary evaluated in Phase 6 if the native dep set (ONNX Runtime, llama.cpp) has clean universal builds; otherwise ship arch-specific and let the OS App Store-style installer picker (or a single dmg with a tiny launcher that fetches the right sidecar) pick correctly. Default: arch-specific dmgs, simplest and most reliable. |
| Linux | `.AppImage` (primary, works across distros without install), `.deb` (Debian/Ubuntu) | x64, arm64 for AppImage/deb; `.rpm` (Fedora) as a stretch target once the first two are stable |

All produced via Tauri's built-in bundler (`tauri build`), which wraps `cargo bundle`-equivalent tooling per target — no separate packaging tool per OS to maintain.

## 2. Two-tier installer (§35–36)

**Standard installer** (default, what most users download):
- Contains: the app binary, UI assets, ONNX Runtime CPU build, English OCR language pack. Target size: well under 100 MB (Tauri baseline is 5–15 MB; the bundled CPU runtime + English models add roughly 25–40 MB).
- First launch runs the Setup Wizard, which downloads anything beyond that baseline (other language packs, GPU runtime variant, AI runtime+model) per `DEPENDENCY_STRATEGY.md`.

**Full Offline Bundle** (§36, opt-in, advertised as "for machines without internet access"):
- Same CI pipeline, `--offline-bundle` build flag, pre-packing: all default-tier OCR language packs, the CPU ONNX Runtime build (GPU variants still downloaded later — they're detected per-machine and pre-bundling every GPU variant would bloat every install for a case that's optional to begin with), the llama.cpp binary, and the default-recommended AI model tier (Medium, the broadest-fit default per `MODEL_STRATEGY.md`).
- Distributed as a separate, clearly-labeled download (multi-GB) alongside the standard installer on the releases page — never the default recommendation, so most users keep the small install.

## 3. Platform binary selection at build time

CI produces one artifact set per `(os, arch)` row of the build matrix; `platform/<target>/` binaries (ONNX Runtime, llama-server) are fetched from their upstream release artifacts (or built from source for targets upstream doesn't publish, e.g., linux-arm64 for some GPU EPs) during the CI job and placed where Tauri's `externalBin`/bundled-resources config expects them, keyed by the target-triple naming convention Tauri's sidecar mechanism requires (`<name>-<target-triple>[.exe]`).

## 4. GitHub Actions matrix (§34)

```
push (tag) 
  → matrix: [windows-latest, macos-latest (arm64), macos-13 (x64), ubuntu-latest]
      → checkout
      → setup Rust toolchain (target-specific)
      → fetch/verify pinned native dep binaries (ONNX Runtime, llama.cpp) for this target
      → cargo build --release
      → tauri build  (produces platform-native package)
      → run test suite (unit + pipeline fixture tests, §43)
      → upload artifact
  → release job: collect all matrix artifacts → attach to GitHub Release, generate/publish the signed manifest (DEPENDENCY_STRATEGY §5) pointing at model/runtime download URLs for this version
```

Model weights and heavy runtime binaries are **not** rebuilt per CI run — they're versioned, hosted objects (e.g., GitHub Releases assets or object storage) referenced by the manifest; CI publishes the manifest, it doesn't re-upload multi-GB files on every commit.

**Implemented** (`.github/workflows/`):
- `ci.yml` — runs on every push/PR to `main`: installs Linux build + runtime deps (webkit2gtk headers, poppler-utils, tesseract + language packs, hunspell-vi), then `tsc --noEmit`, `cargo check --all-targets`, `cargo test --all-targets`. Fast feedback on every change, Linux-only (the full cross-platform matrix is expensive to run on every push).
- `release.yml` — the actual build matrix above, triggered on `v*.*.*` tags or manual dispatch. Uses `tauri-apps/tauri-action` (the framework's own official action) rather than hand-rolling `tauri build` + artifact upload per OS, since it already handles per-platform bundler invocation and GitHub Release asset attachment correctly. Publishes as a **draft** release (a human reviews and publishes, per this project's own guidance to confirm before anything user-facing goes out) rather than publishing automatically.
- ONNX Runtime/llama.cpp binary-fetching and the signed manifest publish step are not yet implemented — they don't exist to fetch/publish yet, since OCR is currently the Tesseract CLI (system-installed, not bundled) per `MVP_PLAN.md`'s Phase 2 engine-sequencing note. `deb.depends` in `tauri.conf.json` currently lists `tesseract-ocr`/`poppler-utils` as hard package dependencies — an accurate reflection of today's CLI-shellout implementation, to be replaced by the bundled-sidecar model (§3 above) once RapidOCR-ONNX + `ort` lands.

## 5. Code signing (deferred, noted for completeness)

Windows (Authenticode) and macOS (Developer ID + notarization) signing are required for a smooth install experience (unsigned builds trigger SmartScreen/Gatekeeper warnings) but depend on the team obtaining certificates — out of scope for the architecture decision itself, tracked as a Phase 6 checklist item in `MVP_PLAN.md` rather than blocking earlier phases. Linux packages are not typically signed in the same way; `.deb`/`.AppImage` integrity instead relies on the same SHA-256 manifest users can verify against the release page.

## 6. Versioning

Single semantic version across the app binary, the manifest schema, and the model/runtime compatibility matrix (a manifest entry declares the minimum app version it's compatible with), so an older app build never silently pulls a runtime/model it doesn't know how to drive.
