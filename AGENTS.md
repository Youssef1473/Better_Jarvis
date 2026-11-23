# AGENTS.md — Better_Jarvis Engineering Constitution

## 1. Chain of Command & Workflow
- **Architecture & Approval**: Governed by `BETTER_JARVIS_PLAN.md` under CEO review.
- **One Ticket Per Turn**: Execute strictly ONE active ticket from `BETTER_JARVIS_PLAN.md`, then STOP and output the `HANDOFF REPORT`. Never advance without explicit `APPROVED: PROCEED TO TICKET-XX`.
- **Read Before Editing**: Inspect real files on disk before modifying code. Never assume structs, flags, or APIs match prior summaries.
- **Scope Discipline**: Edit only files required for the active ticket. Log unrelated bugs under `Discovered Landmines` instead of stealth-refactoring.

## 2. Hard Security Invariants (Non-Negotiable)
- **Zero Shell Interpolation**: Never invoke `cmd.exe` or `powershell.exe` from daemon tools. Launch strictly allowlisted executables directly via `Command::new(abs_path)` with explicit `argv` vectors.
- **Handle-Based Path Containment**: Validate file access against opened handles (`GetFinalPathNameByHandleW`), inspecting reparse points/NTFS junctions. Always reject Alternate Data Streams, reserved Win32 device names (`CON`, `NUL`, `\\.\`), and secret files (`.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `.ssh/`, `.aws/`, `.git/config`) even inside allowed directories.
- **Named Pipe Isolation**: Server pipes must use `FILE_FLAG_FIRST_PIPE_INSTANCE | PIPE_REJECT_REMOTE_CLIENTS`, an explicit Logon-SID SDDL DACL (zero `Everyone`/`Anonymous`), and `GetNamedPipeClientProcessId` verification against the parent host PID.
- **Isolated Browser Automation**: Never attach Chrome CDP to the user's primary browser profile or open a localhost TCP debugging port. Use an ephemeral `--user-data-dir` with `--remote-debugging-pipe` and a strict CDP method allowlist.
- **Prompt Injection & Taint Escalation**: Wrap external/untrusted content in per-turn random nonce delimiters. Once untrusted content enters context, set `context_tainted = true` and require Tier 3 HUD confirmation for all state mutations, file writes, clipboard writes, and outbound navigations.

## 3. C++20 Real-Time Audio & Win32 Laws (`host-cpp`)
- **Real-Time Audio Thread Purity**: Threads guarded by MMCSS (`Pro Audio`) must execute ZERO heap allocations, ZERO mutex locks, ZERO synchronous pipe I/O, and ZERO disk logging.
- **Lock-Free SPSC Hygiene**: Ring buffer producer/consumer indices must sit on separate 64-byte cache lines (`alignas(64)`) using explicit `memory_order_acquire`, `memory_order_release`, and `memory_order_relaxed`—never default `seq_cst` or unguarded `relaxed` cross-thread loads.
- **WASAPI Resilience**: Handle `AUDCLNT_E_DEVICE_INVALIDATED`, `AUDCLNT_BUFFERFLAGS_SILENT`, and `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY`. Never assume native 16kHz mic hardware without `AUTOCONVERTPCM` or explicit resampling.
- **Input Hook Safety**: Never run UI rendering, pipe writes, or blocking work inside a `WH_KEYBOARD_LL` callback (1000ms Windows silent-unhook ceiling). Prefer Raw Input or an isolated zero-work hook thread.
- **Process Supervision**: Always spawn `jarvis-daemon.exe` with `CREATE_SUSPENDED` and `bInheritHandles = FALSE`, assign to the Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), then `ResumeThread`.

## 4. Rust Async & IPC Laws (`daemon-rust`)
- **Zero Blocking on Tokio Workers**: Never run synchronous `rusqlite` queries or Win32 COM calls (`os_control.rs`) on Tokio executor threads. Route them through dedicated actor threads with bounded `mpsc` channels.
- **SQLite Durability**: Always initialize with `PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;`.
- **Bounded IPC Framing**: Enforce hard per-message-type payload limits BEFORE calling `BytesMut::reserve`. Reject oversized or malformed `JRVS` frames immediately.
- **Cancellation**: Tie every voice/LLM/tool turn to a hierarchical `CancellationToken` for instant barge-in teardown.

## 5. Verification Bar
- Never weaken compiler warnings, silence Clippy/MSVC diagnostics without a documented reason, or stub security checks to pass tests.
- Every code-modifying ticket must report exact commands and real output for `cargo test` / `cargo clippy` (`daemon-rust`) and CMake/C++ builds (`host-cpp`).
