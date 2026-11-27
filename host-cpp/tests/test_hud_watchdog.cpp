#include "test_assert.hpp"
#include <chrono>
#include <iostream>
#include <thread>
#include <vector>
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
        TEST_ASSERT(!outcome.triggered);
        TEST_ASSERT(!outcome.state_changed);
        TEST_ASSERT(!outcome.request_redraw);
    }
    std::cout << "[PASS] TestNonThinkingStatesIgnored" << std::endl;
}

void TestThinkingTimeoutBoundary() {
    auto start = std::chrono::steady_clock::now();

    // 19,999ms: should not trigger
    auto just_before = start + std::chrono::milliseconds(19999);
    auto outcome_before = EvaluateThinkingWatchdog(HudState::Thinking, false, start, just_before);
    TEST_ASSERT(!outcome_before.triggered);
    TEST_ASSERT(!outcome_before.state_changed);
    TEST_ASSERT(!outcome_before.request_redraw);

    // Exactly 20,000ms: should trigger
    auto exact = start + std::chrono::milliseconds(20000);
    auto outcome_exact = EvaluateThinkingWatchdog(HudState::Thinking, false, start, exact);
    TEST_ASSERT(outcome_exact.triggered);
    TEST_ASSERT(outcome_exact.state_changed);
    TEST_ASSERT(outcome_exact.new_state == HudState::Ready);
    TEST_ASSERT(outcome_exact.new_label == L"JARVIS // READY");
    TEST_ASSERT(outcome_exact.request_redraw);

    std::cout << "[PASS] TestThinkingTimeoutBoundary" << std::endl;
}

void TestThinkingPausedSuppression() {
    auto start = std::chrono::steady_clock::now();
    auto past_timeout = start + std::chrono::milliseconds(25000);

    // Paused is true: triggered is true, but state must not change and no redraw
    auto outcome_paused = EvaluateThinkingWatchdog(HudState::Thinking, true, start, past_timeout);
    TEST_ASSERT(outcome_paused.triggered);
    TEST_ASSERT(!outcome_paused.state_changed);
    TEST_ASSERT(outcome_paused.new_state == HudState::Thinking);
    TEST_ASSERT(outcome_paused.new_label.empty());
    TEST_ASSERT(!outcome_paused.request_redraw);

    std::cout << "[PASS] TestThinkingPausedSuppression" << std::endl;
}

void TestThinkingTimerAnchoredAtThinkingTransition() {
    auto t0 = std::chrono::steady_clock::now();
    HudRuntimeState hud;
    hud.Transition(HudState::Listening, L"JARVIS // LISTENING...", t0);

    // 14 seconds spent listening, then transitions into Thinking at t0 + 14s
    auto t_think_start = t0 + std::chrono::seconds(14);
    hud.Transition(HudState::Thinking, L"JARVIS // THINKING...", t_think_start);

    // 7 seconds into Thinking (21s since Listening started, but only 7s since Thinking started)
    auto t_check = t_think_start + std::chrono::seconds(7);
    auto outcome_early = EvaluateThinkingWatchdog(
        hud.State(),
        false,
        hud.ThinkingStartTime(),
        t_check,
        std::chrono::milliseconds(20000));
    TEST_ASSERT(!outcome_early.triggered);
    TEST_ASSERT(!outcome_early.state_changed);

    // 20 seconds into Thinking: must trigger
    auto t_expired = t_think_start + std::chrono::seconds(20);
    auto outcome_expired = EvaluateThinkingWatchdog(
        hud.State(),
        false,
        hud.ThinkingStartTime(),
        t_expired,
        std::chrono::milliseconds(20000));
    TEST_ASSERT(outcome_expired.triggered);
    TEST_ASSERT(outcome_expired.state_changed);
    TEST_ASSERT(outcome_expired.new_state == HudState::Ready);

    std::cout << "[PASS] TestThinkingTimerAnchoredAtThinkingTransition" << std::endl;
}

void TestConcurrentHudLabelSafetyAndVadPttGate() {
    HudRuntimeState hud;
    std::vector<std::thread> writers;
    for (int i = 0; i < 4; ++i) {
        writers.emplace_back([&hud, i]() {
            for (int iter = 0; iter < 500; ++iter) {
                hud.Transition(HudState::Speaking, L"JARVIS // SPEAKING #" + std::to_wstring(i * 1000 + iter));
                hud.SetLabelOnly(L"SPEECH: sample " + std::to_wstring(iter));
            }
        });
    }
    for (int iter = 0; iter < 1000; ++iter) {
        std::wstring snapshot = hud.Label();
        TEST_ASSERT(!snapshot.empty());
    }
    for (auto& t : writers) {
        t.join();
    }

    TEST_ASSERT(ShouldDispatchVadUtterance(false, false));
    TEST_ASSERT(!ShouldDispatchVadUtterance(true, false));
    TEST_ASSERT(!ShouldDispatchVadUtterance(false, true));
    TEST_ASSERT(!ShouldDispatchVadUtterance(true, true));

    std::cout << "[PASS] TestConcurrentHudLabelSafetyAndVadPttGate" << std::endl;
}

void TestFormatHudLabelWithVision() {
    using jarvis::ui::FormatHudLabelWithVision;

    TEST_ASSERT(FormatHudLabelWithVision(L"JARVIS // READY", false, L"code.exe") == L"JARVIS // READY");
    TEST_ASSERT(FormatHudLabelWithVision(L"JARVIS // READY", true, L"") == L"JARVIS // READY");
    TEST_ASSERT(
        FormatHudLabelWithVision(L"JARVIS // THINKING", true, L"chrome.exe") ==
        L"JARVIS // THINKING | VISION [chrome.exe]");

    std::cout << "[PASS] TestFormatHudLabelWithVision" << std::endl;
}

void TestUtf8ToWideConversion() {
    using jarvis::ui::Utf8ToWide;

    TEST_ASSERT(Utf8ToWide("").empty());
    TEST_ASSERT(Utf8ToWide("chrome.exe") == L"chrome.exe");
    // "Café" (C, a, f, U+00E9) + " — " (U+2014 Em Dash) + rocket (U+1F680)
    const std::string utf8 = "Caf\xC3\xA9 \xE2\x80\x94 \xF0\x9F\x9A\x80";
    const std::wstring wide = Utf8ToWide(utf8);
    const std::wstring expected = L"Caf\u00E9 \u2014 \U0001F680";
    TEST_ASSERT(wide == expected);
    // Invalid standalone continuation byte safely becomes U+FFFD
    TEST_ASSERT(Utf8ToWide("\xFF") == L"\uFFFD");

    std::cout << "[PASS] TestUtf8ToWideConversion" << std::endl;
}

int main() {
    std::cout << "=== Running HUD Watchdog Regression Tests ===" << std::endl;
    TestNonThinkingStatesIgnored();
    TestThinkingTimeoutBoundary();
    TestThinkingPausedSuppression();
    TestThinkingTimerAnchoredAtThinkingTransition();
    TestConcurrentHudLabelSafetyAndVadPttGate();
    TestFormatHudLabelWithVision();
    TestUtf8ToWideConversion();
    std::cout << "All HUD Watchdog tests passed successfully." << std::endl;
    return 0;
}
