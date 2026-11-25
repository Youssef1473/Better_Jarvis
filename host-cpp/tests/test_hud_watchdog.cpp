#include <cassert>
#include <chrono>
#include <iostream>
#include "ui/hud_state.hpp"
#include "ui/hud_watchdog.hpp"

using namespace jarvis::ui;

void TestNonThinkingStatesIgnored() {
    HudState non_thinking_states[] = {
        HudState::Connecting,
        HudState::Ready,
        HudState::Listening,
        HudState::Speaking,
        HudState::Confirmation,
        HudState::Paused
    };

    auto start = std::chrono::steady_clock::now();
    auto past_timeout = start + std::chrono::milliseconds(25000);

    for (HudState s : non_thinking_states) {
        auto outcome = EvaluateThinkingWatchdog(s, false, start, past_timeout);
        assert(!outcome.triggered);
        assert(!outcome.state_changed);
        assert(!outcome.request_redraw);
    }
    std::cout << "[PASS] TestNonThinkingStatesIgnored" << std::endl;
}

void TestThinkingTimeoutBoundary() {
    auto start = std::chrono::steady_clock::now();

    // 19,999ms: should not trigger
    auto just_before = start + std::chrono::milliseconds(19999);
    auto outcome_before = EvaluateThinkingWatchdog(HudState::Thinking, false, start, just_before);
    assert(!outcome_before.triggered);
    assert(!outcome_before.state_changed);
    assert(!outcome_before.request_redraw);

    // Exactly 20,000ms: should trigger
    auto exact = start + std::chrono::milliseconds(20000);
    auto outcome_exact = EvaluateThinkingWatchdog(HudState::Thinking, false, start, exact);
    assert(outcome_exact.triggered);
    assert(outcome_exact.state_changed);
    assert(outcome_exact.new_state == HudState::Ready);
    assert(outcome_exact.new_label == L"JARVIS // READY");
    assert(outcome_exact.request_redraw);

    std::cout << "[PASS] TestThinkingTimeoutBoundary" << std::endl;
}

void TestThinkingPausedSuppression() {
    auto start = std::chrono::steady_clock::now();
    auto past_timeout = start + std::chrono::milliseconds(25000);

    // Paused is true: triggered is true, but state must not change and no redraw
    auto outcome_paused = EvaluateThinkingWatchdog(HudState::Thinking, true, start, past_timeout);
    assert(outcome_paused.triggered);
    assert(!outcome_paused.state_changed);
    assert(outcome_paused.new_state == HudState::Thinking);
    assert(outcome_paused.new_label.empty());
    assert(!outcome_paused.request_redraw);

    std::cout << "[PASS] TestThinkingPausedSuppression" << std::endl;
}

int main() {
    std::cout << "=== Running HUD Watchdog Regression Tests ===" << std::endl;
    TestNonThinkingStatesIgnored();
    TestThinkingTimeoutBoundary();
    TestThinkingPausedSuppression();
    std::cout << "All HUD Watchdog tests passed successfully." << std::endl;
    return 0;
}
