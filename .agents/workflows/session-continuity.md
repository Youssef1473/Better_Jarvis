# Workflow H: Session Continuity & Checkpointing (`session-continuity`)
**Status:** Standard Engineering Workflow  
**Objective:** Ground-truth context recovery at startup and reliable state checkpointing at shutdown.

---

## Operating Protocol

```
=== SESSION STARTUP ===
[Start] ──► 1. Read Git Status & HEAD ──► 2. Read session_state.md ──► 3. Run Sanity Check ──► [Ready]

=== SESSION CHECKPOINT / HANDOFF ===
[Work Done] ──► 4. Run Full Verification ──► 5. Inspect Final Diff ──► 6. Update State & Ledger ──► [Handoff Ready]
```

### Protocol 1: Session Startup (Context Reconstruction)
1. **Inspect Working Tree & Commit:**
   ```powershell
   git status --short; git log -1 --oneline
   ```
   Confirm active branch, current HEAD commit, and any pre-existing uncommitted files.
2. **Read Authoritative State:**
   Inspect `.agents/docs/session_state.md` to identify:
   - Current active ticket and milestone.
   - Preserved working tree baseline.
   - Last verified test status.
3. **Verify Toolchain Sanity:**
   Execute a non-destructive check:
   ```powershell
   cargo check -p jarvis-daemon
   ```
4. **Formulate Single Active Ticket:**
   State the exact single ticket to be worked on. Do not plan multi-ticket execution.

### Protocol 2: Session Checkpoint & Handoff
1. **Execute Full Verification:**
   Run full test suite:
   ```powershell
   cargo test -p jarvis-daemon
   ```
   Record exact numbers (e.g. `109 passed; 0 failed`).
2. **Inspect Working Tree Modifications:**
   Run `git status --short` and `git diff --stat`.
   Ensure no unauthorized files or secrets are present.
3. **Update Session State File:**
   Update `.agents/docs/session_state.md`:
   - Mark completed ticket status.
   - Update verification metrics.
   - Record remaining tickets and next actions.
4. **Update Failure Ledger:**
   If a non-trivial bug was diagnosed, record the failure signature, root cause, and rule in `.agents/docs/failure_ledger.md`.
5. **Output Handoff Report:**
   Summarize completed work, verified test results, working tree diff, and next actions.
