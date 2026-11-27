# ADR-014: Windows Visual Context, Desktop OCR & Fail-Closed Privacy Guard
**Status:** Accepted  
**Date:** 2026-11-23  
**Context:** TICKET-14 (Screen Vision, Desktop Context & Privacy Boundaries)  
**Governed Subsystems:** `daemon-rust/src/tools/screen_vision.rs`

---

## 1. Context & Problem Statement
Enabling the agent to inspect the user's active desktop screen is necessary for multimodal assistance (e.g. debugging UI, analyzing charts, reading error dialogs). However, unrestricted screen capture introduces severe security and privacy hazards: capturing password managers, online banking portals, medical records, or credential windows can inadvertently inject secrets into cloud LLM contexts or persistent disk logs.

## 2. Decision Drivers
- Strict privacy preservation: password managers and sensitive windows must NEVER be captured.
- Zero persistent raw image files on disk (in-memory capture and processing only).
- Bounded OCR extraction latency on Windows 11.
- Immediate prompt taint escalation when visual content is processed.

## 3. Decision
1. **VisionPrivacyGuard:** Before executing any GDI screen capture, inspect the foreground window title and process image name. If the window matches the sensitive window regex deny list (e.g. `1Password`, `Bitwarden`, `KeePass`, `LastPass`, `Bank`, `Sign In`, `Private Browsing`, `KeePassXC`), immediately abort capture and return an explicit `PrivacyBlocked` status.
2. **In-Memory GDI Capture:** Capture the active window or bounding box exclusively in memory via Win32 GDI (`BitBlt` into a memory device context). Never write raw screenshot `.png` or `.bmp` files to the filesystem.
3. **WinRT OCR Extraction:** Pass raw image buffers directly to the Windows 11 native WinRT OCR engine (`Windows.Media.Ocr.OcrEngine`) for local, on-device optical character recognition.
4. **Taint Tracking Enforcement:** Wrap all extracted OCR text in per-turn random CNG nonces (`<untrusted_data_{nonce}>...`) and set `context_tainted = true`. All subsequent tool actions require Tier 3 interactive HUD confirmation.

## 4. Consequences
- **Positive:** Immune to accidental credential theft from password manager windows; zero disk footprint for screen grabs; prompt injection from on-screen text cannot hijack execution without user approval.
- **Trade-off:** Capturing protected content requires the user to explicitly minimize sensitive windows before invoking vision assistance.

## 5. Verification Evidence
- `daemon-rust` unit test `tools::screen_vision::tests::test_vision_privacy_guard_blocks_sensitive_windows` proves `1Password`, `Bitwarden`, and `Bank` windows are denied with 100% reliability.
- `tools::screen_vision::tests::test_winrt_ocr_extracts_text` verifies text extraction and nonce wrapping.
