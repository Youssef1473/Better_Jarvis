#include "input/hotkey_listener.hpp"

#include <iostream>

namespace jarvis::input {

namespace {

constexpr wchar_t kRawInputClassName[] = L"JarvisRawInputSinkClass";

} // namespace

HotkeyListener::HotkeyListener() = default;

HotkeyListener::~HotkeyListener() {
    Stop();
}

void HotkeyListener::ResetState() noexcept {
    ctrl_left_down_ = false;
    ctrl_right_down_ = false;
    space_down_ = false;
    hotkey_active_.store(false, std::memory_order_relaxed);
}

bool HotkeyListener::ProcessRawKey(USHORT vkey, USHORT flags, USHORT make_code) noexcept {
    (void)make_code;
    const bool is_break = (flags & RI_KEY_BREAK) != 0;

    if (vkey == VK_CONTROL || vkey == VK_LCONTROL || vkey == VK_RCONTROL) {
        if ((flags & RI_KEY_E0) != 0 || vkey == VK_RCONTROL) {
            ctrl_right_down_ = !is_break;
        } else {
            ctrl_left_down_ = !is_break;
        }
    } else if (vkey == VK_SPACE) {
        space_down_ = !is_break;
    }

    const bool ctrl_down = (ctrl_left_down_ || ctrl_right_down_);

    if (ctrl_down && space_down_) {
        // Edge trigger: fire only when transitioning from inactive -> active
        if (!hotkey_active_.load(std::memory_order_relaxed)) {
            hotkey_active_.store(true, std::memory_order_release);
            if (on_hotkey_down_) {
                on_hotkey_down_();
            }
            if (target_hwnd_ && msg_down_) {
                PostMessageW(target_hwnd_, msg_down_, 0, 0);
            }
            return true;
        }
    } else {
        // Edge trigger: fire release only when transitioning from active -> inactive
        if (hotkey_active_.load(std::memory_order_relaxed)) {
            hotkey_active_.store(false, std::memory_order_release);
            if (on_hotkey_up_) {
                on_hotkey_up_();
            }
            if (target_hwnd_ && msg_up_) {
                PostMessageW(target_hwnd_, msg_up_, 0, 0);
            }
        }
    }

    return false;
}

LRESULT CALLBACK HotkeyListener::MsgWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
    if (msg == WM_CREATE) {
        auto* cs = reinterpret_cast<CREATESTRUCTW*>(lParam);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(cs->lpCreateParams));
        return 0;
    }

    auto* listener = reinterpret_cast<HotkeyListener*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));

    if (msg == WM_INPUT && listener) {
        UINT dwSize = 0;
        GetRawInputData(reinterpret_cast<HRAWINPUT>(lParam), RID_INPUT, nullptr, &dwSize, sizeof(RAWINPUTHEADER));

        alignas(RAWINPUT) BYTE raw_buf[sizeof(RAWINPUT) + 64];
        if (dwSize > 0 && dwSize <= sizeof(raw_buf)) {
            if (GetRawInputData(reinterpret_cast<HRAWINPUT>(lParam), RID_INPUT, raw_buf, &dwSize, sizeof(RAWINPUTHEADER)) == dwSize) {
                auto* raw = reinterpret_cast<RAWINPUT*>(raw_buf);
                if (raw->header.dwType == RIM_TYPEKEYBOARD) {
                    listener->ProcessRawKey(
                        raw->data.keyboard.VKey,
                        raw->data.keyboard.Flags,
                        raw->data.keyboard.MakeCode);
                }
            }
        }
        return DefWindowProcW(hwnd, msg, wParam, lParam);
    }

    if (msg == WM_CLOSE) {
        DestroyWindow(hwnd);
        return 0;
    }

    if (msg == WM_DESTROY) {
        PostQuitMessage(0);
        return 0;
    }

    return DefWindowProcW(hwnd, msg, wParam, lParam);
}

void HotkeyListener::WorkerThreadLoop() {
    worker_thread_id_ = GetCurrentThreadId();

    HINSTANCE hInstance = GetModuleHandleW(nullptr);
    WNDCLASSEXW wc{};
    wc.cbSize = sizeof(wc);
    wc.lpfnWndProc = MsgWndProc;
    wc.hInstance = hInstance;
    wc.lpszClassName = kRawInputClassName;
    RegisterClassExW(&wc);

    msg_hwnd_ = CreateWindowExW(
        0,
        kRawInputClassName,
        L"RawInputSink",
        0, 0, 0, 0, 0,
        HWND_MESSAGE,
        nullptr,
        hInstance,
        this);

    if (!msg_hwnd_) {
        running_.store(false, std::memory_order_release);
        return;
    }

    // Register Raw Input Keyboard device with RIDEV_INPUTSINK
    RAWINPUTDEVICE rid{};
    rid.usUsagePage = 0x01; // Generic Desktop Controls
    rid.usUsage     = 0x06; // Keyboard
    rid.dwFlags     = RIDEV_INPUTSINK;
    rid.hwndTarget  = msg_hwnd_;

    if (!RegisterRawInputDevices(&rid, 1, sizeof(rid))) {
        DestroyWindow(msg_hwnd_);
        msg_hwnd_ = nullptr;
        running_.store(false, std::memory_order_release);
        return;
    }

    MSG msg{};
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    // Unregister Raw Input
    rid.dwFlags = RIDEV_REMOVE;
    rid.hwndTarget = nullptr;
    RegisterRawInputDevices(&rid, 1, sizeof(rid));

    UnregisterClassW(kRawInputClassName, hInstance);
    msg_hwnd_ = nullptr;
    running_.store(false, std::memory_order_release);
}

bool HotkeyListener::Start() {
    if (running_.load(std::memory_order_relaxed)) {
        return true;
    }

    running_.store(true, std::memory_order_release);

    HANDLE hReady = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    worker_thread_ = std::thread([this, hReady]() {
        SetEvent(hReady);
        WorkerThreadLoop();
    });

    WaitForSingleObject(hReady, 2000);
    CloseHandle(hReady);

    return running_.load(std::memory_order_acquire);
}

void HotkeyListener::Stop() {
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }

    if (msg_hwnd_) {
        PostMessageW(msg_hwnd_, WM_CLOSE, 0, 0);
    }
    if (worker_thread_id_ != 0) {
        PostThreadMessageW(worker_thread_id_, WM_QUIT, 0, 0);
    }

    if (worker_thread_.joinable()) {
        worker_thread_.join();
    }

    worker_thread_id_ = 0;
    ResetState();
}

} // namespace jarvis::input
