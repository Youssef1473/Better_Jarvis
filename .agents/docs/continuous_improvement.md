# Better_Jarvis Continuous Engineering Improvement Protocol
**Location:** `.agents/docs/continuous_improvement.md`
**Status:** Canonical Engineering Evolution Framework

---

## 1. Overview & Objective
Continuous improvement ensures that Better_Jarvis learns from verified engineering defects, performance bottlenecks, and toolchain failures without indiscriminately accumulating rules, bloated prompt files, or unmaintained documentation.

---

## 2. Seven-Step Failure Analysis & Resolution Protocol

When a significant failure, deadlock, crash, or verification failure occurs:

```
1. Capture Diagnostic ──► 2. Root Cause ──► 3. Classify Domain ──► 4. Smallest Fix
                                                                           │
[Complete] ◄── 7. Verify Invariance ◄── 6. Log Lesson ◄── 5. Target Layer ◄┘
```

1. **Capture Diagnostic Evidence:** Log the exact command, environment, terminal stdout/stderr, and any crash minidumps or panic logs.
2. **Establish Ground-Truth Root Cause:** Use the 3-strike circuit breaker in [root-cause-debugging](../skills/root-cause-debugging/SKILL.md) to isolate the true underlying defect from its surface symptoms.
3. **Classify Failure Domain:**
   - *Knowledge Defect:* Missing architectural facts, unmapped dependencies, or wrong API signatures.
   - *Planning Defect:* Oversized scope, missing acceptance criteria, or unconsidered concurrency risks.
   - *Implementation Defect:* Syntax error, logic bug, memory leak, or broken invariant.
   - *Verification Defect:* Missing regression test, incomplete assertion, or stale mock.
   - *Tooling Defect:* Missing dependency, non-interactive session boundary, or compiler flag discrepancy.
4. **Determine Smallest Durable Fix:** Implement the minimal code change to eliminate the root cause.
5. **Select Target Architectural Layer:**
   - Permanent code rule → `.agents/rules/*.md`
   - Procedural runbook → `.agents/skills/<name>/SKILL.md`
   - Engineering sequence → `.agents/workflows/<name>.md`
   - Foundational design choice → `.agents/docs/adrs/ADR-XXX.md`
   - Past landmine and resolution → `.agents/docs/failure_ledger.md`
6. **Log Lesson in Failure Ledger:** If the failure was non-trivial and reproducible, record the landmine entry in `.agents/docs/failure_ledger.md`.
7. **Verify Invariance & Zero Regressions:** Run `python scripts/verify-env.py` and `python .agents/evals/runner.py` to assert no collateral breakage.

---

## 3. Lesson Quality Filter & Retention Bar

To avoid rules bloat and token budget exhaustion, a lesson must satisfy all 5 criteria to be retained:
1. **Reproducible:** Proven by an observable, failing test or command trace.
2. **Generally Applicable:** Likely to affect future tasks or multiple components (not a one-off typo).
3. **Relevant to Future Development:** Concerns active architecture, security, or concurrency boundaries.
4. **Evidence-Backed:** Supported by verified test commands and diffs.
5. **Narrowly Scoped:** Expressed in concise, actionable directives without generic preaching.

---

## 4. Controlled Self-Improvement & Guardrails

> [!CRITICAL]
> The engineering environment must **NEVER** autonomously:
> 1. Grant itself elevated operating system or toolchain permissions.
> 2. Disable or weaken deterministic lifecycle hooks in `.agents/hooks.json`.
> 3. Weaken evaluation acceptance criteria or baseline scenarios in `.agents/evals/` to disguise failures.
> 4. Modify constitutional invariants in `AGENTS.md`.

All self-improvement modifications to `.agents/` must remain inspectable as uncommitted Git changes subject to CEO review.
