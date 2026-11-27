---
name: root-cause-debugging
description: >-
  Systematic root-cause diagnosis, minidump analysis, panic log triage, and 3-strike circuit breaker protocol for Better_Jarvis. Use when diagnosing test failures, crashes, or runtime regressions.
---

# Systematic Root-Cause Debugging (`root-cause-debugging`)

## 1. Purpose
Provides a disciplined, scientific self-debugging workflow for diagnosing crashes, test failures, and unexpected behaviors in Better_Jarvis without resorting to speculative trial-and-error editing.

## 2. Activation Criteria
- **When to Use:**
  - When any unit test, integration test, or build script fails.
  - When diagnosing a crash minidump (`.dmp`) or panic log (`panic_*.log`).
  - When observing unexpected audio dropouts, IPC disconnects, or async deadlocks.
- **When NOT to Use:**
  - Routine feature development where tests already pass cleanly.

## 3. Preconditions
- Exact failure command and terminal error output captured verbatim.
- Working tree state clean or known uncommitted files preserved.

## 4. Required Knowledge
- Known Landmines & Past Bug Fixes: [failure_ledger.md](../../docs/failure_ledger.md)
- Authority Hierarchy: [authority_system.md](../../docs/authority_system.md)
- Engineering Constitution: [AGENTS.md](../../../AGENTS.md)

## 5. Non-Negotiable Debugging Rules

### 5.1 3-Strike Circuit Breaker
- If two consecutive fix attempts fail with the same or related error signature, **STOP EDITING IMMEDIATELY**.
- Do not attempt a third speculative modification.
- Formulate three distinct, falsifiable hypotheses based on observable evidence.
- Add read-only instrumentation, assertions, or isolated test probes to isolate the root cause.
- Never weaken assertions or suppress linter diagnostics to force a check to turn green.

### 5.2 Evidence Over Assumption
- Never claim a bug is fixed without running the exact failing command and demonstrating that it now succeeds with exit code 0.
- Distinguish symptom from root cause (e.g. an OOM crash is the symptom; missing length bounds on IPC frames is the root cause).

## 6. Step-by-Step Diagnostic Procedure

### Step 1: Capture Diagnostic Evidence
1. Collect exact command line, working directory, and environment variables.
2. Capture full terminal stderr/stdout.
3. Check crash artifacts:
   - C++ minidumps in `<data_dir>/logs/crashes/crash_*.dmp`.
   - Rust panic logs in `<data_dir>/logs/panic_*.log`.
   - Latency logs in `<data_dir>/logs/host_latency_metrics.jsonl`.

### Step 2: Minimize Reproduction
1. Extract the minimal test case that reproduces the failure deterministically:
   - Run a single test: `cargo test -p jarvis-daemon -- <exact_test_name>`
   - Or compile and run a single C++ test executable.
2. Confirm reproduction occurs 100% of the time (or record intermittent frequency).

### Step 3: Falsifiable Hypothesis Generation
Formulate 3 hypotheses:
- **Hypothesis A:** Root cause is in state management or resource lifecycle (e.g. uninitialized buffer, dropped mutex guard, expired token).
- **Hypothesis B:** Root cause is in boundary validation or data conversion (e.g. UTF-8 slice boundary, IPC endianness mismatch).
- **Hypothesis C:** Root cause is in environment, OS session, or toolchain configuration (e.g. non-interactive window station, missing DLL in PATH).

### Step 4: Isolate with Read-Only Probes
Add targeted logging, `eprintln!`, or temporary assertions in a reproduction branch to eliminate 2 of the 3 hypotheses.

### Step 5: Implement Minimal Durable Fix
Apply the smallest correct code change addressing the confirmed root cause.

### Step 6: Verify Regression Prevention
Add a permanent regression test covering the exact failure condition before considering the issue closed.

## 7. Relevant Validation Commands
- Inspect panic logs: `Get-ChildItem -Path "$env:LOCALAPPDATA\Better_Jarvis\logs" -Filter "panic_*.log" | Select-Object -Last 5`
- Run failing Rust test: `cargo test -p jarvis-daemon -- <test_name> --nocapture`
- Run failing C++ test with debug output: `& build/tests/<test_name>.exe`

## 8. Expected Evidence
1. Diagnostic trace pinpointing root cause file and line number.
2. Verification output demonstrating clean pass of the minimal reproduction test.
3. Full regression suite run confirming zero collateral breakage.

## 9. Failure-Handling Guidance
If the defect cannot be isolated after completing the 3-hypothesis probe:
- Preserve diagnostic logs in `.agents/docs/failure_ledger.md`.
- Escalate to the human user with the complete diagnostic trace and test logs.

## 10. Completion Criteria
Root cause is identified, proven by a failing test, resolved by minimal code change, and verified with zero regressions.

## 11. Related Skills
- [tdd-workflow](../tdd-workflow/SKILL.md)
- [rust-engineering](../rust-engineering/SKILL.md)
- [cpp-engineering](../cpp-engineering/SKILL.md)
