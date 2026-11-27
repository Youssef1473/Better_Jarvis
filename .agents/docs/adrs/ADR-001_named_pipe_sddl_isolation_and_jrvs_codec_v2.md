# ADR-001: Named Pipe SDDL DACL Isolation & JRVS Codec v2
**Status:** Accepted  
**Date:** 2026-11-20  
**Context:** TICKET-01 (Hardening & Production Execution Plan)  
**Governed Subsystems:** `daemon-rust/src/ipc/`, `host-cpp/src/main.cpp`

---

## 1. Context & Problem Statement
Prior to this decision, the inter-process communication (IPC) between `jarvis-host.exe` and `jarvis-daemon.exe` used a Windows Named Pipe with default security descriptors. This exposed the daemon to local pipe squatting by unprivileged local processes, local MITM eavesdropping, and Head-of-Line blocking or OOM crashes caused by unbounded JSON length allocations.

## 2. Decision Drivers
- Zero unprivileged local access to the voice/control pipe.
- Zero memory exhaustion from malicious or corrupt frame length prefixes (`0xFFFFFFFF`).
- Strict verification that only the genuine parent host process can connect.

## 3. Decision
1. **Named Pipe Creation Flags:** Create the pipe with `FILE_FLAG_FIRST_PIPE_INSTANCE | PIPE_REJECT_REMOTE_CLIENTS`. If a pipe instance already exists, daemon startup fails immediately.
2. **Explicit SDDL DACL:** Generate an explicit SDDL string restricted exclusively to the current user's Logon SID (`S-1-5-5-X-Y`). Explicitly deny `Everyone` (`WD`), `Anonymous` (`AN`), and network callers.
3. **Parent PID Verification:** On incoming client connection, invoke Win32 `GetNamedPipeClientProcessId` and verify the connecting PID matches the expected parent host PID.
4. **JRVS Codec v2:** Implement a binary framing protocol with:
   - 4-byte Magic (`JRVS`)
   - 2-byte Protocol Version (`0x0002`)
   - 2-byte Message Type
   - 4-byte Turn ID
   - 4-byte Little-Endian Payload Length
   - Enforce hard limits *before* memory reservation: max 262,144 bytes (256 KiB) for Control JSON, 65,536 bytes (64 KiB) for Audio PCM.

## 4. Consequences
- **Positive:** Immune to local pipe hijacking and length-prefix OOM attacks.
- **Trade-off:** Connecting processes must run in the exact same logon session and pass PID validation; standalone debugging requires passing `--parent-pid <pid>`.

## 5. Verification Evidence
- `daemon-rust` unit tests: `ipc::codec::tests::test_codec_u32_max_no_oom`, `ipc::codec::tests::test_codec_oversized_json`, `ipc::pipe_security::tests::test_sddl_generation`, `ipc::pipe_security::tests::test_first_pipe_instance_conflict` pass cleanly.
- `scripts/verify-beta.ps1` Stage 5 live test verifies `--parent-pid 999999` is rejected.
