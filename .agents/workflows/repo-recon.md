# Workflow A: Repository Reconnaissance (`repo-recon`)
**Status:** Standard Engineering Workflow  
**Objective:** Non-destructive investigation of subsystems, symbols, and dependencies before any modification.

---

## Operating Protocol

```
[Start] ──► 1. Check Working Tree ──► 2. Locate Symbols & AST ──► 3. Trace Dependencies
                                                                          │
[Complete] ◄── 6. Deliver Report ◄── 5. Map Blast Radius ◄── 4. Inspect Tests ◄┘
```

### Step 1: Check Working Tree Baseline
- Execute `git status --short`.
- Note all pre-existing modified, staged, or untracked files.
- **Rule:** Never revert, stash, or overwrite existing user work.

### Step 2: Locate Subsystem & Symbols
- Rust: Search `daemon-rust/src/` for target structs, traits, or functions.
- C++: Search `host-cpp/include/` declarations and `host-cpp/src/` implementations.
- Python: Search `scripts/` functions and entrypoints.
- Read real files on disk. Never assume APIs from conversational summaries.

### Step 3: Trace Dependencies & Call Graphs
- Identify immediate callers of the target function or interface.
- Trace downstream dependencies (Win32 APIs, ONNX models, SQLite tables).
- Identify cross-language IPC frame message types (`JRVS` v2).

### Step 4: Inspect Associated Tests
- Find existing unit tests:
  - Rust: `#[cfg(test)]` modules or `daemon-rust/tests/`.
  - C++: `host-cpp/tests/test_*.cpp`.
- Verify test commands and current passing status.

### Step 5: Map Affected Interfaces & Blast Radius
- Assess whether public structs, IPC headers, or file schemas change.
- Identify risk classification (High, Medium, Low).

### Step 6: Produce Findings Report
- Output a structured markdown report detailing:
  1. Subsystem investigated.
  2. File paths and line numbers.
  3. Call graph and blast radius.
  4. Existing test coverage.
  5. Recommended smallest viable change.
- **Strict Boundary:** Zero file edits or destructive commands during reconnaissance.
