---
name: feature-impl
description: >-
  Disciplined end-to-end implementation of new capabilities in Better_Jarvis across Rust, C++, and Python using TDD, strict compiler flags, and independent review.
---

# Feature Implementation (`feature-impl`)

## 1. Purpose
Provides the canonical protocol for implementing new functional capabilities end-to-end in Better_Jarvis without introducing regressions, architectural debt, or weakened security.

## 2. Activation Criteria
- **When to Use:**
  - When assigned a ticket to add a new capability, tool, IPC frame, or subsystem feature.
  - When extending existing subsystem functionality to handle new operations.
- **When NOT to Use:**
  - For bugfixes (use `root-cause-debugging`).
  - For performance optimization (use `performance-engineering`).
  - For purely architectural changes (use `architecture-design`).

## 3. Preconditions
- Session baseline established via `git status --short`.
- Active ticket identified from `BETTER_JARVIS_PLAN.md`.
- No unrelated modifications in the working tree.

## 4. Operating Protocol

### Step 1: Understand Requirements & Boundaries
1. Review the task requirements, active ticket, and architectural invariants.
2. Identify the target subsystem (`daemon-rust`, `host-cpp`, `scripts`).
3. Check `AGENTS.md` for non-negotiable security and real-time audio constraints.

### Step 2: Reconnaissance & Existing Code Check
1. Invoke `repo-intel` to trace symbols, interfaces, and call graphs.
2. Confirm whether similar functionality or reusable primitives exist.

### Step 3: Define Acceptance Gates
1. Determine exact test commands and passing criteria.
2. Enforce strict compiler warnings:
   - Rust: `-D warnings`
   - C++: `-Wall -Wextra -Werror`

### Step 4: Author Failing Test (RED)
1. Write unit or integration test exercising the new feature behavior before editing production code.
2. Run the test and record the observable failing assertion.

### Step 5: Implement Minimal Viable Code (GREEN)
1. Write the minimal code required to satisfy the failing test.
2. Respect real-time audio thread purity, lock-free SPSC hygiene, and Tokio async actor isolation.
3. Enforce zero shell interpolation and handle-based path containment.

### Step 6: Full Verification Gate
1. Execute verification commands:
   - Rust: `cargo check -p jarvis-daemon && cargo clippy -p jarvis-daemon --all-targets -- -D warnings && cargo test -p jarvis-daemon`
   - C++: Compile and execute relevant `host-cpp/tests/` suite.

### Step 7: Inspect Final Diff
1. Run `git diff --stat; git diff`.
2. Verify modifications are surgical, well-typed, and free of dead code or stealth edits.

### Step 8: Independent Code Review
1. Invoke `independent-review` to check invariants, concurrency, and security.

### Step 9: Document & Update State
1. Update `.agents/docs/session_state.md` with verified outcomes.
