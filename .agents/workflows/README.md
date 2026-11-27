# Better_Jarvis Standard Engineering Workflows & Skill Migration Index
**Directory:** `.agents/workflows/`  
**Status:** Modernized & Integrated with Antigravity Skills Architecture

---

## Migration Matrix: Workflows to Agent Skills

In accordance with Antigravity's modernization standards, workflows have been migrated and paired with corresponding first-class Agent Skills in `.agents/skills/`:

| Legacy Workflow | Modern Agent Skill | Primary Responsibility |
|---|---|---|
| [`repo-recon.md`](./repo-recon.md) | [`repo-intel`](../skills/repo-intel/SKILL.md) | Non-destructive symbol tracing and AST call-graph reconnaissance |
| [`feature-impl.md`](./feature-impl.md) | [`feature-impl`](../skills/feature-impl/SKILL.md) | Disciplined end-to-end feature delivery with TDD and strict flags |
| [`bug-fix.md`](./bug-fix.md) | [`root-cause-debugging`](../skills/root-cause-debugging/SKILL.md) | Systematic defect reproduction, 3-strike diagnosis, regression tests |
| [`arch-change.md`](./arch-change.md) | [`architecture-design`](../skills/architecture-design/SKILL.md) | Governed system topology evolution, ADR authoring, coupling audit |
| [`security-mod.md`](./security-mod.md) | [`security-engineering`](../skills/security-engineering/SKILL.md) | Threat modeling, handle-based sandboxing, zero-shell verification |
| [`perf-opt.md`](./perf-opt.md) | [`performance-engineering`](../skills/performance-engineering/SKILL.md) | QPC profiling, ONNX warmup, latency budgets, and ERLE validation |
| [`code-review.md`](./code-review.md) | [`independent-review`](../skills/independent-review/SKILL.md) | Adversarial multi-pass diff review, security and invariant checks |
| [`session-continuity.md`](./session-continuity.md) | [`engineering-continuity`](../skills/engineering-continuity/SKILL.md) | Session startup recovery, durable failure logging, handoff reports |

---

## Operating Protocol
1. **Match Capability:** Activate the corresponding Agent Skill from `.agents/skills/` during execution.
2. **Follow Strict Procedures:** Execute procedures sequentially and record observable evidence.
3. **Preserve Compatibility:** The runbook files in `.agents/workflows/` remain available as human-readable standard operating procedures.
