---
name: cpp-engineering
description: >-
  C++20 real-time audio standards, MMCSS thread purity, lock-free SPSC ring buffers, WASAPI resilience, and UCRT64 compilation for Better_Jarvis host-cpp. Use when modifying or reviewing C++ code.
---

# C++ Engineering Standards & Procedures (`cpp-engineering`)

## 1. Purpose
Defines engineering standards, real-time audio thread constraints, lock-free memory orderings, WASAPI lifecycle management, and compilation standards for `host-cpp` in Better_Jarvis.

## 2. Activation Criteria
- **When to Use:**
  - Writing or modifying any code under `host-cpp/src/`, `host-cpp/include/`, or `host-cpp/tests/`.
  - Investigating audio glitches, buffer overruns, device hot-swap issues, or C++ compiler diagnostics.
- **When NOT to Use:**
  - When editing Rust daemon code or Python scripts.

## 3. Preconditions
- MinGW-w64 UCRT64 `g++` available at `C:\msys64\ucrt64\bin\g++.exe`.
- Inference runtime libraries (`onnxruntime`, `whisper`, `espeak-ng`) verified in `C:\msys64\ucrt64`.

## 4. Required Knowledge
- C++20 Real-Time Audio Laws: [AGENTS.md](../../../AGENTS.md#3-c20-real-time-audio--win32-laws-host-cpp)
- Modern C++ Idioms: [modern-cpp](../modern-cpp/SKILL.md)
- SPSC Ring Buffer & WASAPI: [ADR-006](../../docs/adrs/ADR-006_lock_free_spsc_ring_buffer_and_wasapi_resilience.md)
- Acoustic Echo Cancellation: [ADR-007](../../docs/adrs/ADR-007_acoustic_echo_cancellation_and_barge_in_teardown.md)
- C/C++ Static Review: [c-review](../c-review/SKILL.md)

## 5. Non-Negotiable C++ Real-Time Laws

### 5.1 Real-Time Audio Thread Purity (MMCSS `Pro Audio`)
Any thread registered with Multimedia Class Scheduler Service (MMCSS) under `Pro Audio` must execute:
- **ZERO Heap Allocations:** Pre-allocate all buffers, vectors, and state during startup.
- **ZERO Mutex Locks:** Never acquire a `std::mutex`, critical section, or SRWLock on the audio thread.
- **ZERO Synchronous Pipe or Disk I/O:** Never call synchronous pipe writes, `printf`, `std::cout`, or disk file logging.
- Transfer audio strictly through lock-free SPSC circular ring buffers.

### 5.2 Lock-Free SPSC Ring Buffer Hygiene
- Buffer producer (`write_idx`) and consumer (`read_idx`) indices must sit on separate 64-byte cache lines:
  ```cpp
  alignas(64) std::atomic<size_t> write_idx_{0};
  alignas(64) std::atomic<size_t> read_idx_{0};
  ```
- Use explicit atomic memory orderings:
  - Producer writes data, then updates `write_idx_.store(next, std::memory_order_release)`.
  - Consumer loads `write_idx_.load(std::memory_order_acquire)`, reads data, then updates `read_idx_.store(next, std::memory_order_release)`.
  - Thread-local reads use `std::memory_order_relaxed`.
  - NEVER use default sequentially consistent (`seq_cst`) atomics in the audio inner loop.

### 5.3 WASAPI Resilience & Hot-Swap
- Always pass `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY` in Shared Mode.
- Handle `AUDCLNT_BUFFERFLAGS_SILENT` (zero buffer data) and `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY` (log glitch counter).
- Implement `IMMNotificationClient` to handle `AUDCLNT_E_DEVICE_INVALIDATED` when Bluetooth headsets or USB mics disconnect, re-initializing the stream cleanly without crashing.

### 5.4 Input Hook Decoupling
- Never run UI rendering, pipe writes, or blocking calls inside a `WH_KEYBOARD_LL` callback.
- Windows enforces a ~1000ms silent unhook timeout. Prefer Windows Raw Input or an isolated zero-work hook thread that writes to an atomic flag and calls `CallNextHookEx` immediately.

## 6. Step-by-Step Implementation & Verification Procedure

1. **Inspect Target Files:** Read declarations in `host-cpp/include/**/*.hpp` before modifying implementation in `host-cpp/src/**/*.cpp`.
2. **Apply Minimal Edits:** Maintain strict C++20 standards (`std::span`, `std::unique_ptr`, `constexpr`, no raw owning pointers).
3. **Compile Test Suite:**
   Compile the relevant test using strict compiler flags:
   ```powershell
   $ucrtBin = "C:\msys64\ucrt64\bin"
   $env:PATH = "$ucrtBin;$env:PATH"
   g++ -std=c++20 -O2 -Wall -Wextra -Werror -DJARVIS_HAS_ONNXRUNTIME=1 -DJARVIS_HAS_WHISPER=1 -DJARVIS_HAS_PIPER=1 host-cpp/tests/test_frame_codec.cpp -I host-cpp/include -o build/tests/test_frame_codec.exe
   ```
4. **Execute Test:**
   ```powershell
   & build/tests/test_frame_codec.exe
   ```
5. **Verify Full Release Build:** Run `& scripts/build.ps1`.

## 7. Relevant Validation Commands
- Build release host: `& scripts/build.ps1`
- Run all C++ tests: Stage 4 in `& scripts/verify-beta.ps1`
- Audit for low-level hooks: `Get-ChildItem -Path host-cpp/src -Recurse -Include "*.cpp","*.hpp" | Select-String 'SetWindowsHookEx|WH_KEYBOARD_LL'`

## 8. Expected Evidence
- Compilation succeeds with zero warnings under `-Wall -Wextra -Werror`.
- Executable exit code is 0 with all test assertions passing.

## 9. Failure-Handling Guidance
- **Undispatched Audio Glitches:** Inspect atomic counters for dropped frames in `ring_buffer.hpp`.
- **Linking Errors:** Ensure all 27 UCRT64 runtime DLLs are present in `build/host-cpp/Release/` and `C:\msys64\ucrt64\lib` is in the link path.

## 10. Completion Criteria
Changes satisfy all real-time audio thread constraints, compile without warnings under strict C++20 flags, and pass all corresponding test suites in `host-cpp/tests/`.

## 11. Related Skills
- [modern-cpp](../modern-cpp/SKILL.md)
- [c-review](../c-review/SKILL.md)
- [performance-engineering](../performance-engineering/SKILL.md)
- [tdd-workflow](../tdd-workflow/SKILL.md)
