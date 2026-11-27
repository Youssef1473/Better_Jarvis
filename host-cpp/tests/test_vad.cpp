#include "test_assert.hpp"
#include <cmath>
#include <filesystem>
#include <iostream>
#include <vector>
#include <windows.h>

#include "audio/ring_buffer.hpp"
#include "audio/silero_vad.hpp"

using namespace jarvis::audio;

namespace {

constexpr double kPi = 3.14159265358979323846;

// Generates a 512-sample test frame of a simulated voice tone (250 Hz fundamental + harmonics)
std::vector<float> GenerateSpeechFrame(float amplitude = 0.25f) {
    std::vector<float> frame(SileroVad::kFrameSize);
    for (size_t i = 0; i < SileroVad::kFrameSize; ++i) {
        // 250 Hz tone @ 16 kHz has period of 64 samples
        double t = static_cast<double>(i) / 16000.0;
        double s = std::sin(2.0 * kPi * 250.0 * t) + 0.3 * std::sin(2.0 * kPi * 500.0 * t);
        frame[i] = static_cast<float>(s * amplitude);
    }
    return frame;
}

// Generates a 512-sample test frame of silence / low noise
std::vector<float> GenerateSilenceFrame(float noise_amp = 0.0005f) {
    std::vector<float> frame(SileroVad::kFrameSize);
    for (size_t i = 0; i < SileroVad::kFrameSize; ++i) {
        frame[i] = ((i % 2 == 0) ? 1.0f : -1.0f) * noise_amp;
    }
    return frame;
}

void TestSilenceDoesNotTriggerSpeech() {
    SpscRingBuffer<32768> ring_buffer;
    SileroVad vad(ring_buffer);

    bool utterance_fired = false;
    vad.SetUtteranceCallback([&](std::vector<float>&&) {
        utterance_fired = true;
    });

    // Feed 30 frames of silence
    auto silence = GenerateSilenceFrame();
    for (int i = 0; i < 30; ++i) {
        ring_buffer.PushBatch(silence.data(), silence.size());
        bool processed = vad.ProcessNextFrame();
        TEST_ASSERT(processed);
        TEST_ASSERT(!vad.IsInSpeech());
    }

    TEST_ASSERT(!utterance_fired);
}

void TestSpeechOnsetPreRollAndEndpointing() {
    SpscRingBuffer<32768> ring_buffer;
    SileroVad vad(ring_buffer);

    std::vector<float> received_utterance;
    int callbacks_count = 0;
    vad.SetUtteranceCallback([&](std::vector<float>&& utterance) {
        received_utterance = std::move(utterance);
        callbacks_count++;
    });

    auto silence = GenerateSilenceFrame();
    auto speech = GenerateSpeechFrame(0.3f);

    // 1. Prime the ring buffer with 30 frames of silence (establishes noise floor and pre-roll)
    for (int i = 0; i < 30; ++i) {
        ring_buffer.PushBatch(silence.data(), silence.size());
        vad.ProcessNextFrame();
    }
    TEST_ASSERT(!vad.IsInSpeech());

    // 2. Feed 10 frames of speech
    for (int i = 0; i < 10; ++i) {
        ring_buffer.PushBatch(speech.data(), speech.size());
        vad.ProcessNextFrame();
    }
    TEST_ASSERT(vad.IsInSpeech());

    // 3. Feed 22 frames of trailing silence to trigger SPEECH_END (> 21 frames)
    for (int i = 0; i < 22; ++i) {
        ring_buffer.PushBatch(silence.data(), silence.size());
        vad.ProcessNextFrame();
    }

    TEST_ASSERT(!vad.IsInSpeech());
    TEST_ASSERT(callbacks_count == 1);

    // Utterance must contain pre-roll + speech frames
    // 800 ms pre-roll = 12,800 samples + 10 speech frames * 512 = 5,120 samples
    TEST_ASSERT(!received_utterance.empty());
    TEST_ASSERT(received_utterance.size() >= 12800);
}

void TestMaxUtteranceCap() {
    SpscRingBuffer<32768> ring_buffer;
    SileroVad vad(ring_buffer);

    int callbacks_count = 0;
    vad.SetUtteranceCallback([&](std::vector<float>&&) {
        callbacks_count++;
    });

    auto speech = GenerateSpeechFrame(0.35f);

    // Feed continuous speech beyond 15 seconds (kMaxUtteranceSamples = 240,000 samples = ~468 frames)
    for (int i = 0; i < 500; ++i) {
        ring_buffer.PushBatch(speech.data(), speech.size());
        vad.ProcessNextFrame();
        if (callbacks_count > 0) {
            break;
        }
    }

    // Safety cap must have fired
    TEST_ASSERT(callbacks_count == 1);
    TEST_ASSERT(!vad.IsInSpeech());
}

void TestRealOnnxVad() {
    std::filesystem::path model_path;
    if (std::filesystem::exists("models/silero_vad.onnx")) {
        model_path = "models/silero_vad.onnx";
    } else {
        wchar_t local_app_data[MAX_PATH]{};
        GetEnvironmentVariableW(L"LOCALAPPDATA", local_app_data, MAX_PATH);
        model_path = std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"models" / L"silero_vad.onnx";
    }

    if (!std::filesystem::exists(model_path)) {
        std::cout << "[test_vad] silero_vad.onnx not found at " << model_path << ", skipping real model check" << std::endl;
        return;
    }

    SpscRingBuffer<32768> ring_buffer;
    SileroVad vad(ring_buffer, model_path.wstring());
    TEST_ASSERT(vad.IsOnnxLoaded());

    auto silence = GenerateSilenceFrame();
    for (int i = 0; i < 100; ++i) {
        float prob = vad.EvaluateFrame(silence.data());
        TEST_ASSERT(prob < 0.15f);
    }
    std::cout << "  [test_vad] Real Silero VAD ONNX verified (100 silence frames < 0.15)" << std::endl;
}

} // namespace

int main() {
    TestSilenceDoesNotTriggerSpeech();
    TestSpeechOnsetPreRollAndEndpointing();
    TestMaxUtteranceCap();
    TestRealOnnxVad();
    std::cout << "All SileroVad tests PASSED!" << std::endl;
    return 0;
}
