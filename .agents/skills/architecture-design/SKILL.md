---
name: architecture-design
description: >-
  System architecture governance, cross-language IPC boundaries, coupling analysis, and ADR lifecycle maintenance for Better_Jarvis. Use when proposing architectural changes or evaluating structural regressions.
---

# Architecture & Design Governance (`architecture-design`)

## 1. Purpose
Governs structural boundaries, subsystem coupling, cross-language IPC contracts, and Architectural Decision Record (ADR) authoring for Better_Jarvis.

## 2. Activation Criteria
- **When to Use:**
  - When introducing a new subsystem, background thread, database table, or IPC frame type.
  - When reviewing architectural proposals, evaluating coupling, or creating/updating ADRs.
- **When NOT to Use:**
  - For localized bug fixes or minor refactorings within an existing component boundary.

## 3. Preconditions
- Canonical architecture index and authority system understood.
- Existing ADRs inspected in `.agents/docs/adrs/`.

## 4. Required Knowledge
- Canonical Architecture Map: [architecture_index.md](../../docs/architecture_index.md)
- Authority Hierarchy: [authority_system.md](../../docs/authority_system.md)
- ADR Index: [adrs/README.md](../../docs/adrs/README.md)

## 5. Architectural Invariants

### 5.1 Subsystem Boundary Separation
- `host-cpp` owns all real-time audio hardware capture/render, DSP filters (AEC, VAD), local speech inference (Whisper, Piper), and Windows desktop UI.
- `daemon-rust` owns all async network calls, LLM agent planning, tool execution, security policy sandboxing, database persistence, and semantic memory.
- The two communicate EXCLUSIVELY over the authenticated Windows Named Pipe using `JRVS` v2 binary frames.

### 5.2 Threading & Actor Isolation
- Audio hardware threads must never block on IPC or disk.
- Tokio worker threads must never block on synchronous SQLite queries or Win32 COM calls.
- Dedicated OS threads must be used for single-writer SQLite mutations (`jarvis-db-actor`), ONNX embedding generation (`jarvis-embed-actor`), and Win32 COM execution (`jarvis-sta-controller`).

### 5.3 ADR Authoring Standard
When a significant architectural decision is made:
1. Assign the next sequential ADR ID (`ADR-XXX`).
2. Follow the standard template: Context, Decision Drivers, Considered Options, Decision, Consequences, Verification Evidence.
3. Commit the ADR into `.agents/docs/adrs/` and link it in `README.md`.
4. Never modify an accepted ADR to justify an unapproved shortcut; author a superseding ADR if the architecture must evolve.

## 6. Step-by-Step Procedure

1. **Investigate Current State:** Read [architecture_index.md](../../docs/architecture_index.md) and related ADRs.
2. **Identify Trade-offs:** Analyze latency impact, security implications, memory overhead, and cross-language boundaries.
3. **Draft ADR:** Author the draft ADR in `.agents/docs/adrs/`.
4. **Validate Compatibility:** Ensure proposed changes comply with `AGENTS.md` constitutional laws.
5. **Review Impact:** Verify that existing tests in both `host-cpp` and `daemon-rust` remain valid.

## 7. Relevant Validation Commands
- Inspect current architectural definitions: `Get-ChildItem .agents/docs/adrs -Filter "*.md"`
- Verify build and tests after architectural change: `& scripts/verify-beta.ps1`

## 8. Expected Evidence
- A completed ADR markdown file with all required sections.
- Verification output confirming zero architectural or security regressions.

## 9. Failure-Handling Guidance
- If an architectural change causes test failures in another subsystem, do not weaken the test. Revisit the interface contract and restore backward compatibility.

## 10. Completion Criteria
Architecture documentation is updated, the ADR is recorded, and the implementation adheres to all constitutional boundaries.

## 11. Related Skills
- [repo-intel](../repo-intel/SKILL.md)
- [security-engineering](../security-engineering/SKILL.md)
- [independent-review](../independent-review/SKILL.md)
