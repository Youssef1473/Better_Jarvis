# C++ Engineering Rules & Real-Time Audio Standards
**Scope:** Applied to all files under `host-cpp/`

## 1. C++20 Standard & Idioms
- Enforce strict C++20 standard (`-std=c++20`).
- Prefer smart pointers (`std::unique_ptr`, `std::shared_ptr`) over raw owning pointers.
- Use `std::span<T>` for bounds-checked array and buffer references.
- Use `constexpr` and `const` for immutability.

## 2. Real-Time Audio Purity (MMCSS `Pro Audio`)
Any thread registered with Multimedia Class Scheduler Service under `Pro Audio` must maintain absolute purity:
- **ZERO Heap Allocations:** All buffers, state vectors, and structs must be pre-allocated.
- **ZERO Mutex Locks:** Never acquire a mutex or critical section in the audio render/capture loop.
- **ZERO Synchronous Pipe / Disk I/O:** Never call synchronous I/O, `std::cout`, or disk logging.
- Communicate with worker threads exclusively through lock-free SPSC ring buffers.

## 3. Lock-Free SPSC Hygiene
- Producer and consumer indices must sit on separate 64-byte cache lines: `alignas(64) std::atomic<size_t> write_idx_{0};`.
- Use explicit atomic orderings: `memory_order_release` when updating write indices, `memory_order_acquire` when reading across threads, `memory_order_relaxed` for thread-local reads.

## 4. WASAPI Robustness & Hook Safety
- Handle `AUDCLNT_BUFFERFLAGS_SILENT` and `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY`.
- Implement `IMMNotificationClient` to recover on `AUDCLNT_E_DEVICE_INVALIDATED`.
- Decouple keyboard hooks from UI threads to prevent Windows `LowLevelHooksTimeout` unhooking.

## 5. Verification Gates
- Clean compilation with MinGW-w64 UCRT64 `g++` under `-std=c++20 -O2 -Wall -Wextra -Werror`.
- Test suites in `host-cpp/tests/` must pass with exit code 0.
