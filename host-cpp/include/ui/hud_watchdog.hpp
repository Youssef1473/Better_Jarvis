#pragma once
#include "ui/hud_state.hpp"
#include <chrono>
#include <string>

namespace jarvis::ui {

struct WatchdogOutcome {
    bool triggered = false;
    bool state_changed = false;
    HudState new_state = HudState::Thinking;
    std::wstring new_label;
    bool request_redraw = false;
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

} // namespace jarvis::ui
