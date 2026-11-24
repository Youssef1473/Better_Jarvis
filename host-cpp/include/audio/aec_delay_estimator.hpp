#pragma once

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <vector>

namespace jarvis::audio {

/// Zero-allocation, real-time Acoustic Lag Estimator using decimated Normalized Cross-Correlation (NCC).
/// Runs on the non-MMCSS VadWorkerLoop thread (never on MMCSS audio threads).
/// Correlates far-end TTS reference against near-end raw microphone audio to auto-calibrate bulk acoustic delay.
class AecDelayEstimator {
public:
    static constexpr size_t kSampleRate        = 16000;
    static constexpr size_t kDecimation        = 4;     // 4x downsampling -> 4 kHz (0.25 ms / sample)
    static constexpr size_t kDecimatedRate    = kSampleRate / kDecimation; // 4000 Hz
    static constexpr size_t kFrameSize         = 160;   // 10 ms @ 16 kHz
    static constexpr size_t kDecimatedFrame    = kFrameSize / kDecimation; // 40 samples / 10 ms

    static constexpr size_t kHistorySize       = 2048;  // Power of 2 (512 ms history @ 4 kHz)
    static constexpr size_t kHistMask          = kHistorySize - 1; // 2047

    static constexpr size_t kMaxLagMs          = 200;   // Maximum search lag in ms
    static constexpr size_t kMaxLagSamples     = (kMaxLagMs * kDecimatedRate) / 1000; // 800 samples @ 4 kHz
    static constexpr size_t kCorrWindowSamples = 600;   // 150 ms correlation window

    static constexpr size_t kEvalIntervalFrames = 20;   // Evaluate once every 20 frames (200 ms)
    static constexpr float  kMinPeakNcc        = 0.45f; // Minimum correlation peak to qualify
    static constexpr size_t kCausalLeadSamples  = 64;    // 4 ms (64 raw samples @ 16 kHz) causal safety lead
    static constexpr size_t kConsistencyTolSamples = 32; // +/- 2 ms (32 raw samples @ 16 kHz, 8 decimated)

    AecDelayEstimator();
    ~AecDelayEstimator() = default;

    // Non-copyable, non-movable
    AecDelayEstimator(const AecDelayEstimator&) = delete;
    AecDelayEstimator& operator=(const AecDelayEstimator&) = delete;

    /// Feeds one 160-sample (10 ms) frame. Zero heap allocations.
    /// Evaluates lag estimation when speaking is active, speech RMS > floor, and not in double-talk.
    void UpdateFrame(
        const float* mic_in,
        const float* ref_in,
        bool is_speaking,
        bool is_double_talk) noexcept;

    /// Checks if a new lock event occurred. Returns true and outputs lock info, clearing the event flag.
    bool CheckAndClearJustLocked(
        size_t* out_bulk_delay,
        float* out_delay_ms,
        float* out_peak_ncc) noexcept;

    /// Resets history and calibration state (e.g. on audio device hot-swap).
    void Reset() noexcept;

    // Telemetry accessors
    bool IsDelayLocked() const noexcept { return is_locked_.load(std::memory_order_relaxed); }
    float EstimatedDelayMs() const noexcept { return estimated_delay_ms_.load(std::memory_order_relaxed); }
    float LastCorrelationPeak() const noexcept { return last_peak_ncc_.load(std::memory_order_relaxed); }
    size_t LockedBulkDelaySamples() const noexcept { return locked_bulk_delay_.load(std::memory_order_relaxed); }

private:
    void RunEstimation() noexcept;

    // Circular buffers for decimated signals (4 kHz)
    std::vector<float> mic_history_;
    std::vector<float> ref_history_;
    size_t write_idx_{0};
    size_t frame_count_{0};

    // Candidate tracking
    size_t candidate_raw_lag_{0};
    size_t consecutive_count_{0};

    // Telemetry / lock state
    alignas(64) std::atomic<bool>   is_locked_{false};
    alignas(64) std::atomic<float>  estimated_delay_ms_{0.0f};
    alignas(64) std::atomic<float>  last_peak_ncc_{0.0f};
    alignas(64) std::atomic<size_t> locked_bulk_delay_{160}; // Default 10 ms
    alignas(64) std::atomic<bool>   just_locked_{false};
};

} // namespace jarvis::audio
