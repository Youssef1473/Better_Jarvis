# Better_Jarvis Skill Routing & Discovery Matrix
**Location:** `.agents/docs/skill_routing.md`
**Authoritative Scope:** Skill routing for all autonomous tasks in `Better_Jarvis`.

---

## 1. Skill Catalog & Responsibilities

The Better_Jarvis development environment contains 18 specialized skills organized under `.agents/skills/`:

### Core Operational Engineering Skills (12)
1. **`repo-intel`**: Repository structure, AST dependency mapping, call-graph discovery, blast-radius analysis.
2. **`task-planning`**: Phased execution plans, scope bounding, risk classification, acceptance gates.
3. **`rust-engineering`**: Rust Edition 2021, Tokio async concurrency, SQLite WAL, bounded IPC framing, Cargo tests/clippy.
4. **`cpp-engineering`**: C++20 real-time audio thread purity, SPSC lock-free ring buffers, WASAPI resilience, UCRT64 builds.
5. **`python-engineering`**: Python 3.12+, Win32 ctypes, desktop window station binding, process supervision, test scripts.
6. **`architecture-design`**: System topology, cross-language IPC boundaries, coupling analysis, ADR management.
7. **`tdd-workflow`**: Strict Red-Green-Refactor, no mocking unit under test, negative/boundary testing.
8. **`root-cause-debugging`**: Systematic diagnosis, 3-strike circuit breaker, minidump triage, panic log analysis.
9. **`security-engineering`**: Zero shell execution, handle-based path containment, pipe DACLs, taint engine, CDP isolation.
10. **`performance-engineering`**: Monotonic QPC telemetry, ERLE acoustic echo budgets, local embedding latency, SLO tracking.
11. **`independent-review`**: Adversarial code review of git diffs, security boundary audit, invariant verification.
12. **`engineering-continuity`**: Session state checkpointing, durable failure logging, context reconstruction across turns.

### Specialized Security & Audit Skills (6 Pre-existing)
13. **`modern-cpp`**: Modern C++20/C++23 idiomatic patterns from Trail of Bits.
14. **`c-review`**: Static C/C++ AST analysis and SARIF reporting.
15. **`rust-review`**: Concurrency, memory safety, and async runtime audit prompts for Rust.
16. **`cargo-fuzz`**: Fuzzing harness guidance for Rust parsers and codecs.
17. **`insecure-defaults`**: Checklist for Named Pipe DACLs, CDP flags, CORS, and fallback secrets.
18. **`sharp-edges`**: Language-specific security pitfall catalog.

---

## 2. Dynamic Task Routing Matrix

Agents must only activate skills relevant to their immediate task. Use this matrix to select the required skill combinations:

| Task Type | Primary Skills | Supporting Skills | Verification Seam |
|---|---|---|---|
| **Rust Async / IPC Bug** | `rust-engineering`, `root-cause-debugging` | `tdd-workflow`, `rust-review` | `cargo test -p jarvis-daemon` |
| **C++ Audio Glitch / WASAPI** | `cpp-engineering`, `performance-engineering` | `root-cause-debugging`, `modern-cpp` | `host-cpp/tests/test_aec.cpp` |
| **Security Modification** | `security-engineering`, `task-planning` | `insecure-defaults`, `independent-review` | `scripts/verify-v1.ps1` Stage 2 & 5 |
| **New Agent Tool Addition** | `rust-engineering`, `security-engineering` | `tdd-workflow`, `task-planning` | `daemon-rust/src/tools/` tests |
| **Cross-Language IPC Update** | `architecture-design`, `repo-intel` | `rust-engineering`, `cpp-engineering`, `independent-review` | `test_frame_codec.cpp` + `codec.rs` |
| **AEC / Latency Optimization** | `performance-engineering`, `cpp-engineering` | `tdd-workflow` | `host_latency_metrics.jsonl` |
| **Python Tooling / Desktop** | `python-engineering`, `task-planning` | `tdd-workflow` | `python -m py_compile scripts/*.py` |
| **Session Boundary / Handoff** | `engineering-continuity` | `repo-intel` | `session_state.md`, `failure_ledger.md` |

---

## 3. Discovery & Progressive Disclosure Mechanism

1. **Top-Level Discovery:** Antigravity reads `.agents/skills.json` and scans `.agents/skills/*/SKILL.md` frontmatter (`name`, `description`).
2. **Context Preservation:** Only the skill description is loaded into memory initially. Full skill instructions are loaded on-demand when the activation criteria match.
3. **Deep References:** Bulky documentation (ADRs, failure ledgers, cluster prompts) resides in `.agents/docs/` or `references/` subdirectories and is only read when explicitly needed.
