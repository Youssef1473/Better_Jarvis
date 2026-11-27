---
name: task-planning
description: >-
  Decompose engineering requirements into bounded, verifiable, risk-classified implementation plans for Better_Jarvis. Use before implementing non-trivial features, bugfixes, or refactorings.
---

# Engineering Task Planning (`task-planning`)

## 1. Purpose
Guides the rigorous decomposition of engineering requirements, establishing explicit scope boundaries, risk classifications, acceptance criteria, implementation sequencing, and verification gates before writing code.

## 2. Activation Criteria
- **When to Use:**
  - Before starting any code modifications spanning multiple files or touching core security/real-time boundaries.
  - When responding to a feature request, bug report, or refactoring ticket.
- **When NOT to Use:**
  - Trivial single-line typos or isolated non-functional comments.

## 3. Preconditions
- Reconnaissance ([repo-intel](../repo-intel/SKILL.md)) must be complete.
- Active ticket and acceptance criteria must be understood.

## 4. Required Knowledge
- Precedence & Authority: [authority_system.md](../../docs/authority_system.md)
- Engineering Constitution: [AGENTS.md](../../../AGENTS.md)
- Known Landmines: [failure_ledger.md](../../docs/failure_ledger.md)

## 5. Step-by-Step Procedures

### Step 1: Requirement & Constraint Extraction
Extract:
- Target subsystem (`host-cpp`, `daemon-rust`, `scripts`).
- Non-negotiable invariants (real-time audio purity, zero shell, handle-based paths, taint tracking).
- Memory, latency, or concurrency constraints.

### Step 2: Risk Classification
Assign risk level:
- **High Risk:** Changes to IPC codec, audio thread loop, SQLite durability, path sandbox, or taint engine. Requires strict ADR update and independent review.
- **Medium Risk:** Changes to UI rendering, new tool implementations, model routing logic, or non-MMCSS helper threads.
- **Low Risk:** Script enhancements, documentation, test additions.

### Step 3: Phased Implementation Sequencing
Break work into atomic, verifiable steps:
1. Write failing regression/acceptance test (Red).
2. Implement smallest viable code change (Green).
3. Verify with language compiler and test runner.
4. Refactor and verify zero regressions.
5. Independent code review.

### Step 4: Define Acceptance Criteria
Formulate explicit, observable gates:
- Exact test commands that must pass.
- Compiler warning policy (`-D warnings` / `-Werror`).
- Security assertions (e.g. zero shell invocations, zero memory leaks).

## 6. Engineering Restrictions
- Never plan multi-ticket work in a single turn. Scope must be bounded strictly to one ticket.
- Never plan test suppression or assertion disabling.
- Never plan changes outside workspace boundaries.

## 7. Relevant Validation Commands
- Validate plan feasibility against existing tests:
  - Rust: `cargo test -p jarvis-daemon --no-run`
  - C++: verify headers in `host-cpp/include/`

## 8. Expected Evidence
A structured execution plan containing:
- Objective and Ticket ID.
- Target files to modify (and files to leave untouched).
- Step-by-step implementation order.
- Specific verification commands and expected passing outputs.

## 9. Failure-Handling Guidance
If during planning a requirement contradicts an architectural invariant:
- Stop planning.
- Flag the conflict per [authority_system.md](../../docs/authority_system.md).
- Do not proceed with an unsound plan.

## 10. Completion Criteria
Plan is complete when steps, target files, acceptance criteria, and verification commands are unambiguously specified.

## 11. Related Skills
- [repo-intel](../repo-intel/SKILL.md)
- [tdd-workflow](../tdd-workflow/SKILL.md)
- [independent-review](../independent-review/SKILL.md)
