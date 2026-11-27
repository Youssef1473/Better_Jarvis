# ADR-011: Native Neural Inference & Zero-Stub Release Architecture
**Status:** Accepted  
**Date:** 2026-11-22  
**Context:** TICKET-11 (Local Neural Speech Processing & Vision Pipeline)  
**Governed Subsystems:** `host-cpp/src/audio/silero_vad.cpp`, `host-cpp/src/audio/piper_tts.cpp`, `host-cpp/src/audio/whisper_stt.cpp`

---

## 1. Context & Problem Statement
Early prototypes used placeholder stubs (generating static sine waves or synthetic transcriptions) when neural libraries were not linked. Shipping mock stubs or relying solely on cloud APIs introduces severe privacy leaks, high internet latency, per-request billing, and false test passes that do not exercise actual neural inference runtimes on Windows.

## 2. Decision Drivers
- Zero-stub release requirement: all production builds must execute genuine local inference.
- Complete on-device speech processing with zero external API dependencies.
- Bounded CPU utilization and real-time execution budgets.

## 3. Decision
1. **Production Flag Enforcement:** Production builds of `host-cpp` must define `-DJARVIS_HAS_ONNXRUNTIME=1`, `-DJARVIS_HAS_PIPER=1`, and `-DJARVIS_HAS_WHISPER=1` with strict MinGW-w64 UCRT64 link flags (`-lonnxruntime -lespeak-ng -lwhisper -lggml -lggml-base -lole32 -lavrt`).
2. **Local Voice Pipeline Subsystems:**
   - **Silero VAD:** Evaluates speech probability per 32ms (512 samples @ 16kHz) audio frame using `Ort::Session` on CPU with intra-op thread count = 1.
   - **Piper TTS:** Synthesizes voice audio via eSpeak-ng phonemizer and ONNX medium voice model (`en_US-ryan-medium.onnx`), feeding WASAPI render and AEC reference buffer with interruptible 20ms epoch abort.
   - **Whisper.cpp STT:** Transcribes speech frames into text locally via `ggml-base.en.bin` using quantized ggml CPU/Vulkan acceleration bounded to 4 threads.
3. **Automated Verification:** The unified verification suite compiles and executes test suites against real models on disk to prove zero-stub functionality.

## 4. Consequences
- **Positive:** Complete local voice autonomy, sub-20ms barge-in abort, zero cloud token costs for speech/vision.
- **Trade-off:** Release packages require bundling ONNX Runtime and Whisper binaries (~150MB model weights and ~30MB shared libraries).

## 5. Verification Evidence
- `host-cpp/tests/test_vad.cpp` verifies 100 silence frames produce probability < 0.15.
- `host-cpp/tests/test_piper_tts.cpp` verifies real synthesis generates > 28,000 samples @ 22,050 Hz and aborts under 20 ms.
- `host-cpp/tests/test_whisper_stt.cpp` verifies real Whisper inference executes on `models/ggml-base.en.bin`.
