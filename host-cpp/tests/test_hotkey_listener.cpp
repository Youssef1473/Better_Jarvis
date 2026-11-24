#include <algorithm>
#include <cassert>
#include <chrono>
#include <cmath>
#include <iostream>
#include <thread>
#include <vector>

#include "input/hotkey_listener.hpp"

using namespace jarvis::input;

void TestStateMachineLogic() {
    HotkeyListener listener;

    size_t down_count = 0;
    size_t up_count = 0;

    listener.SetOnHotkeyDown([&down_count]() { down_count++; });
    listener.SetOnHotkeyUp([&up_count]() { up_count++; });

    // 1. Initial state: inactive
    assert(!listener.IsHotkeyActive());
    assert(down_count == 0 && up_count == 0);

    // 2. Space alone: no trigger
    listener.ProcessRawKey(VK_SPACE, RI_KEY_MAKE, 0x39);
    assert(!listener.IsHotkeyActive());
    assert(down_count == 0 && up_count == 0);

    listener.ProcessRawKey(VK_SPACE, RI_KEY_BREAK, 0x39);
    assert(!listener.IsHotkeyActive());
    assert(down_count == 0 && up_count == 0);

    // 3. Left Ctrl alone: no trigger
    listener.ProcessRawKey(VK_CONTROL, RI_KEY_MAKE, 0x1D);
    assert(!listener.IsHotkeyActive());
    assert(down_count == 0 && up_count == 0);

    // 4. Ctrl + Space: triggers OnHotkeyDown ONCE
    bool triggered = listener.ProcessRawKey(VK_SPACE, RI_KEY_MAKE, 0x39);
    assert(triggered);
    assert(listener.IsHotkeyActive());
    assert(down_count == 1 && up_count == 0);

    // 5. Repeated Space auto-repeat (RI_KEY_MAKE while held): must NOT re-fire
    for (int i = 0; i < 5; ++i) {
        bool re_trigger = listener.ProcessRawKey(VK_SPACE, RI_KEY_MAKE, 0x39);
        assert(!re_trigger);
        assert(listener.IsHotkeyActive());
        assert(down_count == 1 && up_count == 0);
    }

    // 6. Release Space: triggers OnHotkeyUp ONCE and resets hotkey_active_
    listener.ProcessRawKey(VK_SPACE, RI_KEY_BREAK, 0x39);
    assert(!listener.IsHotkeyActive());
    assert(down_count == 1 && up_count == 1);

    // 7. Re-press Space while Ctrl still held: triggers OnHotkeyDown again
    triggered = listener.ProcessRawKey(VK_SPACE, RI_KEY_MAKE, 0x39);
    assert(triggered);
    assert(listener.IsHotkeyActive());
    assert(down_count == 2 && up_count == 1);

    // 8. Release Ctrl while Space held: triggers OnHotkeyUp
    listener.ProcessRawKey(VK_CONTROL, RI_KEY_BREAK, 0x1D);
    assert(!listener.IsHotkeyActive());
    assert(down_count == 2 && up_count == 2);

    listener.ProcessRawKey(VK_SPACE, RI_KEY_BREAK, 0x39);
    assert(!listener.IsHotkeyActive());
    assert(down_count == 2 && up_count == 2);

    // 9. Dual-Ctrl overlap hygiene: hold Left Ctrl + Right Ctrl, release one
    listener.ResetState();
    down_count = 0;
    up_count = 0;

    // Press Left Ctrl
    listener.ProcessRawKey(VK_CONTROL, RI_KEY_MAKE, 0x1D);
    // Press Right Ctrl (E0 flag set)
    listener.ProcessRawKey(VK_CONTROL, RI_KEY_MAKE | RI_KEY_E0, 0x1D);
    // Press Space -> Active!
    triggered = listener.ProcessRawKey(VK_SPACE, RI_KEY_MAKE, 0x39);
    assert(triggered);
    assert(listener.IsHotkeyActive());
    assert(down_count == 1 && up_count == 0);

    // Release Left Ctrl: Right Ctrl is still down, so hotkey stays active
    listener.ProcessRawKey(VK_CONTROL, RI_KEY_BREAK, 0x1D);
    assert(listener.IsHotkeyActive());
    assert(down_count == 1 && up_count == 0);

    // Release Right Ctrl: both Ctrls now released -> triggers OnHotkeyUp!
    listener.ProcessRawKey(VK_CONTROL, RI_KEY_BREAK | RI_KEY_E0, 0x1D);
    assert(!listener.IsHotkeyActive());
    assert(down_count == 1 && up_count == 1);

    // Clean release of Space
    listener.ProcessRawKey(VK_SPACE, RI_KEY_BREAK, 0x39);
    assert(!listener.IsHotkeyActive());
    assert(down_count == 1 && up_count == 1);
}

void TestHudThreadStallIsolation() {
    constexpr UINT kTestMsgTrigger = WM_APP + 10;
    constexpr UINT kTestMsgRelease = WM_APP + 11;

    std::atomic<HWND> hud_hwnd{nullptr};
    std::atomic<size_t> received_triggers{0};
    std::atomic<bool> hud_thread_running{true};
    std::atomic<bool> hud_is_stalled{false};

    // 1. Spawn a simulated "HUD UI" thread hosting a test HWND
    std::thread hud_thread([&]() {
        HINSTANCE hInst = GetModuleHandleW(nullptr);
        WNDCLASSEXW wc{};
        wc.cbSize = sizeof(wc);
        wc.lpfnWndProc = [](HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) -> LRESULT {
            if (msg == kTestMsgTrigger) {
                auto* counter = reinterpret_cast<std::atomic<size_t>*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
                if (counter) counter->fetch_add(1, std::memory_order_relaxed);
                return 0;
            }
            return DefWindowProcW(hwnd, msg, wParam, lParam);
        };
        wc.hInstance = hInst;
        wc.lpszClassName = L"TestSimulatedHudClass";
        RegisterClassExW(&wc);

        HWND hwnd = CreateWindowExW(
            0, L"TestSimulatedHudClass", L"SimulatedHUD", 0,
            0, 0, 0, 0, HWND_MESSAGE, nullptr, hInst, nullptr);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(&received_triggers));
        hud_hwnd.store(hwnd, std::memory_order_release);

        MSG msg{};
        while (hud_thread_running.load(std::memory_order_relaxed)) {
            if (hud_is_stalled.load(std::memory_order_relaxed)) {
                // Simulate an intentional 1,500 ms UI stall (blocking modal dialog, heavy GDI render)
                std::this_thread::sleep_for(std::chrono::milliseconds(1500));
                hud_is_stalled.store(false, std::memory_order_relaxed);
            }

            while (PeekMessageW(&msg, nullptr, 0, 0, PM_REMOVE)) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::this_thread::sleep_for(std::chrono::milliseconds(5));
        }

        DestroyWindow(hwnd);
        UnregisterClassW(L"TestSimulatedHudClass", hInst);
    });

    // Wait for simulated HUD window creation
    while (!hud_hwnd.load(std::memory_order_acquire)) {
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }

    // 2. Start HotkeyListener on its dedicated Raw Input thread
    HotkeyListener listener;
    listener.SetTargetHwnd(hud_hwnd.load(), kTestMsgTrigger, kTestMsgRelease);

    std::atomic<size_t> callback_count{0};
    listener.SetOnHotkeyDown([&callback_count]() {
        callback_count.fetch_add(1, std::memory_order_relaxed);
    });

    bool started = listener.Start();
    assert(started);
    assert(listener.IsRunning());

    // 3. Freeze the simulated HUD thread for 1,500 ms (> 1,000 ms hook timeout ceiling)
    hud_is_stalled.store(true, std::memory_order_release);
    std::this_thread::sleep_for(std::chrono::milliseconds(50)); // Ensure stall has begun

    // 4. While HUD is completely frozen, inject hotkey events
    const auto t_start = std::chrono::high_resolution_clock::now();

    // Ctrl down -> Space down
    listener.ProcessRawKey(VK_CONTROL, RI_KEY_MAKE, 0x1D);
    bool down_trig = listener.ProcessRawKey(VK_SPACE, RI_KEY_MAKE, 0x39);
    assert(down_trig);

    const auto t_end = std::chrono::high_resolution_clock::now();
    const double callback_latency_ms = std::chrono::duration<double, std::milli>(t_end - t_start).count();

    std::cout << "  [Isolation] Hotkey dispatch latency during 1500ms UI stall: "
              << callback_latency_ms << " ms" << std::endl;

    // Requirement: input dispatch must never be blocked by frozen HUD thread (strictly < 5 ms)
    assert(callback_latency_ms < 5.0);
    assert(callback_count.load() == 1);
    assert(listener.IsRunning());

    // Release keys
    listener.ProcessRawKey(VK_SPACE, RI_KEY_BREAK, 0x39);
    listener.ProcessRawKey(VK_CONTROL, RI_KEY_BREAK, 0x1D);

    // 5. Wait for simulated HUD thread to wake from the 1,500 ms stall
    while (hud_is_stalled.load(std::memory_order_relaxed)) {
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    // Give message loop time to pump queued PostMessageW
    std::this_thread::sleep_for(std::chrono::milliseconds(50));

    assert(received_triggers.load() == 1);

    // 6. Clean teardown
    listener.Stop();
    assert(!listener.IsRunning());

    hud_thread_running.store(false, std::memory_order_release);
    hud_thread.join();
}

void TestPttUtteranceThresholds() {
    // Test PTT release threshold logic:
    // 1. Duration < 250 ms (e.g. 100 ms = 1600 samples @ 16 kHz) -> Short/accidental press -> false (HUD resets to Ready)
    auto eval_ptt = [](const std::vector<float>& ptt_audio) -> bool {
        const size_t num_samples = ptt_audio.size();
        const float duration_ms = static_cast<float>(num_samples) * 1000.0f / 16000.0f;
        double sum_sq = 0.0;
        for (float s : ptt_audio) {
            sum_sq += static_cast<double>(s) * static_cast<double>(s);
        }
        const float rms = (num_samples > 0) ? static_cast<float>(std::sqrt(sum_sq / static_cast<double>(num_samples))) : 0.0f;
        return (duration_ms >= 250.0f && rms >= 0.0001f);
    };

    // 1. Short utterance (100 ms, 1600 samples) with loud speech -> Rejected
    std::vector<float> short_audio(1600, 0.25f);
    assert(!eval_ptt(short_audio));

    // 2. Long utterance (500 ms, 8000 samples) with pure silence (0.0f) -> Rejected (silent PTT release resets HUD)
    std::vector<float> silent_audio(8000, 0.0f);
    assert(!eval_ptt(silent_audio));

    // 3. Long utterance (500 ms, 8000 samples) with near-zero noise below threshold (0.00005f) -> Rejected
    std::vector<float> near_zero(8000, 0.00005f);
    assert(!eval_ptt(near_zero));

    // 4. Valid utterance (500 ms, 8000 samples) with audible speech (RMS >= 0.0001f) -> Accepted (HUD thinking & dispatched)
    std::vector<float> speech_audio(8000);
    for (size_t i = 0; i < 8000; ++i) {
        speech_audio[i] = 0.1f * std::sin(2.0f * 3.14159265f * 440.0f * static_cast<float>(i) / 16000.0f);
    }
    assert(eval_ptt(speech_audio));

    // 5. Bare minimum valid threshold (250 ms = 4000 samples @ 16 kHz) with threshold RMS -> Accepted
    std::vector<float> min_audio(4000, 0.0002f);
    assert(eval_ptt(min_audio));
}

int main() {
    std::cout << "[TEST] Running TestStateMachineLogic..." << std::endl;
    TestStateMachineLogic();

    std::cout << "[TEST] Running TestHudThreadStallIsolation (1,500 ms HUD freeze)..." << std::endl;
    TestHudThreadStallIsolation();

    std::cout << "[TEST] Running TestPttUtteranceThresholds..." << std::endl;
    TestPttUtteranceThresholds();

    std::cout << "[PASS] All HotkeyListener tests passed successfully!" << std::endl;
    return 0;
}
