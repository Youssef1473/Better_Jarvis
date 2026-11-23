#include <cassert>
#include <cmath>
#include <iostream>
#include <vector>

#include "audio/piper_tts.hpp"

using namespace jarvis::audio;

void TestUpsample22kTo48kStereo() {
    // 22050 Hz to 48000 Hz stereo
    // 2205 samples @ 22050 Hz = 0.1 s
    // Should produce exactly 4800 frames = 9600 interleaved samples @ 48000 Hz stereo
    std::vector<float> mono(2205, 0.75f);
    std::vector<float> out_stereo;

    PiperTts::UpsampleToDevice(mono.data(), mono.size(), 48000, 2, out_stereo);

    assert(out_stereo.size() == 4800 * 2);

    for (size_t i = 0; i < 4800; ++i) {
        float left = out_stereo[i * 2 + 0];
        float right = out_stereo[i * 2 + 1];
        assert(std::abs(left - 0.75f) < 1e-4f);
        assert(std::abs(right - 0.75f) < 1e-4f);
    }
}

void TestBargeInEpochIncrement() {
    PiperTts tts;
    assert(tts.CurrentEpoch() == 0);

    tts.BargeIn();
    assert(tts.CurrentEpoch() == 1);

    tts.BargeIn();
    assert(tts.CurrentEpoch() == 2);
}

int main() {
    TestUpsample22kTo48kStereo();
    TestBargeInEpochIncrement();
    std::cout << "All PiperTts tests PASSED!" << std::endl;
    return 0;
}
