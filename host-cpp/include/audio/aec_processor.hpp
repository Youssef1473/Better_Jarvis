#pragma once

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <vector>

namespace jarvis::audio {

/// Zero-allocation, real-time Acoustic Echo Cancellation (AEC) processor.
/// Uses a Normalized Least Mean Squares (NLMS) adaptive FIR filter, bulk delay alignment,
/// Geigel double-talk detection (DTD), and a speaking-state residual echo suppressor.
/// Designed for 16 kHz mono float audio in 160-sample (10 ms) frames.
class AecProcessor {
public:
    static constexpr size_t kSampleRate    = 16000;
    static constexpr size_t kFrameSize     = 160;  // 10 ms @ 16 kHz
    static constexpr size_t kFilterTaps    = 512;  // 32 ms echo tail
    static constexpr size_t kHistorySize   = 2048; // Power of 2 (128 ms history)
    static constexpr size_t kHistMask      = kHistorySize - 1;

    AecProcessor();
    ~AecProcessor() = default;

    // Non-copyable, non-movable
    AecProcessor(const AecProcessor&) = delete;
    AecProcessor& operator=(const AecProcessor&) = delete;

    /// Configures the estimated speaker-to-mic bulk delay in samples (e.g. 160 = 10 ms).
    void SetBulkDelaySamples(size_t delay_samples) noexcept {
        bulk_delay_ = std::min(delay_samples, kHistorySize - kFilterTaps - 1);
    }

    /// Processes exactly one 160-sample (10 ms) frame with zero heap allocations and zero locks.
    void ProcessFrame(
        const float* mic_in,
        const float* ref_in,
        float* cleaned_out,
        bool is_speaking) noexcept;

    /// Processes arbitrary count of samples in 160-sample chunks.
    void ProcessBlock(
        const float* mic_in,
        const float* ref_in,
        float* cleaned_out,
        size_t count,
        bool is_speaking) noexcept;

    /// Telemetry: Last measured Echo Return Loss Enhancement (ERLE) in dB.
    float LastErleDb() const noexcept { return erle_db_; }

    /// Telemetry: True if double-talk (near-end human voice overlapping echo) was detected.
    bool IsDoubleTalk() const noexcept { return is_double_talk_; }

    /// Clears filter weights, history, and adaptation state.
    void Reset() noexcept;

private:
    std::vector<float> weights_;     // kFilterTaps (512)
    std::vector<float> ref_history_; // kHistorySize (2048, circular)
    size_t             write_idx_{0};
    size_t             bulk_delay_{160}; // 10 ms default

    float erle_db_{0.0f};
    bool  is_double_talk_{false};

    float step_size_{0.25f}; // NLMS learning rate mu
    float epsilon_{1e-3f};   // Regularization factor
};

} // namespace jarvis::audio
