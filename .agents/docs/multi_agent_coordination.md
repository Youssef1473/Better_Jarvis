# Better_Jarvis Multi-Agent Coordination & Independent Review Architecture
**Location:** `.agents/docs/multi_agent_coordination.md`
**Status:** Canonical Multi-Agent Coordination Specification

---

## 1. Overview & Objective
To prevent implementation agents from approving their own flawed assumptions, Better_Jarvis enforces strict role separation and adversarial independent review across engineering tasks.

---

## 2. Agent Roles & Responsibilities

| Role | Primary Responsibility | Authorized Actions | Output Deliverable |
|---|---|---|---|
| **Architect Agent** | Subsystem boundaries, ADR compliance, coupling analysis | Read-only inspection of architecture docs, header declarations, and interface contracts | Architecture Spec & Bounded Task Plan |
| **Implementation Agent** | Code implementation, TDD execution, unit tests | Editing permitted source files, running compilers and unit tests | Minimal code diff + Passing test evidence |
| **Testing Agent** | Test completeness, edge cases, regression coverage | Running test suites, fuzzing, verifying boundary conditions | Test Coverage Audit Report |
| **Security Reviewer** | Trust boundaries, path containment, shell ban, taint engine | Auditing diffs against `AGENTS.md` invariants, running security tests | Security Clearance Report |
| **Independent Reviewer** | Fresh-context adversarial review of the final diff | Inspecting raw `git diff` against acceptance criteria; zero write tools | Independent Review Sign-Off (or Blocking Findings) |

---

## 3. Fresh-Context Review Isolation Invariant

> [!CRITICAL]
> The Independent Reviewer must evaluate the work in an **unbiased, fresh context**.
> To prevent confirmation bias, the reviewer agent is provided ONLY:
> 1. The original task requirements and acceptance criteria.
> 2. The raw `git diff` (`git diff --stat` and `git diff`).
> 3. The raw test/lint execution logs.
> 
> The reviewer must **NEVER** receive the implementation agent's reasoning trace or conversational justifications.

---

## 4. Structured Inter-Agent Communication Schema

When agents communicate or pass tasks, messages must adhere to this structured format:

```json
{
  "taskId": "TICKET-XX",
  "objective": "Brief description of the engineering objective",
  "scope": {
    "targetFiles": ["path/to/file.rs", "path/to/file.hpp"],
    "forbiddenFiles": [".env", "Cargo.toml", "CMakeLists.txt"]
  },
  "constraints": [
    "Zero allocations on MMCSS audio thread",
    "Zero blocking calls on Tokio workers"
  ],
  "verificationCommand": "cargo test -p jarvis-daemon",
  "findings": [
    {
      "severity": "BLOCKING | NON-BLOCKING",
      "file": "daemon-rust/src/ipc/codec.rs",
      "line": 142,
      "description": "Missing payload length bound check before BytesMut::reserve"
    }
  ],
  "status": "APPROVED | REJECTED | IN_PROGRESS"
}
```

---

## 5. Technical Conflict Resolution Protocol

When agents disagree on an implementation path, design choice, or review finding:
1. **No Majority Voting:** Consensus or voting cannot override an architectural invariant or security policy.
2. **Precedence Authority:** Refer to [.agents/docs/authority_system.md](authority_system.md). Higher precedence levels govern.
3. **Empirical Verification:** If the disagreement concerns performance, race conditions, or correctness, both options must be tested with an observable test probe.
4. **Fail-Closed Default:** If uncertainty remains, choose the safer path (fail-closed, tighter permissions, more strict validation).

---

## 6. Single-Agent Fallback Review Protocol

In sessions where subagent invocation is restricted or unavailable:
1. The primary agent must separate the implementation turn from the review pass.
2. After writing code and observing green tests, the agent must execute:
   ```powershell
   git diff --stat; git diff
   ```
3. The agent must adopt an adversarial review mindset, explicitly executing the 4 review passes defined in Workflow G ([code-review.md](../workflows/code-review.md)) against the printed diff before delivering the completion report.
