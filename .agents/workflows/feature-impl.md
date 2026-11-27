# Workflow B: New Feature Implementation (`feature-impl`)
**Status:** Standard Engineering Workflow  
**Objective:** Disciplined end-to-end implementation of new capabilities.

---

## Operating Protocol

```
[Start] ──► 1. Understand Spec ──► 2. Reconnaissance (W-A) ──► 3. Define Acceptance Gates
                                                                         │
┌────────────────────────────────────────────────────────────────────────┘
▼
4. Phased Plan ──► 5. Red Test (TDD) ──► 6. Minimal Code (Green) ──► 7. Verification Gate
                                                                             │
[Complete] ◄── 10. Update Docs ◄── 9. Independent Review ◄── 8. Inspect Diff ◄┘
```

### Step 1: Understand Requirements & Boundaries
- Review the user prompt, active ticket, and architectural constraints.
- Identify the target subsystem (`daemon-rust`, `host-cpp`, `scripts`).

### Step 2: Reconnaissance & Existing Code Check
- Execute Workflow A ([repo-recon.md](./repo-recon.md)).
- Check if similar functionality already exists to prevent duplication.

### Step 3: Define Acceptance Gates
- Specify exact test commands and passing criteria.
- Enforce strict compiler warnings: `-D warnings` for Rust, `-Wall -Wextra -Werror` for C++.

### Step 4: Create Phased Implementation Plan
- Use [task-planning](../skills/task-planning/SKILL.md) to sequence edits reversibly.

### Step 5: Author Failing Test (RED)
- Write unit/integration tests exercising the new behavior before modifying production code.
- Run the test and record the observable failure on the target assertion.

### Step 6: Implement Minimal Viable Code (GREEN)
- Write the minimal code required to satisfy the failing test.
- Respect real-time audio thread laws, async laws, and security invariants.

### Step 7: Verification Gate
- Run compiler checks and test suites:
  - Rust: `cargo check -p jarvis-daemon && cargo clippy -p jarvis-daemon --all-targets -- -D warnings && cargo test -p jarvis-daemon`
  - C++: Compile and execute relevant `host-cpp/tests/` suite.

### Step 8: Inspect Final Diff
- Run `git diff --stat; git diff`.
- Ensure no accidental edits, dead code, or unrelated refactorings occurred.

### Step 9: Independent Code Review
- Execute Workflow G ([code-review.md](./code-review.md)).
- Resolve any blocking security, concurrency, or invariant findings.

### Step 10: Document & Update State
- Update `.agents/docs/session_state.md` with verified outcomes.
