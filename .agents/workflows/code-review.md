# Workflow G: Independent Code Review (`code-review`)
**Status:** Standard Engineering Workflow  
**Objective:** Adversarial multi-pass review of real git diffs to catch defects before completion.

---

## Operating Protocol

```
[Start] ──► 1. Generate Raw Diff ──► 2. Pass 1: Scope ──► 3. Pass 2: Security & Invariants
                                                                          │
┌─────────────────────────────────────────────────────────────────────────┘
▼
4. Pass 3: Real-Time & Async ──► 5. Pass 4: Tests & Errors ──► 6. Classify Findings
                                                                       │
[Complete] ◄── 8. Emit Report ◄── 7. Gate on Blocking Items ◄──────────┘
```

### Step 1: Generate Raw Diff
- Run `git diff --stat; git diff`.
- Base the review exclusively on the observable lines added and removed. Never review solely from the author's summary.

### Step 2: Pass 1 — Scope Discipline
- Compare modified files against task acceptance criteria.
- Verify no unrelated refactoring occurred ("stealth edits").
- Verify no pre-existing uncommitted files were altered or deleted.

### Step 3: Pass 2 — Security & Invariant Audit
- Check for zero shell interpolation (`Command::new("powershell")` or `"cmd"`).
- Check that file paths use handle-based verification (`GetFinalPathNameByHandleW`).
- Check that IPC connections verify parent PID and use Logon-SID SDDL DACL.
- Check that untrusted data is wrapped in CNG nonces and taints the session.
- Check that CDP uses anonymous pipes with zero localhost TCP ports.

### Step 4: Pass 3 — Real-Time Audio & Async Concurrency
- Check `host-cpp` MMCSS audio threads: assert zero allocations, zero mutex locks, zero synchronous I/O.
- Check SPSC ring buffer indices: assert `alignas(64)` and explicit `acquire`/`release` orderings.
- Check `daemon-rust` Tokio workers: assert zero blocking SQLite transactions or COM invocations.

### Step 5: Pass 4 — Test Integrity & Error Handling
- Assert that no tests were deleted, skipped, or weakened.
- Verify negative and boundary test coverage.
- Check for discarded results (`let _ = ...`) or unhandled unwraps on untrusted data.

### Step 6: Classify Findings
- **BLOCKING:** Security violation, data corruption, thread lock, race condition, weakened test. Work cannot complete until fixed.
- **NON-BLOCKING:** Code style, naming, minor redundancy.

### Step 7: Gate & Decision
- If blocking findings exist: report exact lines, stop workflow, require author correction.
- If zero blocking findings: proceed to deliver review sign-off.

### Step 8: Emit Review Report
- Document reviewed diff, passes executed, findings, and formal approval.
