# ADR-012: Dynamic AEC Acoustic Delay Auto-Calibration
**Status:** Accepted  
**Date:** 2026-11-22  
**Context:** TICKET-13 (v1.0 Production Architecture)  
**Governed Subsystems:** `host-cpp/src/audio/aec_delay_estimator.cpp`

---

## 1. Context & Problem Statement
Physical DAC-to-ADC acoustic delays vary widely across hardware devices (15ms to 150ms). Fixed delay assumptions cause NLMS filter divergence and echo leakage.

## 2. Decision
1. **Reference History Expansion:** Expand AEC reference window to 4,096 samples (256 ms @ 16 kHz) with `SetBulkDelaySamples` dynamically clamped to `[0, 3200]`.
2. **Decimated Normalized Cross-Correlation (NCC):** Implement zero-allocation `AecDelayEstimator` running on non-MMCSS `VadWorkerLoop` using 4x decimated NCC across `[0, 200ms]` evaluated every 200 ms, with 3-consecutive estimate hysteresis lock and 4 ms (64-sample) causal safety lead.

## 3. Consequences
- **Positive:** Automatic adaptation to any consumer speaker/mic setup without manual calibration.
- **Trade-off:** Minimal CPU overhead (< 0.5 ms per 10 ms frame) during background estimation.

## 4. Verification Evidence
- `host-cpp/tests/test_aec.cpp` verifies multi-delay convergence across 15ms, 55ms, 120ms with ERLE >= 12 dB.
