# ADR-003: Handle-Based Path Containment & Zero-Shell Process Execution
**Status:** Accepted  
**Date:** 2026-11-21  
**Context:** TICKET-03 (Hardening & Production Execution Plan)  
**Governed Subsystems:** `daemon-rust/src/security/policy.rs`, `sandbox.rs`

---

## 1. Context & Problem Statement
String-based path checks (e.g. `path.starts_with(allowed_root)`) on Windows are vulnerable to NTFS junction escapes, TOCTOU symlink races, 8.3 short filename aliasing, Alternate Data Streams (`file.txt:stream`), and path traversal (`..\`). Furthermore, spawning commands via `cmd.exe /C` or `powershell.exe -Command` enables command injection via shell metacharacters (`&`, `|`, `;`, `$(...)`).

## 2. Decision
1. **Handle-Based Path Containment:**
   - Canonicalize `allowed_directories` once at startup into verbatim normalized Win32 paths.
   - For file access, open the file handle first with non-destructive flags (`FILE_READ_ATTRIBUTES`), then invoke `GetFinalPathNameByHandleW(VOLUME_NAME_DOS)` to resolve all reparse points, junctions, and 8.3 aliases.
   - Verify that the resolved final DOS path starts strictly with an allowed directory prefix.
   - Reject any path containing Alternate Data Streams (`:` beyond drive specifier) or reserved Win32 device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`, `\\.\`).
   - Hard-deny secret file patterns even within allowed directories: `.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `.ssh/`, `.aws/`, `.git/config`.
2. **Zero-Shell Execution:**
   - Ban all invocations of `cmd.exe` and `powershell.exe` from daemon tool execution.
   - Execute strictly allowlisted executables via direct `std::process::Command::new(resolved_abs_path)` with explicit argument vectors (`argv`), passing metacharacters as inert string arguments rather than shell expressions.

## 3. Consequences
- **Positive:** Complete immunity to NTFS junction bypasses and shell injection.
- **Trade-off:** Shell built-ins (e.g. `dir`, `echo`, shell redirection `>`) cannot be run directly; tools must invoke dedicated binary tools.

## 4. Verification Evidence
- `daemon-rust` tests: `security::policy::tests::test_ntfs_junction_escape_prevention`, `security::policy::tests::test_ads_and_reserved_names_rejection`, `security::policy::tests::test_secret_denylist_enforced_inside_allowed_directories`, `security::sandbox::tests::test_shell_execution_rejected`, `security::sandbox::tests::test_lolbin_argument_injection_rejected` all pass.
- `scripts/verify-beta.ps1` Stage 2 audits `daemon-rust/src` to assert zero `powershell` or `cmd` invocations.
