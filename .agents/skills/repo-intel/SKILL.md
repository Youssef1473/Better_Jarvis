---
name: repo-intel
description: >-
  Investigate repository structure, call graphs, AST dependencies, and change impacts across Better_Jarvis (Rust, C++, Python). Use before any non-trivial code modification, architectural inquiry, or refactoring.
---

# Repository Intelligence (`repo-intel`)

## 1. Purpose
Provides procedures for non-destructive repository reconnaissance, symbol tracing, call-graph investigation, code ownership identification, and blast-radius assessment before editing code.

## 2. Activation Criteria
- **When to Use:**
  - Before starting any new feature, bugfix, or architectural task.
  - When locating where a specific capability, struct, or IPC message is implemented.
  - When analyzing the ripple effects or blast radius of an API modification.
- **When NOT to Use:**
  - When reading an explicitly specified small target file (1-2 files) whose call sites are already verified.
  - During write/edit phases (switch to language-specific or implementation skills).

## 3. Preconditions
- Git status must be checked to distinguish clean files from uncommitted working-tree modifications.
- Active directory must be repository root (`C:/Users/Youssef/Projects/Better_Jarvis`).

## 4. Required Knowledge
- Canonical Architecture Map: [architecture_index.md](../../docs/architecture_index.md)
- Authority Hierarchy: [authority_system.md](../../docs/authority_system.md)

## 5. Step-by-Step Procedures

### Step 1: Baseline Working Tree Inspection
Run `git status --short` to identify any modified, staged, or untracked files. Do not modify or revert any existing work.

### Step 2: Symbol & Interface Locating
1. For Rust: Use `cargo check` or targeted text search to find `struct`, `enum`, `trait`, or function definitions.
   - Example: search `struct TurnTaintState` or `enum IpcMessage`.
2. For C++: Search header declarations in `host-cpp/include/**/*.hpp` before searching implementation in `host-cpp/src/**/*.cpp`.
3. For Python: Search functions and classes in `scripts/*.py`.

### Step 3: Call-Site & Dependency Mapping
1. Trace upstream callers of the target function/interface.
2. Trace downstream dependencies (system libraries, Windows Win32 APIs, ONNX models).
3. Check IPC message flows: if a message is emitted in `host-cpp`, find its handler in `daemon-rust/src/ipc/` and vice-versa.

### Step 4: Test Coverage Discovery
1. Identify existing unit tests covering the subsystem:
   - Rust: inspect `#[cfg(test)]` modules in the file and integration tests in `daemon-rust/tests/`.
   - C++: inspect corresponding test files in `host-cpp/tests/test_*.cpp`.
2. Check if the targeted behavior is already tested.

### Step 5: Blast Radius & Impact Report
Summarize:
- Target files and symbols.
- Callers requiring signature updates.
- IPC contract impacts (JRVS framing, buffer bounds).
- Tests requiring execution to verify changes.

## 6. Engineering Restrictions
- **READ-ONLY:** Never edit source files, delete files, or run mutating commands during reconnaissance.
- Never read more than 3 exploratory files without specific symbol targets.
- Never assume an API signature matches memory; inspect the real file on disk.

## 7. Relevant Validation Commands
- Check git status: `git status --short`
- Search symbol across Rust: `Get-ChildItem -Path daemon-rust/src -Recurse -Filter "*.rs" | Select-String -Pattern "<symbol>"`
- Search symbol across C++: `Get-ChildItem -Path host-cpp -Recurse -Include "*.hpp","*.cpp" | Select-String -Pattern "<symbol>"`

## 8. Expected Evidence
A structured findings block listing:
- Exact file paths and line numbers of declaration and callers.
- Existing test files exercising the target logic.
- Potential regression risks and recommended smallest change.

## 9. Failure-Handling Guidance
If a symbol cannot be found:
1. Verify spelling and check for macros or dynamic names.
2. Check whether the capability is handled across the Named Pipe IPC in the other language.

## 10. Completion Criteria
Reconnaissance is complete when the exact location, callers, tests, and blast radius are documented without any source mutations.

## 11. Related Skills
- [task-planning](../task-planning/SKILL.md)
- [architecture-design](../architecture-design/SKILL.md)
