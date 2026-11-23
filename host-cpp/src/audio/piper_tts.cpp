#include "audio/piper_tts.hpp"
#include "audio/mmcss_guard.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <iostream>

namespace jarvis::audio {

struct PiperTts::PiperImpl {
#ifdef JARVIS_HAS_PIPER
    // Real Piper ONNX runtime context
#endif
};

PiperTts::PiperTts(const std::string& model_path, const std::string& config_path)
    : impl_(std::make_unique<PiperImpl>()) {
    render_scratch_.reserve(8192);
    ref_scratch_.reserve(8192);
    h_abort_event_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);

    if (!model_path.empty()) {
        LoadModel(model_path, config_path);
    }
}

PiperTts::~PiperTts() {
    Stop();
    if (h_abort_event_) {
        CloseHandle(h_abort_event_);
        h_abort_event_ = nullptr;
    }
}

bool PiperTts::LoadModel(const std::string& model_path, const std::string& config_path) {
    if (model_path.empty()) return false;

#ifdef JARVIS_HAS_PIPER
    (void)config_path;
    is_model_loaded_ = true;
    return true;
#else
    (void)config_path;
    is_model_loaded_ = true;
    return true;
#endif
}

bool PiperTts::Start() {
    if (running_.load(std::memory_order_relaxed)) {
        return true;
    }

    running_.store(true, std::memory_order_release);
    render_thread_ = std::thread(&PiperTts::RenderThreadLoop, this);
    return true;
}

void PiperTts::Stop() {
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }

    StopPlayback();
    queue_cv_.notify_all();

    if (render_thread_.joinable()) {
        render_thread_.join();
    }
}

void PiperTts::StopPlayback() noexcept {
    playback_epoch_.fetch_add(1, std::memory_order_acq_rel);
    is_speaking_.store(false, std::memory_order_release);

    {
        std::lock_guard<std::mutex> lock(queue_mutex_);
        pcm_queue_.clear();
    }
    queue_cv_.notify_all();

    if (h_abort_event_) {
        SetEvent(h_abort_event_);
    }
}

void PiperTts::Speak(const std::string& text) {
    if (text.empty()) return;

    const uint64_t epoch = playback_epoch_.load(std::memory_order_acquire);
    std::vector<float> pcm_22k;
    SynthesizeInternal(text, epoch, pcm_22k);

    if (pcm_22k.empty()) return;

    // Check epoch to ensure barge-in did not occur during synthesis
    if (playback_epoch_.load(std::memory_order_acquire) != epoch) {
        return;
    }

    {
        std::lock_guard<std::mutex> lock(queue_mutex_);
        pcm_queue_.push_back(AudioChunk{epoch, std::move(pcm_22k)});
    }
    queue_cv_.notify_one();
}

void PiperTts::SynthesizeInternal(const std::string& text, uint64_t epoch, std::vector<float>& pcm_out) {
#ifdef JARVIS_HAS_PIPER
    (void)text;
    (void)epoch;
    (void)pcm_out;
#else
    // Build-safe mock synthesis: generates a rich multi-tone vocal confirmation tone (350 ms @ 22050 Hz)
    (void)text;
    (void)epoch;
    constexpr size_t kSamples = 7717; // ~350 ms @ 22050 Hz
    pcm_out.resize(kSamples);
    constexpr double kPi = 3.14159265358979323846;

    for (size_t i = 0; i < kSamples; ++i) {
        double t = static_cast<double>(i) / kPiperSampleRate;
        double env = std::min(1.0, t * 50.0) * std::exp(-t * 6.0);
        // Harmonics: 440 Hz fundamental + 880 Hz + 1320 Hz formants
        double s = 0.50 * std::sin(2.0 * kPi * 440.0 * t) +
                   0.30 * std::sin(2.0 * kPi * 880.0 * t) +
                   0.20 * std::sin(2.0 * kPi * 1320.0 * t);
        pcm_out[i] = static_cast<float>(s * 0.20 * env);
    }
#endif
}

void PiperTts::UpsampleToDevice(
    const float* mono_22k,
    size_t num_mono_samples,
    uint32_t device_rate,
    uint16_t device_channels,
    std::vector<float>& out_interleaved) {

    if (!mono_22k || num_mono_samples == 0 || device_channels == 0) return;

    const double step = kPiperSampleRate / static_cast<double>(device_rate);
    const size_t out_frames = static_cast<size_t>(static_cast<double>(num_mono_samples) / step);
    out_interleaved.resize(out_frames * device_channels);

    for (size_t i = 0; i < out_frames; ++i) {
        const double pos = static_cast<double>(i) * step;
        const size_t idx = static_cast<size_t>(std::floor(pos));
        const float frac = static_cast<float>(pos - static_cast<double>(idx));

        float s0 = (idx < num_mono_samples) ? mono_22k[idx] : 0.0f;
        float s1 = (idx + 1 < num_mono_samples) ? mono_22k[idx + 1] : s0;

        float sample = s0 * (1.0f - frac) + s1 * frac;

        for (uint16_t c = 0; c < device_channels; ++c) {
            out_interleaved[i * device_channels + c] = sample;
        }
    }
}

void PiperTts::ResampleTo16kMono(
    const float* mono_22k,
    size_t num_mono_samples,
    std::vector<float>& out_16k_mono) {

    if (!mono_22k || num_mono_samples == 0) return;

    const double step = kPiperSampleRate / kRefSampleRate; // ~1.378125
    const size_t out_samples = static_cast<size_t>(static_cast<double>(num_mono_samples) / step);
    out_16k_mono.resize(out_samples);

    for (size_t i = 0; i < out_samples; ++i) {
        const double pos = static_cast<double>(i) * step;
        const size_t idx = static_cast<size_t>(std::floor(pos));
        const float frac = static_cast<float>(pos - static_cast<double>(idx));

        float s0 = (idx < num_mono_samples) ? mono_22k[idx] : 0.0f;
        float s1 = (idx + 1 < num_mono_samples) ? mono_22k[idx + 1] : s0;

        out_16k_mono[i] = s0 * (1.0f - frac) + s1 * frac;
    }
}

bool PiperTts::InitRenderEndpoint(HANDLE event_handle) {
    HRESULT hr = CoCreateInstance(
        __uuidof(MMDeviceEnumerator),
        nullptr,
        CLSCTX_ALL,
        __uuidof(IMMDeviceEnumerator),
        reinterpret_cast<void**>(&enumerator_));

    if (FAILED(hr) || !enumerator_) return false;

    IMMDevice* device = nullptr;
    hr = enumerator_->GetDefaultAudioEndpoint(eRender, eConsole, &device);
    if (FAILED(hr) || !device) {
        hr = enumerator_->GetDefaultAudioEndpoint(eRender, eCommunications, &device);
    }
    if (FAILED(hr) || !device) return false;

    hr = device->Activate(
        __uuidof(IAudioClient),
        CLSCTX_ALL,
        nullptr,
        reinterpret_cast<void**>(&render_client_));
    device->Release();

    if (FAILED(hr) || !render_client_) return false;

    hr = render_client_->GetMixFormat(&mix_format_);
    if (FAILED(hr) || !mix_format_) return false;

    native_rate_ = mix_format_->nSamplesPerSec;
    native_channels_ = mix_format_->nChannels;

    constexpr REFERENCE_TIME kBufferDuration100ns = 200000; // 20 ms
    hr = render_client_->Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        kBufferDuration100ns,
        0,
        mix_format_,
        nullptr);

    if (FAILED(hr)) return false;

    hr = render_client_->SetEventHandle(event_handle);
    if (FAILED(hr)) return false;

    hr = render_client_->GetService(
        __uuidof(IAudioRenderClient),
        reinterpret_cast<void**>(&render_service_));

    if (FAILED(hr) || !render_service_) return false;

    hr = render_client_->GetBufferSize(&buffer_frame_count_);
    if (FAILED(hr)) return false;

    hr = render_client_->Start();
    return SUCCEEDED(hr);
}

void PiperTts::RenderThreadLoop() {
    CoInitializeEx(nullptr, COINIT_MULTITHREADED);

    MmcssGuard mmcss(L"Pro Audio", 0x03);

    HANDLE hRenderEvent = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    if (!hRenderEvent) {
        CoUninitialize();
        return;
    }

    if (!InitRenderEndpoint(hRenderEvent)) {
        CloseHandle(hRenderEvent);
        CoUninitialize();
        return;
    }

    uint64_t local_epoch = playback_epoch_.load(std::memory_order_acquire);
    size_t chunk_cursor = 0;
    AudioChunk current_chunk;

    while (running_.load(std::memory_order_relaxed)) {
        // 1. Instant check for Barge-In trigger
        uint64_t current_epoch = playback_epoch_.load(std::memory_order_acquire);
        if (current_epoch != local_epoch) {
            local_epoch = current_epoch;
            current_chunk.pcm_22k.clear();
            chunk_cursor = 0;

            if (render_client_) {
                render_client_->Stop();
                render_client_->Reset();
                render_client_->Start();
            }
            if (h_abort_event_) {
                ResetEvent(h_abort_event_);
            }
            is_speaking_.store(false, std::memory_order_release);
            continue;
        }

        // 2. Obtain next chunk if current chunk is exhausted
        if (chunk_cursor >= current_chunk.pcm_22k.size()) {
            current_chunk.pcm_22k.clear();
            chunk_cursor = 0;

            std::unique_lock<std::mutex> lock(queue_mutex_);
            if (pcm_queue_.empty()) {
                is_speaking_.store(false, std::memory_order_release);
                queue_cv_.wait_for(lock, std::chrono::milliseconds(10), [this, &local_epoch]() {
                    return !running_.load(std::memory_order_relaxed) ||
                           !pcm_queue_.empty() ||
                           playback_epoch_.load(std::memory_order_relaxed) != local_epoch;
                });
                continue;
            }

            current_chunk = std::move(pcm_queue_.front());
            pcm_queue_.pop_front();

            if (current_chunk.epoch != local_epoch) {
                current_chunk.pcm_22k.clear();
                continue;
            }

            is_speaking_.store(true, std::memory_order_release);
        }

        // 3. Wait for WASAPI render buffer space or abort signal
        HANDLE wait_handles[2] = { hRenderEvent, h_abort_event_ };
        DWORD wait_res = WaitForMultipleObjects(2, wait_handles, FALSE, 20);
        if (wait_res == WAIT_OBJECT_0 + 1) {
            if (h_abort_event_) {
                ResetEvent(h_abort_event_);
            }
            continue;
        }
        if (wait_res != WAIT_OBJECT_0 && wait_res != WAIT_TIMEOUT) {
            continue;
        }

        UINT32 padding = 0;
        HRESULT hr = render_client_->GetCurrentPadding(&padding);
        if (FAILED(hr)) continue;

        UINT32 frames_available = buffer_frame_count_ - padding;
        if (frames_available == 0) continue;

        const double step = kPiperSampleRate / static_cast<double>(native_rate_);
        const size_t needed_mono_samples = static_cast<size_t>(static_cast<double>(frames_available) * step);
        const size_t mono_to_read = std::min(needed_mono_samples, current_chunk.pcm_22k.size() - chunk_cursor);
        if (mono_to_read == 0) continue;

        // Synchronously feed 16 kHz reference samples to Far-End Reference Ring Buffer for AEC
        ref_scratch_.clear();
        ResampleTo16kMono(&current_chunk.pcm_22k[chunk_cursor], mono_to_read, ref_scratch_);
        if (!ref_scratch_.empty()) {
            far_end_ref_.PushBatch(ref_scratch_.data(), ref_scratch_.size());
        }

        // Upsample to native device format for speaker render
        render_scratch_.clear();
        UpsampleToDevice(
            &current_chunk.pcm_22k[chunk_cursor],
            mono_to_read,
            native_rate_,
            native_channels_,
            render_scratch_);

        const UINT32 frames_to_write = static_cast<UINT32>(render_scratch_.size() / native_channels_);
        if (frames_to_write == 0) continue;

        BYTE* pData = nullptr;
        hr = render_service_->GetBuffer(frames_to_write, &pData);
        if (SUCCEEDED(hr) && pData) {
            std::memcpy(pData, render_scratch_.data(), frames_to_write * native_channels_ * sizeof(float));
            render_service_->ReleaseBuffer(frames_to_write, 0);
            chunk_cursor += mono_to_read;
        }
    }

    if (render_client_) {
        render_client_->Stop();
        render_client_->Reset();
        if (render_service_) render_service_->Release();
        if (mix_format_) CoTaskMemFree(mix_format_);
        render_client_->Release();
    }

    if (enumerator_) {
        enumerator_->Release();
    }

    CloseHandle(hRenderEvent);
    CoUninitialize();
}

} // namespace jarvis::audio
