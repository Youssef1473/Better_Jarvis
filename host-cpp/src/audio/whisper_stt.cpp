#include "audio/whisper_stt.hpp"

#include <algorithm>
#include <cctype>
#include <iostream>
#include <sstream>

#ifdef JARVIS_HAS_WHISPER
#include <whisper.h>
#endif

namespace jarvis::audio {

struct WhisperStt::WhisperImpl {
#ifdef JARVIS_HAS_WHISPER
    struct whisper_context* ctx{nullptr};

    ~WhisperImpl() {
        if (ctx) {
            whisper_free(ctx);
            ctx = nullptr;
        }
    }
#else
    int dummy{0};
#endif
};

WhisperStt::WhisperStt(const std::string& model_path)
    : impl_(std::make_unique<WhisperImpl>()) {
    running_.store(true, std::memory_order_release);
    worker_thread_ = std::thread(&WhisperStt::WorkerLoop, this);

    if (!model_path.empty()) {
        LoadModel(model_path);
    }
}

WhisperStt::~WhisperStt() {
    running_.store(false, std::memory_order_release);
    {
        std::lock_guard<std::mutex> lock(mutex_);
        queue_.clear();
    }
    cv_.notify_all();

    if (worker_thread_.joinable()) {
        worker_thread_.join();
    }
}

bool WhisperStt::LoadModel(const std::string& model_path) {
    if (model_path.empty()) return false;

#ifdef JARVIS_HAS_WHISPER
    struct whisper_context_params cparams = whisper_context_default_params();
    cparams.use_gpu = false; // CPU execution on Alder Lake P-cores

    struct whisper_context* ctx = whisper_init_from_file_with_params(model_path.c_str(), cparams);
    if (!ctx) {
        std::cerr << "[WhisperStt] Failed to initialize whisper model from: " << model_path << std::endl;
        return false;
    }

    if (impl_->ctx) {
        whisper_free(impl_->ctx);
    }
    impl_->ctx = ctx;
    is_model_loaded_ = true;
    std::cout << "[WhisperStt] Model loaded successfully: " << model_path << std::endl;
    return true;
#else
    std::cout << "[WhisperStt] Build-safe stub mode active (JARVIS_HAS_WHISPER not defined). Model path: "
              << model_path << std::endl;
    is_model_loaded_ = true;
    return true;
#endif
}

void WhisperStt::EnqueueUtterance(std::vector<float>&& pcm_16k) {
    if (pcm_16k.empty()) return;

    {
        std::lock_guard<std::mutex> lock(mutex_);
        // Bound queue to max 2 items to prevent latency bloat / lag
        while (queue_.size() >= kMaxQueueDepth) {
            queue_.pop_front();
        }
        queue_.push_back(std::move(pcm_16k));
    }
    cv_.notify_one();
}

void WhisperStt::ClearQueue() noexcept {
    std::lock_guard<std::mutex> lock(mutex_);
    queue_.clear();
}

void WhisperStt::WorkerLoop() {
    while (running_.load(std::memory_order_relaxed)) {
        std::vector<float> utterance;
        {
            std::unique_lock<std::mutex> lock(mutex_);
            cv_.wait(lock, [this]() {
                return !running_.load(std::memory_order_relaxed) || !queue_.empty();
            });

            if (!running_.load(std::memory_order_relaxed) && queue_.empty()) {
                break;
            }

            if (!queue_.empty()) {
                utterance = std::move(queue_.front());
                queue_.pop_front();
            }
        }

        if (utterance.empty()) continue;

        const uint64_t duration_ms = (utterance.size() * 1000) / 16000;
        std::string raw_text = TranscribeInternal(utterance);
        std::string clean_text = SanitizeTranscript(raw_text);

        if (!clean_text.empty() && !IsHallucination(clean_text)) {
            TranscriptionCallback cb;
            {
                std::lock_guard<std::mutex> lock(mutex_);
                cb = transcription_cb_;
            }
            if (cb) {
                cb(clean_text, duration_ms);
            }
        }
    }
}

std::string WhisperStt::TranscribeInternal(const std::vector<float>& pcm_16k) {
#ifdef JARVIS_HAS_WHISPER
    if (!impl_ || !impl_->ctx || pcm_16k.empty()) {
        return "";
    }

    struct whisper_full_params params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    params.n_threads        = kDefaultThreads;
    params.no_context       = true;  // Crucial: prevents hallucinating prior command context
    params.single_segment   = true;
    params.no_timestamps    = true;
    params.suppress_blank   = true;
    params.language         = "en";
    params.print_progress   = false;
    params.print_special    = false;
    params.print_realtime   = false;
    params.print_timestamps = false;

    if (whisper_full(impl_->ctx, params, pcm_16k.data(), static_cast<int>(pcm_16k.size())) != 0) {
        std::cerr << "[WhisperStt] whisper_full failed to process utterance" << std::endl;
        return "";
    }

    std::ostringstream ss;
    const int n_segments = whisper_full_n_segments(impl_->ctx);
    for (int i = 0; i < n_segments; ++i) {
        const char* seg_text = whisper_full_get_segment_text(impl_->ctx, i);
        if (seg_text) {
            ss << seg_text;
        }
    }
    return ss.str();
#else
    // Build-safe mock fallback: return empty unless configured for test
    (void)pcm_16k;
    return "";
#endif
}

std::string WhisperStt::SanitizeTranscript(const std::string& raw_text) {
    if (raw_text.empty()) return "";

    size_t start = 0;
    while (start < raw_text.size() && (std::isspace(static_cast<unsigned char>(raw_text[start])) || raw_text[start] == '"')) {
        start++;
    }

    if (start >= raw_text.size()) return "";

    size_t end = raw_text.size() - 1;
    while (end > start && (std::isspace(static_cast<unsigned char>(raw_text[end])) || raw_text[end] == '"')) {
        end--;
    }

    return raw_text.substr(start, end - start + 1);
}

bool WhisperStt::IsHallucination(const std::string& text) {
    if (text.empty()) return true;

    // Convert to lowercase for comparison
    std::string lower = text;
    std::transform(lower.begin(), lower.end(), lower.begin(), [](unsigned char c) {
        return static_cast<char>(std::tolower(c));
    });

    // Check against classic Whisper silent hallucinations
    static const char* kHallucinations[] = {
        "[blank_audio]",
        "(blank_audio)",
        "[silence]",
        "(silence)",
        "[music]",
        "(music)",
        "[applause]",
        "(applause)",
        "[laughter]",
        "(laughter)",
        "(cough)",
        "(sigh)",
        "(groan)",
        "(clears throat)",
        "thank you.",
        "thanks for watching!",
        "subscribe to my channel",
        "subtitles by",
        "translated by"
    };

    for (const char* h : kHallucinations) {
        if (lower == h || lower.find(h) != std::string::npos) {
            return true;
        }
    }

    // Check if string consists purely of punctuation and whitespace
    bool has_alphanumeric = false;
    for (char c : text) {
        if (std::isalnum(static_cast<unsigned char>(c))) {
            has_alphanumeric = true;
            break;
        }
    }

    return !has_alphanumeric;
}

} // namespace jarvis::audio
