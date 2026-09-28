# Dependency & Runtime Management Strategy

Governs how DocuMind gets OCR/AI native runtimes onto a user's machine without requiring any manual system installation (§6, §40).

## 1. What counts as a "dependency" here

| Component | What it is | Mandatory? |
|---|---|---|
| ONNX Runtime | Native shared library (`.dll`/`.dylib`/`.so`) that executes OCR/layout ONNX models | Yes |
| OCR models (detection/recognition/layout, per language) | ONNX weight files | Yes (at least one language pack) |
| llama.cpp (`llama-server` binary) | Native executable, local inference server | Only if AI is installed |
| AI model (GGUF) | Weight file | Only if AI is installed |

Nothing here is a system-wide install (no PATH edits, no registry entries beyond the app's own uninstall entry, no `pip`/`npm`/`brew` invocation on the user's behalf).

## 2. Platform/architecture targeting (§8)

`DependencyManager::detect_platform()` resolves to one of:

```
windows-x64
macos-arm64
macos-x64
linux-x64
linux-arm64
```

using `std::env::consts::{OS, ARCH}` plus a runtime CPU-feature check (AVX2 availability on x86_64, since ONNX Runtime/llama.cpp builds branch on it) and a GPU probe (`HardwareManager`) to pick, where available, a GPU-accelerated build (DirectML EP on Windows, CoreML EP on macOS, CUDA EP on Linux with NVIDIA) over the CPU-only build. Every downloadable artifact is keyed by `(component, version, platform-target, [gpu-variant])`, and the manifest (see §4) only ever lists artifacts matching the detected target — a Windows x64 machine is structurally incapable of being offered a linux-arm64 blob.

## 3. Bundled vs. downloaded

Installer strategy (full detail in `PACKAGING.md`) splits dependencies into two tiers:

- **Bundled in every installer** (small, always needed): ONNX Runtime CPU build for the target platform (~15–20 MB), and a minimal OCR model set for English (detection+recognition, ~8–15 MB combined). This lets the app do useful English OCR **immediately after install with zero network access**, satisfying the spirit of "OCR must not silently fail to be available" even before setup finishes.
- **Downloaded on first run / on demand**: additional language packs, GPU-accelerated runtime variants, the llama.cpp binary, and all AI models. Selected and fetched by the Setup Wizard (§5) or later from Settings (§26/§30).

The **Full Offline Bundle** (§36) is a separate, larger installer artifact that pre-includes everything above (all default-tier language packs + default AI model) for machines with no internet access at all; it is built by the same CI pipeline with a `--offline-bundle` flag, not maintained as separate code.

## 4. Download manager (§27)

A single `DownloadManager` service backs every artifact fetch (runtime, models, app updates):

- Chunked HTTP range requests where the server supports it → pause/resume, including **resume after application restart** (in-progress download state, including byte offset and destination temp path, is persisted to `config.json`/`downloads/*.part.json`).
- Retry with exponential backoff on transient failures; explicit user-triggered retry always available.
- Pre-flight disk space check (§29) against `HardwareManager::disk_free_bytes()` with the artifact's known size from the manifest, comparing `required` vs `available` and refusing to start if insufficient, with the exact numbers shown to the user.
- Cancellation removes partial files; nothing partially-downloaded is ever left in a state `ModelManager`/`DependencyManager` would treat as installed.

## 5. Integrity verification (§28)

Every distributable artifact (runtime binaries, OCR models, AI models) ships with an entry in a **signed manifest** (JSON, fetched over HTTPS, itself checksummed and pinned to a known public key shipped in the app binary):

```json
{
  "component": "onnxruntime",
  "version": "1.20.0",
  "target": "windows-x64",
  "gpu_variant": "directml",
  "url": "https://.../onnxruntime-1.20.0-windows-x64-directml.zip",
  "sha256": "…",
  "size_bytes": 45213184
}
```

Flow: `Download → SHA-256 verify → (match) Install : (mismatch) discard + retry, never install`. No downloaded binary or model is ever executed/loaded before its checksum matches the manifest. The manifest itself is fetched over TLS and its own hash is checked against the value pinned at build time, so a compromised CDN can't silently swap the manifest and pass a malicious binary's checksum as legitimate without also compromising the app's release signing — this is the practical ceiling for a desktop app's supply-chain defense and is documented as such rather than overclaimed.

## 6. Runtime lifecycle (§40)

```
checkRuntime()   → InstallState { Installed(version) | Missing | Corrupt }
downloadRuntime()→ DownloadHandle (progress events over Tauri IPC)
installRuntime() → unpack to app-data/runtime/<target>/, verify checksum, mark installed in local state db
verifyRuntime()  → re-hash on-disk files against manifest, used by Settings "Check for Updates" and periodic self-heal
repairRuntime()  → delete + re-download + reinstall, used when verifyRuntime() fails or the user hits "Reinstall OCR"
```

`repairRuntime` is also the automatic path taken if the app detects at startup that an installed runtime's on-disk hash no longer matches (e.g., interrupted OS update, disk corruption) — it surfaces as a blocking "Repairing OCR engine…" state on launch rather than a silent fallback to a broken engine, satisfying §2's "must not silently continue without OCR."

## 7. No system-wide installs — how each component avoids it

- **ONNX Runtime**: linked as a dynamic library loaded from an app-controlled path (`app-data/runtime/onnxruntime/...`) via explicit `dlopen`/`LoadLibrary` path, not the system loader's default search path. Never installed to a system library directory.
- **llama.cpp**: a plain executable in `app-data/runtime/llama-server/...`, launched as a child process with its working directory and model path passed explicitly. No service/daemon registration.
- **Models**: plain files under `app-data/models/`. Removing them is a file delete (`ModelManager::remove`), nothing else to clean up.

## 8. Failure UX (§2, §27)

If any dependency install fails (download error, checksum mismatch after N retries, unpack failure, disk full), the Setup Wizard and Settings page must show the specific failure reason (not a generic "Setup failed") and a retry action. OCR is mandatory (§2/§26), so the app's main workflow stays gated behind a blocking "OCR is not ready" screen — never a degraded silent mode — until this succeeds or the user explicitly opts into a smaller fallback engine (Tesseract bundled tier) offered as the retry alternative if the primary ONNX runtime repeatedly fails on that machine.
