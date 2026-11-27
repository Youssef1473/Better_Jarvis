---
name: engineering-continuity
description: >-
  Session state checkpointing, durable failure logging, context recovery, and handoff management for Better_Jarvis development sessions. Use at session startup, shutdown, or context boundaries.
---

# Engineering Continuity & Session Management (`engineering-continuity`)

## 1. Purpose
Establishes reliable mechanisms for capturing, preserving, and reconstructing project state, active tickets, verification baselines, and engineering lessons across discontinuous Antigravity sessions.

## 2. Activation Criteria
- **When to Use:**
  - At the start of an engineering session to reconstruct ground-truth project state.
  - At the conclusion of a significant development ticket to record verified outcomes.
  - Before a context compaction (`/compact`) or session handoff (`/handoff`).
- **When NOT to Use:**
  - Mid-turn during active code editing or test execution.

## 3. Preconditions
- Access to repository root and working tree files.

## 4. Required Knowledge
- Active Session State: [session_state.md](../../docs/session_state.md)
- Failure Ledger: [failure_ledger.md](../../docs/failure_ledger.md)
- Authority Hierarchy: [authority_system.md](../../docs/authority_system.md)

## 5. Continuity Principles

### 5.1 Reconstruct from Disk, Not Memory
- Never assume project state from prior conversation turns or hallucinated recollections.
- Always inspect the actual working tree:
  - Branch name: `git branch --show-current`
  - Current commit: `git log -1 --oneline`
  - Uncommitted modifications: `git status --short`
- Read the active session state file: `.agents/docs/session_state.md`.

### 5.2 Durable Knowledge Over Transient Chat
- Important technical discoveries, bug root causes, and failure signatures must be committed to `.agents/docs/failure_ledger.md`.
- Architectural choices must be formalized in `.agents/docs/adrs/`.
- Transient conversation history will be compacted; disk records remain immutable.

## 6. Step-by-Step Session Workflows

### 6.1 Session Startup Protocol (Reconstruction)
1. **Inspect Git Ground Truth:**
   ```powershell
   git status --short; git log -1 --oneline
   ```
2. **Read Session State:**
   Inspect `.agents/docs/session_state.md` to identify the active ticket and milestone.
3. **Verify Toolchain Health:**
   Run a fast sanity check (`cargo check -p jarvis-daemon`) to ensure environment integrity.
4. **Formulate Turn Plan:**
   State the single active objective to the user.

### 6.2 Session Handoff / Checkpoint Protocol
At the conclusion of a ticket or before a handoff:
1. **Run Full Verification:**
   Execute `cargo test -p jarvis-daemon` and note exact passing numbers.
2. **Inspect Working Tree:**
   Run `git status --short` and `git diff --stat` to document exact modified files.
3. **Update State File:**
   Update `.agents/docs/session_state.md` with:
   - Completed ticket ID and status.
   - Exact test metrics.
   - Any discovered landmines or remaining blockers.
4. **Update Failure Ledger:**
   If a non-trivial bug was diagnosed, log it in `.agents/docs/failure_ledger.md`.

## 7. Relevant Validation Commands
- Check git state: `git status --short; git log -1 --oneline`
- Read state file: `.agents/docs/session_state.md`
- Read failure ledger: `.agents/docs/failure_ledger.md`

## 8. Expected Evidence
- Updated `session_state.md` matching current git status and test results.
- Clean summary identifying next actions and uncommitted modifications.

## 9. Failure-Handling Guidance
- If the working tree shows unexpected modifications or stashes, do NOT revert them. Document their presence and seek clarification if they collide with the active ticket.

## 10. Completion Criteria
Session state is recorded accurately on disk, allowing any subsequent session to resume immediately with zero context loss.

## 11. Related Skills
- [repo-intel](../repo-intel/SKILL.md)
- [task-planning](../task-planning/SKILL.md)
- [architecture-design](../architecture-design/SKILL.md)
