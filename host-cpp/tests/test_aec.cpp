#include <cassert>
#include <chrono>
#include <cmath>
#include <iostream>
#include <vector>

#include "audio/aec_processor.hpp"
#include "audio/piper_tts.hpp"

using namespace jarvis::audio;

namespace {

constexpr double kPi = 3.14159265358979323846;

// Synthesizes speech-like multi-tone audio with varying envelope
std::vector<float> GenerateSpeechLikeSignal(size_t num_samples) {
    std::vector<float> sig(num_samples);
    for (size_t i = 0; i < num_samples; ++i) {
        double t = static_cast<double>(i) / 16000.0;
        // Low-frequency vocal cadence modulation
        double cadence = 0.5 + 0.5 * std::sin(2.0 * kPi * 3.5 * t);
        // Formant mixture
        double s = 0.40 * std::sin(2.0 * kPi * 320.0 * t) +
                   0.30 * std::sin(2.0 * kPi * 680.0 * t) +
                   0.20 * std::sin(2.0 * kPi * 1400.0 * t) +
                   0.10 * std::sin(2.0 * kPi * 2800.0 * t);
        sig[i] = static_cast<float>(s * cadence * 0.4);
    }
    return sig;
}

// Simulates room impulse response with direct acoustic flight delay + early reflections
std::vector<float> SimulateAcousticEcho(const std::vector<float>& ref) {
    std::vector<float> mic(ref.size(), 0.0f);
    // Acoustic delay: 160 samples (10 ms)
    // Direct path: delay 160, gain 0.50
    // Reflection 1: delay 220, gain -0.22
    // Reflection 2: delay 310, gain 0.14
    for (size_t i = 0; i < ref.size(); ++i) {
        float sample = 0.0f;
        if (i >= 160) sample += 0.50f * ref[i - 160];
        if (i >= 220) sample -= 0.22f * ref[i - 220];
        if (i >= 310) sample += 0.14f * ref[i - 310];
        mic[i] = sample;
    }
    return mic;
}

} // namespace

void TestEchoCancellationConvergence() {
    AecProcessor aec;
    aec.SetBulkDelaySamples(160); // Match simulated direct acoustic path

    // 1.5 seconds of far-end reference audio (150 frames @ 160 samples = 24,000 samples)
    constexpr size_t kTotalFrames = 150;
    constexpr size_t kTotalSamples = kTotalFrames * AecProcessor::kFrameSize;

    std::vector<float> ref = GenerateSpeechLikeSignal(kTotalSamples);
    std::vector<float> mic = SimulateAcousticEcho(ref);
    std::vector<float> cleaned(kTotalSamples, 0.0f);

    float initial_mic_energy = 0.0f;
    float converged_err_energy = 0.0f;
    float converged_mic_energy = 0.0f;

    for (size_t f = 0; f < kTotalFrames; ++f) {
        const size_t offset = f * AecProcessor::kFrameSize;
        aec.ProcessFrame(&mic[offset], &ref[offset], &cleaned[offset], true);

        // Measure initial energy in first 10 frames
        if (f < 10) {
            for (size_t i = 0; i < AecProcessor::kFrameSize; ++i) {
                initial_mic_energy += mic[offset + i] * mic[offset + i];
            }
        }

        // Measure energy after convergence (frames 80 to 140)
        if (f >= 80 && f < 140) {
            for (size_t i = 0; i < AecProcessor::kFrameSize; ++i) {
                converged_mic_energy += mic[offset + i] * mic[offset + i];
                converged_err_energy += cleaned[offset + i] * cleaned[offset + i];
            }
        }
    }

    assert(converged_mic_energy > 0.0f);
    assert(converged_err_energy > 0.0f);

    // Compute empirical ERLE in dB: 10 * log10(E_mic / E_err)
    const float measured_erle = 10.0f * std::log10(converged_mic_energy / converged_err_energy);
    std::cout << "  [Convergence] Measured ERLE: " << measured_erle
              << " dB (Filter Telemetry: " << aec.LastErleDb() << " dB)" << std::endl;

    // Requirement: Attenuation >= 12 dB
    assert(measured_erle >= 12.0f);
    // DTD must not trigger false positive on pure echo
    assert(!aec.IsDoubleTalk());
}

void TestDoubleTalkAndBargeInPreservation() {
    AecProcessor aec;
    aec.SetBulkDelaySamples(160);

    // 1. First adapt on 80 frames of pure echo
    constexpr size_t kAdaptFrames = 80;
    std::vector<float> ref_adapt = GenerateSpeechLikeSignal(kAdaptFrames * AecProcessor::kFrameSize);
    std::vector<float> mic_adapt = SimulateAcousticEcho(ref_adapt);
    std::vector<float> clean_adapt(kAdaptFrames * AecProcessor::kFrameSize);

    for (size_t f = 0; f < kAdaptFrames; ++f) {
        const size_t offset = f * AecProcessor::kFrameSize;
        aec.ProcessFrame(&mic_adapt[offset], &ref_adapt[offset], &clean_adapt[offset], true);
    }

    // 2. Inject strong near-end user voice on top of echo for 30 frames (4800 samples)
    constexpr size_t kDtFrames = 30;
    constexpr size_t kDtSamples = kDtFrames * AecProcessor::kFrameSize;

    std::vector<float> ref_dt = GenerateSpeechLikeSignal(kDtSamples);
    std::vector<float> echo_dt = SimulateAcousticEcho(ref_dt);
    std::vector<float> voice_near(kDtSamples);

    for (size_t i = 0; i < kDtSamples; ++i) {
        double t = static_cast<double>(i) / 16000.0;
        // High-energy voice signal (amplitude ~0.65)
        voice_near[i] = static_cast<float>(0.65 * std::sin(2.0 * kPi * 450.0 * t));
    }

    std::vector<float> mic_dt(kDtSamples);
    for (size_t i = 0; i < kDtSamples; ++i) {
        mic_dt[i] = echo_dt[i] + voice_near[i];
    }

    std::vector<float> clean_dt(kDtSamples);
    bool dtd_triggered = false;
    float voice_energy = 0.0f;
    float output_energy = 0.0f;

    for (size_t f = 0; f < kDtFrames; ++f) {
        const size_t offset = f * AecProcessor::kFrameSize;
        aec.ProcessFrame(&mic_dt[offset], &ref_dt[offset], &clean_dt[offset], true);

        if (aec.IsDoubleTalk()) {
            dtd_triggered = true;
        }

        for (size_t i = 0; i < AecProcessor::kFrameSize; ++i) {
            voice_energy += voice_near[offset + i] * voice_near[offset + i];
            output_energy += clean_dt[offset + i] * clean_dt[offset + i];
        }
    }

    std::cout << "  [DoubleTalk] DTD Triggered: " << (dtd_triggered ? "YES" : "NO")
              << " | Voice Energy: " << voice_energy
              << " | Output Energy: " << output_energy << std::endl;

    // Requirement: DTD must identify double-talk
    assert(dtd_triggered);
    // Requirement: Near-end human voice must NOT be suppressed (output retains at least 50% energy)
    assert(output_energy >= 0.50f * voice_energy);

    // 3. Verify filter weights did not diverge or produce NaN
    for (size_t f = 0; f < 10; ++f) {
        float test_mic[160]{0.0f};
        float test_ref[160]{0.0f};
        float test_out[160]{0.0f};
        aec.ProcessFrame(test_mic, test_ref, test_out, true);
        for (int i = 0; i < 160; ++i) {
            assert(!std::isnan(test_out[i]));
            assert(!std::isinf(test_out[i]));
        }
    }
}

void TestStopPlaybackLatencyAndTiming() {
    PiperTts tts;

    // 1. Sub-20ms StopPlayback() / Barge-In Latency Benchmark
    tts.Speak("Hello Jarvis, this is an active speech test for latency verification.");

    const auto t_start = std::chrono::high_resolution_clock::now();
    tts.StopPlayback();
    const auto t_end = std::chrono::high_resolution_clock::now();

    const double stop_duration_ms = std::chrono::duration<double, std::milli>(t_end - t_start).count();
    std::cout << "  [Latency] StopPlayback() elapsed: " << stop_duration_ms << " ms" << std::endl;

    // Requirement: StopPlayback() must complete in strictly under 20 ms
    assert(stop_duration_ms < 20.0);
    assert(!tts.IsSpeaking());
    assert(tts.FarEndReferenceBuffer().AvailableRead() == 0);

    // 2. Benchmark 1,000 consecutive ProcessFrame calls for zero-alloc real-time safety
    AecProcessor aec;
    float mic_frame[160]{0.1f};
    float ref_frame[160]{0.1f};
    float out_frame[160]{0.0f};

    const auto aec_start = std::chrono::high_resolution_clock::now();
    for (int i = 0; i < 1000; ++i) {
        aec.ProcessFrame(mic_frame, ref_frame, out_frame, true);
    }
    const auto aec_end = std::chrono::high_resolution_clock::now();

    const double total_aec_ms = std::chrono::duration<double, std::milli>(aec_end - aec_start).count();
    const double avg_per_frame_ms = total_aec_ms / 1000.0;
    std::cout << "  [Timing] 1000 frames executed in " << total_aec_ms
              << " ms (Average: " << avg_per_frame_ms << " ms per 10 ms frame)" << std::endl;

    // Requirement: Average execution time per 10 ms frame must be strictly under 2.0 ms
    assert(avg_per_frame_ms < 2.0);
}

int main() {
    std::cout << "[TEST] Running TestEchoCancellationConvergence..." << std::endl;
    TestEchoCancellationConvergence();

    std::cout << "[TEST] Running TestDoubleTalkAndBargeInPreservation..." << std::endl;
    TestDoubleTalkAndBargeInPreservation();

    std::cout << "[TEST] Running TestStopPlaybackLatencyAndTiming..." << std::endl;
    TestStopPlaybackLatencyAndTiming();

    std::cout << "[PASS] All Acoustic Echo Cancellation (AEC) tests passed successfully!" << std::endl;
    return 0;
}
