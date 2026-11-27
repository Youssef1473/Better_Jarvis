# Workflow E: Security Modification & Hardening (`security-mod`)
**Status:** Standard Engineering Workflow  
**Objective:** Threat-modeled security policy implementation and invariant verification.

---

## Operating Protocol

```
[Start] ──► 1. Threat Modeling ──► 2. Audit Existing Invariants ──► 3. Define Security Criteria
                                                                            │
┌───────────────────────────────────────────────────────────────────────────┘
▼
4. Red Security Test ──► 5. Minimal Hardening (Green) ──► 6. Static Audit Gate
                                                                  │
[Complete] ◄── 8. Adversarial Review ◄── 7. Regression Gate ◄─────┘
```

### Step 1: Threat Modeling & Trust Boundaries
- Identify assets (user files, secrets, browser sessions, audio streams).
- Map trust boundaries:
  - External untrusted inputs (web pages, OCR text, voice transcriptions).
  - Inter-process boundaries (Named Pipe client PID, Logon SID).
  - Host OS boundary (file paths, spawned child binaries).

### Step 2: Audit Existing Invariants
- Review non-negotiable security invariants in `AGENTS.md`.
- Inspect existing implementations in `daemon-rust/src/security/`.

### Step 3: Define Expected Security Properties
- Explicitly define:
  - Normalized path verbatim format.
  - Required SDDL string or PID check.
  - Cryptographic CNG nonce wrapping and taint escalation criteria.

### Step 4: Author Adversarial Test (RED)
- Write an adversarial test covering the exploit or bypass condition (e.g. `..\`, NTFS junction, ADS `:`, secret filename, metacharacter injection).
- Verify the test catches the vulnerability.

### Step 5: Implement Minimal Hardening (GREEN)
- Apply the minimal code change to enforce the security property.
- Re-run the test to confirm clean pass.

### Step 6: Static Invariant Audit Gate
- Run Stage 2 of `scripts/verify-beta.ps1`:
  - Zero `cmd`/`powershell` invocations.
  - Zero unisolated TCP CDP flags (`127.0.0.1:9222`).
  - Zero low-level keyboard hooks (`WH_KEYBOARD_LL`).
  - Valid `manifest.sha256` model hashes.

### Step 7: Full Regression Gate
- Run full Rust test suite: `cargo test -p jarvis-daemon`.

### Step 8: Adversarial Review
- Execute Workflow G ([code-review.md](./code-review.md)) with security focus.
