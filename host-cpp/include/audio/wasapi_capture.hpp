#pragma once

#include <windows.h>
#include <mmdeviceapi.h>
#include <audioclient.h>

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <memory>
#include <thread>
#include <vector>

#include "audio/ring_buffer.hpp"

// MinGW-w64 compatibility definitions for WASAPI stream autoconversion
#ifndef AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
#define AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM 0x80000000
#endif

#ifndef AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
#define AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY 0x08000000
#endif

namespace jarvis::audio {

class AudioDeviceNotifier;

/// High-performance WASAPI event-driven capture engine.
/// Captures from the default system microphone, negotiates native 16 kHz or downsamples to 16 kHz mono float,
/// and feeds the SpscRingBuffer with zero allocations in the real-time MMCSS capture loop.
class WasapiCapture {
public:
    static constexpr size_t kMaxScratchSamples = 65536;

    explicit WasapiCapture(
        SpscRingBuffer<32768>& ring_buffer,
        const std::wstring& target_device = L"default");
    ~WasapiCapture();

    // Non-copyable, non-movable
    WasapiCapture(const WasapiCapture&) = delete;
    WasapiCapture& operator=(const WasapiCapture&) = delete;

    /// Starts the capture thread and registers device notifications.
    bool Start();

    /// Stops capture and waits for the thread to exit cleanly.
    void Stop();

    /// Sets or updates the target audio capture device name (supports case-insensitive substring match).
    void SetTargetDeviceName(const std::wstring& target_device) {
        target_device_name_ = target_device;
    }
    const std::wstring& TargetDeviceName() const noexcept {
        return target_device_name_;
    }

    /// Resolved friendly name of the active capture endpoint.
    std::wstring ActiveDeviceFriendlyName() const {
        return active_friendly_name_;
    }

    /// True if the capture thread is currently running and active.
    bool IsRunning() const noexcept { return running_.load(std::memory_order_relaxed); }

    /// Native hardware sample rate (e.g. 16000, 48000, or 44100).
    uint32_t NativeSampleRate() const noexcept { return native_sample_rate_; }

    /// Native channel count (e.g. 1 or 2).
    uint16_t NativeChannels() const noexcept { return native_channels_; }

    /// Triggers an asynchronous device re-initialization (e.g. on hot-swap).
    void RequestReinit() noexcept { reinit_requested_.store(true, std::memory_order_release); }

    /// Telemetry: count of audio buffer discontinuities detected.
    uint64_t DiscontinuityCount() const noexcept {
        return discontinuity_count_.load(std::memory_order_relaxed);
    }

    /// Reset discontinuity counter.
    void ResetDiscontinuityCount() noexcept {
        discontinuity_count_.store(0, std::memory_order_relaxed);
    }

    /// Whether Windows 11 hardware/OS Acoustic Echo Cancellation was successfully negotiated.
    bool IsOsAecActive() const noexcept {
        return os_aec_active_.load(std::memory_order_relaxed);
    }

    /// Telemetry: active negotiated WASAPI shared-mode engine period in milliseconds.
    float NegotiatedPeriodMs() const noexcept {
        return negotiated_period_ms_.load(std::memory_order_relaxed);
    }

private:
    void CaptureThreadLoop();

    bool InitEndpoint(
        IMMDeviceEnumerator* enumerator,
        IAudioClient** client,
        IAudioCaptureClient** capture_client,
        HANDLE event_handle,
        WAVEFORMATEX** mix_format);

    void ProcessPacket(
        const BYTE* data,
        UINT32 num_frames,
        DWORD flags,
        const WAVEFORMATEX* wfx) noexcept;

    size_t ConvertToMono(
        const BYTE* data,
        UINT32 num_frames,
        const WAVEFORMATEX* wfx,
        float* mono_out,
        size_t max_out) noexcept;

    size_t ResampleTo16k(
        const float* mono_in,
        UINT32 num_frames,
        float* resampled_out,
        size_t max_out) noexcept;

private:
    SpscRingBuffer<32768>& ring_buffer_;

    std::thread       capture_thread_;
    std::atomic<bool> running_{false};
    std::atomic<bool> reinit_requested_{false};

    uint32_t native_sample_rate_{0};
    uint16_t native_channels_{0};

    // Resampler state (continuous across packets to eliminate boundary clicks)
    double             resample_phase_{0.0};
    float              last_input_sample_{0.0f};
    bool               has_last_sample_{false};

    // Pre-allocated scratch buffers to ensure zero allocations on the MMCSS audio thread
    std::vector<float> mono_scratch_;
    std::vector<float> resample_scratch_;

    // Telemetry: audio data discontinuities, OS AEC status, and negotiated period
    alignas(64) std::atomic<uint64_t> discontinuity_count_{0};
    alignas(64) std::atomic<bool>     os_aec_active_{false};
    alignas(64) std::atomic<float>    negotiated_period_ms_{10.0f};

    // Hot-swap notification client
    AudioDeviceNotifier* notifier_{nullptr};
    IMMDeviceEnumerator* enumerator_{nullptr};

    // Target and resolved device names
    std::wstring target_device_name_{L"default"};
    std::wstring active_friendly_name_{L"Default Communications/Console"};
};

} // namespace jarvis::audio
