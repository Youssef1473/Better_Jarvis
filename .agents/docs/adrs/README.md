# Better_Jarvis Architectural Decision Records (ADRs)
**Directory:** `.agents/docs/adrs/`
**Status:** Canonical Architectural Register

---

## ADR Index

| ADR ID | Title | Status | Governed Subsystem | Ticket Reference |
|---|---|---|---|---|
| **ADR-001** | Named Pipe SDDL DACL Isolation & JRVS Codec v2 | Accepted | `daemon-rust/src/ipc`, `host-cpp` | TICKET-01 |
| **ADR-002** | SQLite WAL Durability & Dedicated Actor Thread Isolation | Accepted | `daemon-rust/src/memory/db.rs` | TICKET-02 |
| **ADR-003** | Handle-Based Path Containment & Zero-Shell Process Execution | Accepted | `daemon-rust/src/security` | TICKET-03 |
| **ADR-004** | Cryptographic Nonce Delimiters & Session Taint Escalation | Accepted | `daemon-rust/src/agent`, `security` | TICKET-04 |
| **ADR-005** | Isolated Chrome CDP Pipe Automation (Zero Localhost TCP) | Accepted | `daemon-rust/src/tools/browser_cdp.rs` | TICKET-05 |
| **ADR-006** | Lock-Free SPSC Ring Buffer Hygiene & WASAPI Hot-Swap | Accepted | `host-cpp/src/audio` | TICKET-06 |
| **ADR-007** | Acoustic Echo Cancellation & Voice Gated Barge-In Teardown | Accepted | `host-cpp/src/audio/aec_processor.cpp` | TICKET-07 |
| **ADR-008** | Decoupled Push-To-Talk Input Hook & UI Thread Isolation | Accepted | `host-cpp/src/input` | TICKET-08 |
| **ADR-009** | Monotonic QPC Latency Telemetry & Incremental Clause TTS | Accepted | `daemon-rust/src/telemetry.rs`, `host-cpp` | TICKET-09 |
| **ADR-010** | Cryptographic SHA-256 Supply-Chain Model Pinning | Accepted | Root `manifest.sha256`, `scripts` | TICKET-11 |
| **ADR-011** | Native Neural Inference Linkage & Zero-Stub Release Mode | Accepted | `host-cpp/CMakeLists.txt`, `models/` | TICKET-12 |
| **ADR-012** | Dynamic AEC Acoustic Delay Auto-Calibration (Decimated NCC) | Accepted | `host-cpp/src/audio/aec_delay_estimator.cpp` | TICKET-13 |
| **ADR-013** | Local Semantic Memory via `sqlite-vec` & Hybrid RRF | Accepted | `daemon-rust/src/memory/` | TICKET-14 |
| **ADR-014** | Windows Desktop Visual Context, OCR & Fail-Closed Privacy | Accepted | `daemon-rust/src/tools/screen_vision.rs` | TICKET-15 |
| **ADR-015** | Standalone Distribution Packaging & Minidump Diagnostics | Accepted | `host-cpp/src/diagnostics`, `packaging/` | TICKET-16 |

---

## ADR Template & Standard Structure
Every ADR in this repository must contain:
1. **Title & Status:** [Draft | Accepted | Superseded | Deprecated]
2. **Context & Problem Statement:** Why this architectural decision was needed.
3. **Decision Drivers:** Security invariants, latency constraints, OS boundaries.
4. **Considered Options:** Evaluated alternatives and why they were rejected.
5. **Decision:** The concrete architectural choice and implementation.
6. **Consequences:** Positive gains, trade-offs, and operational limitations.
7. **Verification Evidence:** Test commands and observable output validating the decision.
