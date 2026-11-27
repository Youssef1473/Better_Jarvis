#include "test_assert.hpp"
#include <chrono>
#include <cmath>
#include <filesystem>
#include <iostream>
#include <vector>
#include <windows.h>

#include "audio/piper_tts.hpp"

using namespace jarvis::audio;

void TestUpsample22kTo48kStereo() {
    // 22050 Hz to 48000 Hz stereo
    // 2205 samples @ 22050 Hz = 0.1 s
    // Should produce exactly 4800 frames = 9600 interleaved samples @ 48000 Hz stereo
    std::vector<float> mono(2205, 0.75f);
    std::vector<float> out_stereo;

    PiperTts::UpsampleToDevice(mono.data(), mono.size(), 48000, 2, out_stereo);

    TEST_ASSERT(out_stereo.size() == 4800 * 2);

    for (size_t i = 0; i < 4800; ++i) {
        float left = out_stereo[i * 2 + 0];
        float right = out_stereo[i * 2 + 1];
        TEST_ASSERT(std::abs(left - 0.75f) < 1e-4f);
        TEST_ASSERT(std::abs(right - 0.75f) < 1e-4f);
    }
}

void TestBargeInEpochIncrement() {
    PiperTts tts;
    TEST_ASSERT(tts.CurrentEpoch() == 0);

    tts.BargeIn();
    TEST_ASSERT(tts.CurrentEpoch() == 1);

    tts.BargeIn();
    TEST_ASSERT(tts.CurrentEpoch() == 2);
}

void TestRealPiperSynthesis() {
    std::filesystem::path model_path;
    std::filesystem::path config_path;
    if (std::filesystem::exists("models/en_US-ryan-medium.onnx")) {
        model_path = "models/en_US-ryan-medium.onnx";
        config_path = "models/en_US-ryan-medium.onnx.json";
    } else {
        wchar_t local_app_data[MAX_PATH]{};
        GetEnvironmentVariableW(L"LOCALAPPDATA", local_app_data, MAX_PATH);
        model_path = std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"models" / L"en_US-ryan-medium.onnx";
        config_path = std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"models" / L"en_US-ryan-medium.onnx.json";
    }

    if (!std::filesystem::exists(model_path)) {
        std::cout << "[test_piper_tts] en_US-ryan-medium.onnx not found at " << model_path << ", skipping real model check" << std::endl;
        return;
    }

    PiperTts tts(model_path.string(), config_path.string());
    TEST_ASSERT(tts.IsModelLoaded());
    TEST_ASSERT(tts.TargetDeviceName() == L"default");

    std::vector<float> pcm;
    bool ok = tts.Synthesize("Hello, systems online.", pcm);
    TEST_ASSERT(ok);
    TEST_ASSERT(pcm.size() > 10000);
    std::cout << "  [test_piper_tts] Real Piper synthesis generated " << pcm.size()
              << " samples @ 22050 Hz (> 10000, non-chime)" << std::endl;

    // Verify sub-20ms StopPlayback() interruptibility
    auto t0 = std::chrono::high_resolution_clock::now();
    tts.StopPlayback();
    auto t1 = std::chrono::high_resolution_clock::now();
    auto stop_us = std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count();
    TEST_ASSERT(stop_us < 20000); // Must halt under 20ms
    std::cout << "  [test_piper_tts] StopPlayback completed in " << stop_us << " us (< 20 ms)" << std::endl;
}

void TestOutputDeviceAndHotSwapInvalidation() {
    PiperTts tts("", "", L"Headphones");
    TEST_ASSERT(tts.TargetDeviceName() == L"Headphones");
    tts.SetTargetDeviceName(L"Speakers");
    TEST_ASSERT(tts.TargetDeviceName() == L"Speakers");

    TEST_ASSERT(!tts.IsReinitRequested());
    tts.RequestReinit();
    TEST_ASSERT(tts.IsReinitRequested());

    TEST_ASSERT(PiperTts::IsDeviceInvalidatedHresult(AUDCLNT_E_DEVICE_INVALIDATED));
    TEST_ASSERT(PiperTts::IsDeviceInvalidatedHresult(static_cast<HRESULT>(0x88890026L)));
    TEST_ASSERT(PiperTts::IsDeviceInvalidatedHresult(static_cast<HRESULT>(0x88890010L)));
    TEST_ASSERT(!PiperTts::IsDeviceInvalidatedHresult(S_OK));
}

int main() {
    TestUpsample22kTo48kStereo();
    TestBargeInEpochIncrement();
    TestOutputDeviceAndHotSwapInvalidation();
    TestRealPiperSynthesis();
    std::cout << "All PiperTts tests PASSED!" << std::endl;
    return 0;
}
