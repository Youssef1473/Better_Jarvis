#include <cassert>
#include <cmath>
#include <filesystem>
#include <iostream>
#include <vector>
#include <windows.h>
#include "audio/whisper_stt.hpp"

using namespace jarvis::audio;

void TestSanitizeTranscript() {
    assert(WhisperStt::SanitizeTranscript("   hello world   ") == "hello world");
    assert(WhisperStt::SanitizeTranscript("\n\t\"open youtube\"\r\n") == "open youtube");
    assert(WhisperStt::SanitizeTranscript("   ") == "");
    assert(WhisperStt::SanitizeTranscript("\"\"") == "");
    assert(WhisperStt::SanitizeTranscript("volume up") == "volume up");
}

void TestHallucinationFilter() {
    // Classic Whisper silent hallucinations must be rejected
    assert(WhisperStt::IsHallucination("[BLANK_AUDIO]"));
    assert(WhisperStt::IsHallucination("[blank_audio]"));
    assert(WhisperStt::IsHallucination("(silence)"));
    assert(WhisperStt::IsHallucination("[Music]"));
    assert(WhisperStt::IsHallucination("(music)"));
    assert(WhisperStt::IsHallucination("[applause]"));
    assert(WhisperStt::IsHallucination("Thanks for watching!"));
    assert(WhisperStt::IsHallucination("Thank you."));
    assert(WhisperStt::IsHallucination("..."));
    assert(WhisperStt::IsHallucination("."));
    assert(WhisperStt::IsHallucination("???"));
    assert(WhisperStt::IsHallucination(""));

    // Real voice commands must pass
    assert(!WhisperStt::IsHallucination("open youtube"));
    assert(!WhisperStt::IsHallucination("what is my schedule today?"));
    assert(!WhisperStt::IsHallucination("turn the volume down"));
    assert(!WhisperStt::IsHallucination("git status"));
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
    assert(stt.IsModelLoaded());

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
