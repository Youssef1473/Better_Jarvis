#include "test_assert.hpp"
#include <chrono>
#include <cmath>
#include <iomanip>
#include <iostream>
#include <vector>

#include "audio/aec_processor.hpp"
#include "audio/aec_delay_estimator.hpp"
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

// Simulates room impulse response with dynamic delay + reflections
std::vector<float> SimulateAcousticEchoWithDelay(const std::vector<float>& ref, size_t delay_samples) {
    std::vector<float> mic(ref.size(), 0.0f);
    for (size_t i = 0; i < ref.size(); ++i) {
        float sample = 0.0f;
        if (i >= delay_samples) sample += 0.50f * ref[i - delay_samples];
        if (i >= delay_samples + 60) sample -= 0.20f * ref[i - (delay_samples + 60)];
        if (i >= delay_samples + 140) sample += 0.12f * ref[i - (delay_samples + 140)];
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

    TEST_ASSERT(converged_mic_energy > 0.0f);
    TEST_ASSERT(converged_err_energy > 0.0f);

    // Compute empirical ERLE in dB: 10 * log10(E_mic / E_err)
    const float measured_erle = 10.0f * std::log10(converged_mic_energy / converged_err_energy);
    std::cout << "  [Convergence] Measured ERLE: " << measured_erle
              << " dB (Filter Telemetry: " << aec.LastErleDb()
              << " dB, Initial Mic Energy: " << initial_mic_energy << ")" << std::endl;

    // Requirement: Attenuation >= 12 dB
    TEST_ASSERT(measured_erle >= 12.0f);
    // DTD must not trigger false positive on pure echo
    TEST_ASSERT(!aec.IsDoubleTalk());
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
    TEST_ASSERT(dtd_triggered);
    // Requirement: Near-end human voice must NOT be suppressed (output retains at least 50% energy)
    TEST_ASSERT(output_energy >= 0.50f * voice_energy);

    // 3. Verify filter weights did not diverge or produce NaN
    for (size_t f = 0; f < 10; ++f) {
        float test_mic[160]{0.0f};
        float test_ref[160]{0.0f};
        float test_out[160]{0.0f};
        aec.ProcessFrame(test_mic, test_ref, test_out, true);
        for (int i = 0; i < 160; ++i) {
            TEST_ASSERT(!std::isnan(test_out[i]));
            TEST_ASSERT(!std::isinf(test_out[i]));
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
    TEST_ASSERT(stop_duration_ms < 20.0);
    TEST_ASSERT(!tts.IsSpeaking());
    TEST_ASSERT(tts.FarEndReferenceBuffer().AvailableRead() == 0);
}

// -----------------------------------------------------------------------------
// TICKET-13 Mandatory Verification Tests
// -----------------------------------------------------------------------------

void TestMultiDelayAutoCalibration() {
    struct DelayCase {
        size_t delay_samples;
        float  delay_ms;
        const char* label;
    };

    const DelayCase cases[] = {
        { 240,  15.0f,  "15 ms (240 samples)" },
        { 880,  55.0f,  "55 ms (880 samples)" },
        { 1920, 120.0f, "120 ms (1920 samples)" }
    };

    for (const auto& c : cases) {
        AecDelayEstimator estimator;
        AecProcessor aec;

        constexpr size_t kTotalFrames = 200;
        constexpr size_t kTotalSamples = kTotalFrames * AecProcessor::kFrameSize;

        std::vector<float> ref = GenerateSpeechLikeSignal(kTotalSamples);
        std::vector<float> mic = SimulateAcousticEchoWithDelay(ref, c.delay_samples);
        std::vector<float> clean(kTotalSamples, 0.0f);

        bool locked_seen = false;
        size_t lock_frame = 0;

        for (size_t f = 0; f < kTotalFrames; ++f) {
            const size_t offset = f * AecProcessor::kFrameSize;

            // Process through AEC processor first
            aec.ProcessFrame(&mic[offset], &ref[offset], &clean[offset], true);

            // Feed estimator with raw mic and reference
            const bool is_double_talk = aec.IsDoubleTalk();
            estimator.UpdateFrame(&mic[offset], &ref[offset], true, is_double_talk);

            size_t new_bulk = 0;
            float delay_ms = 0.0f;
            float peak_ncc = 0.0f;
            if (estimator.CheckAndClearJustLocked(&new_bulk, &delay_ms, &peak_ncc)) {
                locked_seen = true;
                lock_frame = f;
                aec.SetBulkDelaySamples(new_bulk);
            }
        }

        std::cout << "  [MultiDelay " << c.label << "] Locked: " << (locked_seen ? "YES" : "NO")
                  << " at frame " << lock_frame
                  << " | Estimated: " << estimator.EstimatedDelayMs()
                  << " ms (Bulk=" << estimator.LockedBulkDelaySamples()
                  << " samples, NCC=" << estimator.LastCorrelationPeak() << ")" << std::endl;

        // 1. Assert estimator locked
        TEST_ASSERT(locked_seen);
        TEST_ASSERT(estimator.IsDelayLocked());

        // 2. Assert estimated delay is within +/- 4 ms of ground truth
        const float delay_err = std::abs(estimator.EstimatedDelayMs() - c.delay_ms);
        TEST_ASSERT(delay_err <= 4.0f);

        // 3. Assert causal safety lead: bulk delay must be <= estimated lag
        TEST_ASSERT(estimator.LockedBulkDelaySamples() <= c.delay_samples);

        // 4. Measure converged ERLE over frames 140 to 195
        float converged_mic = 0.0f;
        float converged_err = 0.0f;
        for (size_t f = 140; f < 195; ++f) {
            const size_t offset = f * AecProcessor::kFrameSize;
            for (size_t i = 0; i < AecProcessor::kFrameSize; ++i) {
                converged_mic += mic[offset + i] * mic[offset + i];
                converged_err += clean[offset + i] * clean[offset + i];
            }
        }
        TEST_ASSERT(converged_mic > 0.0f);
        TEST_ASSERT(converged_err > 0.0f);

        const float measured_erle = 10.0f * std::log10(converged_mic / converged_err);
        std::cout << "    -> Converged ERLE: " << measured_erle << " dB" << std::endl;
        TEST_ASSERT(measured_erle >= 12.0f);
    }
}

void TestDoubleTalkProtectionDuringCalibration() {
    AecDelayEstimator estimator;
    AecProcessor aec;

    // First lock onto 35 ms delay (560 samples)
    constexpr size_t kCalibFrames = 80;
    constexpr size_t kCalibSamples = kCalibFrames * AecProcessor::kFrameSize;
    std::vector<float> ref = GenerateSpeechLikeSignal(kCalibSamples);
    std::vector<float> mic = SimulateAcousticEchoWithDelay(ref, 560);
    std::vector<float> clean(kCalibSamples, 0.0f);

    for (size_t f = 0; f < kCalibFrames; ++f) {
        const size_t offset = f * AecProcessor::kFrameSize;
        aec.ProcessFrame(&mic[offset], &ref[offset], &clean[offset], true);
        estimator.UpdateFrame(&mic[offset], &ref[offset], true, aec.IsDoubleTalk());
        size_t bulk = 0; float dms = 0.0f; float ncc = 0.0f;
        if (estimator.CheckAndClearJustLocked(&bulk, &dms, &ncc)) {
            aec.SetBulkDelaySamples(bulk);
        }
    }

    TEST_ASSERT(estimator.IsDelayLocked());
    const size_t locked_bulk_before = estimator.LockedBulkDelaySamples();
    const float estimated_ms_before = estimator.EstimatedDelayMs();

    // Now inject loud near-end human voice on top of echo with double-talk flag active
    constexpr size_t kDtFrames = 60;
    constexpr size_t kDtSamples = kDtFrames * AecProcessor::kFrameSize;
    std::vector<float> ref_dt = GenerateSpeechLikeSignal(kDtSamples);
    std::vector<float> mic_dt = SimulateAcousticEchoWithDelay(ref_dt, 560);

    for (size_t i = 0; i < kDtSamples; ++i) {
        double t = static_cast<double>(i) / 16000.0;
        mic_dt[i] += static_cast<float>(0.85 * std::sin(2.0 * kPi * 440.0 * t));
    }

    for (size_t f = 0; f < kDtFrames; ++f) {
        const size_t offset = f * AecProcessor::kFrameSize;
        // Double-talk is active; estimator must ignore frames
        estimator.UpdateFrame(&mic_dt[offset], &ref_dt[offset], true, true /* is_double_talk */);
    }

    std::cout << "  [DoubleTalkProtection] Locked bulk before: " << locked_bulk_before
              << " | After: " << estimator.LockedBulkDelaySamples()
              << " | Estimated ms: " << estimator.EstimatedDelayMs() << std::endl;

    // Assert that double-talk did NOT alter or corrupt the locked bulk delay!
    TEST_ASSERT(estimator.LockedBulkDelaySamples() == locked_bulk_before);
    TEST_ASSERT(estimator.EstimatedDelayMs() == estimated_ms_before);
}

void TestDeviceHotSwapRecalibration() {
    AecDelayEstimator estimator;
    AecProcessor aec;

    // 1. Lock onto initial 20 ms (320 samples) speaker delay
    constexpr size_t kPhase1Frames = 80;
    constexpr size_t kPhase1Samples = kPhase1Frames * AecProcessor::kFrameSize;
    std::vector<float> ref1 = GenerateSpeechLikeSignal(kPhase1Samples);
    std::vector<float> mic1 = SimulateAcousticEchoWithDelay(ref1, 320);
    std::vector<float> clean1(kPhase1Samples, 0.0f);

    for (size_t f = 0; f < kPhase1Frames; ++f) {
        const size_t offset = f * AecProcessor::kFrameSize;
        aec.ProcessFrame(&mic1[offset], &ref1[offset], &clean1[offset], true);
        estimator.UpdateFrame(&mic1[offset], &ref1[offset], true, aec.IsDoubleTalk());
        size_t bulk = 0; float dms = 0.0f; float ncc = 0.0f;
        if (estimator.CheckAndClearJustLocked(&bulk, &dms, &ncc)) {
            aec.SetBulkDelaySamples(bulk);
        }
    }

    TEST_ASSERT(estimator.IsDelayLocked());
    TEST_ASSERT(std::abs(estimator.EstimatedDelayMs() - 20.0f) <= 4.0f);
    std::cout << "  [HotSwap Phase 1] Locked at: " << estimator.EstimatedDelayMs() << " ms" << std::endl;

    // 2. Simulate audio device hot-swap / discontinuity (e.g. switching to USB headset / virtual mixer)
    estimator.Reset();
    aec.Reset();
    TEST_ASSERT(!estimator.IsDelayLocked());

    // 3. New device presents 95 ms delay (1,520 samples)
    constexpr size_t kPhase2Frames = 150;
    constexpr size_t kPhase2Samples = kPhase2Frames * AecProcessor::kFrameSize;
    std::vector<float> ref2 = GenerateSpeechLikeSignal(kPhase2Samples);
    std::vector<float> mic2 = SimulateAcousticEchoWithDelay(ref2, 1520);
    std::vector<float> clean2(kPhase2Samples, 0.0f);

    bool relocked = false;
    for (size_t f = 0; f < kPhase2Frames; ++f) {
        const size_t offset = f * AecProcessor::kFrameSize;
        aec.ProcessFrame(&mic2[offset], &ref2[offset], &clean2[offset], true);
        estimator.UpdateFrame(&mic2[offset], &ref2[offset], true, aec.IsDoubleTalk());
        size_t bulk = 0; float dms = 0.0f; float ncc = 0.0f;
        if (estimator.CheckAndClearJustLocked(&bulk, &dms, &ncc)) {
            relocked = true;
            aec.SetBulkDelaySamples(bulk);
        }
    }

    std::cout << "  [HotSwap Phase 2] Relocked: " << (relocked ? "YES" : "NO")
              << " at: " << estimator.EstimatedDelayMs() << " ms" << std::endl;

    TEST_ASSERT(relocked);
    TEST_ASSERT(estimator.IsDelayLocked());
    TEST_ASSERT(std::abs(estimator.EstimatedDelayMs() - 95.0f) <= 4.0f);

    // Verify converged ERLE after relock
    float conv_mic = 0.0f;
    float conv_err = 0.0f;
    for (size_t f = 100; f < 145; ++f) {
        const size_t offset = f * AecProcessor::kFrameSize;
        for (size_t i = 0; i < AecProcessor::kFrameSize; ++i) {
            conv_mic += mic2[offset + i] * mic2[offset + i];
            conv_err += clean2[offset + i] * clean2[offset + i];
        }
    }
    const float erle = 10.0f * std::log10(conv_mic / conv_err);
    std::cout << "    -> Converged ERLE after hot-swap: " << erle << " dB" << std::endl;
    TEST_ASSERT(erle >= 12.0f);
}

void TestCombinedCpuBudget() {
    AecDelayEstimator estimator;
    AecProcessor aec;

    float mic_frame[160]{0.1f};
    float ref_frame[160]{0.1f};
    float clean_frame[160]{0.0f};

    const auto t_start = std::chrono::high_resolution_clock::now();
    for (int i = 0; i < 1000; ++i) {
        aec.ProcessFrame(mic_frame, ref_frame, clean_frame, true);
        estimator.UpdateFrame(mic_frame, ref_frame, true, false);
    }
    const auto t_end = std::chrono::high_resolution_clock::now();

    const double total_ms = std::chrono::duration<double, std::milli>(t_end - t_start).count();
    const double avg_per_frame_ms = total_ms / 1000.0;

    std::cout << "  [CpuBudget] 1,000 combined frames executed in " << total_ms
              << " ms (Average: " << avg_per_frame_ms << " ms per 10 ms frame)" << std::endl;

    // Requirement: Combined runtime must be < 0.5 ms per 10 ms frame (> 20x faster than real-time)
    TEST_ASSERT(avg_per_frame_ms < 0.50);
}

int main() {
    std::cout << "[TEST] Running TestEchoCancellationConvergence..." << std::endl;
    TestEchoCancellationConvergence();

    std::cout << "[TEST] Running TestDoubleTalkAndBargeInPreservation..." << std::endl;
    TestDoubleTalkAndBargeInPreservation();

    std::cout << "[TEST] Running TestStopPlaybackLatencyAndTiming..." << std::endl;
    TestStopPlaybackLatencyAndTiming();

    std::cout << "[TEST] Running TestMultiDelayAutoCalibration (15 ms, 55 ms, 120 ms)..." << std::endl;
    TestMultiDelayAutoCalibration();

    std::cout << "[TEST] Running TestDoubleTalkProtectionDuringCalibration..." << std::endl;
    TestDoubleTalkProtectionDuringCalibration();

    std::cout << "[TEST] Running TestDeviceHotSwapRecalibration (20 ms -> 95 ms)..." << std::endl;
    TestDeviceHotSwapRecalibration();

    std::cout << "[TEST] Running TestCombinedCpuBudget (1,000 frames < 0.5 ms/frame)..." << std::endl;
    TestCombinedCpuBudget();

    std::cout << "[PASS] All Acoustic Echo Cancellation (AEC) and Auto-Calibration tests passed successfully!" << std::endl;
    return 0;
}
