#pragma once

#include <algorithm>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <vector>

namespace jarvis::audio {

/// Lock-free Single-Producer Single-Consumer (SPSC) Ring Buffer.
/// Sized as a power-of-2 to eliminate modulo operations in real-time callbacks.
template <size_t Capacity = 32768> // 32768 samples = 2.048s @ 16 kHz
class SpscRingBuffer {
    static_assert((Capacity & (Capacity - 1)) == 0, "Capacity must be a power of 2");

public:
    static constexpr size_t kCapacity = Capacity;
    static constexpr size_t kMask = Capacity - 1;

    SpscRingBuffer()
        : buffer_(Capacity, 0.0f),
          write_pos_(0),
          read_pos_(0),
          dropped_samples_(0),
          overflow_events_(0) {}

    /// Number of samples currently available to be read.
    size_t AvailableRead() const noexcept {
        const size_t w = write_pos_.load(std::memory_order_acquire);
        const size_t r = read_pos_.load(std::memory_order_relaxed);
        return w - r;
    }

    /// Number of free sample slots available for writing.
    size_t AvailableWrite() const noexcept {
        const size_t w = write_pos_.load(std::memory_order_relaxed);
        const size_t r = read_pos_.load(std::memory_order_acquire);
        return kCapacity - (w - r);
    }

    /// Bulk write samples into the buffer (Producer thread only).
    /// If count exceeds available space, writes as many as fit, drops the excess,
    /// and records the overflow telemetry counters with memory_order_relaxed.
    /// Returns the number of samples actually written.
    size_t PushBatch(const float* data, size_t count) noexcept {
        if (!data || count == 0) return 0;

        const size_t available = AvailableWrite();
        const size_t to_write = std::min(count, available);

        if (count > available) {
            const uint64_t dropped = static_cast<uint64_t>(count - to_write);
            dropped_samples_.fetch_add(dropped, std::memory_order_relaxed);
            overflow_events_.fetch_add(1, std::memory_order_relaxed);
        }

        if (to_write == 0) return 0;

        const size_t w = write_pos_.load(std::memory_order_relaxed);
        const size_t idx = w & kMask;

        const size_t first_chunk = std::min(to_write, kCapacity - idx);
        std::memcpy(&buffer_[idx], data, first_chunk * sizeof(float));

        const size_t second_chunk = to_write - first_chunk;
        if (second_chunk > 0) {
            std::memcpy(&buffer_[0], data + first_chunk, second_chunk * sizeof(float));
        }

        write_pos_.store(w + to_write, std::memory_order_release);
        return to_write;
    }

    /// Bulk read and consume samples from the buffer (Consumer thread only).
    /// Advances the read cursor. Returns the number of samples read.
    size_t PopBatch(float* out, size_t count) noexcept {
        if (!out || count == 0) return 0;

        const size_t available = AvailableRead();
        const size_t to_read = std::min(count, available);
        if (to_read == 0) return 0;

        const size_t r = read_pos_.load(std::memory_order_relaxed);
        const size_t idx = r & kMask;

        const size_t first_chunk = std::min(to_read, kCapacity - idx);
        std::memcpy(out, &buffer_[idx], first_chunk * sizeof(float));

        const size_t second_chunk = to_read - first_chunk;
        if (second_chunk > 0) {
            std::memcpy(out + first_chunk, &buffer_[0], second_chunk * sizeof(float));
        }

        read_pos_.store(r + to_read, std::memory_order_release);
        return to_read;
    }

    /// Inspect the most recently written `count` samples WITHOUT advancing the read pointer.
    /// Crucial for VAD pre-roll (e.g. reading the past 800 ms / 12,800 samples upon speech trigger).
    /// Returns the actual number of samples copied (capped by AvailableRead).
    size_t PeekRecent(float* out, size_t count) const noexcept {
        if (!out || count == 0) return 0;

        const size_t w = write_pos_.load(std::memory_order_acquire);
        const size_t r = read_pos_.load(std::memory_order_relaxed);
        const size_t available = w - r;
        const size_t to_peek = std::min(count, available);
        if (to_peek == 0) return 0;

        // The window starts `to_peek` samples before current `w`
        const size_t start_virtual = w - to_peek;
        const size_t idx = start_virtual & kMask;

        const size_t first_chunk = std::min(to_peek, kCapacity - idx);
        std::memcpy(out, &buffer_[idx], first_chunk * sizeof(float));

        const size_t second_chunk = to_peek - first_chunk;
        if (second_chunk > 0) {
            std::memcpy(out + first_chunk, &buffer_[0], second_chunk * sizeof(float));
        }

        return to_peek;
    }

    /// Reset read cursor to match write cursor (empties pending samples).
    void Clear() noexcept {
        read_pos_.store(write_pos_.load(std::memory_order_relaxed), std::memory_order_release);
    }

    /// Full reset: resets both cursors to 0 and clears telemetry counters.
    void Reset() noexcept {
        write_pos_.store(0, std::memory_order_relaxed);
        read_pos_.store(0, std::memory_order_release);
        ResetTelemetry();
    }

    /// Telemetry: total count of samples dropped due to buffer overflow.
    uint64_t DroppedSamples() const noexcept {
        return dropped_samples_.load(std::memory_order_relaxed);
    }

    /// Telemetry: total count of overflow events encountered.
    uint64_t OverflowEvents() const noexcept {
        return overflow_events_.load(std::memory_order_relaxed);
    }

    /// Reset telemetry counters to zero.
    void ResetTelemetry() noexcept {
        dropped_samples_.store(0, std::memory_order_relaxed);
        overflow_events_.store(0, std::memory_order_relaxed);
    }

private:
    std::vector<float> buffer_;

    // Align to 64 bytes to eliminate false sharing between audio capture and consumer threads
    alignas(64) std::atomic<size_t> write_pos_;
    alignas(64) std::atomic<size_t> read_pos_;

    // Cache-line separated atomic telemetry counters
    alignas(64) std::atomic<uint64_t> dropped_samples_;
    alignas(64) std::atomic<uint64_t> overflow_events_;
};

} // namespace jarvis::audio
