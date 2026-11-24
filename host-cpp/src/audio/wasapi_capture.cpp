#include "audio/wasapi_capture.hpp"
#include "audio/mmcss_guard.hpp"

#include <chrono>
#include <cmath>
#include <cstring>

namespace jarvis::audio {

namespace {

// Static GUID definition for KSDATAFORMAT_SUBTYPE_IEEE_FLOAT ({00000003-0000-0010-8000-00aa00389b71})
static const GUID kSubtypeIeeeFloat = {
    0x00000003, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71}
};

// MinGW-w64 compatibility definitions for Windows 8+ AudioClient2 and Windows 11 IAcousticEchoCancellationControl
#ifndef __IAudioClient2_INTERFACE_DEFINED__
#define __IAudioClient2_INTERFACE_DEFINED__
static const IID IID_IAudioClient2_Local = {
    0x72677821, 0xcd0a, 0x49db, {0x82, 0xbc, 0x57, 0x82, 0xe2, 0xe0, 0x00, 0xfa}
};

enum AUDIO_STREAM_CATEGORY {
    AudioCategory_Other = 0,
    AudioCategory_ForegroundOnlyMedia = 1,
    AudioCategory_Communications = 3
};

struct AudioClientProperties {
    UINT32 cbSize;
    BOOL bIsOffload;
    AUDIO_STREAM_CATEGORY eCategory;
    DWORD Options;
};

struct IAudioClient2 : public IAudioClient {
    virtual HRESULT STDMETHODCALLTYPE IsOffloadCapable(AUDIO_STREAM_CATEGORY Category, BOOL *pbPermitted) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetClientProperties(const AudioClientProperties *pProperties) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetBufferSizeLimits(const WAVEFORMATEX *pFormat, BOOL bEventDriven, REFERENCE_TIME *phnsMinBufferDuration, REFERENCE_TIME *phnsMaxBufferDuration) = 0;
};
#else
static const IID& IID_IAudioClient2_Local = __uuidof(IAudioClient2);
#endif

#ifndef __IAcousticEchoCancellationControl_INTERFACE_DEFINED__
#define __IAcousticEchoCancellationControl_INTERFACE_DEFINED__
static const IID IID_IAcousticEchoCancellationControl_Local = {
    0xf4bed277, 0x2d59, 0x4925, {0x86, 0xb7, 0x33, 0x88, 0xc6, 0x97, 0xe8, 0x8a}
};

struct IAcousticEchoCancellationControl : public IUnknown {
    virtual HRESULT STDMETHODCALLTYPE SetEchoCancellationRenderEndpoint(LPCWSTR endpointId) = 0;
};
#else
static const IID& IID_IAcousticEchoCancellationControl_Local = __uuidof(IAcousticEchoCancellationControl);
#endif

#ifndef __IAudioClient3_INTERFACE_DEFINED__
#define __IAudioClient3_INTERFACE_DEFINED__
static const IID IID_IAudioClient3_Local = {
    0x7ed4ee07, 0x8e67, 0x4cd4, {0x8c, 0x1a, 0x2b, 0x7a, 0x59, 0x87, 0xad, 0x42}
};

struct IAudioClient3 : public IAudioClient2 {
    virtual HRESULT STDMETHODCALLTYPE GetSharedModeEnginePeriod(
        const WAVEFORMATEX *pFormat,
        UINT32 *pDefaultPeriodInFrames,
        UINT32 *pFundamentalPeriodInFrames,
        UINT32 *pMinPeriodInFrames,
        UINT32 *pMaxPeriodInFrames) = 0;

    virtual HRESULT STDMETHODCALLTYPE GetCurrentSharedModeEnginePeriod(
        WAVEFORMATEX **ppFormat,
        UINT32 *pCurrentPeriodInFrames) = 0;

    virtual HRESULT STDMETHODCALLTYPE InitializeSharedAudioStream(
        DWORD StreamFlags,
        UINT32 PeriodInFrames,
        const WAVEFORMATEX *pFormat,
        LPCGUID AudioSessionGuid) = 0;
};
#else
static const IID& IID_IAudioClient3_Local = __uuidof(IAudioClient3);
#endif

constexpr double kTargetSampleRate = 16000.0;
constexpr REFERENCE_TIME kBufferDuration100ns = 200000; // 20 ms buffer

} // namespace

// IMMNotificationClient implementation for audio device hot-swaps
class AudioDeviceNotifier : public IMMNotificationClient {
public:
    explicit AudioDeviceNotifier(WasapiCapture* owner) : owner_(owner), ref_count_(1) {}
    virtual ~AudioDeviceNotifier() = default;

    // IUnknown
    ULONG STDMETHODCALLTYPE AddRef() override {
        return InterlockedIncrement(&ref_count_);
    }

    ULONG STDMETHODCALLTYPE Release() override {
        ULONG count = InterlockedDecrement(&ref_count_);
        if (count == 0) {
            delete this;
        }
        return count;
    }

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        if (riid == __uuidof(IUnknown) || riid == __uuidof(IMMNotificationClient)) {
            *ppv = static_cast<IMMNotificationClient*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }

    // IMMNotificationClient
    HRESULT STDMETHODCALLTYPE OnDefaultDeviceChanged(
        EDataFlow flow, ERole role, LPCWSTR /*pwstrDefaultDeviceId*/) override {
        if (flow == eCapture && (role == eCommunications || role == eConsole)) {
            if (owner_) {
                owner_->RequestReinit();
            }
        }
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE OnDeviceStateChanged(LPCWSTR /*pwstrDeviceId*/, DWORD /*dwNewState*/) override {
        if (owner_) {
            owner_->RequestReinit();
        }
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE OnDeviceAdded(LPCWSTR /*pwstrDeviceId*/) override {
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE OnDeviceRemoved(LPCWSTR /*pwstrDeviceId*/) override {
        if (owner_) {
            owner_->RequestReinit();
        }
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE OnPropertyValueChanged(LPCWSTR /*pwstrDeviceId*/, const PROPERTYKEY /*key*/) override {
        return S_OK;
    }

private:
    WasapiCapture* owner_{nullptr};
    LONG ref_count_{1};
};

WasapiCapture::WasapiCapture(SpscRingBuffer<32768>& ring_buffer)
    : ring_buffer_(ring_buffer),
      mono_scratch_(kMaxScratchSamples, 0.0f),
      resample_scratch_(kMaxScratchSamples, 0.0f) {
}

WasapiCapture::~WasapiCapture() {
    Stop();
}

bool WasapiCapture::Start() {
    if (running_.load(std::memory_order_relaxed)) {
        return true;
    }

    HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(hr) && hr != RPC_E_CHANGED_MODE) {
        return false;
    }

    hr = CoCreateInstance(
        __uuidof(MMDeviceEnumerator),
        nullptr,
        CLSCTX_ALL,
        __uuidof(IMMDeviceEnumerator),
        reinterpret_cast<void**>(&enumerator_));

    if (FAILED(hr) || !enumerator_) {
        CoUninitialize();
        return false;
    }

    notifier_ = new AudioDeviceNotifier(this);
    enumerator_->RegisterEndpointNotificationCallback(notifier_);

    running_.store(true, std::memory_order_release);
    reinit_requested_.store(false, std::memory_order_relaxed);

    capture_thread_ = std::thread(&WasapiCapture::CaptureThreadLoop, this);
    return true;
}

void WasapiCapture::Stop() {
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }

    if (capture_thread_.joinable()) {
        capture_thread_.join();
    }

    if (enumerator_) {
        if (notifier_) {
            enumerator_->UnregisterEndpointNotificationCallback(notifier_);
            notifier_->Release();
            notifier_ = nullptr;
        }
        enumerator_->Release();
        enumerator_ = nullptr;
    }

    CoUninitialize();
}

bool WasapiCapture::InitEndpoint(
    IMMDeviceEnumerator* enumerator,
    IAudioClient** client,
    IAudioCaptureClient** capture_client,
    HANDLE event_handle,
    WAVEFORMATEX** mix_format) {

    if (!enumerator || !client || !capture_client || !mix_format) return false;

    *client = nullptr;
    *capture_client = nullptr;
    *mix_format = nullptr;

    IMMDevice* device = nullptr;
    HRESULT hr = enumerator->GetDefaultAudioEndpoint(eCapture, eCommunications, &device);
    if (FAILED(hr) || !device) {
        hr = enumerator->GetDefaultAudioEndpoint(eCapture, eConsole, &device);
    }
    if (FAILED(hr) || !device) {
        return false;
    }

    hr = device->Activate(
        __uuidof(IAudioClient),
        CLSCTX_ALL,
        nullptr,
        reinterpret_cast<void**>(client));
    device->Release();

    if (FAILED(hr) || !*client) {
        return false;
    }

    // Opportunistic Windows 11 AudioCategory_Communications opt-in via IAudioClient2
    IAudioClient2* client2 = nullptr;
    if (SUCCEEDED((*client)->QueryInterface(IID_IAudioClient2_Local, reinterpret_cast<void**>(&client2))) && client2) {
        AudioClientProperties props{};
        props.cbSize = sizeof(props);
        props.bIsOffload = FALSE;
        props.eCategory = AudioCategory_Communications;
        props.Options = static_cast<decltype(props.Options)>(0);
        client2->SetClientProperties(&props);
        client2->Release();
    }

    // Step 1: Attempt native 16 kHz 1-channel IEEE float shared-mode format via AUTOCONVERTPCM
    WAVEFORMATEXTENSIBLE wfx16k{};
    wfx16k.Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
    wfx16k.Format.nChannels = 1;
    wfx16k.Format.nSamplesPerSec = 16000;
    wfx16k.Format.wBitsPerSample = 32;
    wfx16k.Format.nBlockAlign = 4;
    wfx16k.Format.nAvgBytesPerSec = 16000 * 4;
    wfx16k.Format.cbSize = sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX);
    wfx16k.Samples.wValidBitsPerSample = 32;
    wfx16k.dwChannelMask = SPEAKER_FRONT_CENTER;
    wfx16k.SubFormat = kSubtypeIeeeFloat;

    DWORD stream_flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK |
                         AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM |
                         AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

    // Opportunistic IAudioClient3 low-latency shared-mode probing
    IAudioClient3* client3 = nullptr;
    bool client3_initialized = false;
    if (SUCCEEDED((*client)->QueryInterface(IID_IAudioClient3_Local, reinterpret_cast<void**>(&client3))) && client3) {
        UINT32 def_period = 0, fund_period = 0, min_period = 0, max_period = 0;
        HRESULT period_hr = client3->GetSharedModeEnginePeriod(
            reinterpret_cast<const WAVEFORMATEX*>(&wfx16k),
            &def_period, &fund_period, &min_period, &max_period);

        if (SUCCEEDED(period_hr) && min_period > 0) {
            // Safe clamp min_period to a floor of >= 2 ms (32 frames @ 16 kHz) to prevent DPC crackle on laptops
            const UINT32 min_safe_frames = static_cast<UINT32>(16000 * 0.002f);
            UINT32 safe_period = std::max(min_period, min_safe_frames);
            if (fund_period > 0) {
                safe_period = ((safe_period + fund_period - 1) / fund_period) * fund_period;
            }
            safe_period = std::clamp(safe_period, min_period, max_period);

            hr = client3->InitializeSharedAudioStream(
                stream_flags,
                safe_period,
                reinterpret_cast<const WAVEFORMATEX*>(&wfx16k),
                nullptr);

            if (SUCCEEDED(hr)) {
                *mix_format = reinterpret_cast<WAVEFORMATEX*>(CoTaskMemAlloc(sizeof(WAVEFORMATEXTENSIBLE)));
                if (*mix_format) {
                    std::memcpy(*mix_format, &wfx16k, sizeof(WAVEFORMATEXTENSIBLE));
                    native_sample_rate_ = 16000;
                    native_channels_ = 1;
                    negotiated_period_ms_.store(static_cast<float>(safe_period) * 1000.0f / 16000.0f, std::memory_order_relaxed);
                    client3_initialized = true;
                }
            }
        }
        client3->Release();
    }

    if (!client3_initialized) {
        hr = (*client)->Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            stream_flags,
            kBufferDuration100ns,
            0,
            reinterpret_cast<WAVEFORMATEX*>(&wfx16k),
            nullptr);

        if (SUCCEEDED(hr)) {
            *mix_format = reinterpret_cast<WAVEFORMATEX*>(CoTaskMemAlloc(sizeof(WAVEFORMATEXTENSIBLE)));
            if (!*mix_format) {
                (*client)->Release();
                *client = nullptr;
                return false;
            }
            std::memcpy(*mix_format, &wfx16k, sizeof(WAVEFORMATEXTENSIBLE));
            native_sample_rate_ = 16000;
            native_channels_ = 1;
            negotiated_period_ms_.store(10.0f, std::memory_order_relaxed);
        } else {
            // Fallback: query native device mix format
            hr = (*client)->GetMixFormat(mix_format);
            if (FAILED(hr) || !*mix_format) {
                (*client)->Release();
                *client = nullptr;
                return false;
            }

            native_sample_rate_ = (*mix_format)->nSamplesPerSec;
            native_channels_ = (*mix_format)->nChannels;

            hr = (*client)->Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                kBufferDuration100ns,
                0,
                *mix_format,
                nullptr);

            if (FAILED(hr)) {
                CoTaskMemFree(*mix_format);
                *mix_format = nullptr;
                (*client)->Release();
                *client = nullptr;
                return false;
            }
            negotiated_period_ms_.store(10.0f, std::memory_order_relaxed);
        }
    }

    // Opportunistic Windows 11 hardware/OS Acoustic Echo Cancellation opt-in
    IAcousticEchoCancellationControl* aec_ctrl = nullptr;
    HRESULT aec_hr = (*client)->GetService(
        IID_IAcousticEchoCancellationControl_Local,
        reinterpret_cast<void**>(&aec_ctrl));
    if (SUCCEEDED(aec_hr) && aec_ctrl) {
        HRESULT set_hr = aec_ctrl->SetEchoCancellationRenderEndpoint(nullptr);
        os_aec_active_.store(SUCCEEDED(set_hr), std::memory_order_relaxed);
        aec_ctrl->Release();
    } else {
        os_aec_active_.store(false, std::memory_order_relaxed);
    }

    hr = (*client)->SetEventHandle(event_handle);
    if (FAILED(hr)) {
        CoTaskMemFree(*mix_format);
        *mix_format = nullptr;
        (*client)->Release();
        *client = nullptr;
        return false;
    }

    hr = (*client)->GetService(
        __uuidof(IAudioCaptureClient),
        reinterpret_cast<void**>(capture_client));

    if (FAILED(hr) || !*capture_client) {
        CoTaskMemFree(*mix_format);
        *mix_format = nullptr;
        (*client)->Release();
        *client = nullptr;
        return false;
    }

    hr = (*client)->Start();
    if (FAILED(hr)) {
        (*capture_client)->Release();
        *capture_client = nullptr;
        CoTaskMemFree(*mix_format);
        *mix_format = nullptr;
        (*client)->Release();
        *client = nullptr;
        return false;
    }

    return true;
}

void WasapiCapture::CaptureThreadLoop() {
    CoInitializeEx(nullptr, COINIT_MULTITHREADED);

    // Apply MMCSS "Pro Audio" scheduling and pin to P-Cores (mask 0x0F)
    MmcssGuard mmcss(L"Pro Audio", 0x0F);

    HANDLE hAudioEvent = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    if (!hAudioEvent) {
        CoUninitialize();
        return;
    }

    IAudioClient* client = nullptr;
    IAudioCaptureClient* capture_client = nullptr;
    WAVEFORMATEX* mix_format = nullptr;

    int backoff_ms = 100;

    auto cleanup_endpoint = [&]() {
        if (client) {
            client->Stop();
        }
        if (capture_client) {
            capture_client->Release();
            capture_client = nullptr;
        }
        if (mix_format) {
            CoTaskMemFree(mix_format);
            mix_format = nullptr;
        }
        if (client) {
            client->Release();
            client = nullptr;
        }
    };

    while (running_.load(std::memory_order_relaxed)) {
        // Handle initial init or reinit after hot-swap / device invalidation
        if (!client || reinit_requested_.exchange(false, std::memory_order_acquire)) {
            cleanup_endpoint();

            if (!InitEndpoint(enumerator_, &client, &capture_client, hAudioEvent, &mix_format)) {
                // Bounded backoff retry: 100ms -> 200ms -> 400ms, capped at 2000ms
                std::this_thread::sleep_for(std::chrono::milliseconds(backoff_ms));
                backoff_ms = std::min(backoff_ms * 2, 2000);
                continue;
            }

            // Successful connection resets backoff and resampler state
            backoff_ms = 100;
            resample_phase_ = 0.0;
            last_input_sample_ = 0.0f;
            has_last_sample_ = false;
        }

        DWORD wait_res = WaitForSingleObject(hAudioEvent, 50);
        if (wait_res == WAIT_OBJECT_0) {
            UINT32 packet_length = 0;
            HRESULT hr = capture_client->GetNextPacketSize(&packet_length);

            while (SUCCEEDED(hr) && packet_length > 0 && running_.load(std::memory_order_relaxed)) {
                BYTE* pData = nullptr;
                UINT32 num_frames = 0;
                DWORD flags = 0;

                hr = capture_client->GetBuffer(&pData, &num_frames, &flags, nullptr, nullptr);
                if (SUCCEEDED(hr)) {
                    ProcessPacket(pData, num_frames, flags, mix_format);
                    capture_client->ReleaseBuffer(num_frames);
                } else {
                    break;
                }

                hr = capture_client->GetNextPacketSize(&packet_length);
            }

            if (hr == AUDCLNT_E_DEVICE_INVALIDATED ||
                hr == AUDCLNT_E_RESOURCES_INVALIDATED ||
                hr == AUDCLNT_E_SERVICE_NOT_RUNNING) {
                reinit_requested_.store(true, std::memory_order_release);
            }
        }
    }

    cleanup_endpoint();
    CloseHandle(hAudioEvent);
    CoUninitialize();
}

void WasapiCapture::ProcessPacket(
    const BYTE* data,
    UINT32 num_frames,
    DWORD flags,
    const WAVEFORMATEX* wfx) noexcept {

    if (num_frames == 0 || !wfx) return;

    if (flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY) {
        discontinuity_count_.fetch_add(1, std::memory_order_relaxed);
        resample_phase_ = 0.0;
        has_last_sample_ = false;
        last_input_sample_ = 0.0f;
    }

    if (flags & AUDCLNT_BUFFERFLAGS_SILENT) {
        size_t out_count = 0;
        if (native_sample_rate_ == 16000) {
            out_count = std::min<size_t>(num_frames, kMaxScratchSamples);
        } else {
            const double step = static_cast<double>(wfx->nSamplesPerSec) / kTargetSampleRate;
            out_count = std::min(static_cast<size_t>(num_frames / step), kMaxScratchSamples);
            resample_phase_ = std::fmod(resample_phase_ + num_frames, step);
            last_input_sample_ = 0.0f;
        }

        if (out_count > 0) {
            std::memset(resample_scratch_.data(), 0, out_count * sizeof(float));
            ring_buffer_.PushBatch(resample_scratch_.data(), out_count);
        }
        return;
    }

    const size_t mono_count = ConvertToMono(data, num_frames, wfx, mono_scratch_.data(), kMaxScratchSamples);
    if (mono_count == 0) return;

    const size_t resampled_count = ResampleTo16k(mono_scratch_.data(), static_cast<UINT32>(mono_count), resample_scratch_.data(), kMaxScratchSamples);
    if (resampled_count > 0) {
        ring_buffer_.PushBatch(resample_scratch_.data(), resampled_count);
    }
}

size_t WasapiCapture::ConvertToMono(
    const BYTE* data,
    UINT32 num_frames,
    const WAVEFORMATEX* wfx,
    float* mono_out,
    size_t max_out) noexcept {

    if (!data || num_frames == 0 || !wfx || !mono_out || max_out == 0) return 0;

    const UINT32 frames = std::min<UINT32>(num_frames, static_cast<UINT32>(max_out));
    const WORD channels = wfx->nChannels;
    const WORD bits = wfx->wBitsPerSample;

    bool is_float = (wfx->wFormatTag == WAVE_FORMAT_IEEE_FLOAT);
    if (wfx->wFormatTag == WAVE_FORMAT_EXTENSIBLE) {
        const auto* ext = reinterpret_cast<const WAVEFORMATEXTENSIBLE*>(wfx);
        if (ext->SubFormat == kSubtypeIeeeFloat) {
            is_float = true;
        }
    }

    if (channels == 1 && is_float && bits == 32) {
        std::memcpy(mono_out, data, frames * sizeof(float));
        return frames;
    }

    if (is_float && bits == 32) {
        const float* src = reinterpret_cast<const float*>(data);
        const float inv_channels = 1.0f / static_cast<float>(channels);
        for (UINT32 i = 0; i < frames; ++i) {
            float sum = 0.0f;
            for (WORD c = 0; c < channels; ++c) {
                sum += src[i * channels + c];
            }
            mono_out[i] = sum * inv_channels;
        }
    } else if (bits == 16) {
        const int16_t* src = reinterpret_cast<const int16_t*>(data);
        const float inv_scale = 1.0f / (32768.0f * static_cast<float>(channels));
        for (UINT32 i = 0; i < frames; ++i) {
            float sum = 0.0f;
            for (WORD c = 0; c < channels; ++c) {
                sum += static_cast<float>(src[i * channels + c]);
            }
            mono_out[i] = sum * inv_scale;
        }
    } else {
        std::memset(mono_out, 0, frames * sizeof(float));
    }

    return frames;
}

size_t WasapiCapture::ResampleTo16k(
    const float* mono_in,
    UINT32 num_frames,
    float* resampled_out,
    size_t max_out) noexcept {

    if (!mono_in || num_frames == 0 || !resampled_out || max_out == 0) return 0;

    // 1:1 pass-through if native rate is already 16 kHz
    if (native_sample_rate_ == 16000) {
        const size_t count = std::min<size_t>(num_frames, max_out);
        std::memcpy(resampled_out, mono_in, count * sizeof(float));
        return count;
    }

    const double in_rate = static_cast<double>(native_sample_rate_);
    const double step = in_rate / kTargetSampleRate; // e.g. 3.0 for 48k, 2.75625 for 44.1k
    size_t out_count = 0;

    if (!has_last_sample_) {
        last_input_sample_ = mono_in[0];
        has_last_sample_ = true;
        resample_phase_ = 0.0;
    }

    while (resample_phase_ < static_cast<double>(num_frames - 1) && out_count < max_out) {
        float sample = 0.0f;
        if (resample_phase_ < 0.0) {
            const float frac = static_cast<float>(resample_phase_ + 1.0);
            sample = last_input_sample_ * (1.0f - frac) + mono_in[0] * frac;
        } else {
            const int idx = static_cast<int>(std::floor(resample_phase_));
            const float frac = static_cast<float>(resample_phase_ - idx);
            sample = mono_in[idx] * (1.0f - frac) + mono_in[idx + 1] * frac;
        }

        resampled_out[out_count++] = sample;
        resample_phase_ += step;
    }

    resample_phase_ -= static_cast<double>(num_frames);
    last_input_sample_ = mono_in[num_frames - 1];

    return out_count;
}

} // namespace jarvis::audio
