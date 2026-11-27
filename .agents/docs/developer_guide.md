# Better_Jarvis Developer & Agent Operational Guide
**Location:** `.agents/docs/developer_guide.md`
**Status:** Comprehensive Operational Guide

---

## 1. Environment Architecture & Discovery Mechanics

Antigravity operates within the repository using hierarchical discovery:
- **Root Constitution (`AGENTS.md`):** Loaded automatically. Enforces hard security invariants, C++20 real-time audio laws, and Rust async/IPC laws.
- **Directory Rules (`.agents/rules/*.md`):** Automatically discovered for workspace files. Enforces language-specific standards.
- **Customization Manifest (`.agents/skills.json`):** Points to `.agents/skills` to register all 18 engineering skills.
- **Lifecycle Hooks (`.agents/hooks.json`):** Intercepts tool calls deterministically via `.agents/scripts/guardrail.py`.

---

## 2. Common Engineering Runbooks

### 2.1 Adding a New Daemon Agent Tool
1. **Reconnaissance:** Run Workflow A ([repo-recon.md](../workflows/repo-recon.md)) to inspect `daemon-rust/src/tools/` and `agent/gemini_client.rs`.
2. **Security Gate:** Inspect [security-engineering.md](../skills/security-engineering/SKILL.md). Ensure the tool does not invoke shells, obeys handle-based path containment, and escalates to Tier 3 confirmation if the session is tainted.
3. **TDD Implementation:** Follow Workflow B ([feature-impl.md](../workflows/feature-impl.md)). Author a unit test in `daemon-rust/src/tools/<tool>.rs`. Run `cargo test -p jarvis-daemon -- <test_name>` to observe RED, then implement to GREEN.
4. **Verification:** Run `python scripts/verify-env.py`.
5. **Independent Review:** Perform adversarial diff review per Workflow G ([code-review.md](../workflows/code-review.md)).

### 2.2 Modifying Real-Time Audio Capture / DSP
1. **Audio Laws:** Review [cpp-engineering.md](../skills/cpp-engineering/SKILL.md). Remember: ZERO heap allocations, ZERO mutex locks, ZERO synchronous I/O inside the MMCSS `Pro Audio` thread loop.
2. **Buffer Sizing:** If modifying SPSC buffers, maintain power-of-2 capacity and cache-line separation (`alignas(64)`).
3. **Compilation & Test:** Compile with MinGW-w64 UCRT64 `g++` (`-std=c++20 -O2 -Wall -Wextra -Werror`). Run `build/tests/test_aec.exe` or `test_ring_buffer.exe`.
4. **SLO Verification:** Assert ERLE remains >= 12 dB and audio frame processing stays < 0.5 ms.

### 2.3 Diagnosing a Test Failure or Crash
1. **3-Strike Circuit Breaker:** If 2 fix attempts fail, STOP editing.
2. **Gather Artifacts:** Inspect minidumps in `<data_dir>/logs/crashes/` or panic logs in `<data_dir>/logs/`.
3. **Hypotheses:** List 3 falsifiable hypotheses and use read-only probes per [root-cause-debugging.md](../skills/root-cause-debugging/SKILL.md).
4. **Regression Protection:** Write an automated test reproducing the defect before fixing it.
5. **Failure Ledger:** Record root cause in `.agents/docs/failure_ledger.md`.

---

## 3. Toolchain & Command Quick Reference

| Action | Canonical Command |
|---|---|
| **Fast Rust Check** | `cargo check -p jarvis-daemon` |
| **Strict Rust Clippy** | `cargo clippy -p jarvis-daemon --all-targets -- -D warnings` |
| **Full Rust Tests** | `cargo test -p jarvis-daemon` |
| **Unified Environment Verification** | `python scripts/verify-env.py` |
| **Guardrail Unit Tests** | `python .agents/scripts/test_guardrail.py` |
| **Agent Evaluation Suite** | `python .agents/evals/runner.py` |
| **Knowledge Base Integrity Audit** | `python .agents/scripts/audit_knowledge.py` |
| **Full Release Verification Gate** | `& scripts/verify-v1.ps1` |
| **Release Build** | `& scripts/build.ps1` |
