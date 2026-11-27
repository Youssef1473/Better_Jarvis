# Better_Jarvis Engineering Failure Ledger & Landmines
**Location:** `.agents/docs/failure_ledger.md`
**Status:** Living Engineering Knowledge Base

---

## 1. Purpose
This failure ledger records verified root causes, failure signatures, and permanent engineering resolutions for critical defects encountered during the development of Better_Jarvis. Agents must consult this ledger before proposing architectural changes or debugging similar symptoms.

---

## 2. Landmine Records

### LM-01: Tokio Worker Starvation via Synchronous Disk/COM I/O
- **Symptom:** Voice pipeline stalls; async channels stop receiving; heartbeat timeouts occur during database queries or audio volume changes.
- **Root Cause:** Synchronous `rusqlite` database transactions and Win32 COM invocations executed directly on Tokio worker threads blocked the async runtime's cooperative event loop.
- **Durable Rule:** NEVER execute blocking disk I/O, SQLite queries, or Win32 COM calls on Tokio workers. Always route them through dedicated OS background threads (`jarvis-db-actor`, STA controller thread) communicating via bounded `tokio::sync::mpsc` channels (capacity <= 32).
- **Enforced By:** `daemon-rust/src/memory/db.rs`, `daemon-rust/src/tools/os_control.rs`.

### LM-02: Silent Windows Keyboard Hook Teardown (`LowLevelHooksTimeout`)
- **Symptom:** Push-to-talk hotkey stops responding after several minutes with no error logged.
- **Root Cause:** Windows enforces a silent unhook ceiling (`LowLevelHooksTimeout`, ~1000ms). Any UI stall, pipe write, or memory allocation inside a `WH_KEYBOARD_LL` callback exceeding this threshold causes the OS to silently drop the hook without notification.
- **Durable Rule:** Never execute UI logic, pipe communication, or mutex locks inside a low-level hook. Prefer Windows Raw Input (`RegisterRawInputDevices` with `RIDEV_INPUTSINK`) or isolate the hook to a zero-work thread that only toggles an atomic flag and calls `CallNextHookEx`.
- **Enforced By:** `host-cpp/src/input/hotkey_listener.cpp`, static audit in `scripts/verify-v1.ps1`.

### LM-03: SPSC Ring Buffer Cache Thrashing & Atomic Ordering
- **Symptom:** Audio frame dropouts, crackling, or intermittent data corruption between WASAPI capture thread and DSP worker.
- **Root Cause:** SPSC producer `write_idx` and consumer `read_idx` residing on the same 64-byte CPU cache line caused false sharing and cache-line invalidation across cores; implicit atomic ordering led to race conditions.
- **Durable Rule:** In real-time lock-free buffers, producer and consumer indices must be separated onto distinct cache lines using `alignas(64)`. Use strictly explicit `memory_order_release` when updating write index, `memory_order_acquire` when reading across threads, and `memory_order_relaxed` for thread-local reads.
- **Enforced By:** `host-cpp/include/audio/ring_buffer.hpp`.

### LM-04: Acoustic Echo Feedback Loop & Speaker Retranscription
- **Symptom:** Jarvis hears and transcribes its own Piper TTS speech through speakers, generating an infinite conversational loop.
- **Root Cause:** Microphone capture buffer receives acoustic echo from physical speakers. Standard VAD cannot distinguish between user voice and speaker echo without acoustic cancellation and delay matching.
- **Durable Rule:** All microphone PCM must pass through Normalized Least Mean Squares (NLMS) Acoustic Echo Cancellation fed with the TTS render reference buffer. Real-time decimation cross-correlation auto-calibrates physical DAC-to-ADC delay across `[0, 200ms]`.
- **Enforced By:** `host-cpp/src/audio/aec_processor.cpp`, `aec_delay_estimator.cpp`.

### LM-05: Windows Path Traversal via NTFS Reparse Points (Junctions)
- **Symptom:** An agent tool reading files inside `allowed_directories` escapes to the root filesystem via an NTFS directory junction or symlink.
- **Root Cause:** String-based prefix checking (`path.starts_with(root)`) checks only the lexical path, not the underlying reparse target on disk.
- **Durable Rule:** File access must be verified on an opened handle using `GetFinalPathNameByHandleW(VOLUME_NAME_DOS)` before reading/writing. Reject Alternate Data Streams and Win32 reserved device names unconditionally.
- **Enforced By:** `daemon-rust/src/security/policy.rs`.

### LM-06: ONNX Runtime Thread Contention & Startup Latency
- **Symptom:** High CPU spike across all cores during local embedding generation; warm latency exceeded 130ms.
- **Root Cause:** Unbounded default thread allocation in ONNX Runtime spawned excessive worker threads competing with real-time audio threads; memory arenas were uninitialized.
- **Durable Rule:** Configure ONNX session options explicitly: 2 intra-op threads, 1 inter-op thread, and run a synthetic warmup pass at startup to initialize memory pools (slashes warm latency to sub-12ms).
- **Enforced By:** `daemon-rust/src/memory/embedder.rs`.
