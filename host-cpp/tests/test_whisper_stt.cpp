#include <cassert>
#include <iostream>
#include <vector>
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

int main() {
    TestSanitizeTranscript();
    TestHallucinationFilter();
    TestQueueBoundingAndBargeIn();
    std::cout << "All WhisperStt tests PASSED!" << std::endl;
    return 0;
}
