#pragma once

#include <cstddef>
#include <cstdint>
#include <deque>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>
#include <condition_variable>
#include <atomic>

namespace jarvis::audio {

/// Low-latency Whisper speech-to-text transcription engine.
/// Runs on a dedicated background worker thread with bounded queue depth (max 2 items)
/// to ensure WASAPI capture and Silero VAD never stall.
class WhisperStt {
public:
    static constexpr size_t kMaxQueueDepth = 2;
    static constexpr int    kDefaultThreads = 4; // Alder Lake P-cores

    using TranscriptionCallback = std::function<void(const std::string& text, uint64_t duration_ms)>;

    explicit WhisperStt(const std::string& model_path = "");
    ~WhisperStt();

    // Non-copyable, non-movable
    WhisperStt(const WhisperStt&) = delete;
    WhisperStt& operator=(const WhisperStt&) = delete;

    /// Loads the ggml Whisper model file (e.g. ggml-base.en.bin).
    bool LoadModel(const std::string& model_path);

    /// Enqueues a completed 16 kHz float mono utterance from Silero VAD.
    /// Drops the oldest queued utterance if queue depth exceeds kMaxQueueDepth.
    void EnqueueUtterance(std::vector<float>&& pcm_16k);

    /// Registers the callback fired when transcription completes.
    void SetTranscriptionCallback(TranscriptionCallback cb) {
        std::lock_guard<std::mutex> lock(mutex_);
        transcription_cb_ = std::move(cb);
    }

    /// Clears any pending queued utterances (e.g. on user barge-in / hotkey interruption).
    void ClearQueue() noexcept;

    /// Whether a Whisper context is initialized and ready.
    bool IsModelLoaded() const noexcept { return is_model_loaded_; }

    /// Cleans up raw Whisper output (trims whitespace, quotes, artifacts).
    static std::string SanitizeTranscript(const std::string& raw_text);

    /// Synchronously transcribes a 16 kHz float PCM audio buffer.
    std::string Transcribe(const std::vector<float>& pcm_16k);

    /// Detects classic Whisper silent-segment hallucinations (e.g. [BLANK_AUDIO], [Music]).
    static bool IsHallucination(const std::string& text);

private:
    void WorkerLoop();
    std::string TranscribeInternal(const std::vector<float>& pcm_16k);

private:
    std::thread             worker_thread_;
    std::mutex              mutex_;
    std::condition_variable cv_;
    std::deque<std::vector<float>> queue_;
    std::atomic<bool>       running_{false};

    TranscriptionCallback   transcription_cb_;
    bool                    is_model_loaded_{false};

    // Opaque pointer to Whisper context
    struct WhisperImpl;
    std::unique_ptr<WhisperImpl> impl_;
};

} // namespace jarvis::audio
