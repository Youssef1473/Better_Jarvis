#include "test_assert.hpp"
#include <cmath>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <vector>

namespace {

constexpr double kTargetSampleRate = 16000.0;
constexpr size_t kMaxScratch = 65536;

struct ResamplerTestHarness {
    double resample_phase{0.0};
    float  last_input_sample{0.0f};
    bool   has_last_sample{false};

    void ResetDiscontinuity() noexcept {
        resample_phase = 0.0;
        last_input_sample = 0.0f;
        has_last_sample = false;
    }

    // Zero-allocation mono conversion matching wasapi_capture.cpp
    size_t ConvertToMono(
        const float* stereo_in,
        size_t num_frames,
        uint16_t channels,
        float* mono_out,
        size_t max_out) noexcept {

        if (!stereo_in || num_frames == 0 || !mono_out || max_out == 0) return 0;
        const size_t frames = std::min(num_frames, max_out);

        if (channels == 1) {
            std::memcpy(mono_out, stereo_in, frames * sizeof(float));
            return frames;
        }

        const float inv_channels = 1.0f / static_cast<float>(channels);
        for (size_t i = 0; i < frames; ++i) {
            float sum = 0.0f;
            for (uint16_t c = 0; c < channels; ++c) {
                sum += stereo_in[i * channels + c];
            }
            mono_out[i] = sum * inv_channels;
        }
        return frames;
    }

    // Zero-allocation linear resampler matching wasapi_capture.cpp
    size_t ResampleTo16k(
        const float* mono_in,
        size_t num_frames,
        double native_rate,
        float* resampled_out,
        size_t max_out) noexcept {

        if (!mono_in || num_frames == 0 || !resampled_out || max_out == 0) return 0;

        if (std::abs(native_rate - 16000.0) < 1e-3) {
            const size_t count = std::min(num_frames, max_out);
            std::memcpy(resampled_out, mono_in, count * sizeof(float));
            return count;
        }

        const double step = native_rate / kTargetSampleRate;
        size_t out_count = 0;

        if (!has_last_sample) {
            last_input_sample = mono_in[0];
            has_last_sample = true;
            resample_phase = 0.0;
        }

        while (resample_phase < static_cast<double>(num_frames - 1) && out_count < max_out) {
            float sample = 0.0f;
            if (resample_phase < 0.0) {
                const float frac = static_cast<float>(resample_phase + 1.0);
                sample = last_input_sample * (1.0f - frac) + mono_in[0] * frac;
            } else {
                const int idx = static_cast<int>(std::floor(resample_phase));
                const float frac = static_cast<float>(resample_phase - idx);
                sample = mono_in[idx] * (1.0f - frac) + mono_in[idx + 1] * frac;
            }

            resampled_out[out_count++] = sample;
            resample_phase += step;
        }

        resample_phase -= static_cast<double>(num_frames);
        last_input_sample = mono_in[num_frames - 1];
        return out_count;
    }
};

void Test48kStereoResampling() {
    ResamplerTestHarness harness;
    std::vector<float> mono_scratch(kMaxScratch);
    std::vector<float> resample_scratch(kMaxScratch);

    // 48 kHz stereo to 16 kHz mono: exact 3:1 ratio
    // Feed 10 packets of 480 stereo frames each (960 floats per packet)
    // 480 frames @ 48 kHz = 10ms. Each packet should yield 160 mono output samples.
    size_t total_out = 0;
    for (int packet = 0; packet < 10; ++packet) {
        std::vector<float> stereo_in(480 * 2);
        for (size_t i = 0; i < 480; ++i) {
            stereo_in[i * 2 + 0] = 0.6f;
            stereo_in[i * 2 + 1] = 0.4f;
        }

        size_t mono_count = harness.ConvertToMono(
            stereo_in.data(), 480, 2, mono_scratch.data(), kMaxScratch);
        TEST_ASSERT(mono_count == 480);

        for (size_t i = 0; i < mono_count; ++i) {
            TEST_ASSERT(std::abs(mono_scratch[i] - 0.5f) < 1e-4f);
        }

        size_t out_count = harness.ResampleTo16k(
            mono_scratch.data(), mono_count, 48000.0, resample_scratch.data(), kMaxScratch);
        TEST_ASSERT(out_count == 160);
        total_out += out_count;

        for (size_t i = 0; i < out_count; ++i) {
            TEST_ASSERT(std::abs(resample_scratch[i] - 0.5f) < 1e-4f);
        }
    }
    TEST_ASSERT(total_out == 1600);
}

void Test441kStereoResamplingPhaseContinuity() {
    ResamplerTestHarness harness;
    std::vector<float> mono_scratch(kMaxScratch);
    std::vector<float> resample_scratch(kMaxScratch);

    // 44.1 kHz stereo to 16 kHz mono: 2.75625 ratio
    // 10 packets of 441 frames = 4410 frames = 0.1s
    // Output must equal exactly 1600 samples (0.1s @ 16 kHz)
    size_t total_out = 0;
    for (int packet = 0; packet < 10; ++packet) {
        std::vector<float> stereo_in(441 * 2);
        for (size_t i = 0; i < 441; ++i) {
            stereo_in[i * 2 + 0] = 0.8f;
            stereo_in[i * 2 + 1] = 0.2f;
        }

        size_t mono_count = harness.ConvertToMono(
            stereo_in.data(), 441, 2, mono_scratch.data(), kMaxScratch);
        TEST_ASSERT(mono_count == 441);

        size_t out_count = harness.ResampleTo16k(
            mono_scratch.data(), mono_count, 44100.0, resample_scratch.data(), kMaxScratch);
        total_out += out_count;

        for (size_t i = 0; i < out_count; ++i) {
            TEST_ASSERT(std::abs(resample_scratch[i] - 0.5f) < 1e-4f);
        }
    }
    TEST_ASSERT(total_out == 1600);
}

void Test16kPassThrough() {
    ResamplerTestHarness harness;
    std::vector<float> mono_scratch(kMaxScratch);
    std::vector<float> resample_scratch(kMaxScratch);

    // 16 kHz mono native negotiation -> 1:1 pass-through
    std::vector<float> in(320);
    for (size_t i = 0; i < in.size(); ++i) {
        in[i] = static_cast<float>(i) * 0.01f;
    }

    size_t out_count = harness.ResampleTo16k(
        in.data(), in.size(), 16000.0, resample_scratch.data(), kMaxScratch);
    TEST_ASSERT(out_count == 320);
    for (size_t i = 0; i < 320; ++i) {
        TEST_ASSERT(resample_scratch[i] == in[i]);
    }
}

void TestDiscontinuityReset() {
    ResamplerTestHarness harness;
    std::vector<float> scratch(kMaxScratch);

    // Feed a burst at 44.1 kHz
    std::vector<float> burst1(441, 0.7f);
    size_t out1 = harness.ResampleTo16k(burst1.data(), burst1.size(), 44100.0, scratch.data(), kMaxScratch);
    TEST_ASSERT(out1 > 0);
    TEST_ASSERT(harness.has_last_sample);

    // Signal discontinuity
    harness.ResetDiscontinuity();
    TEST_ASSERT(!harness.has_last_sample);
    TEST_ASSERT(harness.resample_phase == 0.0);
    TEST_ASSERT(harness.last_input_sample == 0.0f);

    // Next packet starts cleanly with no leftover phase corruption
    std::vector<float> burst2(441, 0.3f);
    size_t out2 = harness.ResampleTo16k(burst2.data(), burst2.size(), 44100.0, scratch.data(), kMaxScratch);
    TEST_ASSERT(out2 > 0);
    for (size_t i = 0; i < out2; ++i) {
        TEST_ASSERT(std::abs(scratch[i] - 0.3f) < 1e-4f);
    }
}

void TestSineWaveContinuousPhaseNoClicks() {
    ResamplerTestHarness harness;
    std::vector<float> scratch(kMaxScratch);

    // 440 Hz test tone sampled at 48 kHz
    constexpr double freq = 440.0;
    constexpr double pi = 3.14159265358979323846;

    std::vector<float> full_signal(4800); // 100 ms @ 48 kHz
    for (size_t i = 0; i < full_signal.size(); ++i) {
        full_signal[i] = static_cast<float>(std::sin(2.0 * pi * freq * (static_cast<double>(i) / 48000.0)));
    }

    // Process in small 240-frame chunks across 20 iterations
    std::vector<float> output;
    for (size_t chunk = 0; chunk < 20; ++chunk) {
        size_t n = harness.ResampleTo16k(
            &full_signal[chunk * 240], 240, 48000.0, scratch.data(), kMaxScratch);
        output.insert(output.end(), scratch.begin(), scratch.begin() + n);
    }

    // Output must be exactly 1600 samples (100 ms @ 16 kHz)
    TEST_ASSERT(output.size() == 1600);

    // Verify there are no boundary clicks: maximum sample-to-sample difference
    // in 440 Hz sine wave @ 16 kHz is bounded by 2 * pi * 440 / 16000 ~ 0.173
    for (size_t i = 1; i < output.size(); ++i) {
        float delta = std::abs(output[i] - output[i - 1]);
        TEST_ASSERT(delta < 0.25f);
    }
}

} // namespace

int main() {
    std::cout << "[TEST] Running Test48kStereoResampling..." << std::endl;
    Test48kStereoResampling();

    std::cout << "[TEST] Running Test441kStereoResamplingPhaseContinuity..." << std::endl;
    Test441kStereoResamplingPhaseContinuity();

    std::cout << "[TEST] Running Test16kPassThrough..." << std::endl;
    Test16kPassThrough();

    std::cout << "[TEST] Running TestDiscontinuityReset..." << std::endl;
    TestDiscontinuityReset();

    std::cout << "[TEST] Running TestSineWaveContinuousPhaseNoClicks..." << std::endl;
    TestSineWaveContinuousPhaseNoClicks();

    std::cout << "[PASS] All Resampler tests passed successfully!" << std::endl;
    return 0;
}
