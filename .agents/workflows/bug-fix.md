# Workflow C: Bug Fixing & Defect Resolution (`bug-fix`)
**Status:** Standard Engineering Workflow  
**Objective:** Systematic reproduction, root-cause isolation, and permanent defect resolution without masking symptoms.

---

## Operating Protocol

```
[Start] ──► 1. Reproduce Defect ──► 2. Gather Evidence ──► 3. Root Cause (3 Strikes)
                                                                     │
┌────────────────────────────────────────────────────────────────────┘
▼
4. Add Regression Test (Red) ──► 5. Implement Fix (Green) ──► 6. Verify Full Suite
                                                                     │
[Complete] ◄── 9. Log in Ledger ◄── 8. Independent Review ◄── 7. Inspect Diff ◄┘
```

### Step 1: Deterministic Reproduction
- Formulate the minimal command line or test case reproducing the bug.
- Confirm whether the failure is 100% deterministic or intermittent.

### Step 2: Evidence Gathering
- Capture full terminal error traces.
- Check minidumps in `<data_dir>/logs/crashes/` or panic logs in `<data_dir>/logs/`.
- Check QPC latency metrics in `<data_dir>/logs/host_latency_metrics.jsonl`.

### Step 3: Root-Cause Diagnosis (3-Strike Protocol)
- Use [root-cause-debugging](../skills/root-cause-debugging/SKILL.md).
- If two fix attempts fail, **STOP EDITING**. Formulate 3 distinct hypotheses and use read-only probes.
- Never weaken assertions or suppress compiler warnings to turn checks green.

### Step 4: Add Regression Test (RED)
- Write an automated unit or integration test that fails specifically due to the discovered bug.
- Observe the failing assertion and log the exact failure message.

### Step 5: Implement Minimal Fix (GREEN)
- Apply the smallest durable code change addressing the root cause.
- Re-run the regression test and confirm it passes cleanly.

### Step 6: Full Regression Gate
- Run full test suites:
  - `cargo test -p jarvis-daemon`
  - Relevant `host-cpp/tests/` suites.

### Step 7: Inspect Final Diff
- Run `git diff` to confirm modifications are surgical and clean.

### Step 8: Independent Review
- Execute Workflow G ([code-review.md](./code-review.md)).

### Step 9: Log Landmine in Failure Ledger
- If the bug was non-trivial, document symptom, root cause, and durable prevention rule in `.agents/docs/failure_ledger.md`.
