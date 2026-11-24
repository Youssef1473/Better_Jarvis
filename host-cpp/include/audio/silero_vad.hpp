#pragma once

#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <vector>

#include "audio/ring_buffer.hpp"

namespace jarvis::audio {

enum class VadState {
    kSilence,
    kSpeech
};

/// Silero VAD v4 Voice Activity Detector.
/// Analyzes 512-sample (32 ms @ 16 kHz) frames using either:
/// 1) Exact Silero VAD v4 ONNX Runtime model (when JARVIS_HAS_ONNXRUNTIME is defined), or
/// 2) Adaptive RMS energy + Zero-Crossing Rate fallback detector (zero-dependency build).
class SileroVad {
public:
    static constexpr size_t kFrameSize            = 512;     // 32 ms @ 16 kHz
    static constexpr size_t kPreRollSamples        = 12800;   // 800 ms pre-roll
    static constexpr size_t kMaxUtteranceSamples   = 240000;  // 15 seconds max cap
    static constexpr float  kStartThreshold        = 0.50f;   // Speech onset threshold
    static constexpr float  kEndThreshold          = 0.35f;   // Silence offset threshold
    static constexpr int    kStartMinFrames        = 2;       // 64 ms consecutive speech to trigger
    static constexpr int    kEndSilenceFrames      = 21;      // ~672 ms silence to end speech

    using UtteranceCallback = std::function<void(std::vector<float>&&)>;

    explicit SileroVad(
        SpscRingBuffer<32768>& ring_buffer,
        const std::wstring& model_path = L"");
    ~SileroVad();

    // Non-copyable, non-movable
    SileroVad(const SileroVad&) = delete;
    SileroVad& operator=(const SileroVad&) = delete;

    /// Registers the callback fired when a complete speech utterance finishes.
    void SetUtteranceCallback(UtteranceCallback cb) {
        utterance_cb_ = std::move(cb);
    }

    /// Pulls up to 512 samples from the ring buffer, evaluates speech probability,
    /// and advances the hysteresis state machine.
    /// Returns true if a frame was processed, false if not enough samples were ready.
    bool ProcessNextFrame();

    /// Evaluates raw speech probability [0.0 - 1.0] for a single 512-sample frame.
    float EvaluateFrame(const float* frame_512);

    /// Resets the RNN hidden state (e.g. between utterances).
    void ResetState() noexcept;

    /// Sets whether Jarvis TTS is actively speaking, enforcing stricter barge-in onset thresholds.
    void SetSpeakingMode(bool is_speaking) noexcept { is_speaking_mode_ = is_speaking; }
    bool IsSpeakingMode() const noexcept { return is_speaking_mode_; }

    /// Current state machine status.
    VadState State() const noexcept { return state_; }
    bool IsInSpeech() const noexcept { return state_ == VadState::kSpeech; }
    bool IsOnnxLoaded() const noexcept { return is_onnx_loaded_; }
    float LastProbability() const noexcept { return last_prob_; }

private:
    float EvaluateOnnx(const float* frame_512);
    float EvaluateFallback(const float* frame_512);

private:
    SpscRingBuffer<32768>& ring_buffer_;
    UtteranceCallback      utterance_cb_;

    VadState state_{VadState::kSilence};
    bool     is_speaking_mode_{false};
    int      consecutive_speech_frames_{0};
    int      consecutive_silence_frames_{0};

    std::vector<float> frame_scratch_;
    std::vector<float> active_utterance_;

    // Silero VAD v4 Hidden State: float[2, 1, 128] = 256 floats
    std::vector<float> hidden_state_;
    bool               is_onnx_loaded_{false};

    // Fallback RMS energy detector state
    float noise_floor_{0.005f};
    float last_prob_{0.0f};

    // Opaque pointer to ONNX Runtime session implementation
    struct OnnxImpl;
    std::unique_ptr<OnnxImpl> onnx_impl_;
};

} // namespace jarvis::audio
