# Model Strategy

Covers OCR model selection and Local AI model tiering/selection (§9–12, §30).

## 1. OCR models

Default engine (see `TECH_DECISION.md` §2): RapidOCR's ONNX export of PP-OCR detection + recognition models, plus a PP-Structure-derived ONNX layout/table model. Organized as independently installable **language packs**, not one monolithic blob, so a user who only reads English doesn't pay for Japanese weights:

```
models/ocr/
├── detection/           # mostly language-agnostic (DBNet-style), one shared model
├── recognition/
│   ├── english/         (~5–10 MB, bundled with installer, see DEPENDENCY_STRATEGY §3)
│   ├── vietnamese/
│   ├── japanese/
│   ├── chinese/
│   └── multilingual/    # broader coverage, larger, lower per-language accuracy than dedicated packs
└── layout/               # PP-Structure ONNX (layout + table), one shared model, downloaded on demand
```

`ModelManager.recommended()` for OCR defaults to `english` + whichever language pack matches the OS locale, offered (not forced) during setup (§5 Step 2 shows only the required minimum downloading; additional language packs are offered from Settings → Models, §30).

**Benchmark gate before default-freeze (§12):** Phase 2 of `MVP_PLAN.md` runs the fixture set from §43 (EN/VI/JA/ZH, native/scanned/hybrid PDFs, low-quality scans, rotated pages, tables) through RapidOCR-ONNX and Tesseract, scoring character/word accuracy and layout preservation. The result is committed as `docs/benchmarks/ocr-engine-comparison.md` and either confirms RapidOCR-ONNX as default or revises this document — this file states the hypothesis and the gate, not a final unverified claim.

## 2. Local AI models — tiering strategy (§9–10)

`LocalAIProvider` is backed by one of several GGUF models, selected automatically by `HardwareManager` + `ModelManager.recommended()`, overridable by the user:

| Tier | Example model class | Quantization | Disk | RAM at load | Recommended when |
|---|---|---|---|---|---|
| Small | ~1–3B instruct model | Q4_K_M | ~1.5–2.5 GB | ~2–3 GB | RAM < 8 GB, or no GPU + weak CPU |
| Medium | ~7–8B instruct model | Q4_K_M | ~4.5–5.5 GB | ~6–7 GB | RAM 8–16 GB, or 16GB+ with no/low-VRAM GPU |
| Large | ~13B+ instruct model | Q4_K_M / Q5_K_M | ~8–10 GB | ~10–12 GB | RAM > 16 GB and (GPU with ≥8GB VRAM, or a strong multi-core CPU willing to accept slower inference) |

The task these models perform (§20: structure/heading/chapter **classification**, short-span OCR-error suggestions — never open-ended generation) does not need a frontier-scale model; a well-tuned small/medium instruct model is expected to saturate accuracy on this task well before 13B+, so "Large" exists for users who want it, not because the task demands it. This assumption is validated in Phase 4 alongside the OCR benchmark gate.

Model family choice is deliberately left as a swappable config (`model_id` in the manifest), not hardcoded to one vendor's weights, so the specific GGUF model per tier can be updated as better small instruct models ship, without an architecture change — this is exactly what the `LocalAIProvider` abstraction (§10, `ARCHITECTURE.md` §2) is for.

`ModelManager.recommended()` decision logic:

```
if vram_bytes >= 8_GB && gpu.supports_offload() → Large (GPU-offloaded)
elif ram_bytes >= 16_GB → Medium
elif ram_bytes >= 8_GB  → Small
else                     → Small, with a warning that AI features may be slow; still user-installable
```

Stability over size (§9): the selector never recommends a tier whose *load-time* RAM estimate exceeds a safety margin (default: 70%) of detected free RAM, to avoid the AI feature itself becoming a source of OOM/instability in an app whose primary job (OCR over huge documents) must keep working regardless.

## 3. Manual override (§10)

Settings → Models lets the user pick any installed tier or install a non-recommended one, with the size/RAM estimate always shown before download starts (§27/§29 disk-space and size transparency requirements apply identically here). Switching tiers doesn't require reprocessing already-completed documents; it only affects `LocalAIProvider` calls in future/re-run jobs.

## 4. Distribution

Same mechanism as `DEPENDENCY_STRATEGY.md` §4–5: manifest-listed, SHA-256-verified, resumable downloads into `app-data/models/`. AI models are the largest artifacts in the system (multi-GB) so they are always download-on-demand, never bundled in the standard installer (§35), and are the main content of the optional Full Offline Bundle (§36).

## 5. What the AI model is not allowed to do (§20, §46)

`LocalAIProvider::classify_structure` returns a typed `StructureHints` (labels + confidence), never free-text replacing OCR output. This is enforced at the trait boundary (`ARCHITECTURE.md` §2), not by prompt instruction alone — the return type has no field capable of carrying rewritten body text — so "AI silently rewrote the book" is structurally excluded rather than merely discouraged. A future, explicitly user-invoked "rewrite/clean up" feature (§20's stated exception) would be a distinct, separately-labeled provider call, never the default structure pass.

The same boundary applies to OCR error correction (§2's "OCR error correction suggestions"): a planned `LocalAIProvider::suggest_correction` would return a *suggestion* (original span + proposed replacement + confidence) for the UI to show, never a silent in-place rewrite of `raw/page-NNN.txt`. Until that lands, `core/spellcheck.rs` (`MVP_PLAN.md` Phase 4) provides the non-AI half of this: dictionary-based *flagging* only — it marks a word as not found in any loaded dictionary, which is evidence a word might be wrong, not a correction. The UI must keep these two clearly distinct ("flagged" vs. "corrected") so users don't mistake a flag for a fix.
