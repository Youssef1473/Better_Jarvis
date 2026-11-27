# ADR-006: Lock-Free SPSC Ring Buffer Hygiene & WASAPI Resilience
**Status:** Accepted  
**Date:** 2026-11-21  
**Context:** TICKET-06 (Hardening & Production Execution Plan)  
**Governed Subsystems:** `host-cpp/include/audio/ring_buffer.hpp`, `host-cpp/src/audio/wasapi_capture.cpp`

---

## 1. Context & Problem Statement
High-frequency consumer audio devices frequently experience buffer underruns, device hot-swap disconnections (e.g. Bluetooth HFP profile switching, USB mic replugging), and atomic false sharing between capture and processing threads.

## 2. Decision
1. **Cache-Line Alignment:** Separate producer `write_pos_` and consumer `read_pos_` onto distinct 64-byte CPU cache lines using `alignas(64)`.
2. **Explicit Atomic Ordering:** Use `memory_order_release` when updating write indices, `memory_order_acquire` when reading across threads, and `memory_order_relaxed` for thread-local reads.
3. **Power-of-2 Sizing:** Size buffer to 32,768 float samples (2.048s @ 16 kHz) with bitwise masking (`idx & (Capacity - 1)`) eliminating expensive modulo division on MMCSS audio threads.
4. **WASAPI Resilience:** Pass `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY`. Implement `IMMNotificationClient` to re-initialize the audio stream automatically on `AUDCLNT_E_DEVICE_INVALIDATED`.

## 3. Consequences
- **Positive:** Zero audio glitches, zero cache contention across CPU cores, and seamless recovery upon device hot-swap.
- **Trade-off:** Pre-allocates buffer memory at startup.

## 4. Verification Evidence
- `host-cpp/tests/test_ring_buffer.cpp` and `test_resampler.cpp` pass cleanly.
