# ADR-009: Monotonic QPC Latency Telemetry & Incremental Clause TTS
**Status:** Accepted  
**Date:** 2026-11-21  
**Context:** TICKET-09 (Hardening & Production Execution Plan)  
**Governed Subsystems:** `daemon-rust/src/telemetry.rs`, `host-cpp/src/audio/piper_tts.cpp`

---

## 1. Context & Problem Statement
Voice-to-voice turn latency lacked fine-grained instrumentation, and waiting for full Gemini LLM responses before starting Piper TTS delayed audio playback by several seconds.

## 2. Decision
1. **Monotonic QPC Telemetry:** Instrument high-resolution `QueryPerformanceCounter` timestamps for every stage: `t_vad_end`, `t_stt_done`, `t_ipc_rx`, `t_router_done`, `t_llm_ttft`, `t_tts_first_audio`. Persist local rolling metrics to `<data_dir>/logs/host_latency_metrics.jsonl`.
2. **Incremental Clause Streaming:** Stream LLM tokens into sentence-boundary buffers (`sentence_chunker.rs`) and dispatch incremental audio synthesis to Piper so playback begins on the first completed clause.

## 3. Consequences
- **Positive:** Measurable latency breakdown; perceived voice-to-voice response time slashed to sub-second clauses.
- **Trade-off:** Requires clause boundary parsing and trailing flush logic.

## 4. Verification Evidence
- `daemon-rust` tests: `telemetry::tests::*`, `agent::sentence_chunker::tests::*` pass.
