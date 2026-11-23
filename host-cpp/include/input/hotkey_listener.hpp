#pragma once

#include <windows.h>
#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <thread>

namespace jarvis::input {

/// Dedicated-thread Raw Input Hotkey Listener.
/// Uses a dedicated OS thread ("jarvis-raw-input") with a message-only HWND
/// and RegisterRawInputDevices(RIDEV_INPUTSINK) to track Ctrl+Space without
/// WH_KEYBOARD_LL hook injection, eliminating LowLevelHooksTimeout unhooking risks.
///
/// Windows UIPI Notice: Raw Input delivers global background input across
/// normal Medium-Integrity desktop applications. If an elevated Administrator
/// window has foreground focus, Windows UIPI security policy restricts background
/// input delivery to un-elevated processes. To receive hotkeys while an elevated
/// app is active, Better_Jarvis must run with elevated privileges or uiAccess.
class HotkeyListener {
public:
    using HotkeyCallback = std::function<void()>;

    HotkeyListener();
    ~HotkeyListener();

    // Non-copyable, non-movable
    HotkeyListener(const HotkeyListener&) = delete;
    HotkeyListener& operator=(const HotkeyListener&) = delete;

    /// Sets callbacks for edge-triggered hotkey state transitions
    void SetOnHotkeyDown(HotkeyCallback cb) { on_hotkey_down_ = std::move(cb); }
    void SetOnHotkeyUp(HotkeyCallback cb) { on_hotkey_up_ = std::move(cb); }

    /// Sets target HWND to receive non-blocking PostMessageW dispatch
    void SetTargetHwnd(HWND target_hwnd, UINT msg_down = 0, UINT msg_up = 0) noexcept {
        target_hwnd_ = target_hwnd;
        msg_down_ = msg_down;
        msg_up_ = msg_up;
    }

    /// Starts the dedicated worker thread and registers Raw Input
    bool Start();

    /// Stops the worker thread, unregisters Raw Input, and destroys the message-only window
    void Stop();

    /// Whether the listener thread is active
    bool IsRunning() const noexcept { return running_.load(std::memory_order_relaxed); }

    /// Whether Ctrl+Space is currently actively held
    bool IsHotkeyActive() const noexcept { return hotkey_active_.load(std::memory_order_relaxed); }

    /// Pure state-machine feed for unit testing without live OS Raw Input
    /// Returns true if HotkeyDown edge was triggered, false otherwise
    bool ProcessRawKey(USHORT vkey, USHORT flags, USHORT make_code) noexcept;

    /// Resets internal key state
    void ResetState() noexcept;

private:
    void WorkerThreadLoop();
    static LRESULT CALLBACK MsgWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam);

private:
    std::thread       worker_thread_;
    std::atomic<bool> running_{false};
    DWORD             worker_thread_id_{0};
    HWND              msg_hwnd_{nullptr};

    HWND              target_hwnd_{nullptr};
    UINT              msg_down_{0};
    UINT              msg_up_{0};

    HotkeyCallback    on_hotkey_down_;
    HotkeyCallback    on_hotkey_up_;

    // Deterministic key-state tracking (no GetAsyncKeyState)
    bool              ctrl_left_down_{false};
    bool              ctrl_right_down_{false};
    bool              space_down_{false};
    std::atomic<bool> hotkey_active_{false};
};

} // namespace jarvis::input
