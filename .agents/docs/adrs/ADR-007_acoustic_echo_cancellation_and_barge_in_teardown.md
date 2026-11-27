# ADR-007: Acoustic Echo Cancellation & Voice Gated Barge-In Teardown
**Status:** Accepted  
**Date:** 2026-11-21  
**Context:** TICKET-07 (Hardening & Production Execution Plan)  
**Governed Subsystems:** `host-cpp/src/audio/aec_processor.cpp`, `host-cpp/src/audio/piper_tts.cpp`

---

## 1. Context & Problem Statement
When Piper TTS plays through desktop speakers, microphone capture picks up the audio, causing self-triggering transcription loops and preventing user interruptions (barge-in).

## 2. Decision
1. **AEC NLMS Filter:** Implement Normalized Least Mean Squares (NLMS) filter in `host-cpp` fed with the Piper TTS render buffer reference.
2. **Double-Talk Detection & Barge-In:** When user speech exceeds clean post-AEC VAD threshold during `Speaking` state, abort Piper TTS playback immediately (sub-20ms epoch abort) and dispatch a `BargeInCancel` frame over IPC to trigger Tokio cancellation.

## 3. Consequences
- **Positive:** Zero acoustic feedback loops; clean barge-in interruption.
- **Trade-off:** Requires acoustic reference history buffer.

## 4. Verification Evidence
- `host-cpp/tests/test_aec.cpp` confirms >= 12 dB ERLE and clean double-talk handling.
