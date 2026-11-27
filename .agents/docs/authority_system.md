# Better_Jarvis Authority System & Precedence Specification
**Location:** `.agents/docs/authority_system.md`
**Authoritative Scope:** All autonomous agent sessions operating in `Better_Jarvis`.

---

## 1. Explicit Precedence Hierarchy

When making engineering decisions, resolving ambiguous requirements, or choosing between conflicting guidelines, the following precedence hierarchy is strictly authoritative (Level 1 is highest priority):

```
Level 1: Platform Safety Constraints & Operating System Guardrails
   │     (Never violate sandbox boundaries, access controls, or unapproved destructive ops)
   ▼
Level 2: Explicit Human User Instructions in the Active Conversation
   │     (Direct user prompts take precedence over generic project templates)
   ▼
Level 3: Repository Engineering Constitution (`AGENTS.md`)
   │     (Non-negotiable security invariants, C++20 real-time audio laws, Rust async/IPC laws)
   ▼
Level 4: Accepted Architectural Decision Records (`.agents/docs/adrs/`) & Master Plan (`BETTER_JARVIS_PLAN.md`)
   │     (CEO-approved architectural specifications and formal decision boundaries)
   ▼
Level 5: Active Ticket Acceptance Criteria & Verification Gates (`scripts/verify-v1.ps1`)
   │     (Specific functional, security, and performance criteria for the task)
   ▼
Level 6: Specialized Engineering Skills (`.agents/skills/`) & Workflows (`.agents/workflows/`)
   │     (Procedural guidance, checklists, and runbooks)
   ▼
Level 7: Convenience Documentation, Code Comments, and Synthetic Examples
         (Non-normative materials; cannot override higher levels)
```

> [!CRITICAL]
> A lower-level document (such as a skill, workflow, or code comment) must **NEVER** silently override or weaken a higher-level security invariant, architectural decision, or platform constraint.

---

## 2. Conflict Resolution Procedure

When two engineering documents, rules, or instructions provide contradictory guidance:

1. **Identify the Contradiction:** Quote the exact conflicting clauses and record their source file paths and line numbers.
2. **Evaluate Hierarchy Levels:** Determine the hierarchy level of each source using Section 1.
3. **Apply the Higher Precedence:** The higher-level requirement governs. If both documents are on the same level, apply the **safer existing requirement** (fail-closed, tighter permissions, more stringent verification).
4. **Log the Conflict:** Record the conflict in `.agents/docs/failure_ledger.md` under "Architectural Conflicts".
5. **Report to User:** Clearly state the contradiction, the resolution applied, and propose a formal update to eliminate the ambiguity. Never silently rewrite an ADR or rule.

---

## 3. Progressive Disclosure & Context Retrieval Strategy

To avoid saturating the agent's context window with thousands of lines of unused documentation, agents must follow progressive disclosure:

### 3.1 Tier 1: Core Grounding (Always Active)
- `AGENTS.md`: Loaded automatically by Antigravity from repository root.
- Working tree status (`git status --short`).
- Active ticket acceptance criteria.

### 3.2 Tier 2: Domain-Specific Retrieval (Loaded on Demand)
- **Rust Tasks:** Load `rust-engineering` skill + relevant Rust ADRs (`ADR-001`, `ADR-002`, `ADR-003`, `ADR-004`).
- **C++ Audio Tasks:** Load `cpp-engineering` skill + relevant C++ ADRs (`ADR-006`, `ADR-007`, `ADR-008`, `ADR-011`, `ADR-012`).
- **Security & Sandbox Tasks:** Load `security-engineering` skill + `ADR-001`, `ADR-003`, `ADR-004`, `ADR-005`, `ADR-014`.
- **Memory & Retrieval Tasks:** Load `ADR-013` (`sqlite-vec` + local embeddings).
- **Packaging & Desktop Process Tasks:** Load `ADR-015` (`run-desktop.py` + minidump crash handlers).

### 3.3 Tier 3: Deep Technical References (Targeted Read Only)
- Specific header files (`host-cpp/include/**/*.hpp`) or source files (`daemon-rust/src/**/*.rs`).
- Raw test logs or compiler outputs.
- Never read more than 3 exploratory files without specific symbol targets.

---

## 4. Documentation Modification Rules

1. **No Competing Authorities:** Never introduce secondary rules files that contradict `AGENTS.md`.
2. **Immutable History:** Accepted ADRs cannot be edited in place to justify a failing test or convenient hack; a new superseding ADR must be authored and approved.
3. **Evidence-Backed Updates:** Any change to architecture documentation must reference actual passing test commands and git diff lines.
