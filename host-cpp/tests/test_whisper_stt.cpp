#include "test_assert.hpp"
#include <cmath>
#include <filesystem>
#include <iostream>
#include <vector>
#include <windows.h>
#include "audio/whisper_stt.hpp"

using namespace jarvis::audio;

void TestSanitizeTranscript() {
    TEST_ASSERT(WhisperStt::SanitizeTranscript("   hello world   ") == "hello world");
    TEST_ASSERT(WhisperStt::SanitizeTranscript("\n\t\"open youtube\"\r\n") == "open youtube");
    TEST_ASSERT(WhisperStt::SanitizeTranscript("   ") == "");
    TEST_ASSERT(WhisperStt::SanitizeTranscript("\"\"") == "");
    TEST_ASSERT(WhisperStt::SanitizeTranscript("volume up") == "volume up");
}

void TestHallucinationFilter() {
    // Classic Whisper silent hallucinations must be rejected
    TEST_ASSERT(WhisperStt::IsHallucination("[BLANK_AUDIO]"));
    TEST_ASSERT(WhisperStt::IsHallucination("[blank_audio]"));
    TEST_ASSERT(WhisperStt::IsHallucination("(silence)"));
    TEST_ASSERT(WhisperStt::IsHallucination("[Music]"));
    TEST_ASSERT(WhisperStt::IsHallucination("(music)"));
    TEST_ASSERT(WhisperStt::IsHallucination("[applause]"));
    TEST_ASSERT(WhisperStt::IsHallucination("Thanks for watching!"));
    TEST_ASSERT(WhisperStt::IsHallucination("Thank you."));
    TEST_ASSERT(WhisperStt::IsHallucination("..."));
    TEST_ASSERT(WhisperStt::IsHallucination("."));
    TEST_ASSERT(WhisperStt::IsHallucination("???"));
    TEST_ASSERT(WhisperStt::IsHallucination(""));

    // Real voice commands must pass
    TEST_ASSERT(!WhisperStt::IsHallucination("open youtube"));
    TEST_ASSERT(!WhisperStt::IsHallucination("what is my schedule today?"));
    TEST_ASSERT(!WhisperStt::IsHallucination("turn the volume down"));
    TEST_ASSERT(!WhisperStt::IsHallucination("git status"));
}

void TestQueueBoundingAndBargeIn() {
    WhisperStt stt;

    // Enqueue 5 utterances
    for (int i = 0; i < 5; ++i) {
        std::vector<float> pcm(16000, static_cast<float>(i));
        stt.EnqueueUtterance(std::move(pcm));
    }

    // Clearing queue on barge-in
    stt.ClearQueue();
}

void TestRealWhisperStt() {
    std::filesystem::path model_path;
    if (std::filesystem::exists("models/ggml-base.en.bin")) {
        model_path = "models/ggml-base.en.bin";
    } else {
        wchar_t local_app_data[MAX_PATH]{};
        GetEnvironmentVariableW(L"LOCALAPPDATA", local_app_data, MAX_PATH);
        model_path = std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"models" / L"ggml-base.en.bin";
    }

    if (!std::filesystem::exists(model_path)) {
        std::cout << "[test_whisper_stt] ggml-base.en.bin not found at " << model_path << ", skipping real model check" << std::endl;
        return;
    }

    WhisperStt stt(model_path.string());
    TEST_ASSERT(stt.IsModelLoaded());

    // Generate 1 second of synthetic 16 kHz audio
    std::vector<float> audio(16000, 0.0f);
    for (size_t i = 0; i < audio.size(); ++i) {
        audio[i] = 0.2f * std::sin(2.0 * 3.1415926535 * 440.0 * i / 16000.0);
    }

    std::string text = stt.Transcribe(audio);
    std::cout << "  [test_whisper_stt] Real Whisper inference ran cleanly (result: \"" << text << "\")" << std::endl;
}

int main() {
    TestSanitizeTranscript();
    TestHallucinationFilter();
    TestQueueBoundingAndBargeIn();
    TestRealWhisperStt();
    std::cout << "All WhisperStt tests PASSED!" << std::endl;
    return 0;
}
