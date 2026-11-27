---
name: independent-review
description: >-
  Adversarial, multi-pass code review of git diffs, security boundaries, error paths, and architectural invariants for Better_Jarvis. Use before approving PRs or completing significant engineering tasks.
---

# Independent Code Review (`independent-review`)

## 1. Purpose
Provides an adversarial, evidence-based code review procedure to inspect code modifications independently, verifying correctness, security invariants, error handling, and architectural boundaries without relying on the author's claims.

## 2. Activation Criteria
- **When to Use:**
  - Before completing any task modifying >2 files or changing core logic.
  - Prior to merging branches, creating PRs, or releasing build artifacts.
  - When reviewing proposed security or architectural changes.
- **When NOT to Use:**
  - Non-functional documentation fixes or small comment updates.

## 3. Preconditions
- Implementation phase complete.
- Tests passing with verifiable terminal output.
- `git diff` generated and accessible.

## 4. Required Knowledge
- Adversarial Sign-Off Rules: Universal Orchestration Directives
- Engineering Constitution: [AGENTS.md](../../../AGENTS.md)
- Authority Hierarchy: [authority_system.md](../../docs/authority_system.md)

## 5. Review Principles

### 5.1 Inspect the Real Diff, Not the Summary
- Never base review approval on the author agent's conversational explanation.
- Always read the actual lines added and removed via `git diff`.
- Check surrounding context to detect subtle broken invariants.

### 5.2 Zero Confirmation Bias
- The reviewer role operates under the assumption that defects exist until verified absent.
- Actively seek out:
  - Weakened test assertions or removed test cases.
  - Silent error discarding (`.unwrap_or_default()`, `let _ = ...`, catch-all `catch (...) {}`).
  - Unbounded allocations or missing payload limits.
  - Blocking operations introduced into real-time or async worker threads.
  - Missing bounds checks or path canonicalization escapes.

## 6. Step-by-Step Review Procedure

### Pass 1: Scope & Specification Alignment
1. Compare `git diff --stat` against the original task acceptance criteria.
2. Verify that no unrelated files were touched ("Stealth Refactoring").
3. Ensure no pre-existing uncommitted files were overwritten or reverted.

### Pass 2: Security & Invariant Audit
1. Verify zero shell interpolation (`powershell`/`cmd`).
2. Verify handle-based path containment on any filesystem operations.
3. Verify Named Pipe DACL and parent PID checks on any IPC changes.
4. Verify nonces and taint flags on any external input ingestion.
5. Verify zero localhost TCP CDP debugging flags.

### Pass 3: Real-Time & Concurrency Purity
1. Check that `host-cpp` MMCSS audio paths perform zero allocations, zero mutex locks, and zero synchronous I/O.
2. Check that SPSC indices remain aligned to 64 bytes (`alignas(64)`).
3. Check that Tokio workers do not execute blocking SQLite queries or COM calls.

### Pass 4: Test Integrity & Error Handling
1. Confirm that tests exercise the actual behavioral change.
2. Confirm that negative/failure paths are tested.
3. Check that compiler diagnostics show zero warnings under strict flags (`-D warnings`, `-Werror`).

## 7. Findings Severity Classification

- **BLOCKING (Critical / High):**
  - Security invariant violation (e.g. shell invocation, path traversal, missing taint).
  - Memory leak, race condition, or deadlock.
  - Test deletion or weakened assertions.
  - Real-time audio thread violation (heap allocation or mutex lock).
  *Action:* Stop work. Fix required immediately before proceeding.
- **NON-BLOCKING (Medium / Low):**
  - Minor naming inconsistency or stylistic variance.
  - Code duplication that does not introduce bugs.
  *Action:* Log for future improvement; does not block task completion.

## 8. Relevant Validation Commands
- Generate exact diff: `git diff --stat; git diff`
- Audit security invariants: Stage 2 of `& scripts/verify-beta.ps1`
- Audit compiler warnings: `cargo clippy -p jarvis-daemon --all-targets -- -D warnings`

## 9. Expected Evidence
An Independent Review Report listing:
- Scope audited (`git diff --stat`).
- Findings by severity.
- Pass/Fail recommendation with specific technical justification.

## 10. Completion Criteria
Review is complete when all blocking findings are resolved, tests are confirmed passing against the final diff, and an audit report is generated.

## 11. Related Skills
- [security-engineering](../security-engineering/SKILL.md)
- [rust-engineering](../rust-engineering/SKILL.md)
- [cpp-engineering](../cpp-engineering/SKILL.md)
