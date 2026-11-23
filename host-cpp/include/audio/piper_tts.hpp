#pragma once

#include <windows.h>
#include <mmdeviceapi.h>
#include <audioclient.h>

#include <atomic>
#include <condition_variable>
#include <cstddef>
#include <cstdint>
#include <deque>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "audio/ring_buffer.hpp"

namespace jarvis::audio {

/// Low-latency Piper TTS synthesis and WASAPI render engine.
/// Runs playback on a dedicated Core 0 thread (0x03 mask) with MMCSS "Pro Audio" priority,
/// supporting sub-20 ms epoch-based barge-in and feeding a lock-free far-end reference buffer for AEC.
class PiperTts {
public:
    static constexpr double kPiperSampleRate = 22050.0;
    static constexpr double kRefSampleRate   = 16000.0;

    explicit PiperTts(const std::string& model_path = "", const std::string& config_path = "");
    ~PiperTts();

    // Non-copyable, non-movable
    PiperTts(const PiperTts&) = delete;
    PiperTts& operator=(const PiperTts&) = delete;

    /// Loads the Piper neural voice model and config file (e.g. en_US-ryan-medium.onnx).
    bool LoadModel(const std::string& model_path, const std::string& config_path);

    /// Starts the background WASAPI render playback thread.
    bool Start();

    /// Stops playback and joins the render thread.
    void Stop();

    /// Synthesizes text and enqueues audio for real-time chunked playback under the current epoch.
    void Speak(const std::string& text);

    /// Sub-20 ms Barge-In / Stop: advances speech epoch, flushes pending audio chunks,
    /// resets the WASAPI render client buffer, clears the reference buffer, and returns immediately (< 20 ms).
    void StopPlayback() noexcept;

    /// Alias for StopPlayback() for backwards compatibility.
    void BargeIn() noexcept { StopPlayback(); }

    /// Whether audio is actively rendering/playing through the speakers.
    bool IsSpeaking() const noexcept { return is_speaking_.load(std::memory_order_relaxed); }

    /// Whether a voice model is loaded and ready.
    bool IsModelLoaded() const noexcept { return is_model_loaded_; }

    /// Current speech epoch counter.
    uint64_t CurrentEpoch() const noexcept { return playback_epoch_.load(std::memory_order_relaxed); }

    /// Exposes the lock-free SPSC Far-End Reference Ring Buffer (16 kHz mono float)
    /// fed synchronously with speaker playback for Acoustic Echo Cancellation.
    SpscRingBuffer<32768>& FarEndReferenceBuffer() noexcept { return far_end_ref_; }

    /// Linear upsampler from 22050 Hz mono float to native device rate & channel count.
    static void UpsampleToDevice(
        const float* mono_22k,
        size_t num_mono_samples,
        uint32_t device_rate,
        uint16_t device_channels,
        std::vector<float>& out_interleaved);

    /// Linear resampler from 22050 Hz mono float to 16 kHz mono float reference buffer.
    static void ResampleTo16kMono(
        const float* mono_22k,
        size_t num_mono_samples,
        std::vector<float>& out_16k_mono);

private:
    struct AudioChunk {
        uint64_t epoch{0};
        std::vector<float> pcm_22k;
    };

    void RenderThreadLoop();
    bool InitRenderEndpoint(HANDLE event_handle);
    void SynthesizeInternal(const std::string& text, uint64_t epoch, std::vector<float>& pcm_out);

private:
    std::thread             render_thread_;
    std::atomic<bool>       running_{false};

    // Monotonic playback epoch and speaking status
    alignas(64) std::atomic<uint64_t> playback_epoch_{0};
    alignas(64) std::atomic<bool>     is_speaking_{false};

    std::mutex              queue_mutex_;
    std::condition_variable queue_cv_;
    std::deque<AudioChunk>  pcm_queue_;

    // Lock-free Far-End Reference Ring Buffer (16 kHz mono float for AEC)
    SpscRingBuffer<32768>   far_end_ref_;

    // Win32 abort/wake event for sub-20ms StopPlayback() notification
    HANDLE                  h_abort_event_{nullptr};

    // WASAPI Render pointers
    IMMDeviceEnumerator*    enumerator_{nullptr};
    IAudioClient*           render_client_{nullptr};
    IAudioRenderClient*     render_service_{nullptr};
    WAVEFORMATEX*           mix_format_{nullptr};

    uint32_t                native_rate_{48000};
    uint16_t                native_channels_{2};
    UINT32                  buffer_frame_count_{0};

    // Pre-allocated scratch buffers
    std::vector<float>      render_scratch_;
    std::vector<float>      ref_scratch_;

    bool                    is_model_loaded_{false};

    struct PiperImpl;
    std::unique_ptr<PiperImpl> impl_;
};

} // namespace jarvis::audio
