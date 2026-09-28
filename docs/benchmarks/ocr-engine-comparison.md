# OCR Engine Comparison — Status: Not Yet Run

Referenced by `MODEL_STRATEGY.md` §1 and `MVP_PLAN.md` Phase 2 as the gate before freezing RapidOCR-ONNX as the default OCR engine (TECH_DECISION.md §2 records it as a starting hypothesis, not a final commitment).

**This document intentionally contains no results yet.** Recording a benchmark methodology without data would be indistinguishable from fabricating numbers; this placeholder exists so the gate isn't silently skipped, not to stand in for the real comparison.

## Why it hasn't run

Two prerequisites are both still open:

1. **A RapidOCR-ONNX `OCRProvider` implementation to benchmark.** Phase 2 shipped Tesseract first (see `MVP_PLAN.md` Phase 2 engine-sequencing note) because it required no model download pipeline. RapidOCR-ONNX is the remaining Phase 2 OCR work.
2. **A real fixture corpus** (spec §43): EN/VI/JA/ZH books, technical books, scanned/native/hybrid PDFs, low-quality scans, rotated pages, multi-column layouts, tables. None of this exists in the repo yet — synthetic single-line test images (used for unit tests, see `core/preprocess.rs`, `core/job.rs`) are not a substitute for real-document accuracy measurement.

## Methodology (to run once both prerequisites land)

For each fixture document:

1. Run through each candidate engine (`OCRProvider` implementation), producing `full_document.md` + per-page confidence.
2. Score against a hand-verified ground-truth transcript using:
   - **Character Error Rate (CER)** and **Word Error Rate (WER)** (Levenshtein distance / reference length).
   - **Layout preservation**: whether paragraph/line breaks and reading order match the source (manual spot-check, since this is not well captured by CER/WER alone).
3. Record per-fixture and aggregate scores in a table here, broken out by language and document type (native/scanned/hybrid, clean/low-quality, rotated), since "best on average" can hide an engine that's unusable on one language or document class.
4. State the conclusion explicitly: confirm RapidOCR-ONNX as default, or revise `TECH_DECISION.md` §2 with the reasoning.

## Result table (empty until the run above happens)

| Fixture | Language | Type | Engine | CER | WER | Layout OK? |
|---|---|---|---|---|---|---|
| — | — | — | — | — | — | — |
