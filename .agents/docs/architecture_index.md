# Better_Jarvis Canonical Architecture Index
**Location:** `.agents/docs/architecture_index.md`
**System:** Better_Jarvis 1.0 (Production Windows 11 Native Voice/Desktop Assistant)

---

## 1. High-Level System Topology

```
┌────────────────────────────────────────────────────────────────────────┐
│                        WINDOWS 11 USER SESSION                         │
└────────────────────────────────────┬───────────────────────────────────┘
                                     │
           ┌─────────────────────────┴─────────────────────────┐
           │                                                   │
           ▼                                                   ▼
┌───────────────────────────────┐               ┌───────────────────────────────┐
│           HOST-CPP            │   JRVS v2     │          DAEMON-RUST          │
│   (C++20 Native Audio & UI)   │  Named Pipe   │      (Rust Edition 2021)      │
│                               │◄─────────────►│                               │
│ - WASAPI Audio Loop (MMCSS)   │ Logon SID SDDL│ - Tokio Async Runtime         │
│ - Lock-Free SPSC Ring Buffer  │ Verified PID  │ - SQLite WAL Durability       │
│ - AEC Delay Auto-Calibration  │ Frame Bounds  │ - Local Memory (sqlite-vec)   │
│ - Silero VAD (ONNX CPU)       │ Zero Port Exp │ - WinRT OCR & Vision Guard    │
│ - Whisper.cpp (Local STT)     │               │ - Isolated Chrome CDP Pipe    │
│ - Piper Neural TTS + eSpeak   │               │ - Gemini Client (ReAct+Taint) │
│ - Raw Input Hotkey Listener   │               │ - Model Router (Fast/Heavy)   │
│ - Minidump Crash Handler      │               │ - Monotonic QPC Telemetry     │
└───────────────────────────────┘               └───────────────────────────────┘
```

---

## 2. Subsystem Directory Map & Ownership

### 2.1 `host-cpp/` (Native Real-Time Audio, UI, & Local Inference)
- **`include/audio/ring_buffer.hpp`**: Lock-free SPSC circular ring buffer. Indices aligned to 64-byte cache lines (`alignas(64)`), explicit `acquire`/`release`/`relaxed` memory orderings. Zero locks, zero allocations in audio path.
- **`src/audio/wasapi_capture.cpp`**: Low-latency WASAPI capture stream. Handles `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM`, `AUDCLNT_BUFFERFLAGS_SILENT`, `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY`, and device invalidation.
- **`src/audio/aec_processor.cpp` & `aec_delay_estimator.cpp`**: Acoustic Echo Cancellation (NLMS filter) with zero-allocation 4x decimated Normalized Cross-Correlation delay auto-calibration across `[0, 200ms]`.
- **`src/audio/silero_vad.cpp`**: Silero VAD inference via `Ort::Session` on CPU. Evaluates speech probability per 32ms frame.
- **`src/audio/whisper_stt.cpp`**: Whisper.cpp local speech-to-text transcription bounded to 4 CPU threads.
- **`src/audio/piper_tts.cpp`**: Neural speech synthesis via eSpeak-ng phonemizer and ONNX Piper medium voice, feeding WASAPI render and AEC reference buffer with interruptible 20ms epoch abort.
- **`src/input/hotkey_listener.cpp`**: Push-To-Talk global hotkey capture decoupled from UI rendering to avoid Windows `LowLevelHooksTimeout` unhooking.
- **`src/diagnostics/crash_handler.cpp`**: Native unhandled exception filter writing minidumps (`MiniDumpWriteDump`) to `<data_dir>/logs/crashes` with 5-file FIFO rotation and zero telemetry.

### 2.2 `daemon-rust/` (Async Service, Security, Memory, & Tool Execution)
- **`src/ipc/`**:
  - `codec.rs`: Binary `JRVS` frame codec v2 with protocol versioning, `turn_id` correlation, and strict per-message byte bounds (256 KiB control JSON, 64 KiB audio).
  - `pipe_security.rs`: Windows Named Pipe server with `FILE_FLAG_FIRST_PIPE_INSTANCE | PIPE_REJECT_REMOTE_CLIENTS`, Logon-SID SDDL DACL, and client PID verification.
  - `reassembler.rs`: Turn utterance reassembly with hard buffer caps.
- **`src/memory/`**:
  - `db.rs`: SQLite database with `PRAGMA journal_mode=WAL; synchronous=NORMAL; foreign_keys=ON;`. All queries isolated on a dedicated single-writer actor thread with bounded channels.
  - `embedder.rs`: `LocalEmbedder` on actor thread running `all-MiniLM-L6-v2-q8.onnx` (sub-12ms warm latency, L2 unit normalized to `[f32; 384]`).
  - `db.rs` (Hybrid RRF): Merges `sqlite-vec` cosine KNN and SQLite FTS5 BM25 with $k=60$.
- **`src/security/`**:
  - `policy.rs`: Handle-based path containment via `GetFinalPathNameByHandleW`. Rejects NTFS junctions, Alternate Data Streams, reserved names (`CON`, `NUL`), and sensitive secrets (`.env`, `id_rsa`).
  - `sandbox.rs`: Zero-shell execution. Strictly allowlisted binaries spawned directly via `Command::new(abs_path)` with explicit `argv` vectors.
- **`src/tools/`**:
  - `browser_cdp.rs`: Isolated Chrome instance via ephemeral `--user-data-dir` and OS `--remote-debugging-pipe`. Method allowlist; zero localhost TCP debugging ports.
  - `screen_vision.rs`: `VisionPrivacyGuard` blocking password managers and sensitive titles; in-memory Win32 GDI capture and WinRT OCR extraction.
  - `os_control.rs`: Win32 COM, audio endpoints, and keyboard events isolated on a dedicated STA OS thread.
  - `confirmation.rs`: Interactive HUD confirmation gates for state mutations.
- **`src/agent/`**:
  - `gemini_client.rs`: Cryptographic CNG per-turn random nonce delimiters (`<untrusted_data_{nonce}>...`); `TurnTaintState` taint tracking triggering Tier 3 HUD confirmation.
  - `model_router.rs`: Deterministic routing between fast-tier model and cloud-heavy model based on prompt complexity, tools, and taint state.
  - `sentence_chunker.rs`: Incremental streaming clause chunker for low-latency TTS dispatch.
- **`src/telemetry.rs`**: Monotonic QPC per-stage latency tracking (`t_vad_end`, `t_stt_done`, `t_llm_ttft`, `t_tts_first_audio`) persisted locally.

### 2.3 `scripts/` (Build, Verification, & Packaging)
- **`scripts/build.ps1`**: Builds `daemon-rust` (`cargo build --release`) and `host-cpp` (via `g++` UCRT64 with strict C++20 flags), staging runtime DLLs and models.
- **`scripts/verify-v1.ps1`**: 6-stage automated gate enforcing release packaging, static security/least-privilege invariants, Rust tests/clippy, all 12 C++ test suites, Job Object crash-safety smoke tests, and clean-PATH launch proof.
- **`scripts/run-desktop.py`**: Python supervisor launching desktop processes under `winsta0\default` with GDI/OCR desktop binding.
- **`scripts/package-release.ps1`**: Assembles release bundle into `dist/` with cryptographic SHA-256 manifest.

---

## 3. Cross-Language IPC Interface Contract

- **Transport:** Windows Named Pipe `\\.\pipe\jarvis_ipc_<session>`
- **Framing:**
  ```
  [0..4]   Magic: 0x4A 0x52 0x56 0x53 ('J', 'R', 'V', 'S')
  [4..6]   Protocol Version: 0x0002
  [6..8]   Message Type: u16 (ControlJson = 0x01, AudioPcm = 0x02, Event = 0x03)
  [8..12]  Turn ID: u32
  [12..16] Payload Length: u32 (Little-Endian, max 262,144 bytes)
  [16..N]  Payload Bytes
  ```
- **Error Handling:** Any frame with invalid magic, unknown version, or length exceeding bounds triggers immediate socket disconnect and audit log emission.
