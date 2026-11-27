#include "test_assert.hpp"
#include <chrono>
#include <iostream>
#include <thread>
#include <vector>
#include "audio/ring_buffer.hpp"

using namespace jarvis::audio;

void TestBasicPushPop() {
    SpscRingBuffer<64> rb;
    TEST_ASSERT(rb.AvailableRead() == 0);
    TEST_ASSERT(rb.AvailableWrite() == 64);
    TEST_ASSERT(rb.DroppedSamples() == 0);
    TEST_ASSERT(rb.OverflowEvents() == 0);

    std::vector<float> input = {1.0f, 2.0f, 3.0f, 4.0f};
    size_t written = rb.PushBatch(input.data(), input.size());
    TEST_ASSERT(written == 4);
    TEST_ASSERT(rb.AvailableRead() == 4);
    TEST_ASSERT(rb.AvailableWrite() == 60);

    std::vector<float> output(4, 0.0f);
    size_t read = rb.PopBatch(output.data(), 4);
    TEST_ASSERT(read == 4);
    TEST_ASSERT(output == input);
    TEST_ASSERT(rb.AvailableRead() == 0);
    TEST_ASSERT(rb.AvailableWrite() == 64);
}

void TestWrapAround() {
    SpscRingBuffer<16> rb;

    // Fill 12 samples
    std::vector<float> data1(12, 1.0f);
    size_t w1 = rb.PushBatch(data1.data(), 12);
    TEST_ASSERT(w1 == 12);

    // Consume 10 samples (write_pos = 12, read_pos = 10)
    std::vector<float> out(10);
    size_t r1 = rb.PopBatch(out.data(), 10);
    TEST_ASSERT(r1 == 10);
    TEST_ASSERT(rb.AvailableRead() == 2);
    TEST_ASSERT(rb.AvailableWrite() == 14);

    // Now push 8 more samples. This will wrap around (12 + 8 = 20 > 16)
    std::vector<float> data2 = {2.0f, 3.0f, 4.0f, 5.0f, 6.0f, 7.0f, 8.0f, 9.0f};
    size_t written = rb.PushBatch(data2.data(), 8);
    TEST_ASSERT(written == 8);
    TEST_ASSERT(rb.AvailableRead() == 10); // 2 remaining from data1 + 8 from data2

    std::vector<float> read_all(10);
    size_t r2 = rb.PopBatch(read_all.data(), 10);
    TEST_ASSERT(r2 == 10);
    TEST_ASSERT(read_all[0] == 1.0f);
    TEST_ASSERT(read_all[1] == 1.0f);
    TEST_ASSERT(read_all[2] == 2.0f);
    TEST_ASSERT(read_all[9] == 9.0f);
}

void TestPeekRecentPreRoll() {
    SpscRingBuffer<32> rb;

    // Write samples 1 to 20
    std::vector<float> input(20);
    for (size_t i = 0; i < 20; ++i) {
        input[i] = static_cast<float>(i + 1);
    }
    rb.PushBatch(input.data(), 20);

    // Peek the most recent 5 samples (should be 16, 17, 18, 19, 20)
    std::vector<float> peek(5);
    size_t peeked = rb.PeekRecent(peek.data(), 5);
    TEST_ASSERT(peeked == 5);
    TEST_ASSERT(peek[0] == 16.0f);
    TEST_ASSERT(peek[1] == 17.0f);
    TEST_ASSERT(peek[2] == 18.0f);
    TEST_ASSERT(peek[3] == 19.0f);
    TEST_ASSERT(peek[4] == 20.0f);

    // Ensure read cursor did NOT move
    TEST_ASSERT(rb.AvailableRead() == 20);

    // Pop the first 5 samples (should be 1, 2, 3, 4, 5)
    std::vector<float> popped(5);
    rb.PopBatch(popped.data(), 5);
    TEST_ASSERT(popped[0] == 1.0f);
    TEST_ASSERT(popped[4] == 5.0f);
    TEST_ASSERT(rb.AvailableRead() == 15);
}

void TestOverflowTelemetry() {
    SpscRingBuffer<32> rb;
    TEST_ASSERT(rb.DroppedSamples() == 0);
    TEST_ASSERT(rb.OverflowEvents() == 0);

    // Write 20 samples -> 12 slots remain
    std::vector<float> data1(20, 1.0f);
    size_t w1 = rb.PushBatch(data1.data(), 20);
    TEST_ASSERT(w1 == 20);
    TEST_ASSERT(rb.DroppedSamples() == 0);
    TEST_ASSERT(rb.OverflowEvents() == 0);

    // Attempt to write 25 samples when only 12 fit
    // 12 should be written, 13 dropped, 1 overflow event recorded
    std::vector<float> data2(25, 2.0f);
    size_t w2 = rb.PushBatch(data2.data(), 25);
    TEST_ASSERT(w2 == 12);
    TEST_ASSERT(rb.AvailableRead() == 32);
    TEST_ASSERT(rb.AvailableWrite() == 0);
    TEST_ASSERT(rb.DroppedSamples() == 13);
    TEST_ASSERT(rb.OverflowEvents() == 1);

    // Attempt another write into full buffer (0 fit, 10 dropped)
    std::vector<float> data3(10, 3.0f);
    size_t w3 = rb.PushBatch(data3.data(), 10);
    TEST_ASSERT(w3 == 0);
    TEST_ASSERT(rb.DroppedSamples() == 23);
    TEST_ASSERT(rb.OverflowEvents() == 2);

    // Reset telemetry only
    rb.ResetTelemetry();
    TEST_ASSERT(rb.DroppedSamples() == 0);
    TEST_ASSERT(rb.OverflowEvents() == 0);
    TEST_ASSERT(rb.AvailableRead() == 32);

    // Full Reset
    rb.Reset();
    TEST_ASSERT(rb.AvailableRead() == 0);
    TEST_ASSERT(rb.AvailableWrite() == 32);
    TEST_ASSERT(rb.DroppedSamples() == 0);
    TEST_ASSERT(rb.OverflowEvents() == 0);
}

void TestMultiThreadedSpscStress() {
    constexpr size_t kTotalSamples = 2000000;
    SpscRingBuffer<16384> rb;

    std::atomic<bool> producer_done{false};

    // Producer thread: pushes exactly 2,000,000 monotonic floats
    std::thread producer([&rb, &producer_done]() {
        size_t current_val = 0;
        std::vector<float> chunk(256);

        while (current_val < kTotalSamples) {
            const size_t to_send = std::min<size_t>(chunk.size(), kTotalSamples - current_val);
            for (size_t i = 0; i < to_send; ++i) {
                chunk[i] = static_cast<float>(current_val + i);
            }

            size_t written = 0;
            while (written < to_send) {
                const size_t avail = rb.AvailableWrite();
                if (avail == 0) {
                    std::this_thread::yield();
                    continue;
                }
                const size_t batch = std::min(to_send - written, avail);
                const size_t n = rb.PushBatch(chunk.data() + written, batch);
                written += n;
            }
            current_val += to_send;
        }
        producer_done.store(true, std::memory_order_release);
    });

    // Consumer thread: pops and verifies continuous monotonic sequence
    size_t received_count = 0;
    size_t expected_val = 0;
    std::vector<float> read_buf(512);

    while (received_count < kTotalSamples) {
        size_t read = rb.PopBatch(read_buf.data(), read_buf.size());
        if (read == 0) {
            std::this_thread::yield();
            continue;
        }

        for (size_t i = 0; i < read; ++i) {
            float val = read_buf[i];
            float expected = static_cast<float>(expected_val);
            TEST_ASSERT(val == expected);
            expected_val++;
            received_count++;
        }
    }

    producer.join();

    TEST_ASSERT(received_count == kTotalSamples);
    TEST_ASSERT(rb.AvailableRead() == 0);
    TEST_ASSERT(rb.DroppedSamples() == 0);
    TEST_ASSERT(rb.OverflowEvents() == 0);
}

void TestConcurrentOverflowIntegrity() {
    // Stress test ring buffer under intentional heavy overflow:
    // Producer pushes bursts continuously without pacing; consumer reads in chunks.
    // Assert that every sample consumed is strictly monotonic (zero corruption/inversions),
    // and telemetry correctly records non-zero dropped samples and overflow events.
    SpscRingBuffer<2048> rb;
    constexpr size_t kBurstCount = 5000;
    std::atomic<bool> producer_done{false};

    std::thread producer([&rb, &producer_done]() {
        std::vector<float> chunk(512);
        size_t counter = 0;
        for (size_t b = 0; b < kBurstCount; ++b) {
            for (size_t i = 0; i < chunk.size(); ++i) {
                chunk[i] = static_cast<float>(counter++);
            }
            // Deliberately push without waiting for AvailableWrite to induce drops
            rb.PushBatch(chunk.data(), chunk.size());
        }
        producer_done.store(true, std::memory_order_release);
    });

    float last_val = -1.0f;
    std::vector<float> read_buf(256);
    size_t total_popped = 0;

    while (!producer_done.load(std::memory_order_acquire) || rb.AvailableRead() > 0) {
        size_t n = rb.PopBatch(read_buf.data(), read_buf.size());
        if (n == 0) {
            std::this_thread::yield();
            continue;
        }
        for (size_t i = 0; i < n; ++i) {
            float val = read_buf[i];
            TEST_ASSERT(val > last_val); // Strictly strictly increasing - zero corruption!
            last_val = val;
            total_popped++;
        }
    }

    producer.join();

    TEST_ASSERT(total_popped > 0);
    TEST_ASSERT(rb.DroppedSamples() > 0);
    TEST_ASSERT(rb.OverflowEvents() > 0);
}

int main() {
    std::cout << "[TEST] Running TestBasicPushPop..." << std::endl;
    TestBasicPushPop();

    std::cout << "[TEST] Running TestWrapAround..." << std::endl;
    TestWrapAround();

    std::cout << "[TEST] Running TestPeekRecentPreRoll..." << std::endl;
    TestPeekRecentPreRoll();

    std::cout << "[TEST] Running TestOverflowTelemetry..." << std::endl;
    TestOverflowTelemetry();

    std::cout << "[TEST] Running TestMultiThreadedSpscStress (2,000,000 samples)..." << std::endl;
    TestMultiThreadedSpscStress();

    std::cout << "[TEST] Running TestConcurrentOverflowIntegrity..." << std::endl;
    TestConcurrentOverflowIntegrity();

    std::cout << "[PASS] All SPSC Ring Buffer tests passed successfully!" << std::endl;
    return 0;
}
