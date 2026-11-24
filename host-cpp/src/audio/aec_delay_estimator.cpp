#include "audio/aec_delay_estimator.hpp"

namespace jarvis::audio {

AecDelayEstimator::AecDelayEstimator()
    : mic_history_(kHistorySize, 0.0f),
      ref_history_(kHistorySize, 0.0f) {
}

void AecDelayEstimator::Reset() noexcept {
    std::fill(mic_history_.begin(), mic_history_.end(), 0.0f);
    std::fill(ref_history_.begin(), ref_history_.end(), 0.0f);
    write_idx_ = 0;
    frame_count_ = 0;
    candidate_raw_lag_ = 0;
    consecutive_count_ = 0;
    is_locked_.store(false, std::memory_order_relaxed);
    estimated_delay_ms_.store(0.0f, std::memory_order_relaxed);
    last_peak_ncc_.store(0.0f, std::memory_order_relaxed);
    locked_bulk_delay_.store(160, std::memory_order_relaxed);
    just_locked_.store(false, std::memory_order_relaxed);
}

void AecDelayEstimator::UpdateFrame(
    const float* mic_in,
    const float* ref_in,
    bool is_speaking,
    bool is_double_talk) noexcept {

    if (!mic_in) return;

    // Decimate 160-sample frame by 4x to 40 samples
    float ref_energy = 0.0f;
    for (size_t j = 0; j < kDecimatedFrame; ++j) {
        float m_sum = 0.0f;
        float r_sum = 0.0f;
        for (size_t s = 0; s < kDecimation; ++s) {
            const size_t raw_idx = j * kDecimation + s;
            m_sum += mic_in[raw_idx];
            const float r = ref_in ? ref_in[raw_idx] : 0.0f;
            r_sum += r;
            ref_energy += r * r;
        }
        const size_t buf_idx = (write_idx_ + j) & kHistMask;
        mic_history_[buf_idx] = m_sum * 0.25f;
        ref_history_[buf_idx] = r_sum * 0.25f;
    }
    write_idx_ = (write_idx_ + kDecimatedFrame) & kHistMask;

    // Check gating conditions:
    // Only calibrate when speaking, far-end reference has active energy, and NOT double-talk!
    const float ref_rms = std::sqrt(ref_energy / static_cast<float>(kFrameSize));
    const bool can_calibrate = is_speaking && !is_double_talk && (ref_rms >= 0.005f);

    if (can_calibrate) {
        frame_count_++;
        if (frame_count_ >= kEvalIntervalFrames) {
            frame_count_ = 0;
            RunEstimation();
        }
    }
}

void AecDelayEstimator::RunEstimation() noexcept {
    // Correlate most recent kCorrWindowSamples of mic_history_ with ref_history_[n - lag]
    // where lag ranges from 0 to kMaxLagSamples (0 to 800 decimated samples = 0 to 200 ms).
    const size_t N = kCorrWindowSamples;

    // Compute mic power over the correlation window
    float mic_power = 0.0f;
    for (size_t n = 0; n < N; ++n) {
        const size_t idx = (write_idx_ - N + n) & kHistMask;
        const float m = mic_history_[idx];
        mic_power += m * m;
    }

    if (mic_power < 1e-6f) {
        return; // No microphone energy to correlate against
    }

    float best_ncc = -1.0f;
    size_t best_lag = 0;

    for (size_t lag = 0; lag <= kMaxLagSamples; ++lag) {
        float cross = 0.0f;
        float ref_power = 0.0f;

        for (size_t n = 0; n < N; ++n) {
            const size_t mic_idx = (write_idx_ - N + n) & kHistMask;
            const size_t ref_idx = (write_idx_ - N + n - lag) & kHistMask;

            const float m = mic_history_[mic_idx];
            const float r = ref_history_[ref_idx];

            cross += m * r;
            ref_power += r * r;
        }

        if (ref_power > 1e-6f) {
            const float ncc = cross / std::sqrt(mic_power * ref_power);
            if (ncc > best_ncc) {
                best_ncc = ncc;
                best_lag = lag;
            }
        }
    }

    last_peak_ncc_.store(best_ncc, std::memory_order_relaxed);

    if (best_ncc >= kMinPeakNcc) {
        const size_t raw_lag = best_lag * kDecimation;

        if (consecutive_count_ > 0) {
            const size_t diff = (raw_lag > candidate_raw_lag_) ? (raw_lag - candidate_raw_lag_) : (candidate_raw_lag_ - raw_lag);
            if (diff <= kConsistencyTolSamples) {
                consecutive_count_++;
                // Smooth candidate raw lag
                candidate_raw_lag_ = (candidate_raw_lag_ * (consecutive_count_ - 1) + raw_lag) / consecutive_count_;
            } else {
                candidate_raw_lag_ = raw_lag;
                consecutive_count_ = 1;
            }
        } else {
            candidate_raw_lag_ = raw_lag;
            consecutive_count_ = 1;
        }

        if (consecutive_count_ >= 3) {
            // Apply 4 ms (64-sample) causal safety lead: target_bulk_delay = (estimated_lag > 64) ? (estimated_lag - 64) : 0
            const size_t target_bulk = (candidate_raw_lag_ > kCausalLeadSamples)
                ? (candidate_raw_lag_ - kCausalLeadSamples)
                : 0;
            const size_t clamped_bulk = std::clamp(target_bulk, size_t{0}, size_t{3200});

            const size_t prev_bulk = locked_bulk_delay_.load(std::memory_order_relaxed);
            const bool was_locked = is_locked_.load(std::memory_order_relaxed);

            is_locked_.store(true, std::memory_order_release);
            estimated_delay_ms_.store(static_cast<float>(candidate_raw_lag_) / 16.0f, std::memory_order_relaxed);
            locked_bulk_delay_.store(clamped_bulk, std::memory_order_release);

            // Signal just locked if newly locked or bulk delay changed by > 16 samples
            const size_t bulk_diff = (clamped_bulk > prev_bulk) ? (clamped_bulk - prev_bulk) : (prev_bulk - clamped_bulk);
            if (!was_locked || bulk_diff > 16) {
                just_locked_.store(true, std::memory_order_release);
            }
        }
    } else {
        // Weak correlation peak; decay consecutive count
        if (consecutive_count_ > 0) {
            consecutive_count_--;
        }
    }
}

bool AecDelayEstimator::CheckAndClearJustLocked(
    size_t* out_bulk_delay,
    float* out_delay_ms,
    float* out_peak_ncc) noexcept {

    if (just_locked_.exchange(false, std::memory_order_acq_rel)) {
        if (out_bulk_delay) *out_bulk_delay = locked_bulk_delay_.load(std::memory_order_relaxed);
        if (out_delay_ms) *out_delay_ms = estimated_delay_ms_.load(std::memory_order_relaxed);
        if (out_peak_ncc) *out_peak_ncc = last_peak_ncc_.load(std::memory_order_relaxed);
        return true;
    }
    return false;
}

} // namespace jarvis::audio
