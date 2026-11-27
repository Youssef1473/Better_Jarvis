---
name: performance-engineering
description: >-
  Latency instrumentation, monotonic QPC telemetry, ERLE acoustic echo measurement, ONNX warmup optimization, and benchmark methodology for Better_Jarvis. Use when profiling or optimizing performance.
---

# Performance Engineering & Latency Telemetry (`performance-engineering`)

## 1. Purpose
Defines measurement protocols, telemetry instrumentation, performance budgets (SLOs), and benchmark procedures for the Better_Jarvis voice-to-voice pipeline and on-device neural inference.

## 2. Activation Criteria
- **When to Use:**
  - Measuring or optimizing voice-to-voice turn latency (`TTFT`, `TTFA`).
  - Profiling on-device ONNX inference (Silero VAD, Whisper STT, MiniLM-L6, Piper TTS).
  - Validating AEC echo reduction (ERLE) or audio buffer frame timing.
- **When NOT to Use:**
  - Code changes with zero latency or performance implications.

## 3. Preconditions
- Target execution path identified and instrumented with high-resolution timers.

## 4. Required Knowledge
- Telemetry Architecture: [ADR-009](../../docs/adrs/ADR-009_per_stage_qpc_telemetry_and_streaming_tts.md)
- AEC Performance: [ADR-012](../../docs/adrs/ADR-012_dynamic_aec_delay_auto_calibration.md)
- Failure Ledger (ONNX latency lessons): [failure_ledger.md](../../docs/failure_ledger.md#lm-06-onnx-runtime-thread-contention--startup-latency)

## 5. Performance SLOs & Budgets

| Stage | Metric Target | Budget / Threshold | Verification Seam |
|---|---|---|---|
| **VAD Detection** | `vad_to_stt_ms` | < 35 ms | `host-cpp/tests/test_vad.cpp` |
| **Local STT** | `stt_to_ipc_ms` | < 250 ms (base.en) | `host-cpp/tests/test_whisper_stt.cpp` |
| **AEC Filter Frame** | Frame processing | < 0.5 ms per 10 ms frame | `host-cpp/tests/test_aec.cpp` |
| **Echo Reduction** | ERLE | >= 12 dB across [15..120ms] | `host-cpp/tests/test_aec.cpp` |
| **Local Embedding** | `memory_recall_ms` | < 15 ms (warm) | `daemon-rust/src/memory/embedder.rs` |
| **Fast-Path TTFT** | Time-To-First-Token | < 350 ms | `daemon-rust/src/telemetry.rs` |
| **TTS First Audio** | `ipc_to_first_audio_ms` | < 180 ms | `host-cpp/tests/test_piper_tts.cpp` |

## 6. Step-by-Step Profiling & Optimization Procedure

### Step 1: Establish Baseline Measurement
1. Never optimize without a measured baseline.
2. Read historical stage latencies from `<data_dir>/logs/host_latency_metrics.jsonl` or run dedicated performance unit tests:
   ```powershell
   cargo test -p jarvis-daemon -- test_local_embedder_real_onnx_semantics_and_latency --nocapture
   ```
3. Record p50 and p95 timings.

### Step 2: Identify Bottleneck
Determine whether the bottleneck is:
- **Compute-Bound:** Excessive threads causing cache thrashing or matrix multiplication overhead.
- **Lock-Contention:** Worker threads blocked on mutexes or channel backpressure.
- **I/O-Bound:** Synchronous disk writes or unbuffered IPC reads.

### Step 3: Implement Minimal Optimization
- Pre-allocate buffers and memory arenas during initialization.
- Decouple execution onto worker pools with bounded capacities.
- Use SIMD-optimized routines where available.

### Step 4: Verify Optimization & Functional Invariance
1. Measure the new timings under identical test conditions.
2. Verify functional correctness: ensure output audio, text, or embeddings are identical or meet accuracy criteria.
3. Assert that ERLE remains >= 12 dB and audio frame processing stays < 0.5 ms.

## 7. Relevant Validation Commands
- Run embedding latency test: `cargo test -p jarvis-daemon -- test_local_embedder_real_onnx_semantics_and_latency --nocapture`
- Run AEC stress & ERLE test:
  ```powershell
  $env:PATH = "C:\msys64\ucrt64\bin;$env:PATH"
  g++ -std=c++20 -O2 -Wall -Wextra -Werror host-cpp/tests/test_aec.cpp host-cpp/src/audio/aec_processor.cpp host-cpp/src/audio/aec_delay_estimator.cpp host-cpp/src/audio/piper_tts.cpp -I host-cpp/include -I C:/msys64/ucrt64/include -I C:/msys64/ucrt64/include/onnxruntime -I C:/msys64/ucrt64/include/espeak-ng C:/msys64/ucrt64/bin/onnxruntime.dll -L C:/msys64/ucrt64/lib -lespeak-ng -lole32 -lavrt -luser32 -o build/tests/test_aec.exe
  & build/tests/test_aec.exe
  ```

## 8. Expected Evidence
- Benchmark logs reporting measured before/after p50/p95 latency deltas.
- Observable test output showing ERLE >= 12 dB and zero dropped frames.

## 9. Failure-Handling Guidance
- If an optimization improves speed but compromises functional output (e.g. lowering VAD accuracy or reducing audio fidelity), reject the change. Correctness takes precedence over speed.

## 10. Completion Criteria
Optimization delivers a measurable, recorded improvement against baseline SLOs without degrading stability, security, or output fidelity.

## 11. Related Skills
- [cpp-engineering](../cpp-engineering/SKILL.md)
- [rust-engineering](../rust-engineering/SKILL.md)
- [tdd-workflow](../tdd-workflow/SKILL.md)
