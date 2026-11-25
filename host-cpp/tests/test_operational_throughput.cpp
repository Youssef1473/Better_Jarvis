#include <iostream>
#include <vector>
#include <thread>
#include <chrono>
#include <cassert>
#include <numeric>
#include <algorithm>
#include <bit>
#include "audio/ring_buffer.hpp"

using namespace jarvis::audio;

constexpr size_t BUFFER_CAPACITY = 16384;
constexpr size_t TOTAL_SAMPLES = 100000;
constexpr size_t CHUNK_SIZE = 160; // 10ms at 16kHz

// 1. Deterministic Correctness Verification (True Bitwise Representation Equality)
void VerifyDeterministicCorrectness() {
    std::cout << "[Part 1] Deterministic Correctness & Bitwise FIFO Verification..." << std::endl;
    
    SpscRingBuffer<BUFFER_CAPACITY> ring;
    assert(ring.kCapacity == BUFFER_CAPACITY);
    assert(ring.AvailableRead() == 0);
    assert(ring.AvailableWrite() == BUFFER_CAPACITY);
    
    std::vector<float> source_data(TOTAL_SAMPLES);
    for (size_t i = 0; i < TOTAL_SAMPLES; ++i) {
        source_data[i] = static_cast<float>(i % 1000) * 0.001f;
    }
    
    std::vector<float> received_data;
    received_data.reserve(TOTAL_SAMPLES);
    
    std::thread consumer([&]() {
        std::vector<float> chunk(CHUNK_SIZE);
        while (received_data.size() < TOTAL_SAMPLES) {
            size_t to_read = std::min(CHUNK_SIZE, TOTAL_SAMPLES - received_data.size());
            size_t n = ring.PopBatch(chunk.data(), to_read);
            if (n > 0) {
                received_data.insert(received_data.end(), chunk.begin(), chunk.begin() + n);
            } else {
                std::this_thread::yield();
            }
        }
    });
    
    std::thread producer([&]() {
        size_t written = 0;
        while (written < TOTAL_SAMPLES) {
            size_t to_write = std::min(CHUNK_SIZE, TOTAL_SAMPLES - written);
            size_t n = ring.PushBatch(source_data.data() + written, to_write);
            written += n;
            if (n < to_write) {
                std::this_thread::yield();
            }
        }
    });
    
    producer.join();
    consumer.join();
    
    // Strict Assertions: Total count and C++20 bitwise representation equality
    assert(received_data.size() == TOTAL_SAMPLES);
    for (size_t i = 0; i < TOTAL_SAMPLES; ++i) {
        // True bitwise equality using C++20 std::bit_cast (exact IEEE-754 representation match)
        assert(std::bit_cast<uint32_t>(received_data[i]) == std::bit_cast<uint32_t>(source_data[i]));
    }
    
    std::cout << "  [PASS] Verified " << TOTAL_SAMPLES << " samples: True bitwise equality verified via C++20 std::bit_cast<uint32_t> with zero data loss or FIFO reordering." << std::endl;
}

// 2. Comparative In-Memory Microbenchmark Measurement
void MeasureStatisticalThroughput() {
    std::cout << "[Part 2] Comparative Microbenchmark Latency & Throughput..." << std::endl;
    
    constexpr int WARMUP_ROUNDS = 3;
    constexpr int MEASURE_ROUNDS = 20;
    
    std::vector<float> source_data(TOTAL_SAMPLES);
    for (size_t i = 0; i < TOTAL_SAMPLES; ++i) {
        source_data[i] = static_cast<float>(i % 1000) * 0.001f;
    }
    
    auto run_single_iteration = [&](SpscRingBuffer<BUFFER_CAPACITY>& ring) -> long long {
        std::vector<float> received_data;
        received_data.reserve(TOTAL_SAMPLES);
        
        auto start = std::chrono::high_resolution_clock::now();
        
        std::thread consumer([&]() {
            std::vector<float> chunk(CHUNK_SIZE);
            while (received_data.size() < TOTAL_SAMPLES) {
                size_t to_read = std::min(CHUNK_SIZE, TOTAL_SAMPLES - received_data.size());
                size_t n = ring.PopBatch(chunk.data(), to_read);
                if (n > 0) {
                    received_data.insert(received_data.end(), chunk.begin(), chunk.begin() + n);
                } else {
                    std::this_thread::yield();
                }
            }
        });
        
        std::thread producer([&]() {
            size_t written = 0;
            while (written < TOTAL_SAMPLES) {
                size_t to_write = std::min(CHUNK_SIZE, TOTAL_SAMPLES - written);
                size_t n = ring.PushBatch(source_data.data() + written, to_write);
                written += n;
                if (n < to_write) {
                    std::this_thread::yield();
                }
            }
        });
        
        producer.join();
        consumer.join();
        
        auto end = std::chrono::high_resolution_clock::now();
        return std::chrono::duration_cast<std::chrono::microseconds>(end - start).count();
    };
    
    // Warm-up rounds
    std::cout << "  Running " << WARMUP_ROUNDS << " warm-up rounds to prime caches and thread schedulers..." << std::endl;
    for (int w = 0; w < WARMUP_ROUNDS; ++w) {
        SpscRingBuffer<BUFFER_CAPACITY> ring;
        run_single_iteration(ring);
    }
    
    // Measured rounds
    std::cout << "  Executing " << MEASURE_ROUNDS << " comparative rounds (" << TOTAL_SAMPLES << " samples each)..." << std::endl;
    std::vector<long long> latencies_us;
    latencies_us.reserve(MEASURE_ROUNDS);
    
    for (int r = 0; r < MEASURE_ROUNDS; ++r) {
        SpscRingBuffer<BUFFER_CAPACITY> ring;
        long long us = run_single_iteration(ring);
        latencies_us.push_back(us);
    }
    
    // Compute comparative summary statistics
    std::sort(latencies_us.begin(), latencies_us.end());
    long long min_us = latencies_us.front();
    long long max_us = latencies_us.back();
    long long median_us = latencies_us[MEASURE_ROUNDS / 2];
    double mean_us = std::accumulate(latencies_us.begin(), latencies_us.end(), 0.0) / MEASURE_ROUNDS;
    
    // Environment Metadata
    std::cout << "\n--- Environment Telemetry ---" << std::endl;
#if defined(__GNUC__) || defined(__GNUG__)
    std::cout << "  Compiler: MinGW-w64 GCC " << __VERSION__ << " (-std=c++20 -O2)" << std::endl;
#elif defined(_MSC_VER)
    std::cout << "  Compiler: MSVC " << _MSC_VER << std::endl;
#else
    std::cout << "  Compiler: Unknown C++20" << std::endl;
#endif
    std::cout << "  Hardware Concurrency: " << std::thread::hardware_concurrency() << " logical cores" << std::endl;
    
    // Comparative Metrics
    // 100,000 samples @ 16 kHz = 6.25 seconds of real-time audio
    double audio_duration_sec = static_cast<double>(TOTAL_SAMPLES) / 16000.0;
    double median_sec = static_cast<double>(median_us) / 1000000.0;
    double throughput_x_realtime = audio_duration_sec / median_sec;
    
    std::cout << "\n--- Comparative Microbenchmark Latency (" << MEASURE_ROUNDS << " rounds of 100k samples) ---" << std::endl;
    std::cout << "  Min:    " << min_us << " us (" << (min_us / 1000.0) << " ms)" << std::endl;
    std::cout << "  Median: " << median_us << " us (" << (median_us / 1000.0) << " ms)" << std::endl;
    std::cout << "  Mean:   " << static_cast<long long>(mean_us) << " us (" << (mean_us / 1000.0) << " ms)" << std::endl;
    std::cout << "  Max:    " << max_us << " us (" << (max_us / 1000.0) << " ms)" << std::endl;
    std::cout << "  Throughput Factor: " << throughput_x_realtime << "x Real-Time" << std::endl;
    std::cout << "  Note: Tail percentiles (e.g. P99) are intentionally omitted for N=" << MEASURE_ROUNDS 
              << " as small sample sizes cannot statistically establish tail latency guarantees." << std::endl;
    std::cout << "  Scope Boundary: This microbenchmark measures isolated in-memory SPSC buffer transfer only." << std::endl;
    std::cout << "                  It does NOT represent full end-to-end Jarvis pipeline latency (which incorporates" << std::endl;
    std::cout << "                  WASAPI hardware endpoints, neural STT/TTS models, and LLM turnaround)." << std::endl;
    
    // Comparative assertion: In-memory SPSC buffer must operate comfortably within audio budget
    assert(median_us < 50000 && "Median throughput exceeded real-time audio budget");
    std::cout << "  [PASS] Microbenchmark throughput comfortably exceeds real-time audio transfer rate." << std::endl;
}

int main() {
    std::cout << "=== Better_Jarvis C++ SPSC Concurrency & Microbenchmark ===" << std::endl;
    VerifyDeterministicCorrectness();
    MeasureStatisticalThroughput();
    std::cout << "=== C++ SPSC Concurrency & Microbenchmark Completed Cleanly ===" << std::endl;
    return 0;
}
