# Better_Jarvis Autonomous Engineering Environment
**Location:** `.agents/`
**Target Stack:** Rust (Edition 2021), C++ (C++20), Python (3.12+) on Windows 11
**Engine:** Google Antigravity IDE & Antigravity CLI

---

## 1. Quick Navigation Hub

```
.agents/
├── README.md                     # This master navigation index
├── hooks.json                    # Deterministic lifecycle hooks (PreToolUse, SessionStart, Stop)
├── skills.json                   # Antigravity skill manifest & registration
├── docs/                         # Canonical architectural & operational knowledge
│   ├── authority_system.md       # Precedence hierarchy & conflict resolution rules
│   ├── architecture_index.md     # Subsystem map (host-cpp, daemon-rust, scripts)
│   ├── skill_routing.md          # Skill routing matrix & discovery catalog
│   ├── guardrails.md             # Deterministic guardrails & blocked command specifications
│   ├── language_verification.md  # Rust, C++, Python verification gates & policies
│   ├── multi_agent_coordination.md # Adversarial review & multi-agent roles
│   ├── evaluation_system.md      # Agent evaluation methodology & baseline score
│   ├── continuous_improvement.md # 7-step failure triage & lesson retention filter
│   ├── session_state.md          # Ground-truth project state & continuity checkpoint
│   ├── failure_ledger.md         # Durable engineering lessons & past landmines
│   ├── baseline_report.md        # Initial Phase 0 environment audit
│   └── adrs/                     # Architectural Decision Records (ADR-001 through ADR-015)
├── skills/                       # 18 Modular specialized engineering skills
│   ├── repo-intel/               # Repository reconnaissance & blast radius
│   ├── task-planning/            # Bounded implementation plans
│   ├── rust-engineering/         # Tokio async, SQLite WAL, bounded IPC framing
│   ├── cpp-engineering/          # C++20 real-time audio purity, WASAPI, lock-free SPSC
│   ├── python-engineering/       # Win32 desktop interop, typing, scripts
│   ├── architecture-design/      # System topology, coupling, ADR maintenance
│   ├── tdd-workflow/             # Red-Green-Refactor, strict Red proof
│   ├── root-cause-debugging/     # 3-strike circuit breaker, minidump analysis
│   ├── security-engineering/     # Zero shell, path sandbox, pipe SDDL, taint tracking
│   ├── performance-engineering/  # Monotonic QPC latency telemetry, ERLE budgets
│   ├── independent-review/       # Adversarial diff review & invariant audit
│   ├── engineering-continuity/   # Session reconstruction & checkpointing
│   ├── modern-cpp/               # C++20/C++23 idiomatic patterns
│   ├── c-review/                 # C/C++ AST analysis & SARIF reporting
│   ├── rust-review/              # Rust concurrency, memory, and async audits
│   ├── cargo-fuzz/               # Fuzzing harness patterns for Rust
│   ├── insecure-defaults/        # Named Pipe, CDP, and security checklist
│   └── sharp-edges/              # Language security pitfalls catalog
├── workflows/                    # 8 Standard engineering runbooks
│   ├── repo-recon.md             # Workflow A: Repository Reconnaissance
│   ├── feature-impl.md           # Workflow B: New Feature Implementation
│   ├── bug-fix.md                # Workflow C: Bug Fixing & Defect Resolution
│   ├── arch-change.md            # Workflow D: Architecture Modification
│   ├── security-mod.md           # Workflow E: Security Modification & Hardening
│   ├── perf-opt.md               # Workflow F: Performance Optimization
│   ├── code-review.md            # Workflow G: Independent Code Review
│   └── session-continuity.md     # Workflow H: Session Continuity & Checkpointing
├── rules/                        # Modular directory rules
│   ├── rust_standards.md         # Tokio async laws, SQLite WAL, error handling
│   ├── cpp_standards.md          # MMCSS audio thread purity, SPSC hygiene
│   ├── python_standards.md       # Win32 window station binding, typing
│   └── cross_language.md         # JRVS frame codec v2, pipe DACL, PID check
├── evals/                        # Agent evaluation & regression benchmarking
│   ├── scenarios.json            # Controlled evaluation scenarios
│   ├── runner.py                 # Empirical scoring harness
│   └── baseline_results.json     # Empirical baseline results (Score: 40/40, 100%)
└── scripts/                      # Environment automation helpers
    ├── guardrail.py              # Hook handler for .agents/hooks.json
    ├── test_guardrail.py         # Unit tests for guardrail hook (5/5 passing)
    └── audit_knowledge.py        # Knowledge integrity and link auditor
```

---

## 2. Onboarding for New Agent Sessions

When starting an engineering session in Better_Jarvis:
1. **Reconstruct Context from Disk:**
   Read `.agents/docs/session_state.md` to identify the active objective, completed tickets, and preserved baseline files.
2. **Consult Authority:**
   `AGENTS.md` at root defines constitutional laws. If instructions conflict, refer to `.agents/docs/authority_system.md`.
3. **Route Required Skills:**
   Consult `.agents/docs/skill_routing.md` to load only skills relevant to the active task.
4. **Follow Standard Workflows:**
   Match task to a workflow in `.agents/workflows/` (e.g. `feature-impl.md` or `bug-fix.md`).
5. **Verify Changes:**
   Run `python scripts/verify-env.py` before concluding any turn.
