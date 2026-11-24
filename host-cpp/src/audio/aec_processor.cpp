#include "audio/aec_processor.hpp"

namespace jarvis::audio {

AecProcessor::AecProcessor()
    : weights_(kFilterTaps, 0.0f),
      ref_history_(kHistorySize, 0.0f) {
}

void AecProcessor::Reset() noexcept {
    std::fill(weights_.begin(), weights_.end(), 0.0f);
    std::fill(ref_history_.begin(), ref_history_.end(), 0.0f);
    write_idx_ = 0;
    erle_db_ = 0.0f;
    is_double_talk_ = false;
    max_ref_energy_ = 0.0f;
}

void AecProcessor::SetBulkDelaySamples(size_t delay_samples) noexcept {
    const size_t clamped = std::clamp(delay_samples, size_t{0}, size_t{3200});
    const size_t shift = (clamped > bulk_delay_) ? (clamped - bulk_delay_) : (bulk_delay_ - clamped);
    bulk_delay_ = clamped;
    if (shift > 32) {
        std::fill(weights_.begin(), weights_.end(), 0.0f);
    }
}

void AecProcessor::ProcessFrame(
    const float* mic_in,
    const float* ref_in,
    float* cleaned_out,
    bool is_speaking) noexcept {

    if (!mic_in || !cleaned_out) return;

    float mic_energy = 0.0f;
    float ref_energy = 0.0f;
    float err_energy = 0.0f;

    // Calculate reference power and mic energy across this 160-sample frame
    for (size_t i = 0; i < kFrameSize; ++i) {
        const float m = mic_in[i];
        const float r = ref_in ? ref_in[i] : 0.0f;
        mic_energy += m * m;
        ref_energy += r * r;
    }

    // Double-Talk Detection (DTD):
    // In typical desktop acoustics, acoustic coupling is < 0.85 (echo is quieter than peak speaker output).
    // Using a peak envelope over recent reference history (Geigel DTD principle),
    // if mic energy exceeds 0.85 * max_ref_energy_, near-end human voice is dominant.
    max_ref_energy_ = std::max(ref_energy, max_ref_energy_ * 0.95f);
    if (max_ref_energy_ > 1e-4f) {
        if (mic_energy > 0.85f * max_ref_energy_) {
            is_double_talk_ = true;
        } else {
            is_double_talk_ = false;
        }
    } else {
        is_double_talk_ = false;
    }

    // Determine adaptation status
    const bool adapt = is_speaking && !is_double_talk_ && (ref_energy > 1e-5f);

    // Compute reference history power for step normalization
    float ref_power = 0.0f;
    if (adapt) {
        for (size_t k = 0; k < kFilterTaps; ++k) {
            const size_t idx = (write_idx_ - bulk_delay_ - k) & kHistMask;
            const float val = ref_history_[idx];
            ref_power += val * val;
        }
        ref_power = std::max(ref_power, 1e-4f);
    }

    const float mu_norm = adapt ? (step_size_ / (ref_power + epsilon_)) : 0.0f;

    // Sample-by-sample NLMS filtering and adaptation
    for (size_t i = 0; i < kFrameSize; ++i) {
        const float mic_sample = mic_in[i];
        const float ref_sample = ref_in ? ref_in[i] : 0.0f;

        // Insert new reference sample into circular history buffer
        ref_history_[write_idx_ & kHistMask] = ref_sample;

        // FIR convolution: y(n) = sum_k w_k * x(n - bulk_delay - k)
        float echo_est = 0.0f;
        for (size_t k = 0; k < kFilterTaps; ++k) {
            const size_t idx = (write_idx_ - bulk_delay_ - k) & kHistMask;
            echo_est += weights_[k] * ref_history_[idx];
        }

        // Error: e(n) = d(n) - y(n)
        const float err = mic_sample - echo_est;

        // Adapt filter weights
        if (adapt) {
            for (size_t k = 0; k < kFilterTaps; ++k) {
                const size_t idx = (write_idx_ - bulk_delay_ - k) & kHistMask;
                weights_[k] += mu_norm * err * ref_history_[idx];
                weights_[k] = std::clamp(weights_[k], -2.0f, 2.0f);
            }
        }

        // Advance circular index
        write_idx_++;

        // Residual Echo Suppression / Non-Linear Gate:
        // When Jarvis is speaking and DTD indicates pure echo, suppress remaining linear filter residual
        // by ~16 dB (factor 0.15) so it drops below the Silero VAD activation floor.
        // When double-talk is active or Jarvis is not speaking, zero suppression is applied.
        const float suppression = (is_speaking && !is_double_talk_ && (ref_energy > 1e-4f)) ? 0.15f : 1.0f;
        const float clean_sample = err * suppression;

        cleaned_out[i] = clean_sample;
        err_energy += clean_sample * clean_sample;
    }

    // Telemetry: Compute smoothed ERLE in dB
    if (ref_energy > 1e-4f && mic_energy > 1e-5f) {
        const float current_erle = 10.0f * std::log10((mic_energy + 1e-6f) / (err_energy + 1e-6f));
        erle_db_ = 0.85f * erle_db_ + 0.15f * std::clamp(current_erle, 0.0f, 40.0f);
    } else {
        erle_db_ = 0.90f * erle_db_;
    }
}

void AecProcessor::ProcessBlock(
    const float* mic_in,
    const float* ref_in,
    float* cleaned_out,
    size_t count,
    bool is_speaking) noexcept {

    if (!mic_in || !cleaned_out || count == 0) return;

    size_t offset = 0;
    while (offset + kFrameSize <= count) {
        const float* ref_ptr = ref_in ? (ref_in + offset) : nullptr;
        ProcessFrame(mic_in + offset, ref_ptr, cleaned_out + offset, is_speaking);
        offset += kFrameSize;
    }

    if (offset < count) {
        const size_t rem = count - offset;
        float mic_pad[kFrameSize]{0.0f};
        float ref_pad[kFrameSize]{0.0f};
        float clean_pad[kFrameSize]{0.0f};

        std::memcpy(mic_pad, mic_in + offset, rem * sizeof(float));
        if (ref_in) {
            std::memcpy(ref_pad, ref_in + offset, rem * sizeof(float));
        }

        ProcessFrame(mic_pad, ref_in ? ref_pad : nullptr, clean_pad, is_speaking);
        std::memcpy(cleaned_out + offset, clean_pad, rem * sizeof(float));
    }
}

} // namespace jarvis::audio
