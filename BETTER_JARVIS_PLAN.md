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

---

## Phase 3: v1.0 Production Architecture & Native Neural Inference

### TICKET-12: Native Neural Inference Integration & Host Telemetry Persistence
- **Objective**: Replace build-safe stubs with true on-device neural inference across VAD, STT, and TTS; persist host QPC latency metrics.
- **Tasks**:
  1. Link native UCRT64 `onnxruntime`, `whisper.cpp`, and `espeak-ng` in `host-cpp/CMakeLists.txt` and define `JARVIS_HAS_ONNXRUNTIME=1`, `JARVIS_HAS_WHISPER=1`, `JARVIS_HAS_PIPER=1` unconditionally for Release builds.
  2. Wire `silero_vad.cpp` to run `models/silero_vad.onnx` via `Ort::Session` on CPU with dynamic tensor shape verification and real speech probability evaluation.
  3. Wire `whisper_stt.cpp` to run `models/ggml-base.en.bin` via `whisper.cpp` (bounded to 4 CPU threads, modular ggml backends), transcribe post-AEC utterances locally, and send `speech.transcribed` JSON-RPC over IPC.
  4. Wire `piper_tts.cpp` to phonemize text via `espeak-ng` and synthesize neural audio via `Ort::Session` on `models/en_US-ryan-medium.onnx` + `.json`, feeding WASAPI render and far-end AEC reference buffer with interruptible sub-20ms epoch abort.
  5. Asynchronously persist host QPC edge timings (`vad_to_stt_ms`, `stt_to_ipc_ms`, `ipc_to_first_audio_ms`, `end_to_end_ttfa_ms`) to `<data_dir>\logs\host_latency_metrics.jsonl`.
- **Exit Gate**: Real ONNX/Whisper/Piper inference runs locally with live weights, zero stub banners appear in Release runtime, and all 9 C++ test suites + 96 Rust tests pass.

### TICKET-13: Dynamic AEC Acoustic Delay Auto-Calibration & Audio Tuning
- **Objective**: Dynamically determine physical DAC-to-ADC acoustic echo delay per-device, expand reference history, auto-calibrate NLMS filter delay, and probe opportunistic `IAudioClient3` low-latency periods.
- **Tasks**:
  1. Expand AEC reference history window from 2,048 (128 ms) to 4,096 (256 ms @ 16 kHz) with `SetBulkDelaySamples` dynamically clamped to `[0, 3200]`, auto-zeroing NLMS weights if shift exceeds 32 samples (2 ms).
  2. Implement zero-allocation `AecDelayEstimator` running on non-MMCSS `VadWorkerLoop` using 4x decimated Normalized Cross-Correlation (NCC) across `[0, 200 ms]` evaluated every 200 ms, with 3-consecutive estimate hysteresis lock, 4 ms (64-sample) causal safety lead, and device hot-swap auto-recalibration.
  3. Probe opportunistic Windows 11 `IAudioClient3::InitializeSharedAudioStream` in `WasapiCapture` with `min_period` clamped to `>= 2 ms` floor, seamlessly falling back to `IAudioClient::Initialize` on unsupported configurations.
  4. Expand `host-cpp/tests/test_aec.cpp` with multi-delay auto-calibration convergence (15 ms, 55 ms, 120 ms; ERLE >= 12 dB), double-talk lock preservation, simulated hot-swap recalibration (20 ms -> 95 ms), and CPU budget (< 0.5 ms per 10 ms frame).
- **Exit Gate**: `scripts/verify-beta.ps1` passes all 5 gates with 100% green tests across Rust and C++, showing sub-0.5ms frame timing and >= 12 dB ERLE across all delays.

### TICKET-14: Local Semantic Memory via sqlite-vec & Local Embeddings
- **Objective**: Embed `sqlite-vec` extension and local ONNX embedding model for sub-5ms local fact recall, hybrid Reciprocal Rank Fusion, and stored prompt-injection defense.
- **Tasks**:
  1. Provision `all-MiniLM-L6-v2-q8.onnx` (23.0 MB) and `all-MiniLM-L6-v2-tokenizer.json` (711 KB) pinned in `manifest.sha256` and automated in `scripts/setup-models.ps1`.
  2. Implement `LocalEmbedder` on dedicated background thread `jarvis-embed-actor` with bounded channel (`capacity = 32`), dynamic `onnxruntime.dll` loading with System32 collision avoidance, tokenization, mean pooling over non-padded tokens, and unit L2 normalization to `[f32; 384]`.
  3. Register `sqlite-vec` static extension across all SQLite connections via `sqlite3_auto_extension`. Create virtual tables `notes_vec` and `facts_vec` using `vec0(embedding float[384] distance_metric=cosine)`, and `notes_fts` using `fts5`.
  4. Implement hybrid Reciprocal Rank Fusion (`hybrid_search_memory`) merging `sqlite-vec` cosine KNN and FTS5 BM25 with $k = 60$.
  5. Enforce Stored Prompt-Injection Defense: track `source_trusted` provenance across facts and notes; query-scoped recall in `retrieve_context_for_turn` wraps untrusted notes in active per-turn CNG nonce delimiters and immediately triggers `TurnTaintState::mark_tainted("memory:untrusted_note")` for dynamic model escalation and Tier 3 mutation gates. Expose `search_memory` tool for explicit queries.
- **Exit Gate**: `scripts/verify-beta.ps1` passes all 5 gates (101/101 Rust tests, 0 Clippy warnings under `-D warnings`, 9/9 C++ test suites, and live binary smoke tests).

### TICKET-15: Windows Visual Context & Desktop OCR
- **Objective**: Integrate active window inspection and Windows OCR for multimodal desktop grounding with fail-closed privacy guards and prompt injection defenses.
- **Implementation**:
  1. Implemented `VisionPrivacyGuard` in `daemon-rust/src/tools/screen_vision.rs` hard-rejecting password managers (`1password`, `bitwarden`, `keepass`, `keepassxc`, `lastpass`, `dashlane`, `enpass`, `protonpass`, `credentialuibroker`, `consent`, `lockapp`, `logonui`, `taskmgr`, `mmc`), sensitive window title substrings (`inprivate`, `incognito`, `password`, `vault`, `sign in`, `2fa`, `bank`, `credit card`, etc.), non-zero display affinity (`WDA_MONITOR`, `WDA_EXCLUDEFROMCAPTURE`), and self (`jarvis-host`, `jarvis-daemon`) via Z-order walking.
  2. Built in-memory active-window capture using Win32 GDI `PrintWindow` with `PW_RENDERFULLCONTENT` to 32-bit BGRA bytes with zero disk writes.
  3. Integrated Windows WinRT `Windows.Media.Ocr.OcrEngine` via Microsoft `windows` crate (v0.58) transferring BGRA bytes directly to in-memory `SoftwareBitmap` via `DataWriter` / `IBuffer`, with text extraction and 16 KiB UTF-8 truncation (`truncate_utf8_16kib`).
  4. Added `VisionHudGuard` sending `hud.vision_capture` (`active: true`/`false`) via Named Pipe to `host-cpp/src/main.cpp`, rendering a glowing purple vision indicator dot on the HUD pill.
  5. Wired agent tools `inspect_active_window` and `read_active_window_ocr` in `gemini_client.rs`, wrapping outputs in active turn cryptographic CNG nonces and setting `TurnTaintState::mark_tainted` (`"vision:window_title"`, `"vision:ocr"`), automatically routing next turn to `cloud_heavy` and escalating all state mutations to Tier 3 Amber user confirmation.
  6. Closed Step 0 Polish: configured `ort` session with 2 intra-op threads, 1 inter-op thread, startup arena warmup (slashing warm embedding latency from 134.52 ms to 11.74 ms), and serialized `memory_recall_ms` to `latency_metrics.jsonl`.
- **Exit Gate**: `scripts/verify-beta.ps1` passes all 5 gates (106/106 Rust tests, 0 Clippy warnings under `-D warnings`, 9/9 C++ test suites, and live binary smoke tests with 4 resident neural models at 448.4 MB total RAM).

### TICKET-16: Production Distribution, Standalone Packaging & Zero-Telemetry Diagnostics
- **Objective**: Produce standalone zero-dependency distribution layout with zero-telemetry crash minidumps, panic logging, Task Scheduler autostart, and clean-PATH verification.
- **Implementation**:
  1. Implemented native C++ crash handler in `host-cpp/include/diagnostics/crash_handler.hpp` and `host-cpp/src/diagnostics/crash_handler.cpp` writing `MDMP` minidumps via `MiniDumpWriteDump` (`MiniDumpNormal | MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules`) to `<data_dir>\logs\crashes` with strict 5-file FIFO rotation and zero telemetry upload.
  2. Implemented Rust daemon panic hook in `daemon-rust/src/diagnostics.rs` intercepting panics, scrubbing API keys/bearer tokens/nonces, writing formatted crash logs, and pruning to 5 files.
  3. Created `scripts/package-release.ps1` staging standalone distribution layout in `dist/Better_Jarvis-v1.0.0-win64/` with stripped binaries, all 27 runtime DLLs, `espeak-ng-data`, configs, and BOM-free `dist/SHA256SUMS.txt`. Output: `dist/Better_Jarvis-v1.0.0-win64.zip` (59.16 MB).
  4. Authored installer manifests `packaging/better_jarvis.iss` (`PrivilegesRequired=lowest`) and `packaging/better_jarvis.wxs` (`Scope="perUser"`), plus `scripts/install-startup.ps1` (`-RunLevel Limited`) and `scripts/uninstall-startup.ps1`.
  5. Implemented `scripts/run-desktop.py` binding `winsta0\default` window station for zero-`0xC0000142` test and process supervision execution.
  6. Implemented `scripts/verify-v1.ps1` verifying all 6 production gates.

### TICKET-16.1: P0 Least-Privilege Hotfix & Hygiene
- **Objective**: Guarantee zero privilege elevation on user logon autostart, enforce strict static audit gates, and clean repo hygiene.
- **Tasks**:
  1. Enforce strict `RunLevel = Limited` (Medium Integrity Level) in `scripts/install-startup.ps1` and verify zero elevation in `packaging/better_jarvis.iss` and `packaging/better_jarvis.wxs`.
  2. Add static gate in `scripts/verify-v1.ps1` Phase [2/6] failing on any occurrence of `HighestAvailable`, `-RunLevel Highest`, or `PrivilegesRequired=admin`.
  3. Add `dist/` and `scripts/__pycache__/` to `.gitignore`. Remove unneeded `scripts/run-cargo.py`.
  4. Verify Rust panic secret-scrubbing test `diagnostics::tests::test_secret_scrubbing`.
- **Exit Gate**: `scripts/verify-v1.ps1` passes all 6 production gates end-to-end.

### TICKET-16.2: Audio PTT Onset, Watchdog & Device Selection (`77d6d4c`)
- **Objective**: Eliminate startup audio feedback loops, support explicit audio device selection, and guard against stuck listening states.
- **Implementation**:
  1. Implemented silent PTT onset with 300 ms `PeekRecent` pre-roll, PTT release duration/RMS gating (`>= 250 ms`, `RMS >= 0.0001`), and a 15-second listening watchdog in `host-cpp/src/main.cpp`.
  2. Added `--probe-mic`, `[audio]` config parsing, and substring-based `WasapiCapture` device selection (`--input-device` / `--output-device`).

### TICKET-17: Dual-Provider Gemini Routing, Ordered Tool Scheduler & HUD Watchdog (`172ed2c`)
- **Objective**: Support Vertex AI Enterprise + AI Studio fallback routing, parallel read-only tool execution, and a pure-evaluated thinking watchdog.
- **Implementation**:
  1. Added `fallback_model`, per-tier `max_output_tokens_*`, and `[enterprise]` ADC/Vertex AI routing in `daemon-rust/src/config.rs`, `model_router.rs`, and `gemini_client.rs`.
  2. Implemented ordered parallel/serialized tool scheduler (`ToolExecutionClass`, `execute_parallel_tool_calls`) preserving `thoughtSignature` and taint propagation.
  3. Extracted `HudState` (`host-cpp/include/ui/hud_state.hpp`) and `EvaluateThinkingWatchdog` (`host-cpp/include/ui/hud_watchdog.hpp`) with unit tests in `host-cpp/tests/test_hud_watchdog.cpp`.

---

## Phase 4: Stabilization, Security Edge Closure & Modularization

### TICKET-18: P0 Concurrency, Streaming TTS Finalization & Blocking-Voice-Loop Fix
- **Objective**: Eliminate cross-thread HUD races/GDI leaks, prevent TTS synthesis and cloud audio transcription from blocking IPC/barge-in, and fix stuck `Thinking` HUD states.
- **Tasks**:
  1. **`host-cpp/src/main.cpp` & UI Watchdog (`host-cpp/include/ui/hud_watchdog.hpp`)**:
     - Guard `g_hud_label` and HUD state updates with a mutex/thread-safe helper; restore prior `HGDIOBJ` via `SelectObject` before `DeleteObject` in `WM_PAINT`.
     - Offload `g_tts->Speak(text)` from `IpcWorkerLoop` onto a dedicated cancellable/turn-filtered TTS synthesis worker thread so `IpcWorkerLoop` never blocks on ONNX/eSpeak.
     - Anchor `EvaluateThinkingWatchdog` to a dedicated `g_thinking_start_time` updated on transitions into `HudState::Thinking`; suppress `VadWorkerLoop` utterance triggers while `g_ptt_active` is true; assign `ptt_turn` before calling `RecordPttUp`; join `ipc_thread` cleanly on exit instead of detaching.
  2. **`daemon-rust/src/main.rs`, `gemini_client.rs`, `diagnostics.rs`, `telemetry.rs`**:
     - Move `MSG_TYPE_AUDIO_PCM` cloud transcription off `handle_client`'s `select!` loop into a `tokio::spawn` task selectable against `turn_token.cancelled()`, and emit `tts.speak_chunk` (`is_final: true`, empty text) on empty/failed transcription or empty LLM reply so the host HUD never stalls in `Thinking`.
     - Fix `parse_sse_bytes_stream_full` to finalize emitted pre-tool text chunks cleanly and prevent unfinalized TTS streams on tool turns.
     - Replace `.lock().unwrap()` with poison-recovering locking (`unwrap_or_else(|e| e.into_inner())`) in `diagnostics.rs`, `telemetry.rs`, and `main.rs`.
- **Exit Gate**: `cargo test --manifest-path daemon-rust/Cargo.toml`, `cargo clippy --manifest-path daemon-rust/Cargo.toml --all-targets -- -D warnings`, and `host-cpp` release + C++ unit tests pass cleanly with new regression tests covering TTS finalization, thinking-start watchdog tracking, and non-blocking turn cancellation.

### TICKET-19: Security Edge-Case Closure (`browser_cdp`, `os_control`, `sandbox`)
- **Objective**: Remove remaining fail-open fallbacks and TOCTOU/SSRF gaps in tool execution.
- **Tasks**:
  1. Remove `explorer.exe <url>` fallback in `daemon-rust/src/tools/browser_cdp.rs` when Chromium/CDP fails; harden `validate_navigation_url` against RFC1918, hex/int IPs, and `user@host` userinfo bypasses.
  2. Replace bare executable names (`code.cmd`, `chrome.exe`) in `os_control.rs` with resolved absolute paths and fix `"open discord"` router/allowlist mismatch.
  3. Spawn sandboxed child processes in `daemon-rust/src/security/sandbox.rs` with `CREATE_SUSPENDED` prior to `JobObjectGuard::assign_process`, and align `policy.rs` dangerous-flag checks (`rg --pre`, `fd -x`).
- **Exit Gate**: Adversarial unit tests for SSRF URLs, `CREATE_SUSPENDED` job assignment, and zero bare-name launches pass cleanly.

### TICKET-20: Config/TOML Hardening, Audio Output Parity & Test/Packaging Alignment
- **Objective**: Eliminate silent config parsing failures, wire `--output-device` + TTS render hot-swap, and align CMake/PS1/WiX verification gates.
- **Tasks**:
  1. Replace hand-rolled TOML parsing in `daemon-rust/src/config.rs` with `toml` + `serde`, respect `[ai].provider`, fail/fall back explicitly on empty `enterprise.project_id`, replace hardcoded user paths in `config/config.toml`, and add `.env.example`.
  2. Wire `--output-device` into `PiperTts` and add `AUDCLNT_E_DEVICE_INVALIDATED` render endpoint recovery in `host-cpp/src/audio/piper_tts.cpp`.
  3. Wire `test_hud_watchdog.cpp` and `test_operational_throughput.cpp` into `scripts/verify-v1.ps1` and `host-cpp/CMakeLists.txt` (`enable_testing()`), replace `assert()` in C++ tests with an always-active `TEST_ASSERT`, fix UTF-8 BOM in `scripts/setup-models.ps1`, align `packaging/better_jarvis.wxs`, and update `.agents/docs/` references to `verify-v1.ps1`.
- **Exit Gate**: `scripts/verify-v1.ps1` passes all 6 gates including all 12 C++ test suites and updated packaging/config checks.

### TICKET-21: `gemini_client.rs` Modularization & Partially-Wired Capabilities
- **Objective**: Decompose the 6,265-line `gemini_client.rs` monolith and wire built-in security/memory/browser capabilities into the agent loop.
- **Tasks**:
  1. Split `daemon-rust/src/agent/gemini_client.rs` into focused modules (`auth_adc.rs`, `sse_parser.rs`, `tool_schemas.rs`, `tool_scheduler.rs`, `react_loop.rs`) and deduplicate `max_output_tokens_for_model` and guarded tool execution helpers.
  2. Wire `PolicyEngine::open_validated_file` as sandboxed read/list agent tools, query `facts_vec` in semantic recall, route `memory.upsert_fact` through `LocalEmbedder`, and expose `Accessibility.getFullAXTree` / `g_vision_app` HUD display.
- **Exit Gate**: Full Rust + C++ test suites pass with zero regressions and reduced module complexity.

### TICKET-22: Local Ollama Fallback Provider (`qwen3.5:2b`) on Gemini Quota/Availability Failure
- **Objective**: Keep Jarvis operational offline and under Gemini free-tier quota exhaustion (`HTTP 429` / `RESOURCE_EXHAUSTED` / missing API key / network outage) by transparently falling back to local Ollama (`qwen3.5:2b`) while preserving all tool-calling, sandboxing, taint tracking, and streaming TTS guarantees.
- **Tasks**:
  1. **`daemon-rust/src/config.rs` & `config/config.toml`**: Add `[ai.local_fallback]` (`enabled`, `provider`, `base_url`, `model = "qwen3.5:2b"`, `max_output_tokens`, `timeout_secs`), enforce strict loopback-only URL validation (`127.0.0.1` / `localhost` / `[::1]`, zero userinfo), and support `[ai] provider = "ollama"` local-first mode.
  2. **`daemon-rust/src/agent/ollama_client.rs` & `tool_schemas.rs`**: Convert `get_tool_declarations()` into Ollama/OpenAI `tools` JSON Schema without duplicating definitions, implement `/api/chat` NDJSON streaming with `<think>...</think>` stripping before `SentenceChunker`, require `"done": true` before dispatching tool calls, and reuse `tool_scheduler` ordered execution + nonced wrapping + taint propagation.
  3. **`daemon-rust/src/agent/gemini_client.rs`, `react_loop.rs`, `telemetry.rs`, `main.rs`**: Add typed fallback-eligible Gemini errors (`429`/quota, missing key, transport failure — never `SAFETY` or `CancellationToken`), wire automatic Turn-1 fallback into `chat_with_tools_streaming*`, and record `ExecutionPath::LocalFallback`.
- **Exit Gate**: `cargo test` (with simulated `HTTP 429` -> Ollama NDJSON tool-loop & loopback URL rejection tests) and `cargo clippy --all-targets -- -D warnings` pass with zero warnings, followed by independent adversarial review.
