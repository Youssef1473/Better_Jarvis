---
name: tdd-workflow
description: >-
  Test-driven development (TDD) workflow, strict Red-Green-Refactor proof, negative/boundary testing, and test integrity rules for Better_Jarvis (Rust and C++). Use when implementing features or fixing defects.
---

# Test-Driven Development Workflow (`tdd-workflow`)

## 1. Purpose
Enforces disciplined Test-Driven Development (TDD) for Better_Jarvis, requiring a verified failing test before implementation code is written, followed by the minimal change to reach green, and behavior-preserving refactoring.

## 2. Activation Criteria
- **When to Use:**
  - Implementing any new capability, tool, parser, or security check.
  - Fixing any reproduced bug or defect.
  - Adding regression protection against reported vulnerabilities.
- **When NOT to Use:**
  - Non-functional changes (comment updates, pure documentation).

## 3. Preconditions
- Target subsystem and behavior identified via [repo-intel](../repo-intel/SKILL.md).
- Test harness compiles cleanly prior to writing the test.

## 4. Required Knowledge
- Testing Invariants: [AGENTS.md](../../../AGENTS.md#5-verification-bar)
- Universal Directives: Strict Red-Green Proof (Rule 4 in Global Directives)

## 5. Non-Negotiable TDD Rules

### 5.1 Strict Red-Green Proof
- A test only counts as **RED** if it fails specifically on the target behavioral assertion.
- Failures caused by `SyntaxError`, linker errors, missing imports, or panicking test setup do NOT count as a valid RED state.
- You must record the exact failing assertion output before writing implementation code.

### 5.2 No Mocking the Unit Under Test
- Never mock the exact boundary or component where the contract or bug lives.
- At least one test in the suite must exercise the real integration seam (e.g. real SQLite file, real Named Pipe socket, real SPSC ring buffer).

### 5.3 Test-Integrity Invariant
- NEVER weaken an assertion, increase timeouts to hide race conditions, skip tests, or delete tests to achieve a green result.
- If a test fails unexpectedly, the defect is in the implementation or the premise—never alter the acceptance bar without CEO approval.

## 6. Step-by-Step TDD Procedure

### Phase 1: RED (Write the Failing Test)
1. Write a focused unit test in the relevant test module:
   - Rust: `#[test]` inside `daemon-rust/src/<module>.rs` or `daemon-rust/tests/`.
   - C++: Add test case function in `host-cpp/tests/test_<subsystem>.cpp`.
2. Cover positive path, negative path (invalid inputs, malformed frames), and boundary conditions.
3. Compile and execute the test:
   ```powershell
   cargo test -p jarvis-daemon -- <test_name>
   ```
4. Confirm failure output matches the expected behavioral assertion.

### Phase 2: GREEN (Smallest Viable Implementation)
1. Write the minimal production code needed to satisfy the assertion.
2. Avoid speculative features or tangential refactorings.
3. Re-run the test command:
   ```powershell
   cargo test -p jarvis-daemon -- <test_name>
   ```
4. Confirm test passes with exit code 0.

### Phase 3: REFACTOR (Clean Code & Invariant Checks)
1. Clean up duplicate code, improve variable naming, and optimize if justified.
2. Verify linting and static analysis:
   - Rust: `cargo clippy -p jarvis-daemon --all-targets -- -D warnings`
   - C++: verify zero warnings with `-Wall -Wextra -Werror`
3. Run the full test suite to confirm zero regressions:
   ```powershell
   cargo test -p jarvis-daemon
   ```

## 7. Relevant Validation Commands
- Run single Rust test: `cargo test -p jarvis-daemon -- <test_name>`
- Run single C++ test:
  ```powershell
  $env:PATH = "C:\msys64\ucrt64\bin;$env:PATH"
  g++ -std=c++20 -O2 -Wall -Wextra -Werror host-cpp/tests/<test_file>.cpp -I host-cpp/include -o build/tests/<test_file>.exe
  & build/tests/<test_file>.exe
  ```

## 8. Expected Evidence
1. **RED Evidence:** Exact terminal log showing failure on target assertion.
2. **GREEN Evidence:** Exact terminal log showing test passing.
3. **REGRESSION Evidence:** Full test suite run showing 100% passing tests.

## 9. Failure-Handling Guidance
- If the test passes immediately without code changes, the test is invalid or exercises pre-existing behavior. Revise the test to target the missing requirement.

## 10. Completion Criteria
The new test has been observed failing, then passing after implementation, with clean lint results and zero regressions across the codebase.

## 11. Related Skills
- [rust-engineering](../rust-engineering/SKILL.md)
- [cpp-engineering](../cpp-engineering/SKILL.md)
- [root-cause-debugging](../root-cause-debugging/SKILL.md)
