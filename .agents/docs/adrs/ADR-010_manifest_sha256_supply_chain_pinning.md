# ADR-010: Model Manifest SHA-256 Supply Chain Pinning
**Status:** Accepted  
**Date:** 2026-11-21  
**Context:** TICKET-10 (Artifact Supply Chain Integrity & Model Verification)  
**Governed Subsystems:** `scripts/package-release.ps1`, `scripts/verify-beta.ps1`, `daemon-rust/src/config.rs`

---

## 1. Context & Problem Statement
Better_Jarvis depends on pre-trained local neural models (Silero VAD ONNX, Piper TTS ONNX, Whisper.cpp binary weights, and all-MiniLM-L6-v2 vector embedder). Downloading or copying model files without cryptographic pinning creates critical supply chain risks: corrupted weights cause silent inference failures or NaN audio output, and tampered weights could execute adversarial behavior or arbitrary code in vulnerable runtime parsers.

## 2. Decision Drivers
- Deterministic supply chain verification before any binary release or execution.
- Automated gate blocking builds if any model file is missing or altered.
- Zero reliance on unauthenticated network downloads at runtime.

## 3. Decision
1. **Canonical Manifest File:** Maintain an authoritative `manifest.sha256` at the repository root containing exact SHA-256 hashes of all required model artifacts:
   - `models/silero_vad.onnx`
   - `models/all-MiniLM-L6-v2-q8.onnx`
   - `models/en_US-ryan-medium.onnx`
   - `models/en_US-ryan-medium.onnx.json`
   - `models/ggml-base.en.bin`
2. **Build-Time Verification Gate:** In `scripts/verify-beta.ps1` Stage 2, compute SHA-256 hashes of all files in `models/` against `manifest.sha256`. If any hash fails or a file is missing, abort verification immediately with exit code 1.
3. **Packaging Integrity:** `scripts/package-release.ps1` copies verified models into `dist/models/` and re-verifies hashes during release assembly.

## 4. Consequences
- **Positive:** Guarantees 100% bit-for-bit authenticity of all neural weights deployed to user environments.
- **Trade-off:** Updating any neural model requires an explicit update to `manifest.sha256` approved through the standard engineering change workflow.

## 5. Verification Evidence
- Stage 2 of `scripts/verify-beta.ps1` executes `Get-FileHash -Algorithm SHA256` on every model artifact and confirms 100% match against `manifest.sha256`.
