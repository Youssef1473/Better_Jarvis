# Workflow F: Performance Optimization (`perf-opt`)
**Status:** Standard Engineering Workflow  
**Objective:** Evidence-backed latency reduction and throughput optimization.

---

## Operating Protocol

```
[Start] ──► 1. Measure Baseline ──► 2. Isolate Bottleneck ──► 3. Define SLO Target
                                                                      │
┌─────────────────────────────────────────────────────────────────────┘
▼
4. Minimal Optimization ──► 5. Measure New Latency ──► 6. Functional Equivalence
                                                                 │
[Complete] ◄── 8. Log Telemetry ◄── 7. Regression Check ◄────────┘
```

### Step 1: Measure Baseline Timings
- Never optimize without an observable baseline.
- Run dedicated latency benchmarks:
  - Local embedding: `test_local_embedder_real_onnx_semantics_and_latency`
  - Audio AEC / frame timing: `test_aec`
- Record baseline p50 and p95 numbers.

### Step 2: Isolate the Bottleneck
- Profile execution path using monotonic QPC / `std::time::Instant`.
- Determine whether overhead stems from memory allocations, lock contention, excessive threads, or algorithmic complexity.

### Step 3: Define Measurable SLO Target
- Set specific latency target according to the budget table in [performance-engineering](../skills/performance-engineering/SKILL.md).

### Step 4: Implement Minimal Optimization
- Apply targeted optimization:
  - Pre-allocate memory pools/arenas.
  - Separate SPSC atomic indices to distinct cache lines (`alignas(64)`).
  - Constrain ONNX thread pools (2 intra-op, 1 inter-op).

### Step 5: Measure New Latency
- Re-run the identical benchmark under the same hardware and environment conditions.
- Calculate speedup delta.

### Step 6: Verify Functional Equivalence
- Ensure the optimization did not degrade output quality:
  - VAD speech detection accuracy preserved.
  - Echo Return Loss Enhancement (ERLE) remains >= 12 dB.
  - Embedding cosine similarity rankings remain valid.

### Step 7: Regression Check
- Run full test suite: `cargo test -p jarvis-daemon` and C++ test suite.

### Step 8: Log Telemetry Results
- Record verified measurements in `.agents/docs/session_state.md`.
