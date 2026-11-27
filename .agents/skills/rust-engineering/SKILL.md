---
name: rust-engineering
description: >-
  Idiomatic Rust engineering standards, Tokio async concurrency, SQLite WAL durability, bounded IPC framing, and Cargo verification for Better_Jarvis daemon. Use when modifying or reviewing Rust code.
---

# Rust Engineering Standards & Procedures (`rust-engineering`)

## 1. Purpose
Defines engineering standards, ownership patterns, async architecture, safety rules, and verification procedures for `daemon-rust` in Better_Jarvis.

## 2. Activation Criteria
- **When to Use:**
  - Writing or modifying any code under `daemon-rust/src/` or `daemon-rust/tests/`.
  - Reviewing Rust pull requests, diagnosing async deadlocks, or fixing Clippy diagnostics.
- **When NOT to Use:**
  - When editing C++ audio host code or Python scripts (use `cpp-engineering` or `python-engineering`).

## 3. Preconditions
- Rust toolchain (`cargo`, `rustc`) verified operational (Rust 2021 edition).
- Working tree state inspected (`git status --short`).

## 4. Required Knowledge
- Rust Async & IPC Laws: [AGENTS.md](../../../AGENTS.md#4-rust-async--ipc-laws-daemon-rust)
- ADR-001 (Named Pipe SDDL & JRVS Codec): [ADR-001](../../docs/adrs/ADR-001_named_pipe_sddl_isolation_and_jrvs_codec_v2.md)
- ADR-002 (SQLite WAL & Actor Threads): [ADR-002](../../docs/adrs/ADR-002_sqlite_wal_durability_and_tokio_actor_isolation.md)
- ADR-003 (Path Sandbox & Zero Shell): [ADR-003](../../docs/adrs/ADR-003_handle_based_path_containment_and_zero_shell.md)
- ADR-004 (Nonces & Taint Engine): [ADR-004](../../docs/adrs/ADR-004_cryptographic_nonce_delimiters_and_session_taint_escalation.md)
- Trail of Bits Rust Review prompts: [rust-review](../rust-review/SKILL.md)

## 5. Non-Negotiable Rust Laws

### 5.1 Zero Blocking on Tokio Workers
- NEVER execute synchronous file I/O, `rusqlite` transactions, or Win32 COM invocations on Tokio worker threads.
- Route database queries through dedicated single-writer actor threads communicating via bounded channels (`tokio::sync::mpsc::channel(32)`).
- Route Win32 COM / `os_control` calls to dedicated STA OS threads.

### 5.2 SQLite Durability & Crash Safety
- Every SQLite connection must execute:
  ```rust
  conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
  ```
- Always invoke `PRAGMA wal_checkpoint(TRUNCATE);` before graceful daemon shutdown.

### 5.3 Bounded IPC Framing (`JRVS` Codec v2)
- Always enforce hard byte limits BEFORE calling `BytesMut::reserve`:
  - Max 262,144 bytes (256 KiB) for Control JSON.
  - Max 65,536 bytes (64 KiB) for Audio PCM.
- Reject invalid magic (`JRVS`), wrong protocol versions, or oversized payloads with immediate disconnection.

### 5.4 Instant Barge-In Cancellation
- Every voice/LLM/tool turn must bind to a hierarchical `tokio_util::sync::CancellationToken`.
- On user speech or `BargeInCancel` IPC frame, cancel the token immediately to abort HTTP streams and running child processes.

### 5.5 Taint Tracking & Cryptographic Nonces
- Wrap external/untrusted content in per-turn CNG nonces: `<untrusted_data_{nonce}>...</untrusted_data_{nonce}>`.
- Sanitize payload to prevent breakout (`</untrusted_data_`).
- Mark turn tainted (`TurnTaintState::mark_tainted`) upon receiving external data; enforce Tier 3 HUD confirmation for state mutations.

## 6. Step-by-Step Implementation Procedure

1. **Verify Baseline:** Run `cargo check -p jarvis-daemon` to confirm a clean compile before editing.
2. **Implement Minimal Edit:** Apply scoped changes strictly within `daemon-rust/src/`.
3. **Unit Test Coverage:** Add or update tests in `#[cfg(test)] mod tests` exercising positive, negative, and boundary cases.
4. **Compile & Lint:**
   ```powershell
   cargo check -p jarvis-daemon
   cargo clippy -p jarvis-daemon --all-targets -- -D warnings
   ```
5. **Execute Tests:**
   ```powershell
   cargo test -p jarvis-daemon
   ```

## 7. Relevant Validation Commands
- Fast check: `cargo check -p jarvis-daemon`
- Targeted test: `cargo test -p jarvis-daemon -- <test_name>`
- Full test suite: `cargo test -p jarvis-daemon`
- Strict lint check: `cargo clippy -p jarvis-daemon --all-targets -- -D warnings`

## 8. Expected Evidence
- Clean `cargo check` exit code 0.
- Zero Clippy warnings under `-D warnings`.
- Test runner output showing all tests passing (e.g. `109 passed; 0 failed`).

## 9. Failure-Handling Guidance
- **Borrow Checker Errors:** Do not reflexively add `.clone()` everywhere. Investigate ownership boundaries, use borrowing (`&T`), or consider passing by value if ownership transfers.
- **Async Deadlocks:** Check for mutex guards held across `.await` points. Replace `std::sync::Mutex` with `tokio::sync::Mutex` or restructure to drop the guard before yielding.

## 10. Completion Criteria
Implementation is complete when code adheres to all Rust laws, compiles with zero warnings, and passes 100% of unit and integration tests.

## 11. Related Skills
- [rust-review](../rust-review/SKILL.md)
- [cargo-fuzz](../cargo-fuzz/SKILL.md)
- [security-engineering](../security-engineering/SKILL.md)
- [tdd-workflow](../tdd-workflow/SKILL.md)
