# Better_Jarvis — Hardening & Production Execution Plan

**Owner**: CEO / Principal Architect (Gemini)  
**Executor**: Antigravity CLI (Senior Systems & AI Engineer)  
**Target**: Windows 11 (x86_64) — `host-cpp` (C++20) + `daemon-rust` (Rust Edition 2021)  
**Rule**: Execute strictly ONE ticket at a time. STOP after each ticket for CEO audit and sign-off.

---

## Phase 0: Ground-Truth Reconnaissance (Read-Only)

### TICKET-00: Full Codebase & Toolchain Audit Snapshot
- **Objective**: Establish verified ground truth before touching any code.
- **Tasks**:
  1. Print the complete directory tree for `host-cpp/`, `daemon-rust/`, and `scripts/` (excluding `target/`, `build/`, model binaries).
  2. Inspect and summarize the exact current implementation (with file paths and line numbers) of:
     - Named Pipe server/client creation flags, DACL/security attributes, and `JRVS` frame codec (`codec.rs` + C++ IPC client).
     - Job Object spawning sequence in `host-cpp/src/main.cpp`.
     - SPSC ring buffer memory orderings and cache alignment (`include/audio/ring_buffer.hpp`).
     - WASAPI initialization flags, sample-rate handling, and device disconnect logic (`src/audio/wasapi_capture.cpp`).
     - `WH_KEYBOARD_LL` thread ownership and callback work (`src/main.cpp`).
     - `rusqlite` and `os_control.rs` COM threading model inside Tokio.
     - `security/policy.rs`, `security/sandbox.rs`, `agent/gemini_client.rs`, and `tools/browser_cdp.rs`.
  3. Report installed toolchain versions (`rustc`, `cargo`, `cmake`, `cl.exe`/`g++`) and verify whether the baseline project currently builds cleanly.
- **Exit Gate**: Read-only report delivered to CEO. Zero source files modified.

---

## Phase 1: P0 Security & Stability Hardening (Daily-Driver Gate)

### TICKET-01: Harden Named Pipe ACLs & Upgrade `JRVS` Frame Codec (v2)
- **Objective**: Eliminate local pipe squatting, unprivileged local MITM, OOM crashes, and control-plane Head-of-Line blocking.
- **Tasks**:
  1. **Named Pipe Server (`daemon-rust`)**:
     - Create pipe with `FILE_FLAG_FIRST_PIPE_INSTANCE | PIPE_REJECT_REMOTE_CLIENTS`.
     - Apply an explicit SDDL DACL restricted exclusively to the current user's Logon SID (deny `Everyone`, `Anonymous`, and `FILE_CREATE_PIPE_INSTANCE`).
     - On client connect, verify `GetNamedPipeClientProcessId` matches the expected parent `jarvis-host.exe` PID (passed via secure startup env/arg or verified parent PID chain).
  2. **Codec Hardening (`daemon-rust` + `host-cpp`)**:
     - Enforce strict per-message-type maximum payload limits BEFORE allocating buffers (e.g., 256 KiB control JSON, 64 KiB audio frame). Disconnect and log on violation.
     - Add protocol version and `turn_id` correlation field to the binary header (or remove redundant raw PCM streaming from the control pipe if STT stays 100% local in `host-cpp`).
  3. Add unit tests for malformed magic bytes, oversized length prefixes (`0xFFFFFFFF`), truncated headers, and partial reads.
- **Exit Gate**: Rust + C++ compile cleanly; codec fuzz/unit tests pass; verified pipe rejects unauthorized second instance.

### TICKET-02: Fix Tokio Executor Starvation & Crash-Safe Lifecycle
- **Objective**: Prevent async deadlocks, SQLite corruption on hard kill, and daemon orphan races.
- **Tasks**:
  1. **SQLite Durability & Threading (`daemon-rust/src/memory/db.rs`)**:
     - Enable `PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;`.
     - Isolate all `rusqlite` operations behind a dedicated single-writer background thread with bounded `tokio::sync::mpsc` request/response channels (zero synchronous DB calls on Tokio workers).
  2. **Win32 COM Isolation (`daemon-rust/src/tools/os_control.rs`)**:
     - Move all COM / audio endpoint / `SendInput` calls onto a dedicated STA OS thread communicating via bounded channels.
  3. **Job Object Race Fix (`host-cpp/src/main.cpp`)**:
     - Spawn `jarvis-daemon.exe` with `CREATE_SUSPENDED` and `bInheritHandles = FALSE`.
     - Assign to the Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), then call `ResumeThread`.
     - On normal host exit, send a graceful `Shutdown` IPC frame with a 2-second drain window before closing the Job Object handle.
- **Exit Gate**: Verified clean startup, graceful shutdown, and forced `taskkill /F` on `jarvis-host.exe` leaves zero orphaned `jarvis-daemon.exe` and intact SQLite WAL database.

### TICKET-03: Bulletproof Windows Path Sandbox & Zero-Shell Execution
- **Objective**: Close NTFS junction escapes, TOCTOU path races, secret file leaks, and shell command injection.
- **Tasks**:
  1. **Handle-Based Path Containment (`daemon-rust/src/security/policy.rs`)**:
     - Canonicalize `allowed_directories` once at startup into normalized verbatim paths.
     - Validate file access by opening target handles safely, inspecting reparse points, and verifying `GetFinalPathNameByHandleW` on the open handle remains strictly inside an allowed root.
     - Reject NTFS Alternate Data Streams (`:` outside drive letter), reserved Win32 device names (`CON`, `NUL`, `PRN`, `\\.\`), and sensitive secret patterns (`.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `.aws/`, `.ssh/`, `.git/config`) even inside `allowed_directories`.
  2. **Eliminate Shell Interpolation (`daemon-rust/src/security/sandbox.rs`)**:
     - Remove all `cmd.exe /C` and `powershell.exe -Command` execution paths from agent tools.
     - Execute strictly allowlisted binaries via direct `std::process::Command::new(resolved_abs_path)` with explicit `argv` vectors and argument-injection guards (e.g., disallow dangerous flags like `git -c`).
  3. Write adversarial Windows unit tests covering `..\`, NTFS junctions, 8.3 paths, ADS, `.env` reads, and `& | ; $(...)` injection strings.
- **Exit Gate**: All adversarial security tests pass with zero bypasses.

### TICKET-04: Taint-Aware Policy Engine & ReAct Guardrails
- **Objective**: Defeat indirect prompt injection and Tier-2 read-plus-exfiltrate chains.
- **Tasks**:
  1. **Cryptographic Nonce Delimiters (`daemon-rust/src/agent/gemini_client.rs`)**:
     - Generate a fresh random hex nonce per request (`<untrusted_data_{nonce}>...</untrusted_data_{nonce}>`) and strip/escape any collision inside untrusted payloads.
  2. **Session Taint Tracking (`daemon-rust/src/security/policy.rs` + `agent/`)**:
     - Add a `context_tainted: bool` flag per agent turn/session. Set `true` whenever external web content, browser DOMs, or untrusted file contents enter the LLM context.
     - While `context_tainted == true`, automatically escalate ALL state-mutating tools, file writes, clipboard writes, and outbound network/browser navigations to **Tier 3 (Interactive HUD Confirmation)** showing the exact target URL/path/action.
  3. **ReAct Loop Resilience**:
     - Add hierarchical `tokio_util::sync::CancellationToken` for instant barge-in cancellation across HTTP streams and running tools.
     - Add duplicate tool-call loop detection, per-tool execution timeouts, and tool output byte caps (e.g., 16 KiB max injected into context).
- **Exit Gate**: Simulated indirect prompt injection attempting `.env` read + browser exfiltration is blocked/escalated by automated tests.

### TICKET-05: Isolate Chrome CDP Sandbox
- **Objective**: Eliminate localhost CDP hijacking of the user's real browser sessions and cross-origin pivoting.
- **Tasks**:
  1. Replace `--remote-debugging-port` attachment to the user's primary Chrome profile with a dedicated isolated automation Chrome instance using an ephemeral `--user-data-dir`.
  2. Switch transport from localhost TCP WebSocket to `--remote-debugging-pipe` (anonymous OS pipes owned strictly by `daemon-rust`), eliminating localhost port exposure entirely.
  3. Enforce a strict CDP method allowlist (e.g., `Page.navigate`, `Runtime.evaluate` scoped to active automation tab only, `Accessibility.getFullAXTree`) and explicitly deny `Network.getAllCookies`, `Storage.*`, and cross-target enumeration.
- **Exit Gate**: Verified automation works in isolated Chrome profile with zero open localhost debugging TCP ports (`netstat` check).

---

## Phase 2: P1 Real-Time Audio & Latency Hardening

### TICKET-06: Lock-Free SPSC Verification & WASAPI Hot-Swap Resilience
- **Objective**: Guarantee glitch-free real-time capture across consumer Windows 11 audio devices.
- **Tasks**:
  1. Audit and harden `ring_buffer.hpp`: pad `write_idx` and `read_idx` to separate 64-byte cache lines (`alignas(64)`), verify `acquire`/`release`/`relaxed` memory orderings, add dropped-frame atomic counters, and ensure zero locks/allocations in producer/consumer paths.
  2. Update `wasapi_capture.cpp`:
     - Pass `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY` in Shared Mode (or integrate explicit low-latency resampling) so 44.1kHz/48kHz devices cleanly yield 16kHz mono PCM.
     - Handle `AUDCLNT_BUFFERFLAGS_SILENT` and `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY`.
     - Implement `IMMNotificationClient` + automatic recovery on `AUDCLNT_E_DEVICE_INVALIDATED` (Bluetooth HFP switch / USB unplug).
  3. Verify `MMCSS` thread performs zero blocking IPC, mutex locks, or disk logging.
- **Exit Gate**: C++ stress test on SPSC ring buffer passes; unplugging/switching default audio device recovers cleanly without crash or deadlock.

### TICKET-07: Acoustic Echo Cancellation (AEC) & Barge-In Protection
- **Objective**: Stop Jarvis from hearing and transcribing its own Piper TTS output on desktop speakers.
- **Tasks**:
  1. Integrate WebRTC Audio Processing (`AEC3`) or Windows Voice Capture DSP (`CLSID_CWMAudioAEC`) fed with the Piper TTS render reference buffer.
  2. Implement speaking-state gating/barge-in detection: when user voice exceeds clean post-AEC VAD threshold during `Speaking`, immediately stop Piper playback and send `BargeInCancel` over IPC to trigger the Rust `CancellationToken`.
- **Exit Gate**: Desktop speaker playback test confirms zero self-triggering loops and clean user barge-in interruption.

### TICKET-08: Decouple Global Hotkey Hook from HUD Thread
- **Objective**: Eliminate silent Windows `LowLevelHooksTimeout` unhooking and UI stalls.
- **Tasks**:
  1. Migrate Push-To-Talk / toggle hotkeys to Windows **Raw Input** (`RegisterRawInputDevices` with `RIDEV_INPUTSINK`) if key-swallowing is not required, OR isolate `WH_KEYBOARD_LL` onto a dedicated zero-work thread that only writes to a lock-free atomic/event and returns `CallNextHookEx` immediately.
  2. Document UIPI limitation over elevated Admin windows in HUD tooltip/docs.
- **Exit Gate**: Artificial 1500ms UI thread stall test does not drop or unhook PTT hotkeys.

### TICKET-09: Per-Stage Latency Telemetry & Streaming TTS Pipeline
- **Objective**: Replace aspirational latency claims with measured SLOs and cut voice-to-voice turnaround.
- **Tasks**:
  1. Instrument monotonic `QueryPerformanceCounter` / `std::time::Instant` stage timestamps per `turn_id`: `t_ptt_up`, `t_vad_end`, `t_stt_done`, `t_ipc_rx`, `t_router_done`, `t_llm_ttft`, `t_tts_first_audio`. Log p50/p95 summary metrics locally.
  2. Stream Gemini SSE text chunks into sentence-boundary buffers and dispatch incremental TTS frames to Piper so audio playback begins on the first completed clause rather than waiting for full LLM completion.
  3. Define explicit, deterministic routing criteria between `fast_model` and `heavy_model` (documented and unit-tested).
- **Exit Gate**: Telemetry logs show measured p50/p95 breakdown across fast-path and cloud-path turns; streaming TTS begins playback before full LLM response completes.

---

## Phase 3: Final Verification & Production Gate

### TICKET-10: End-to-End Security & Build Verification Suite
- **Objective**: Lock in all P0/P1 gains against regressions.
- **Tasks**:
  1. Verify `scripts/build.ps1` produces a clean release bundle for both `host-cpp` and `daemon-rust`.
  2. Run full Rust test suite (`cargo test --workspace`) + C++ unit/stress tests.
  3. Run a checklist verification of all 10 audit vulnerabilities (pipe ACL, CDP isolation, path sandbox, shell ban, taint escalation, WAL crash safety, WASAPI hot-swap, AEC, hook isolation, latency metrics).
- **Exit Gate**: CEO final sign-off for **Beta Release Candidate** (`v0.9.0-beta-rc1`).

### TICKET-11: Pre-Distribution Polish & Full State Isolation
- **Objective**: Close pre-distribution operational gaps and enforce complete runtime state isolation.
- **Tasks**:
  1. Complete runtime state isolation via `--data-dir <path>` across `daemon-rust` (`RuntimePaths`, SQLite WAL, profile mirror, telemetry logs, ephemeral CDP profiles) and `host-cpp` pass-through.
  2. Implement mandatory supply-chain model pinning via `manifest.sha256` and enforce cryptographic SHA-256 validation in `scripts/verify-beta.ps1` and `scripts/setup-models.ps1`.
  3. Upgrade verification suite `scripts/verify-beta.ps1` with isolated temporary smoke directory, true host `taskkill /F` Job Object cascade termination asserting zero orphan daemons, and WAL integrity check on the killed smoke database.
- **Exit Gate**: 100% pass across all 5 verification phases of `scripts/verify-beta.ps1` with zero mutations to the user's live profile.
