---
name: security-engineering
description: >-
  Security invariants, threat modeling, handle-based path sandboxing, zero-shell execution, pipe DACLs, taint tracking, and CDP isolation for Better_Jarvis. Use when implementing or auditing security-critical code.
---

# Security Engineering Standards & Invariants (`security-engineering`)

## 1. Purpose
Defines mandatory security controls, trust boundary models, path containment verification, shell injection prevention, prompt injection defenses, and audit procedures for Better_Jarvis.

## 2. Activation Criteria
- **When to Use:**
  - Modifying or reviewing any code under `daemon-rust/src/security/`, `daemon-rust/src/tools/`, or IPC subsystems.
  - Adding new tools that touch the filesystem, network, browser, or desktop UI.
  - Reviewing changes for prompt injection vulnerabilities, secret leaks, or path traversal.
- **When NOT to Use:**
  - Non-security-related documentation or internal unit test refactoring.

## 3. Preconditions
- Git status verified clean.
- Existing security policies inspected in `daemon-rust/src/security/`.

## 4. Required Knowledge
- Hard Security Invariants: [AGENTS.md](../../../AGENTS.md#2-hard-security-invariants-non-negotiable)
- ADR-001 (Named Pipe SDDL Isolation): [ADR-001](../../docs/adrs/ADR-001_named_pipe_sddl_isolation_and_jrvs_codec_v2.md)
- ADR-003 (Handle-Based Path Sandbox): [ADR-003](../../docs/adrs/ADR-003_handle_based_path_containment_and_zero_shell.md)
- ADR-004 (Nonces & Taint Engine): [ADR-004](../../docs/adrs/ADR-004_cryptographic_nonce_delimiters_and_session_taint_escalation.md)
- ADR-005 (Chrome CDP Pipe Isolation): [ADR-005](../../docs/adrs/ADR-005_isolated_chrome_cdp_pipe_automation.md)
- Trail of Bits Insecure Defaults Checklist: [insecure-defaults](../insecure-defaults/SKILL.md)
- Language Sharp Edges: [sharp-edges](../sharp-edges/SKILL.md)

## 5. Non-Negotiable Security Invariants

### 5.1 Zero Shell Interpolation
- NEVER invoke `cmd.exe` or `powershell.exe` from daemon tools.
- Launch strictly allowlisted binaries via direct `Command::new(resolved_abs_path)` with explicit `argv` vectors.
- Ban shell redirection (`>`, `|`, `&`) and command substitution (`$(...)`).

### 5.2 Handle-Based Path Containment
- Validate file access against open handles using `GetFinalPathNameByHandleW(VOLUME_NAME_DOS)` to resolve reparse points, NTFS junctions, and 8.3 aliases.
- Always reject:
  - Alternate Data Streams (`:` outside drive specifier).
  - Reserved Win32 device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`, `\\.\`).
  - Secret patterns even inside allowed directories: `.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `.ssh/`, `.aws/`, `.git/config`.

### 5.3 Named Pipe SDDL Isolation
- Server pipes must specify `FILE_FLAG_FIRST_PIPE_INSTANCE | PIPE_REJECT_REMOTE_CLIENTS`.
- SDDL DACL must be restricted strictly to the current user's Logon SID.
- On client connect, verify `GetNamedPipeClientProcessId` matches the parent host PID.

### 5.4 Isolated Browser Automation (CDP)
- Never attach to the user's primary Chrome profile.
- Spawn a dedicated Chrome instance with an ephemeral `--user-data-dir`.
- Communicate strictly over anonymous OS pipes (`--remote-debugging-pipe`). NEVER open a localhost TCP debugging port (`127.0.0.1:9222`).
- Enforce a strict CDP method allowlist; deny cookie and storage extraction across origins.

### 5.5 Prompt Injection & Session Taint Tracking
- Wrap all external untrusted content in per-turn random CNG nonces: `<untrusted_data_{nonce}>...</untrusted_data_{nonce}>`.
- When untrusted data enters context, set `context_tainted = true`.
- While context is tainted, automatically escalate all state mutations, file writes, clipboard writes, and outbound navigations to **Tier 3 (Interactive HUD Confirmation)**.

## 6. Step-by-Step Security Audit Procedure

1. **Static Invariant Audit:**
   Run the static security audit rules from `scripts/verify-beta.ps1`:
   - Check for forbidden shell invocations:
     ```powershell
     Get-ChildItem -Path "daemon-rust/src" -Recurse -Filter "*.rs" | Select-String 'Command::new\s*\(\s*["''](powershell|cmd)'
     ```
   - Check for unisolated CDP ports:
     ```powershell
     Get-ChildItem -Path "daemon-rust/src" -Recurse -Filter "*.rs" | Select-String '127\.0\.0\.1:9222|remote-debugging-port'
     ```
   - Check for low-level keyboard hooks:
     ```powershell
     Get-ChildItem -Path "host-cpp/src" -Recurse -Include "*.cpp","*.hpp" | Select-String 'SetWindowsHookEx|WH_KEYBOARD_LL'
     ```
2. **Execute Security Unit Tests:**
   ```powershell
   cargo test -p jarvis-daemon -- security::
   ```
3. **Execute Adversarial Taint Tests:**
   ```powershell
   cargo test -p jarvis-daemon -- taint
   ```

## 7. Relevant Validation Commands
- Run security test suite: `cargo test -p jarvis-daemon -- security::policy::tests`
- Run sandbox test suite: `cargo test -p jarvis-daemon -- security::sandbox::tests`
- Run static security audit: Stage 2 of `& scripts/verify-beta.ps1`

## 8. Expected Evidence
- Static audit reports zero forbidden patterns across the codebase.
- Security and sandbox test suites pass with 100% success.

## 9. Failure-Handling Guidance
- If a security test fails, do NOT adjust the test assertions. Investigate how the input bypassed normalization or why permissions were granted inappropriately.

## 10. Completion Criteria
All security invariants are verified intact, static security audits report zero violations, and all security unit tests pass.

## 11. Related Skills
- [insecure-defaults](../insecure-defaults/SKILL.md)
- [sharp-edges](../sharp-edges/SKILL.md)
- [rust-engineering](../rust-engineering/SKILL.md)
- [independent-review](../independent-review/SKILL.md)
