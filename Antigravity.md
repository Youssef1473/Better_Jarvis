# JARVIS 2.0 — AUTONOMOUS ANTIGRAVITY ENGINEERING ENVIRONMENT TRANSFORMATION

## MASTER IMPLEMENTATION PLAN

**Role:** Principal AI Systems Architect, Senior Rust Engineer, Senior C++ Engineer, Senior Python Engineer, Agentic Workflow Architect, Software Reliability Engineer, Security Engineer, and Independent Quality Auditor.

**Repository:** `C:/Users/Youssef/Projects/Better_Jarvis`

**Primary technology stack:**
- Rust
- C++
- Python

**Target environment:** Google Antigravity IDE and Antigravity CLI, including whichever workspace-level agent configuration, skills, workflows, and integrations are supported by the installed version.

**Mission:** Autonomously transform the existing Antigravity development environment into a sophisticated, disciplined, context-aware software engineering environment capable of developing Jarvis 2.0 with exceptional architectural consistency, code quality, security, maintainability, and reliability.

---

# 1. EXECUTIVE MISSION

You are being assigned responsibility for improving the engineering environment you use to develop Jarvis 2.0.

This is not a normal application-development task.

You must improve the complete engineering system surrounding your own coding activities.

The goal is to make future engineering tasks more reliable, predictable, understandable, verifiable, and maintainable.

You must move away from an approach where an AI agent receives a prompt, writes code, and assumes that the result is correct.

Instead, establish an engineering operating environment in which every significant task follows a disciplined process:

1. Understand the repository.
2. Understand the architectural requirements.
3. Identify the actual problem.
4. Investigate before making modifications.
5. Plan the implementation.
6. Identify potential risks and regressions.
7. Implement the smallest correct change.
8. Validate the implementation.
9. Independently review the modifications.
10. Record relevant engineering knowledge.
11. Improve the engineering system when a genuine weakness is discovered.

The desired result is an environment that supports sophisticated autonomous engineering without sacrificing correctness or control.

**You are responsible for implementing the infrastructure, not merely proposing it.**

Do not stop after producing a theoretical improvement plan.

Inspect the repository, design the implementation, make the permitted changes, execute the available verification procedures, and produce a comprehensive completion report.

---

# 2. NON-NEGOTIABLE OPERATING PRINCIPLES

These principles govern the entire transformation.

## 2.1 Architectural understanding before implementation

Never assume that you understand a subsystem simply because its filename or function name appears familiar.

Before modifying anything:

- Inspect the relevant implementation.
- Inspect its callers and dependencies.
- Identify public interfaces.
- Understand the ownership boundaries.
- Inspect the corresponding tests.
- Read relevant architectural decisions.
- Identify security and reliability implications.
- Understand the subsystem's relationship with other components.

Do not implement based on assumptions when repository evidence is available.

## 2.2 No code slop

Do not optimize for the amount of code produced.

Optimize for correctness and maintainability.

Prohibit:

- Unnecessary abstractions.
- Duplicate implementations.
- Dead code.
- Unnecessary dependencies.
- Overengineered solutions.
- Arbitrary architectural changes.
- Unnecessary boilerplate.
- Hardcoded assumptions that should come from configuration.
- Broad exception handling that hides errors.
- Silent failure.
- Unexplained fallback behavior.
- Disabling tests to obtain successful results.
- Suppressing compiler or static-analysis warnings without justification.
- Introducing new architectural patterns without demonstrating their necessity.

Prefer existing project conventions unless there is evidence that they are defective.

## 2.3 Evidence over confidence

Never report that something works merely because it looks correct.

A successful implementation requires appropriate evidence.

Depending on the task, this evidence may include:

- Compiler results.
- Static-analysis results.
- Unit tests.
- Integration tests.
- Security tests.
- Performance measurements.
- Regression tests.
- Independent code-review findings.
- Runtime behavior.

Clearly distinguish between:

1. Implemented.
2. Compiled.
3. Tested.
4. Independently reviewed.
5. Verified in an actual runtime environment.

Never claim a higher verification level than the evidence supports.

## 2.4 Never weaken engineering controls

Never modify a test, security boundary, permission, or validation rule merely to make an implementation pass.

When a test fails:

1. Establish whether the test is valid.
2. Inspect the implementation.
3. Identify the root cause.
4. Determine whether the defect is in the code, test, environment, or assumptions.
5. Implement the appropriate correction.
6. Add regression coverage where justified.

Test changes must preserve the original behavioral and security requirements.

## 2.5 Preserve existing project authority

Existing repository instructions, architecture documents, ADRs, engineering policies, and reliability mechanisms take precedence over newly generated convenience documentation.

Before adding anything:

- Find the existing equivalent.
- Determine whether it is authoritative.
- Identify conflicting instructions.
- Reuse or improve the existing implementation.
- Avoid creating duplicate sources of truth.

Do not assume that existing mechanisms are absent simply because you do not immediately find them.

---

# 3. AUTONOMY AND SAFETY BOUNDARIES

You have authorization to independently investigate and implement low-risk engineering infrastructure improvements within the repository.

However, autonomous execution does not mean unrestricted system access.

## 3.1 Permitted activities

You may independently:

- Inspect repository files.
- Inspect project architecture and existing instructions.
- Analyze Antigravity-supported workspace configuration.
- Create and improve repository-level agent instructions.
- Create and improve specialized engineering skills.
- Create reusable agent workflows.
- Improve task-planning templates.
- Improve testing and verification procedures.
- Create non-destructive validation scripts.
- Improve documentation and engineering knowledge organization.
- Create agent evaluation scenarios.
- Run existing non-destructive tests and static-analysis commands.
- Perform independent reviews.
- Generate reports documenting implementation evidence.

## 3.2 Activities that must not be performed autonomously

Do not:

- Delete existing project files or user data.
- Reset, clean, stash, or overwrite pre-existing user modifications.
- Switch branches or rewrite Git history.
- Commit, push, merge, or release anything.
- Deploy Jarvis.
- Modify production application behavior as part of this infrastructure task.
- Introduce or upgrade dependencies without explicit authorization.
- Modify database schemas or production data.
- Change operating-system security settings.
- Modify registry settings.
- Modify global system-level permissions.
- Disable security protections.
- Introduce unrestricted execution permissions.
- Install or connect external MCP integrations.
- Access credentials, tokens, or private keys unnecessarily.
- Upload repository content to external services.
- Modify Antigravity application binaries or claim to have changed its underlying model.

If an improvement requires one of these activities, document the requirement and continue with the remaining safe work.

Do not wait for user confirmation. Skip the restricted action and clearly report the blocker.

## 3.3 Protect the existing working environment

Before implementation:

1. Identify the actual repository root.
2. Record the active Git branch.
3. Record the current HEAD commit.
4. Record all modified, staged, and untracked files.
5. Identify existing user modifications.
6. Identify existing generated artifacts.
7. Establish which files are safe to modify.

Never remove or overwrite pre-existing user work.

Do not assume that the working tree is clean.

Do not change branches to simplify your implementation.

All modifications must remain inspectable as uncommitted workspace changes.

---

# 4. PHASE 0 — ANTIGRAVITY ENVIRONMENT RECONNAISSANCE

**Objective:** Understand the environment before attempting to improve it.

Inspect the installed Antigravity version and determine which configuration mechanisms it actually supports.

Investigate:

- Workspace instructions.
- Repository-level agent instructions.
- Rules.
- Skills.
- Workflows.
- Task planning capabilities.
- Agent Manager capabilities.
- Available subagent functionality.
- CLI functionality.
- MCP configuration and existing integrations.
- Supported lifecycle hooks.
- Permission and execution controls.
- Context-management mechanisms.
- Relevant workspace documentation.

Use the installed version's actual documentation and existing configurations.

Do not invent unsupported file formats, configuration keys, or capabilities.

If a particular feature is unavailable, identify a compatible alternative.

## 4.1 Repository reconnaissance

Inspect:

- Existing repository structure.
- Root documentation.
- Architecture indexes.
- Architectural decision records.
- Project-state records.
- Session continuity records.
- Engineering lessons.
- Existing skills.
- Existing workflows.
- Existing testing infrastructure.
- Existing reliability and security tools.
- CI configuration.
- Rust project configuration.
- C++ build configuration.
- Python project configuration.

Specifically investigate whether existing engineering infrastructure already provides:

- Architectural authority management.
- Session continuity.
- Failure collection.
- Execution evidence.
- Pre-tool validation.
- Approval management.
- Multi-tier execution.
- Reliability testing.

Verify every finding from the actual repository.

## 4.2 Produce a baseline report

Create a baseline report describing:

- Existing Antigravity capabilities.
- Existing repository engineering infrastructure.
- Missing capabilities.
- Duplicate mechanisms.
- Conflicting instructions.
- Reliability weaknesses.
- Security weaknesses.
- Context-management weaknesses.
- Language-specific development weaknesses.
- Opportunities for improvement.

Classify every finding:

P0 — Critical security or data-integrity issue.

P1 — Significant reliability or architectural issue.

P2 — Important engineering improvement.

P3 — Optional enhancement.

Do not modify anything during reconnaissance.

**Phase acceptance:** A complete, evidence-based understanding of the current environment.

---

# 5. PHASE 1 — ENGINEERING KNOWLEDGE AND AUTHORITY SYSTEM

**Objective:** Make Antigravity consistently understand Jarvis before performing engineering work.

Improve the existing knowledge organization rather than creating a parallel documentation hierarchy.

Establish a clear mechanism for retrieving authoritative information.

## 5.1 Required knowledge categories

Ensure the environment can retrieve:

1. Project mission and development objectives.
2. System architecture.
3. Module responsibilities.
4. Dependency relationships.
5. Architectural decisions.
6. Security requirements.
7. Coding conventions.
8. Testing requirements.
9. Performance requirements.
10. Known limitations.
11. Previous engineering failures.
12. Current development state.

## 5.2 Authority and conflict resolution

Establish an explicit precedence model.

The following must remain authoritative:

- Platform and safety requirements.
- Explicit human instructions.
- Existing repository authority and security policies.
- Accepted architectural decisions.
- Current task acceptance criteria.
- Relevant specialized skills.
- Convenience documentation and examples.

A low-level skill must never silently override a higher-level security or architectural requirement.

When two authoritative documents conflict:

1. Identify the conflicting statements.
2. Determine their scope and authority.
3. Identify the affected subsystem.
4. Record the conflict.
5. Follow the safer existing requirement.
6. Recommend a formal architectural resolution.

Do not silently rewrite an ADR to make an implementation convenient.

## 5.3 Context retrieval

Implement an appropriate context-retrieval strategy.

For every engineering task, Antigravity should retrieve only the information relevant to that task.

For example:

A Rust ownership-related change should retrieve:

- Relevant subsystem architecture.
- Rust-specific engineering standards.
- Ownership and lifecycle requirements.
- Related tests.
- Relevant ADRs.

It should not unnecessarily load every architectural document in the repository.

Use progressive disclosure:

- Short top-level instructions.
- Task-specific workflow guidance.
- Specialized skills.
- Detailed reference documents when required.

Avoid enormous instruction files that consume context without improving decisions.

## 5.4 Documentation consistency

Create or improve a navigation mechanism that identifies:

- Canonical architecture documentation.
- Current project state.
- Relevant ADRs.
- Relevant engineering policies.
- Available skills.
- Available workflows.

Maintain one authoritative source for each category.

**Phase acceptance:** Antigravity can discover the correct engineering knowledge without relying on repeated manual explanations.

---

# 6. PHASE 2 — SPECIALIZED ENGINEERING SKILLS

**Objective:** Build a reusable engineering skill system.

Do not create generic skills that simply say to write high-quality code.

Each skill must contain practical procedures, constraints, decision criteria, and verification requirements.

First inspect the existing skills and preserve any that are already correct.

## 6.1 Required skill categories

Implement or improve the following capabilities.

### Skill 1 — Repository Intelligence

Responsibilities:

- Repository reconnaissance.
- Dependency analysis.
- Architecture discovery.
- Call-graph investigation.
- Code ownership identification.
- Existing implementation discovery.
- Change-impact assessment.

The agent must understand the relevant implementation before proposing modifications.

### Skill 2 — Engineering Task Planning

Responsibilities:

- Requirement decomposition.
- Scope identification.
- Risk classification.
- Dependency identification.
- Acceptance-criteria creation.
- Implementation sequencing.
- Verification planning.

Every non-trivial engineering task must have a concrete execution plan.

### Skill 3 — Rust Engineering

Specialized requirements:

- Ownership and borrowing.
- Lifetime correctness.
- Concurrency safety.
- Error handling.
- Trait design.
- Async architecture.
- Memory and resource management.
- Cargo workspace discipline.
- Unit and integration testing.

### Skill 4 — C++ Engineering

Specialized requirements:

- RAII.
- Resource ownership.
- Object lifetime.
- Memory safety.
- Undefined-behavior awareness.
- Concurrency.
- CMake.
- Compiler diagnostics.
- CTest.
- Static analysis.

Avoid unnecessary raw ownership and unsafe resource-management patterns.

### Skill 5 — Python Engineering

Specialized requirements:

- Module boundaries.
- Type safety.
- Exception handling.
- Dependency management.
- Async programming where applicable.
- Resource lifecycle management.
- Testing.
- Static analysis.
- Maintainability.

### Skill 6 — Architecture and Design

Responsibilities:

- Architectural investigations.
- Boundary analysis.
- Interface design.
- Dependency direction.
- Coupling analysis.
- Design-pattern selection.
- ADR maintenance.
- Architectural regression prevention.

Do not introduce abstractions without demonstrating why they are necessary.

### Skill 7 — Test-Driven Development

Responsibilities:

- Acceptance-test design.
- Regression-test creation.
- Unit testing.
- Integration testing.
- Negative testing.
- Boundary testing.
- Failure-path verification.
- Test-quality analysis.

Apply Red-Green-Refactor when appropriate.

The agent must verify that its tests actually exercise the intended behavior.

### Skill 8 — Root-Cause Debugging

Responsibilities:

- Reproduce failures.
- Collect evidence.
- Identify root causes.
- Distinguish symptoms from causes.
- Minimize reproduction cases.
- Validate corrective changes.
- Prevent regression.

Avoid speculative fixes.

### Skill 9 — Security Engineering

Responsibilities:

- Threat modeling.
- Trust-boundary analysis.
- Authorization validation.
- Input validation.
- Filesystem boundaries.
- Process isolation.
- Secret handling.
- Dependency security.
- Security regression testing.

Existing security policies must never be weakened for convenience.

### Skill 10 — Performance Engineering

Responsibilities:

- Profiling methodology.
- Bottleneck identification.
- Benchmark design.
- Memory profiling.
- CPU profiling.
- Concurrency analysis.
- Latency measurements.
- Regression detection.

No optimization should be accepted without appropriate measurements.

### Skill 11 — Independent Code Review

Responsibilities:

- Review implementation correctness.
- Identify architectural violations.
- Detect security regressions.
- Detect unnecessary complexity.
- Evaluate test completeness.
- Detect hidden failure conditions.
- Check interface compatibility.

The reviewer must independently inspect the actual changes rather than simply approving the implementation agent's explanation.

### Skill 12 — Engineering Documentation and Continuity

Responsibilities:

- Maintain authoritative project state.
- Record accepted decisions.
- Document meaningful failures.
- Update relevant lessons.
- Preserve development continuity.
- Avoid duplicated knowledge.

Only record durable, verified engineering information.

Do not store secrets or indiscriminately copy entire sessions into permanent project documentation.

## 6.2 Skill quality requirements

Every skill must contain:

1. Purpose.
2. Activation criteria.
3. Preconditions.
4. Required knowledge.
5. Step-by-step procedures.
6. Engineering restrictions.
7. Relevant validation commands.
8. Expected evidence.
9. Failure-handling guidance.
10. Completion criteria.
11. Related skills and documentation.

Skills must be narrowly scoped.

Do not duplicate the same long engineering instructions across every skill.

Use shared references where supported.

## 6.3 Skill routing

Create a clear skill-discovery and routing mechanism.

Examples:

- Rust memory bug → Rust Engineering + Root-Cause Debugging + TDD.
- C++ filesystem vulnerability → C++ Engineering + Security Engineering + TDD + Independent Review.
- Python performance regression → Python Engineering + Performance Engineering + Root-Cause Debugging.
- Cross-language architecture change → Repository Intelligence + Architecture and Design + relevant language skills + Independent Review.

Only activate skills relevant to the task.

**Phase acceptance:** Every required engineering capability is reusable, discoverable, properly scoped, and compatible with Antigravity's actual skill mechanism.

---

# 7. PHASE 3 — STRUCTURED ENGINEERING WORKFLOWS

**Objective:** Replace inconsistent task execution with repeatable workflows.

Implement reusable workflows supported by the installed Antigravity environment.

At minimum, establish the following.

## Workflow A — Repository Reconnaissance

1. Identify the requested subsystem.
2. Inspect relevant files.
3. Trace dependencies.
4. Read relevant architecture documents.
5. Inspect associated tests.
6. Map affected interfaces.
7. Produce a findings report.
8. Recommend the smallest appropriate change.

No modifications during reconnaissance.

## Workflow B — New Feature Implementation

1. Understand the requirement.
2. Identify affected subsystems.
3. Investigate existing functionality.
4. Define acceptance criteria.
5. Establish architectural compatibility.
6. Identify failure modes.
7. Create a phased implementation plan.
8. Create or identify tests.
9. Implement incrementally.
10. Run verification.
11. Perform independent review.
12. Document significant changes.

## Workflow C — Bug Fixing

1. Reproduce the problem.
2. Gather evidence.
3. Identify the root cause.
4. Determine the appropriate fix.
5. Add a regression test.
6. Implement the correction.
7. Execute relevant tests.
8. Inspect the final diff.
9. Independently review the solution.

Never mask the original problem.

## Workflow D — Architecture Modification

1. Investigate the current architecture.
2. Identify the architectural deficiency.
3. Examine alternatives.
4. Analyze trade-offs.
5. Assess backward compatibility.
6. Evaluate security and performance implications.
7. Establish acceptance criteria.
8. Prepare the necessary ADR changes.
9. Implement only the approved scope.
10. Perform architectural regression verification.

## Workflow E — Security Modification

1. Identify assets and trust boundaries.
2. Analyze the threat model.
3. Identify relevant attack surfaces.
4. Inspect existing protections.
5. Identify the exact deficiency.
6. Define expected security properties.
7. Implement a minimal correction.
8. Run positive and negative security tests.
9. Inspect for regressions.
10. Produce evidence.

## Workflow F — Performance Optimization

1. Establish a baseline.
2. Identify the actual bottleneck.
3. Profile the relevant execution path.
4. Establish measurable objectives.
5. Identify optimization alternatives.
6. Implement the smallest justified improvement.
7. Measure the new result.
8. Check functional equivalence.
9. Check memory and concurrency implications.
10. Document the measurements.

## Workflow G — Independent Code Review

1. Obtain the actual diff.
2. Identify the task's original acceptance criteria.
3. Inspect modified files.
4. Inspect surrounding code.
5. Examine test coverage.
6. Check architectural compliance.
7. Check security.
8. Check error handling.
9. Identify unverified assumptions.
10. Produce findings by severity.

Do not review only the implementation summary.

## Workflow H — Session Continuity

Before a development session:

- Retrieve current project state.
- Retrieve active engineering objectives.
- Identify incomplete work.
- Retrieve relevant lessons.
- Verify the repository's actual state.

At the end of a significant session:

- Record completed work.
- Record verified test results.
- Record unresolved issues.
- Update relevant project-state records.
- Preserve relevant decisions.
- Avoid recording unverified assumptions as facts.

**Phase acceptance:** Antigravity has repeatable workflows for the major classes of engineering activity.

---

# 8. PHASE 4 — DETERMINISTIC ENGINEERING GUARDRAILS

**Objective:** Make engineering quality enforceable rather than dependent on model judgment.

Inspect the installed Antigravity version and existing repository mechanisms before implementing any hooks or automated controls.

Use supported mechanisms only.

## 8.1 Pre-execution validation

Before potentially consequential operations, verify:

- Correct repository.
- Correct working directory.
- Appropriate task scope.
- Relevant file ownership.
- Existing working-tree changes.
- Permission requirements.
- Potential destructive consequences.

## 8.2 Implementation boundaries

Establish safeguards against:

- Unnecessary production-code modifications.
- Unauthorized dependency installation.
- Security-control modification.
- Test deletion or weakening.
- Broad unrelated refactoring.
- Unexpected generated files.
- Unnecessary changes to public interfaces.
- Unexplained changes to build configuration.

## 8.3 Test-integrity protection

Build procedures that detect suspicious test modifications.

Examples:

- Removing assertions.
- Weakening expected results.
- Skipping failing tests.
- Replacing meaningful tests with trivial tests.
- Increasing timeouts without evidence.
- Suppressing errors.
- Changing fixtures to conceal regressions.

Every significant test modification must be justified by the underlying requirement.

## 8.4 Completion verification

Require appropriate verification before an engineering task can be described as complete.

A successful tool invocation alone is not sufficient.

The result must include evidence from the relevant checks.

## 8.5 Safe failure handling

When an operation fails:

- Preserve diagnostic information.
- Do not repeatedly execute destructive commands.
- Do not silently bypass validation.
- Identify the cause.
- Attempt a safe correction when appropriate.
- Report unresolved failures accurately.

Do not enable broad automatic-approval or unrestricted-execution settings to simplify automation.

**Phase acceptance:** Critical engineering restrictions are represented by deterministic controls wherever supported, with documented fallbacks where they are not.

---

# 9. PHASE 5 — LANGUAGE-SPECIFIC VERIFICATION INFRASTRUCTURE

**Objective:** Ensure that Rust, C++, and Python each have appropriate validation standards.

Do not introduce additional tools or dependencies simply to make the checklist appear comprehensive.

Discover the tools already configured in the repository.

## 9.1 Rust verification

Investigate and apply relevant existing checks:

- cargo fmt --check
- cargo check
- cargo clippy
- cargo test
- Workspace-specific tests.

Assess:

- Ownership correctness.
- Lifetime correctness.
- Error propagation.
- Async behavior.
- Concurrency safety.
- Resource lifecycle.
- Public API compatibility.

Only enforce additional strictness when compatible with the established codebase and baseline.

## 9.2 C++ verification

Investigate existing:

- CMake configuration.
- Build presets.
- Compiler configuration.
- CTest integration.
- Static-analysis configuration.

Apply applicable checks:

- Successful configuration.
- Successful compilation.
- Compiler warning analysis.
- CTest execution.
- Static analysis.
- Appropriate sanitizer testing when supported.

Assess:

- RAII.
- Memory safety.
- Resource ownership.
- Lifetime errors.
- Undefined behavior.
- Thread safety.
- Error handling.

## 9.3 Python verification

Investigate available:

- Ruff.
- Pytest.
- Mypy or Pyright.
- Existing linting and formatting configuration.

Apply the existing project standards.

Assess:

- Type correctness.
- Module boundaries.
- Exception handling.
- Resource lifecycle.
- Dependency management.
- Async correctness.
- Regression coverage.

## 9.4 Cross-language integration

For functionality spanning multiple languages, investigate:

- FFI boundaries.
- ABI compatibility.
- Data serialization.
- Ownership transfer.
- Error propagation.
- Threading assumptions.
- Resource lifetime.
- Interface compatibility.

A module passing its individual tests does not prove that cross-language integration works.

Create or improve integration tests where justified.

## 9.5 Verification policy

Run tests progressively:

1. Focused tests.
2. Subsystem tests.
3. Integration tests.
4. Relevant full test suites.

Record unavailable checks and environment limitations.

Do not install dependencies, update lockfiles, or alter toolchain configuration without authorization.

**Phase acceptance:** Each language has a practical, repository-compatible validation process, and cross-language boundaries are explicitly considered.

---

# 10. PHASE 6 — INDEPENDENT REVIEW AND MULTI-AGENT COORDINATION

**Objective:** Reduce the risk of an implementation agent approving its own mistakes.

Investigate the capabilities of Antigravity Agent Manager, CLI, and available subagents.

Do not assume that a particular multi-agent feature is available.

## 10.1 Role separation

Where supported, establish distinct responsibilities:

**Architect Agent**

- Investigates architecture.
- Identifies constraints.
- Defines implementation boundaries.
- Reviews design alternatives.

**Implementation Agent**

- Implements the approved scope.
- Uses relevant engineering skills.
- Writes and runs tests.
- Produces implementation evidence.

**Testing Agent**

- Reviews test coverage.
- Identifies missing edge cases.
- Executes independent verification.
- Checks regression behavior.

**Security Reviewer**

- Reviews trust boundaries.
- Examines authorization and input validation.
- Checks security regressions.
- Evaluates unsafe assumptions.

**Independent Review Agent**

- Examines the final diff.
- Compares it against requirements.
- Identifies defects and unnecessary complexity.
- Produces independent findings.

## 10.2 Agent communication

Agents must exchange structured information:

- Task identifier.
- Objective.
- Relevant context.
- Scope.
- Constraints.
- Expected output.
- Findings.
- Evidence.
- Unresolved issues.

Avoid flooding agents with irrelevant repository information.

## 10.3 Conflict resolution

If agents disagree:

1. Identify the precise disagreement.
2. Gather repository evidence.
3. Compare requirements.
4. Evaluate the consequences of each option.
5. Preserve security and architectural requirements.
6. Record the resolution.

Do not use majority voting as a substitute for technical verification.

## 10.4 Independent-review requirement

The reviewer must not merely repeat the implementer's explanation.

It must inspect the actual implementation independently.

If independent subagents are unavailable, implement a documented alternative review process using a separate review pass.

**Phase acceptance:** Significant engineering changes can receive independent verification rather than relying exclusively on the implementation agent's judgment.

---

# 11. PHASE 7 — AGENT EVALUATION AND QUALITY MEASUREMENT

**Objective:** Establish a measurable way to determine whether Antigravity's engineering environment is improving.

Do not measure success through generated code volume or task completion claims alone.

Create an evaluation system.

## 11.1 Evaluation categories

Evaluate:

1. Repository understanding.
2. Architecture compliance.
3. Requirement interpretation.
4. Scope discipline.
5. Correct implementation.
6. Test quality.
7. Security awareness.
8. Error handling.
9. Context continuity.
10. Documentation quality.
11. Independent-review accuracy.
12. Evidence quality.

## 11.2 Evaluation scenarios

Create a controlled set of repository-appropriate test tasks.

Include scenarios such as:

- Locate an existing implementation without duplicating it.
- Explain a module's architecture accurately.
- Identify an incorrect dependency boundary.
- Implement a small Rust correction.
- Diagnose a C++ resource-lifecycle defect.
- Diagnose a Python exception-handling problem.
- Identify an integration-test deficiency.
- Detect an architectural regression.
- Identify an unsafe security-policy modification.
- Reject an unnecessary dependency addition.
- Recover from a failed test without weakening it.
- Resume a development task using existing project context.

Use synthetic or non-destructive scenarios where possible.

Do not introduce intentional vulnerabilities into production code.

## 11.3 Scoring

Use a five-point scale for each applicable category.

Score:

- Correctness.
- Architectural compliance.
- Maintainability.
- Verification completeness.
- Security discipline.
- Context accuracy.
- Scope control.
- Evidence quality.

Record:

- Evaluation scenario.
- Expected outcome.
- Actual outcome.
- Failed acceptance criteria.
- Evidence.
- Score.
- Root cause.

Do not fabricate baseline scores.

Establish the initial baseline using actual evaluation results.

## 11.4 Regression tracking

Preserve a stable set of evaluation scenarios.

When skills, rules, workflows, or hooks are modified, rerun the relevant scenarios.

Reject an improvement when it introduces significant regressions elsewhere.

Do not optimize solely for the evaluation score.

**Phase acceptance:** The engineering environment has a repeatable method for measuring its performance and detecting regressions.

---

# 12. PHASE 8 — CONTINUOUS ENGINEERING IMPROVEMENT

**Objective:** Make the engineering environment improve through verified lessons.

Antigravity should learn from engineering failures without blindly accumulating instructions.

## 12.1 Failure analysis

When a significant failure occurs:

1. Record the observed behavior.
2. Establish the root cause.
3. Determine whether the failure concerns knowledge, planning, implementation, verification, or tooling.
4. Identify the smallest durable improvement.
5. Decide whether the correction belongs in a skill, workflow, rule, test, or architectural document.
6. Implement the correction within authorized scope.
7. Verify that it does not introduce regressions.

## 12.2 Lesson quality

Do not turn every individual failure into a permanent rule.

A lesson should be retained when it is:

- Reproducible.
- Generally useful.
- Relevant to future development.
- Supported by evidence.
- Clearly scoped.

Avoid duplicating existing guidance.

## 12.3 Knowledge lifecycle

Periodically identify:

- Outdated instructions.
- Conflicting guidance.
- Unused skills.
- Duplicate workflows.
- Broken references.
- Obsolete technical assumptions.
- Excessively long instructions.

Recommend improvements and make only authorized, evidence-supported modifications.

## 12.4 Controlled self-improvement

Changes to the engineering environment must themselves be reviewed.

Do not permit a skill or workflow to automatically grant itself new permissions.

Do not permit agent evaluations to modify their own success criteria simply because the results are unfavorable.

Maintain an inspectable record of meaningful changes.

**Phase acceptance:** Failures can produce durable improvements without creating uncontrolled documentation growth or weakening engineering requirements.

---

# 13. PHASE 9 — DOCUMENTATION AND DEVELOPER EXPERIENCE

**Objective:** Make the enhanced environment easy for future Antigravity sessions to operate.

Create or improve the necessary entry points.

Documentation should explain:

- How Antigravity initializes a Jarvis workspace.
- Where authoritative context resides.
- How skills are discovered.
- How workflows are selected.
- How language standards are applied.
- How verification works.
- How independent review works.
- How evaluations work.
- How session continuity works.
- How to recover from failures.

Produce a clear engineering navigation document.

A newly initialized Antigravity session should be able to discover the required procedures without relying on the previous session's conversation history.

Documentation must remain synchronized with actual working configurations.

Never document a feature as operational unless it has been verified.

---

# 14. PHASE 10 — FINAL INDEPENDENT AUDIT

**Objective:** Independently verify the completed transformation.

Do not simply inspect whether the files have been created.

Audit whether the engineering environment actually works.

## 14.1 Structural verification

Check:

- Instruction discovery.
- Skill discovery.
- Workflow discovery.
- Documentation references.
- Configuration validity.
- Duplicate instructions.
- Conflicting instructions.
- Unsupported features.
- Broken links.
- Incorrect paths.

## 14.2 Engineering verification

Check:

- Rust validation procedures.
- C++ validation procedures.
- Python validation procedures.
- Cross-language testing procedures.
- TDD guidance.
- Root-cause debugging procedures.
- Independent code-review procedures.

## 14.3 Security verification

Check that the transformation did not introduce:

- Permission escalation.
- Unrestricted command execution.
- Automatic destructive actions.
- Security-policy bypasses.
- Secret exposure.
- Unsafe network dependencies.
- Test-integrity weaknesses.
- Unauthorized external connections.

## 14.4 Practical evaluation

Execute the available evaluation scenarios.

Demonstrate that the new environment can:

1. Discover authoritative project context.
2. Select relevant skills.
3. Follow an appropriate workflow.
4. Identify the correct implementation boundaries.
5. Perform appropriate validation.
6. Generate reliable evidence.
7. Identify unresolved defects.
8. Preserve existing project state.

## 14.5 Final acceptance

Do not declare the transformation completely successful if critical acceptance criteria remain unverified.

Classify outstanding issues by severity.

---

# 15. REQUIRED DELIVERABLES

At the end of the transformation, produce a comprehensive report containing:

## A. Initial environment assessment

- Existing environment capabilities.
- Existing infrastructure.
- Identified deficiencies.
- Improvement priorities.

## B. Implemented enhancements

For every modification:

- File path.
- Purpose.
- Reason for modification.
- Technical implementation.
- Dependencies.
- Verification evidence.

## C. Skills inventory

For every skill:

- Name.
- Purpose.
- Activation conditions.
- Scope.
- Validation status.

## D. Workflow inventory

Document all implemented workflows and their responsibilities.

## E. Engineering standards

Report the final Rust, C++, Python, and cross-language verification standards.

## F. Guardrail inventory

Document all implemented safety and quality controls.

Distinguish between:

- Deterministic enforcement.
- Documented agent instructions.
- Manual or independent review.
- Unsupported capabilities.

## G. Evaluation results

Report:

- Scenarios executed.
- Expected outcomes.
- Actual outcomes.
- Scores.
- Failures.
- Regressions.
- Unverified scenarios.

## H. Security assessment

Confirm exactly which security-sensitive settings and configurations were inspected or changed.

The default result should be that global security settings and permissions remain untouched.

## I. Remaining limitations

Explicitly identify:

- Missing Antigravity capabilities.
- Tooling limitations.
- Unavailable tests.
- Unresolved architectural concerns.
- Improvements requiring additional authorization.

## J. Final engineering status

Classify the result:

- COMPLETE — All applicable acceptance criteria verified.
- PARTIALLY COMPLETE — Core infrastructure implemented, but specific criteria remain unverified.
- BLOCKED — A critical dependency prevents safe completion.

Never claim COMPLETE solely because the requested files exist.

---

# 16. IMPLEMENTATION ORDER AND EXECUTION DISCIPLINE

Execute the phases in the following order:

| Phase | Objective | Required outcome |
|---|---|---|
| 0 | Reconnaissance | Verified baseline |
| 1 | Knowledge and authority | Reliable project context |
| 2 | Specialized skills | Reusable engineering expertise |
| 3 | Workflows | Repeatable execution |
| 4 | Guardrails | Enforceable controls |
| 5 | Language verification | Rust/C++/Python quality gates |
| 6 | Independent review | Separate verification |
| 7 | Agent evaluation | Measurable reliability |
| 8 | Continuous improvement | Controlled learning |
| 9 | Documentation | Operational clarity |
| 10 | Final audit | Evidence-backed acceptance |

Within each phase:

1. Inspect existing implementations.
2. Identify the actual deficiency.
3. Design the smallest appropriate correction.
4. Implement incrementally.
5. Validate the changes.
6. Inspect the actual diff.
7. Record the evidence.
8. Proceed to the next phase.

If a phase encounters an obstacle, complete all independent safe tasks and document the specific blocker.

Do not abandon the entire transformation because one optional Antigravity feature is unavailable.

Do not execute speculative improvements simply to complete the phase checklist.

---

# 17. DEFINITION OF SUCCESS

The transformation is successful when Antigravity operates under the following conditions:

**Understanding**

It investigates before coding and retrieves authoritative project context.

**Planning**

It translates requirements into bounded, verifiable engineering work.

**Expertise**

It uses appropriate Rust, C++, Python, architecture, security, testing, and performance practices.

**Discipline**

It avoids unnecessary code, dependencies, abstractions, and unrelated modifications.

**Verification**

It relies on compilation, automated tests, static analysis, and actual evidence.

**Independence**

It subjects significant work to independent review.

**Continuity**

It preserves durable engineering knowledge across sessions.

**Adaptability**

It can improve its workflows when repeatable weaknesses are discovered.

**Security**

It never sacrifices security or data integrity for convenience or completion speed.

**Transparency**

It reports exactly what it did, what it tested, what failed, and what remains uncertain.

---

# 18. FINAL EXECUTION INSTRUCTION

Begin with PHASE 0.

You are expected to perform this transformation autonomously within the stated boundaries.

Do not merely return suggestions, recommendations, or a list of files that should theoretically be created.

Inspect the real environment and implement the improvements.

Do not blindly trust your existing configuration.

Do not blindly assume that new configurations are better.

Every important decision must be supported by repository evidence, compatibility, and a clear engineering justification.

Preserve the existing Jarvis architecture and all pre-existing user work.

Do not modify production application functionality as part of this infrastructure transformation.

Do not commit or push changes.

The final result must be an inspectable, evidence-backed engineering environment that makes future Jarvis development more reliable, structured, secure, and maintainable.

**The objective is not to make you generate code faster.**

**The objective is to make you consistently produce better engineering work.**