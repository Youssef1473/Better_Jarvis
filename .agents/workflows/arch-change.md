# Workflow D: Architecture Modification (`arch-change`)
**Status:** Standard Engineering Workflow  
**Objective:** Governed evolution of system topology, cross-language IPC, and durable ADR recording.

---

## Operating Protocol

```
[Start] ──► 1. Investigate Current State ──► 2. Identify Deficiency ──► 3. Trade-off Analysis
                                                                               │
┌──────────────────────────────────────────────────────────────────────────────┘
▼
4. Author ADR (Draft) ──► 5. Bounded Implementation ──► 6. Cross-Language Gate
                                                                │
[Complete] ◄── 8. Commit ADR & Docs ◄── 7. Architectural Review ◄┘
```

### Step 1: Investigate Current Architecture
- Inspect [architecture_index.md](../docs/architecture_index.md) and existing ADRs in `.agents/docs/adrs/`.
- Identify the exact boundaries, threading models, and IPC contracts involved.

### Step 2: Identify Structural Deficiency
- Formulate the precise architectural limitation (e.g. latency bottleneck, thread starvation, IPC payload cap).
- Distinguish between a genuine structural flaw and a local bug.

### Step 3: Trade-off & Alternative Analysis
- Evaluate at least two alternative designs.
- Assess backward compatibility, memory footprint, thread safety, and security.

### Step 4: Author Architectural Decision Record (ADR)
- Draft an ADR in `.agents/docs/adrs/ADR-XXX_<title>.md` following the standard template.
- Link the new ADR in `.agents/docs/adrs/README.md`.

### Step 5: Bounded Implementation
- Implement strictly within the approved architectural scope.
- Maintain real-time audio thread purity and Tokio async actor isolation.

### Step 6: Cross-Language Verification Gate
- Verify both C++ host and Rust daemon compile and pass tests:
  - Rust: `cargo test -p jarvis-daemon`
  - C++: `host-cpp/tests/test_frame_codec.cpp` or relevant test suite.
  - End-to-End: `& scripts/verify-beta.ps1`

### Step 7: Independent Architectural Review
- Verify that no architectural boundaries were violated and no secondary sources of truth were created.

### Step 8: Finalize Documentation
- Update [architecture_index.md](../docs/architecture_index.md) and `.agents/docs/session_state.md`.
