#include "audio/silero_vad.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <iostream>

#ifdef JARVIS_HAS_ONNXRUNTIME
#include <onnxruntime_cxx_api.h>
#endif

namespace jarvis::audio {

struct SileroVad::OnnxImpl {
#ifdef JARVIS_HAS_ONNXRUNTIME
    Ort::Env env{ORT_LOGGING_LEVEL_WARNING, "SileroVad"};
    Ort::SessionOptions session_options;
    std::unique_ptr<Ort::Session> session;
    Ort::MemoryInfo memory_info{Ort::MemoryInfo::CreateCpu(OrtArenaAllocator, OrtMemTypeDefault)};

    OnnxImpl(const std::wstring& model_path) {
        session_options.SetIntraOpNumThreads(1);
        session_options.SetInterOpNumThreads(1);
        session_options.SetGraphOptimizationLevel(GraphOptimizationLevel::ORT_ENABLE_ALL);
        try {
            session = std::make_unique<Ort::Session>(env, model_path.c_str(), session_options);
            std::wcout << L"[SileroVad] Loaded real ONNX model: " << model_path << std::endl;
        } catch (const std::exception& ex) {
            std::cerr << "[SileroVad] Failed to load ONNX model: " << ex.what() << std::endl;
        }
    }
#else
    explicit OnnxImpl(const std::wstring&) {}
#endif
};

SileroVad::SileroVad(SpscRingBuffer<32768>& ring_buffer, const std::wstring& model_path)
    : ring_buffer_(ring_buffer),
      frame_scratch_(kFrameSize, 0.0f),
      hidden_state_(256, 0.0f) {
    active_utterance_.reserve(kMaxUtteranceSamples);

    if (!model_path.empty()) {
#ifdef JARVIS_HAS_ONNXRUNTIME
        onnx_impl_ = std::make_unique<OnnxImpl>(model_path);
        if (onnx_impl_->session) {
            is_onnx_loaded_ = true;
        }
#else
        (void)model_path;
#endif
    }
}

SileroVad::~SileroVad() = default;

void SileroVad::ResetState() noexcept {
    std::fill(hidden_state_.begin(), hidden_state_.end(), 0.0f);
    consecutive_speech_frames_ = 0;
    consecutive_silence_frames_ = 0;
}

float SileroVad::EvaluateFrame(const float* frame_512) {
    if (!frame_512) return 0.0f;

    if (is_onnx_loaded_) {
        return EvaluateOnnx(frame_512);
    }
    return EvaluateFallback(frame_512);
}

float SileroVad::EvaluateOnnx(const float* frame_512) {
#ifdef JARVIS_HAS_ONNXRUNTIME
    if (!onnx_impl_ || !onnx_impl_->session) {
        return EvaluateFallback(frame_512);
    }

    // Input shapes:
    // "input": [1, 512]
    // "state": [2, 1, 128] (256 floats)
    // "sr":    [1] int64 = 16000
    const int64_t input_shape[2] = {1, static_cast<int64_t>(kFrameSize)};
    const int64_t state_shape[3] = {2, 1, 128};
    const int64_t sr_shape[1]    = {1};
    const int64_t sr_val         = 16000;

    auto input_tensor = Ort::Value::CreateTensor<float>(
        onnx_impl_->memory_info,
        const_cast<float*>(frame_512),
        kFrameSize,
        input_shape,
        2);

    auto state_tensor = Ort::Value::CreateTensor<float>(
        onnx_impl_->memory_info,
        hidden_state_.data(),
        hidden_state_.size(),
        state_shape,
        3);

    auto sr_tensor = Ort::Value::CreateTensor<int64_t>(
        onnx_impl_->memory_info,
        const_cast<int64_t*>(&sr_val),
        1,
        sr_shape,
        1);

    const char* input_names[] = {"input", "state", "sr"};
    Ort::Value in_tensors[] = {std::move(input_tensor), std::move(state_tensor), std::move(sr_tensor)};

    const char* output_names[] = {"output", "stateN"};

    try {
        auto output_tensors = onnx_impl_->session->Run(
            Ort::RunOptions{nullptr},
            input_names,
            in_tensors,
            3,
            output_names,
            2);

        // "output": float[1, 1]
        float prob = output_tensors[0].GetTensorMutableData<float>()[0];

        // "stateN": float[2, 1, 128]
        const float* next_state = output_tensors[1].GetTensorMutableData<float>();
        std::memcpy(hidden_state_.data(), next_state, hidden_state_.size() * sizeof(float));

        return std::clamp(prob, 0.0f, 1.0f);
    } catch (const std::exception& ex) {
        std::cerr << "[SileroVad] Inference error: " << ex.what() << std::endl;
        return EvaluateFallback(frame_512);
    }
#else
    return EvaluateFallback(frame_512);
#endif
}

float SileroVad::EvaluateFallback(const float* frame_512) {
    // High-precision adaptive RMS energy + Zero-Crossing Rate fallback detector
    double sum_sq = 0.0;
    int zero_crossings = 0;

    for (size_t i = 0; i < kFrameSize; ++i) {
        const float s = frame_512[i];
        sum_sq += static_cast<double>(s) * static_cast<double>(s);
        if (i > 0 && ((s >= 0.0f && frame_512[i - 1] < 0.0f) || (s < 0.0f && frame_512[i - 1] >= 0.0f))) {
            zero_crossings++;
        }
    }

    const float rms = static_cast<float>(std::sqrt(sum_sq / static_cast<double>(kFrameSize)));
    const float zcr = static_cast<float>(zero_crossings) / static_cast<float>(kFrameSize);

    // Adapt dynamic background noise floor during silence
    if (state_ == VadState::kSilence) {
        noise_floor_ = 0.98f * noise_floor_ + 0.02f * std::clamp(rms, 0.001f, 0.05f);
    }

    const float snr = rms / std::max(noise_floor_, 0.002f);

    // Voice speech has robust energy (SNR > 2.5) with moderate zero-crossings (ZCR between 0.03 and 0.40)
    float prob = 0.0f;
    if (snr > 1.8f && zcr > 0.02f && zcr < 0.45f) {
        prob = std::clamp((snr - 1.8f) / 3.5f, 0.0f, 1.0f);
    }

    return prob;
}

bool SileroVad::ProcessNextFrame() {
    if (ring_buffer_.AvailableRead() < kFrameSize) {
        return false;
    }

    size_t read_bytes = ring_buffer_.PopBatch(frame_scratch_.data(), kFrameSize);
    if (read_bytes < kFrameSize) {
        return false;
    }

    const float prob = EvaluateFrame(frame_scratch_.data());

    const float start_thresh = is_speaking_mode_ ? 0.75f : kStartThreshold;
    const int   min_speech_frames = is_speaking_mode_ ? 3 : kStartMinFrames;

    float frame_rms = 0.0f;
    if (is_speaking_mode_) {
        double sum_sq = 0.0;
        for (size_t i = 0; i < kFrameSize; ++i) {
            sum_sq += static_cast<double>(frame_scratch_[i]) * static_cast<double>(frame_scratch_[i]);
        }
        frame_rms = static_cast<float>(std::sqrt(sum_sq / static_cast<double>(kFrameSize)));
    }

    const bool speech_detected = (prob >= start_thresh) && (!is_speaking_mode_ || frame_rms >= 0.02f);

    if (state_ == VadState::kSilence) {
        if (speech_detected) {
            consecutive_speech_frames_++;
            if (consecutive_speech_frames_ >= min_speech_frames) {
                // Transition to SPEECH_START!
                state_ = VadState::kSpeech;
                consecutive_silence_frames_ = 0;

                active_utterance_.clear();

                // 1. Snapshot the 800 ms pre-roll from the ring buffer
                std::vector<float> pre_roll(kPreRollSamples);
                const size_t peeked = ring_buffer_.PeekRecent(pre_roll.data(), kPreRollSamples);
                pre_roll.resize(peeked);
                active_utterance_.insert(active_utterance_.end(), pre_roll.begin(), pre_roll.end());

                // 2. Append the current trigger frame
                active_utterance_.insert(active_utterance_.end(), frame_scratch_.begin(), frame_scratch_.end());
            }
        } else {
            consecutive_speech_frames_ = 0;
        }
    } else { // VadState::kSpeech
        // Accumulate active speech
        active_utterance_.insert(active_utterance_.end(), frame_scratch_.begin(), frame_scratch_.end());

        if (prob < kEndThreshold) {
            consecutive_silence_frames_++;
            if (consecutive_silence_frames_ >= kEndSilenceFrames) {
                // Transition to SPEECH_END!
                state_ = VadState::kSilence;
                consecutive_speech_frames_ = 0;
                consecutive_silence_frames_ = 0;

                if (utterance_cb_ && !active_utterance_.empty()) {
                    utterance_cb_(std::move(active_utterance_));
                }
                active_utterance_.clear();
                ResetState();
            }
        } else {
            consecutive_silence_frames_ = 0;
        }

        // Safety cap: terminate if speech exceeds 15 seconds (240,000 samples)
        if (active_utterance_.size() >= kMaxUtteranceSamples) {
            state_ = VadState::kSilence;
            consecutive_speech_frames_ = 0;
            consecutive_silence_frames_ = 0;

            if (utterance_cb_ && !active_utterance_.empty()) {
                utterance_cb_(std::move(active_utterance_));
            }
            active_utterance_.clear();
            ResetState();
        }
    }

    return true;
}

} // namespace jarvis::audio
