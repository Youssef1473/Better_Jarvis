---
name: insecure-defaults
description: "Audit a file, directory, or whole repo for insecure default configuration: fallback secrets, default credentials, fail-open switches, weak crypto, permissive access, and debug leakage. Use when auditing Named Pipe DACLs, open localhost CDP ports, CORS, and permissive path defaults."
---

# Insecure Defaults Detection

Audit codebase for insecure default configuration, tracing each candidate before reporting it.

## Categories & Reference Guides

Check the reference documentation in `./references/`:
- **Fail-Open Security**: `./references/fail-open-security.md` (e.g. error handling that grants access on failure)
- **Permissive Access**: `./references/permissive-access.md` (e.g. open named pipes, missing DACL, world-readable permissions, 0.0.0.0 bindings, open debug ports like CDP without auth)
- **Fallback Secrets**: `./references/fallback-secrets.md` (e.g. hardcoded keys, default encryption keys)
- **Default Credentials**: `./references/default-credentials.md` (e.g. admin:admin, test accounts in prod)
- **Weak Crypto**: `./references/weak-crypto.md` (e.g. broken PRNG, hardcoded IVs, insecure modes)
- **Debug Features**: `./references/debug-features.md` (e.g. debug endpoints or bypasses active in production)

## Procedure

1. Review configuration files, named pipe initialization, IPC listeners, and sandbox policies against the rules in `references/`.
2. Trace each candidate finding to determine if it is reached in production execution or if safe guards exist.
3. For every confirmed issue, cite the exact file, line number, default value, and the secure alternative required.
