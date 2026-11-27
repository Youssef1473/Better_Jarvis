#pragma once
#include "ui/hud_state.hpp"
#include <atomic>
#include <chrono>
#include <cstdint>
#include <mutex>
#include <string>

namespace jarvis::ui {

struct WatchdogOutcome {
    bool triggered = false;
    bool state_changed = false;
    HudState new_state = HudState::Thinking;
    std::wstring new_label;
    bool request_redraw = false;
};

inline bool ShouldDispatchVadUtterance(bool ptt_active, bool is_paused) noexcept {
    return !ptt_active && !is_paused;
}

class HudRuntimeState {
public:
    explicit HudRuntimeState(
        HudState initial_state = HudState::Connecting,
        std::wstring initial_label = L"JARVIS // CONNECTING...")
        : state_(initial_state),
          label_(std::move(initial_label)),
          listening_start_time_(std::chrono::steady_clock::now()),
          thinking_start_time_(listening_start_time_) {}

    void Transition(
        HudState next_state,
        std::wstring next_label,
        std::chrono::steady_clock::time_point now = std::chrono::steady_clock::now())
    {
        std::lock_guard<std::mutex> lock(mutex_);
        const HudState prev = state_.load(std::memory_order_relaxed);
        state_.store(next_state, std::memory_order_release);
        label_ = std::move(next_label);
        if (next_state == HudState::Listening && prev != HudState::Listening) {
            listening_start_time_ = now;
        } else if (next_state == HudState::Thinking && prev != HudState::Thinking) {
            thinking_start_time_ = now;
        }
    }

    void ResetListeningStart(std::chrono::steady_clock::time_point now = std::chrono::steady_clock::now()) {
        std::lock_guard<std::mutex> lock(mutex_);
        listening_start_time_ = now;
    }

    void SetLabelOnly(std::wstring next_label) {
        std::lock_guard<std::mutex> lock(mutex_);
        label_ = std::move(next_label);
    }

    HudState State() const noexcept {
        return state_.load(std::memory_order_acquire);
    }

    std::wstring Label() const {
        std::lock_guard<std::mutex> lock(mutex_);
        return label_;
    }

    std::chrono::steady_clock::time_point ListeningStartTime() const {
        std::lock_guard<std::mutex> lock(mutex_);
        return listening_start_time_;
    }

    std::chrono::steady_clock::time_point ThinkingStartTime() const {
        std::lock_guard<std::mutex> lock(mutex_);
        return thinking_start_time_;
    }

private:
    mutable std::mutex mutex_;
    std::atomic<HudState> state_;
    std::wstring label_;
    std::chrono::steady_clock::time_point listening_start_time_;
    std::chrono::steady_clock::time_point thinking_start_time_;
};

inline WatchdogOutcome EvaluateThinkingWatchdog(
    HudState current_state,
    bool is_paused,
    std::chrono::steady_clock::time_point start_time,
    std::chrono::steady_clock::time_point now,
    std::chrono::milliseconds timeout_threshold = std::chrono::milliseconds(20000))
{
    WatchdogOutcome outcome;
    if (current_state != HudState::Thinking) {
        return outcome;
    }

    if ((now - start_time) >= timeout_threshold) {
        outcome.triggered = true;
        if (!is_paused) {
            outcome.state_changed = true;
            outcome.new_state = HudState::Ready;
            outcome.new_label = L"JARVIS // READY";
            outcome.request_redraw = true;
        } else {
            outcome.state_changed = false;
            outcome.new_state = HudState::Thinking;
            outcome.request_redraw = false;
        }
    }
    return outcome;
}

inline std::wstring Utf8ToWide(const std::string& utf8)
{
    std::wstring out;
    out.reserve(utf8.size());
    size_t i = 0;
    const size_t n = utf8.size();
    while (i < n) {
        uint32_t cp = 0xFFFD;
        const auto b0 = static_cast<unsigned char>(utf8[i]);
        if (b0 < 0x80) {
            cp = b0;
            ++i;
        } else if ((b0 & 0xE0) == 0xC0 && i + 1 < n) {
            const auto b1 = static_cast<unsigned char>(utf8[i + 1]);
            if ((b1 & 0xC0) == 0x80 && b0 >= 0xC2) {
                cp = ((static_cast<uint32_t>(b0 & 0x1F) << 6) |
                      static_cast<uint32_t>(b1 & 0x3F));
                i += 2;
            } else {
                ++i;
            }
        } else if ((b0 & 0xF0) == 0xE0 && i + 2 < n) {
            const auto b1 = static_cast<unsigned char>(utf8[i + 1]);
            const auto b2 = static_cast<unsigned char>(utf8[i + 2]);
            if ((b1 & 0xC0) == 0x80 && (b2 & 0xC0) == 0x80 &&
                !(b0 == 0xE0 && b1 < 0xA0) && !(b0 == 0xED && b1 >= 0xA0)) {
                cp = ((static_cast<uint32_t>(b0 & 0x0F) << 12) |
                      (static_cast<uint32_t>(b1 & 0x3F) << 6) |
                      static_cast<uint32_t>(b2 & 0x3F));
                i += 3;
            } else {
                ++i;
            }
        } else if ((b0 & 0xF8) == 0xF0 && i + 3 < n) {
            const auto b1 = static_cast<unsigned char>(utf8[i + 1]);
            const auto b2 = static_cast<unsigned char>(utf8[i + 2]);
            const auto b3 = static_cast<unsigned char>(utf8[i + 3]);
            if ((b1 & 0xC0) == 0x80 && (b2 & 0xC0) == 0x80 && (b3 & 0xC0) == 0x80 &&
                !(b0 == 0xF0 && b1 < 0x90) && !(b0 == 0xF4 && b1 > 0x8F) && b0 <= 0xF4) {
                cp = ((static_cast<uint32_t>(b0 & 0x07) << 18) |
                      (static_cast<uint32_t>(b1 & 0x3F) << 12) |
                      (static_cast<uint32_t>(b2 & 0x3F) << 6) |
                      static_cast<uint32_t>(b3 & 0x3F));
                i += 4;
            } else {
                ++i;
            }
        } else {
            ++i;
        }

        if (cp <= 0xFFFF) {
            out.push_back(static_cast<wchar_t>(cp));
        } else if constexpr (sizeof(wchar_t) == 2) {
            cp -= 0x10000;
            out.push_back(static_cast<wchar_t>(0xD800 + (cp >> 10)));
            out.push_back(static_cast<wchar_t>(0xDC00 + (cp & 0x3FF)));
        } else {
            out.push_back(static_cast<wchar_t>(cp));
        }
    }
    return out;
}

inline std::wstring FormatHudLabelWithVision(
    const std::wstring& base_label,
    bool vision_active,
    const std::wstring& vision_app)
{
    if (!vision_active || vision_app.empty()) {
        return base_label;
    }
    return base_label + L" | VISION [" + vision_app + L"]";
}

} // namespace jarvis::ui
