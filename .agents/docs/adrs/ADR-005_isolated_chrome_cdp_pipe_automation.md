# ADR-005: Isolated Chrome CDP Pipe Automation
**Status:** Accepted  
**Date:** 2026-11-21  
**Context:** TICKET-05 (Hardening & Production Execution Plan)  
**Governed Subsystems:** `daemon-rust/src/tools/browser_cdp.rs`

---

## 1. Context & Problem Statement
Attaching Chrome DevTools Protocol (CDP) via `--remote-debugging-port=9222` to the user's primary browser profile exposes active session cookies, credentials, and browsing history to localhost socket hijacking. Any unprivileged process listening on `127.0.0.1` could steal session tokens or manipulate existing browser tabs.

## 2. Decision
1. **Ephemeral Profile Isolation:**
   - Never attach to the user's primary Google Chrome profile.
   - Always spawn a dedicated automation Chrome instance using an ephemeral `--user-data-dir` created under the isolated data directory.
2. **Anonymous Pipe Transport:**
   - Eliminate localhost TCP listening sockets entirely.
   - Use Chrome's `--remote-debugging-pipe` flag. The browser communicates over anonymous OS file descriptors (pipes) inherited directly and exclusively by `daemon-rust`.
3. **CDP Method Allowlist:**
   - Enforce a strict allowlist of permitted CDP domains and methods: `Page.navigate`, `Runtime.evaluate` (scoped strictly to the active automation tab), `DOM.getDocument`, `Accessibility.getFullAXTree`.
   - Explicitly deny `Network.getAllCookies`, `Storage.*`, and cross-target enumeration.

## 3. Consequences
- **Positive:** Zero open TCP debugging ports; user's real browser profile, passwords, and sessions remain isolated.
- **Trade-off:** Automation tabs cannot share the user's logged-in state without deliberate session import.

## 4. Verification Evidence
- `daemon-rust` tests: `tools::browser_cdp::tests::test_cdp_security_gate_method_allowlist`, `tools::browser_cdp::tests::test_cdp_security_gate_navigation_urls`, `tools::browser_cdp::tests::test_managed_cdp_browser_pipe_lifecycle` pass.
- `scripts/verify-beta.ps1` Stage 2 asserts zero instances of `127.0.0.1:9222` or `remote-debugging-port` across `daemon-rust/src`.
