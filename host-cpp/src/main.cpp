#include <windows.h>
#include <dwmapi.h>
#include <shellapi.h>

#include <atomic>
#include <chrono>
#include <cmath>
#include <cstring>
#include <filesystem>
#include <mutex>
#include <string>
#include <thread>
#include <vector>
#include <iostream>

#include <nlohmann/json.hpp>
#include "ipc/frame_codec.hpp"
#include "audio/mmcss_guard.hpp"
#include "audio/ring_buffer.hpp"
#include "audio/aec_processor.hpp"
#include "audio/wasapi_capture.hpp"
#include "audio/silero_vad.hpp"
#include "audio/whisper_stt.hpp"
#include "audio/piper_tts.hpp"
#include "input/hotkey_listener.hpp"
#include "telemetry/latency_tracker.hpp"

using json = nlohmann::json;

namespace {

constexpr wchar_t kPipeName[]         = L"\\\\.\\pipe\\jarvis-ipc";
constexpr wchar_t kHudClassName[]     = L"JarvisFloatingPillWnd";

constexpr UINT    WM_APP_IPC_READY    = WM_APP + 1;
constexpr UINT    WM_APP_TRAY         = WM_APP + 2;
constexpr UINT    WM_APP_SPEECH       = WM_APP + 3;
constexpr UINT    WM_APP_CONFIRMATION = WM_APP + 4;
constexpr UINT    WM_APP_TRIGGER_SPEAK = WM_APP + 5;
constexpr UINT    WM_APP_RELEASE_SPEAK = WM_APP + 6;

constexpr UINT    ID_TRAY_EXIT        = 1001;
constexpr UINT    ID_TIMER_ANIMATION  = 3001;
constexpr UINT    ID_TIMER_CONFIRM    = 3002;

enum class HudState {
    Connecting,
    Ready,
    Listening,
    Thinking,
    Speaking,
    Confirmation,
    Paused
};

HWND                  g_hwnd = nullptr;
HANDLE                g_job  = nullptr;
HANDLE                g_daemon_process = nullptr;
std::atomic<DWORD>    g_daemon_pid{0};
std::wstring          g_custom_data_dir;
std::unique_ptr<jarvis::input::HotkeyListener> g_hotkey;
std::atomic<bool>     g_running{true};
std::atomic<bool>     g_daemon_ready{false};
std::atomic<bool>     g_is_paused{false};
std::atomic<HudState> g_hud_state{HudState::Connecting};
std::wstring          g_hud_label = L"JARVIS // CONNECTING...";
float                 g_anim_phase = 0.0f;

// Amber Confirmation State
std::atomic<uint64_t> g_confirm_id{0};
std::atomic<uint64_t> g_confirm_timeout_ms{8000};
std::mutex            g_confirm_mutex;
std::wstring          g_confirm_command;

// Audio Subsystems
jarvis::audio::SpscRingBuffer<32768>           g_raw_mic_buffer;
jarvis::audio::SpscRingBuffer<32768>           g_clean_ring_buffer;
jarvis::audio::AecProcessor                    g_aec;
std::atomic<uint32_t>                          g_active_turn_id{1};
std::atomic<uint32_t>                          g_cancelled_turn_id{0};
jarvis::telemetry::LatencyTracker              g_latency_tracker;
std::unique_ptr<jarvis::audio::WasapiCapture>  g_capture;
std::unique_ptr<jarvis::audio::SileroVad>      g_vad;
std::unique_ptr<jarvis::audio::WhisperStt>     g_stt;
std::unique_ptr<jarvis::audio::PiperTts>       g_tts;

std::mutex g_ipc_out_mutex;
std::condition_variable g_ipc_out_cv;
std::deque<std::vector<uint8_t>> g_ipc_out_queue;

void EnqueueIpcFrame(std::vector<uint8_t>&& frame) {
    {
        std::lock_guard<std::mutex> lock(g_ipc_out_mutex);
        if (g_ipc_out_queue.size() > 32) {
            g_ipc_out_queue.pop_front();
        }
        g_ipc_out_queue.push_back(std::move(frame));
    }
    g_ipc_out_cv.notify_one();
}

bool SendIpcJson(const json& msg) {
    auto frame = jarvis::ipc::EncodeFrame(jarvis::ipc::kMsgTypeControlJson, msg.dump());
    EnqueueIpcFrame(std::move(frame));
    return true;
}

bool OverlappedReadExact(HANDLE pipe, void* dst, DWORD bytes_to_read, HANDLE hEvent, std::atomic<bool>& running) {
    uint8_t* ptr = static_cast<uint8_t*>(dst);
    DWORD total = 0;
    while (total < bytes_to_read && running.load()) {
        OVERLAPPED ov{};
        ov.hEvent = hEvent;
        ResetEvent(hEvent);
        DWORD read_now = 0;
        BOOL ok = ReadFile(pipe, ptr + total, bytes_to_read - total, &read_now, &ov);
        if (!ok) {
            DWORD err = GetLastError();
            if (err == ERROR_IO_PENDING) {
                while (running.load()) {
                    DWORD wait_res = WaitForSingleObject(hEvent, 100);
                    if (wait_res == WAIT_OBJECT_0) {
                        if (!GetOverlappedResult(pipe, &ov, &read_now, FALSE) || read_now == 0) {
                            return false;
                        }
                        break;
                    } else if (wait_res == WAIT_TIMEOUT) {
                        continue;
                    } else {
                        return false;
                    }
                }
            } else {
                return false;
            }
        }
        if (read_now == 0) return false;
        total += read_now;
    }
    return total == bytes_to_read;
}

bool OverlappedWriteExact(HANDLE pipe, const void* src, DWORD bytes_to_write, HANDLE hEvent, std::atomic<bool>& running) {
    const uint8_t* ptr = static_cast<const uint8_t*>(src);
    DWORD total = 0;
    while (total < bytes_to_write && running.load()) {
        OVERLAPPED ov{};
        ov.hEvent = hEvent;
        ResetEvent(hEvent);
        DWORD written = 0;
        BOOL ok = WriteFile(pipe, ptr + total, bytes_to_write - total, &written, &ov);
        if (!ok) {
            DWORD err = GetLastError();
            if (err == ERROR_IO_PENDING) {
                while (running.load()) {
                    DWORD wait_res = WaitForSingleObject(hEvent, 100);
                    if (wait_res == WAIT_OBJECT_0) {
                        if (!GetOverlappedResult(pipe, &ov, &written, FALSE) || written == 0) {
                            return false;
                        }
                        break;
                    } else if (wait_res == WAIT_TIMEOUT) {
                        continue;
                    } else {
                        return false;
                    }
                }
            } else {
                return false;
            }
        }
        if (written == 0) return false;
        total += written;
    }
    return total == bytes_to_write;
}

void ResolveConfirmation(bool approved) {
    uint64_t id = g_confirm_id.exchange(0);
    if (id == 0) return;

    KillTimer(g_hwnd, ID_TIMER_CONFIRM);

    json reply = {
        {"jsonrpc", "2.0"},
        {"method", "system.confirm"},
        {"params", {
            {"id", id},
            {"approved", approved}
        }}
    };
    SendIpcJson(reply);

    // Restore normal pill geometry (270 x 40)
    const int pill_w = 270;
    const int pill_h = 40;
    const int screen_w = GetSystemMetrics(SM_CXSCREEN);
    const int pos_x = (screen_w - pill_w) / 2;
    SetWindowPos(g_hwnd, HWND_TOPMOST, pos_x, 18, pill_w, pill_h, SWP_NOACTIVATE | SWP_SHOWWINDOW);

    g_hud_state.store(approved ? HudState::Thinking : HudState::Ready);
    g_hud_label = approved ? L"JARVIS // EXECUTING..." : L"JARVIS // CANCELLED";
    InvalidateRect(g_hwnd, nullptr, TRUE);
}

void IpcWorkerLoop() {
    CoInitializeEx(nullptr, COINIT_MULTITHREADED);

    while (g_running.load()) {
        HANDLE pipe = CreateFileW(
            kPipeName,
            GENERIC_READ | GENERIC_WRITE,
            0,
            nullptr,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            nullptr);

        if (pipe == INVALID_HANDLE_VALUE) {
            std::this_thread::sleep_for(std::chrono::milliseconds(250));
            continue;
        }

        // Verify connected server PID against expected daemon PID
        const DWORD expected_pid = g_daemon_pid.load();
        if (expected_pid != 0) {
            ULONG server_pid = 0;
            if (GetNamedPipeServerProcessId(pipe, &server_pid)) {
                if (server_pid != expected_pid) {
                    // Mismatched server PID! Close immediately and fail closed
                    CloseHandle(pipe);
                    std::this_thread::sleep_for(std::chrono::milliseconds(250));
                    continue;
                }
            }
        }

        HANDLE hReadEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        HANDLE hWriteEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (!hReadEvent || !hWriteEvent) {
            if (hReadEvent) CloseHandle(hReadEvent);
            if (hWriteEvent) CloseHandle(hWriteEvent);
            CloseHandle(pipe);
            std::this_thread::sleep_for(std::chrono::milliseconds(250));
            continue;
        }

        std::atomic<bool> pipe_alive{true};

        // Dedicated background writer thread
        std::thread writer_thread([pipe, hWriteEvent, &pipe_alive]() {
            while (g_running.load() && pipe_alive.load()) {
                std::vector<uint8_t> frame;
                {
                    std::unique_lock<std::mutex> lock(g_ipc_out_mutex);
                    g_ipc_out_cv.wait_for(lock, std::chrono::milliseconds(100), [&pipe_alive]() {
                        return !g_running.load() || !pipe_alive.load() || !g_ipc_out_queue.empty();
                    });
                    if (!g_running.load() || !pipe_alive.load()) break;
                    if (g_ipc_out_queue.empty()) continue;
                    frame = std::move(g_ipc_out_queue.front());
                    g_ipc_out_queue.pop_front();
                }

                if (!frame.empty()) {
                    if (!OverlappedWriteExact(pipe, frame.data(), static_cast<DWORD>(frame.size()), hWriteEvent, g_running)) {
                        pipe_alive.store(false);
                        break;
                    }
                }
            }
        });

        // Handshake Ping
        json ping = {
            {"jsonrpc", "2.0"},
            {"method", "system.ping"},
            {"params", {{"client", "jarvis-host"}}},
            {"id", 1}
        };
        SendIpcJson(ping);

        // Frame read loop
        while (g_running.load() && pipe_alive.load()) {
            uint8_t header_buf[jarvis::ipc::kHeaderSize]{};
            if (!OverlappedReadExact(pipe, header_buf, jarvis::ipc::kHeaderSize, hReadEvent, g_running)) {
                break;
            }

            jarvis::ipc::Header hdr{};
            if (!jarvis::ipc::DecodeHeader(header_buf, jarvis::ipc::kHeaderSize, hdr)) {
                // Protocol violation! Bad magic, wrong version, or oversized payload. Close immediately.
                break;
            }

            std::string payload(hdr.payload_len, '\0');
            if (hdr.payload_len > 0 && !OverlappedReadExact(pipe, payload.data(), hdr.payload_len, hReadEvent, g_running)) {
                break;
            }

            if (hdr.msg_type == jarvis::ipc::kMsgTypeControlJson) {
                auto resp = json::parse(payload, nullptr, false);
                if (!resp.is_discarded()) {
                    if (resp.contains("result")) {
                        const std::string status = resp["result"].value("status", "");
                        if (status == "READY") {
                            g_daemon_ready.store(true);
                            if (!g_is_paused.load()) {
                                g_hud_state.store(HudState::Ready);
                            }
                            if (g_hwnd) {
                                PostMessageW(g_hwnd, WM_APP_IPC_READY, 0, 0);
                            }
                        }
                    } else if (resp.contains("method")) {
                        const std::string method = resp["method"].get<std::string>();
                        if (method == "tts.speak_chunk" && resp.contains("params")) {
                            const auto& p = resp["params"];
                            const uint32_t turn_id = p.value("turn_id", 0U);
                            const uint32_t seq = p.value("seq", 0U);
                            const std::string text = p.value("text", "");
                            const bool is_final = p.value("is_final", false);

                            const uint32_t active = g_active_turn_id.load(std::memory_order_acquire);
                            const uint32_t cancelled = g_cancelled_turn_id.load(std::memory_order_acquire);

                            // Stale-turn filtering: drop immediately if not active or if cancelled
                            if (turn_id != 0 && (turn_id != active || turn_id == cancelled || g_latency_tracker.IsTurnCancelled(turn_id))) {
                                // Drop stale / cancelled chunk
                            } else {
                                if (g_tts && !text.empty() && !g_is_paused.load()) {
                                    if (seq == 0) {
                                        g_latency_tracker.RecordTtsFirstAudio(turn_id);
                                        g_hud_state.store(HudState::Speaking);
                                        g_hud_label = L"JARVIS // SPEAKING...";
                                        if (g_hwnd) {
                                            PostMessageW(g_hwnd, WM_APP_SPEECH, 0, 0);
                                        }
                                    }
                                    g_tts->Speak(text);
                                }

                                // When stream is final, if nothing is speaking, cleanly return HUD to Ready
                                if (is_final && g_tts && !g_tts->IsSpeaking() && g_hud_state.load() == HudState::Thinking) {
                                    g_hud_state.store(g_is_paused.load() ? HudState::Paused : HudState::Ready);
                                    g_hud_label = g_is_paused.load() ? L"JARVIS // PAUSED" : L"JARVIS // READY";
                                    if (g_hwnd) {
                                        PostMessageW(g_hwnd, WM_APP_SPEECH, 0, 0);
                                    }
                                }
                            }
                        } else if (method == "tts.speak" && resp.contains("params")) {
                            const uint32_t turn_id = resp["params"].value("turn_id", g_active_turn_id.load());
                            const uint32_t active = g_active_turn_id.load(std::memory_order_acquire);
                            const uint32_t cancelled = g_cancelled_turn_id.load(std::memory_order_acquire);

                            if (turn_id == 0 || (turn_id == active && turn_id != cancelled && !g_latency_tracker.IsTurnCancelled(turn_id))) {
                                g_latency_tracker.RecordTtsFirstAudio(turn_id);
                                const std::string text = resp["params"].value("text", "");
                                if (g_tts && !text.empty() && !g_is_paused.load()) {
                                    g_hud_state.store(HudState::Speaking);
                                    g_hud_label = L"JARVIS // SPEAKING...";
                                    if (g_hwnd) {
                                        PostMessageW(g_hwnd, WM_APP_SPEECH, 0, 0);
                                    }
                                    g_tts->Speak(text);
                                }
                            }
                        } else if (method == "tts.stop") {
                            if (g_tts) {
                                g_tts->BargeIn();
                            }
                            if (g_hud_state.load() == HudState::Speaking) {
                                g_hud_state.store(g_is_paused.load() ? HudState::Paused : HudState::Ready);
                                g_hud_label = g_is_paused.load() ? L"JARVIS // PAUSED" : L"JARVIS // READY";
                                if (g_hwnd) {
                                    PostMessageW(g_hwnd, WM_APP_SPEECH, 0, 0);
                                }
                            }
                        } else if (method == "hud.show_confirmation" && resp.contains("params")) {
                            const uint64_t id = resp["params"].value("id", 0ULL);
                            const std::string cmd = resp["params"].value("command", "");
                            const uint64_t timeout_ms = resp["params"].value("timeout_ms", 8000ULL);

                            g_confirm_id.store(id);
                            {
                                std::lock_guard<std::mutex> lock(g_confirm_mutex);
                                g_confirm_command = std::wstring(cmd.begin(), cmd.end());
                            }
                            g_confirm_timeout_ms.store(timeout_ms);
                            g_hud_state.store(HudState::Confirmation);

                            if (g_hwnd) {
                                PostMessageW(g_hwnd, WM_APP_CONFIRMATION, 0, 0);
                            }
                        }
                    }
                }
            }
        }

        pipe_alive.store(false);
        g_ipc_out_cv.notify_all();
        if (writer_thread.joinable()) {
            writer_thread.join();
        }

        CloseHandle(hReadEvent);
        CloseHandle(hWriteEvent);
        CloseHandle(pipe);

        g_daemon_ready.store(false);
        g_hud_state.store(HudState::Connecting);
        std::this_thread::sleep_for(std::chrono::milliseconds(500));
    }

    CoUninitialize();
}

void VadWorkerLoop() {
    float mic_block[160];
    float ref_block[160];
    float clean_block[160];

    bool aec_logged = false;

    while (g_running.load(std::memory_order_relaxed)) {
        const bool os_aec_active = (g_capture && g_capture->IsOsAecActive());
        if (!aec_logged) {
            aec_logged = true;
            if (os_aec_active) {
                std::cout << "[AudioPipeline] Active AEC mode: Windows 11 Hardware/OS AEC (software NLMS bypassed)" << std::endl;
            } else {
                std::cout << "[AudioPipeline] Active AEC mode: C++20 Software NLMS AEC" << std::endl;
            }
        }

        // 1. Drain raw microphone samples through AEC processor into clean ring buffer
        while (g_raw_mic_buffer.AvailableRead() >= 160) {
            size_t read_mic = g_raw_mic_buffer.PopBatch(mic_block, 160);
            if (read_mic < 160) break;

            const bool is_speaking = (g_tts && g_tts->IsSpeaking());

            // Fetch matching reference samples from TTS far-end reference buffer
            size_t read_ref = 0;
            if (g_tts) {
                read_ref = g_tts->FarEndReferenceBuffer().PopBatch(ref_block, 160);
            }
            if (read_ref < 160) {
                std::fill(ref_block + read_ref, ref_block + 160, 0.0f);
            }

            if (os_aec_active) {
                // Prevent double-AEC: Windows 11 hardware/driver already cancels echo
                g_clean_ring_buffer.PushBatch(mic_block, 160);
            } else {
                g_aec.ProcessFrame(mic_block, ref_block, clean_block, is_speaking);
                g_clean_ring_buffer.PushBatch(clean_block, 160);
            }
        }

        // 2. Feed clean ring buffer to Silero VAD
        if (g_vad) {
            const bool is_speaking = (g_tts && g_tts->IsSpeaking());
            g_vad->SetSpeakingMode(is_speaking);

            bool processed = g_vad->ProcessNextFrame();

            // When paused, bypass audio handoff
            if (g_is_paused.load(std::memory_order_relaxed)) {
                std::this_thread::sleep_for(std::chrono::milliseconds(15));
                continue;
            }

            if (g_vad->IsInSpeech()) {
                if (is_speaking) {
                    // Barge-In Action Sequence:
                    // 1. Instant sub-20ms audio cutoff
                    g_tts->StopPlayback();

                    // Consumer-side drain of residual far-end reference samples
                    float drain_buf[160];
                    while (g_tts->FarEndReferenceBuffer().AvailableRead() > 0) {
                        g_tts->FarEndReferenceBuffer().PopBatch(drain_buf, 160);
                    }

                    // 2. Emit JRVS v2 system.barge_in frame to cancel active daemon turn
                    const uint32_t turn_id = g_active_turn_id.load(std::memory_order_relaxed);
                    g_cancelled_turn_id.store(turn_id, std::memory_order_release);
                    g_latency_tracker.CancelTurn(turn_id);
                    json barge_in_msg = {
                        {"jsonrpc", "2.0"},
                        {"method", "system.barge_in"},
                        {"params", {{"turn_id", turn_id}}}
                    };
                    SendIpcJson(barge_in_msg);

                    // 3. Immediate HUD state transition to Listening
                    g_hud_state.store(HudState::Listening);
                    g_hud_label = L"JARVIS // LISTENING (BARGE-IN)...";
                    if (g_hwnd) {
                        PostMessageW(g_hwnd, WM_APP_SPEECH, 0, 0);
                    }
                } else if (g_hud_state.load() != HudState::Listening && g_hud_state.load() != HudState::Confirmation) {
                    g_hud_state.store(HudState::Listening);
                    g_hud_label = L"JARVIS // LISTENING...";
                    if (g_hwnd) {
                        PostMessageW(g_hwnd, WM_APP_SPEECH, 0, 0);
                    }
                }
            }
            if (!processed) {
                std::this_thread::sleep_for(std::chrono::milliseconds(4));
            }
        }
    }
}

void SpawnDaemonInJob() {
    g_job = CreateJobObjectW(nullptr, nullptr);
    if (g_job) {
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION jeli{};
        jeli.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(g_job, JobObjectExtendedLimitInformation, &jeli, sizeof(jeli));
    }

    wchar_t self_path[MAX_PATH]{};
    GetModuleFileNameW(nullptr, self_path, MAX_PATH);
    std::filesystem::path exe_dir = std::filesystem::path(self_path).parent_path();
    std::filesystem::path daemon_path = exe_dir / L"jarvis-daemon.exe";

    if (!std::filesystem::exists(daemon_path)) {
        // Fallback to Cargo release/debug path during local dev
        daemon_path = std::filesystem::current_path() / L"target" / L"release" / L"jarvis-daemon.exe";
        if (!std::filesystem::exists(daemon_path)) {
            daemon_path = std::filesystem::current_path() / L"target" / L"debug" / L"jarvis-daemon.exe";
        }
    }

    STARTUPINFOW si{};
    si.cb = sizeof(si);
    PROCESS_INFORMATION pi{};
    std::wstring cmd = L"\"" + daemon_path.wstring() + L"\" --parent-pid " + std::to_wstring(GetCurrentProcessId());
    if (!g_custom_data_dir.empty()) {
        cmd += L" --data-dir \"" + g_custom_data_dir + L"\"";
    }

    if (CreateProcessW(
            nullptr,
            cmd.data(),
            nullptr,
            nullptr,
            FALSE,
            CREATE_NO_WINDOW | CREATE_SUSPENDED,
            nullptr,
            nullptr,
            &si,
            &pi)) {
        g_daemon_pid.store(pi.dwProcessId);
        if (g_job) {
            AssignProcessToJobObject(g_job, pi.hProcess);
        }
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);
        g_daemon_process = pi.hProcess;
    }
}

void AddTrayIcon(HWND hwnd) {
    NOTIFYICONDATAW nid{};
    nid.cbSize = sizeof(nid);
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_APP_TRAY;
    nid.hIcon = LoadIconW(nullptr, MAKEINTRESOURCEW(32512));
    // Windows UIPI Notice: Raw Input receives background input across normal desktop windows,
    // but Windows UIPI blocks background input if an elevated Administrator app has foreground focus.
    wcscpy_s(nid.szTip, L"Jarvis Assistant (Ctrl+Space to talk)\n[Note: Run as Admin if using elevated apps - UIPI]");
    Shell_NotifyIconW(NIM_ADD, &nid);
}

void RemoveTrayIcon(HWND hwnd) {
    NOTIFYICONDATAW nid{};
    nid.cbSize = sizeof(nid);
    nid.hWnd = hwnd;
    nid.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

LRESULT CALLBACK HudWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
    switch (msg) {
        case WM_CREATE: {
            SetTimer(hwnd, ID_TIMER_ANIMATION, 33, nullptr); // ~30 FPS animation timer
            return 0;
        }
        case WM_APP_TRIGGER_SPEAK: {
            // Unpause if paused
            if (g_is_paused.load()) {
                g_is_paused.store(false);
            }

            // Immediate barge-in on voice playback
            if (g_tts && g_tts->IsSpeaking()) {
                g_tts->BargeIn();
                const uint32_t turn_id = g_active_turn_id.load(std::memory_order_relaxed);
                g_cancelled_turn_id.store(turn_id, std::memory_order_release);
                g_latency_tracker.CancelTurn(turn_id);
            }
            if (g_stt) {
                g_stt->ClearQueue();
            }
            if (g_vad) {
                g_vad->ResetState();
            }

            json stop_msg = {
                {"jsonrpc", "2.0"},
                {"method", "tts.stop"},
                {"params", json::object()}
            };
            SendIpcJson(stop_msg);

            g_hud_state.store(HudState::Listening);
            g_hud_label = L"JARVIS // LISTENING...";
            InvalidateRect(hwnd, nullptr, TRUE);

            // Immediate voice acknowledgment
            if (g_tts) {
                g_tts->Speak("Yes, sir?");
            }

            return 0;
        }
        case WM_APP_RELEASE_SPEAK: {
            // Push-to-talk release marks PTT up boundary
            g_latency_tracker.RecordPttUp(g_active_turn_id.load(std::memory_order_relaxed));
            return 0;
        }
        case WM_APP_IPC_READY: {
            if (!g_is_paused.load()) {
                g_hud_state.store(HudState::Ready);
                g_hud_label = L"JARVIS // READY";
            }
            InvalidateRect(hwnd, nullptr, TRUE);
            return 0;
        }
        case WM_APP_SPEECH: {
            InvalidateRect(hwnd, nullptr, TRUE);
            return 0;
        }
        case WM_APP_CONFIRMATION: {
            // Expand window geometry to confirmation card (440 x 100)
            const int card_w = 440;
            const int card_h = 100;
            const int screen_w = GetSystemMetrics(SM_CXSCREEN);
            const int pos_x = (screen_w - card_w) / 2;
            SetWindowPos(hwnd, HWND_TOPMOST, pos_x, 18, card_w, card_h, SWP_SHOWWINDOW);

            SetForegroundWindow(hwnd);
            SetFocus(hwnd);

            SetTimer(hwnd, ID_TIMER_CONFIRM, static_cast<UINT>(g_confirm_timeout_ms.load()), nullptr);
            InvalidateRect(hwnd, nullptr, TRUE);
            return 0;
        }
        case WM_KEYDOWN: {
            if (g_hud_state.load() == HudState::Confirmation) {
                if (wParam == VK_RETURN) {
                    ResolveConfirmation(true);
                    return 0;
                } else if (wParam == VK_ESCAPE) {
                    ResolveConfirmation(false);
                    return 0;
                }
            } else if (wParam == VK_ESCAPE) {
                // Global escape key stops any active speaking/processing
                if (g_tts) g_tts->BargeIn();
                if (g_stt) g_stt->ClearQueue();
                if (!g_is_paused.load()) {
                    g_hud_state.store(HudState::Ready);
                    g_hud_label = L"JARVIS // READY";
                    InvalidateRect(hwnd, nullptr, TRUE);
                }
                return 0;
            }
            return 0;
        }
        case WM_LBUTTONUP: {
            int mx = LOWORD(lParam);
            int my = HIWORD(lParam);
            RECT rc{};
            GetClientRect(hwnd, &rc);

            const HudState state = g_hud_state.load();

            if (state == HudState::Confirmation) {
                // Confirmation button clicks
                if (my >= 60 && my <= 88) {
                    if (mx >= 24 && mx <= 190) {
                        ResolveConfirmation(true);
                        return 0;
                    } else if (mx >= 250 && mx <= 416) {
                        ResolveConfirmation(false);
                        return 0;
                    }
                }
            } else {
                // Stop / Pause / Resume button click on right side of pill
                RECT stop_btn_rc{rc.right - 34, rc.top + 8, rc.right - 8, rc.bottom - 8};
                if (mx >= stop_btn_rc.left && mx <= stop_btn_rc.right &&
                    my >= stop_btn_rc.top && my <= stop_btn_rc.bottom) {

                    bool new_paused = !g_is_paused.load();
                    g_is_paused.store(new_paused);

                    // Immediate cutoff
                    if (g_tts) g_tts->BargeIn();
                    if (g_stt) g_stt->ClearQueue();
                    if (g_vad) g_vad->ResetState();

                    json stop_msg = {
                        {"jsonrpc", "2.0"},
                        {"method", "tts.stop"},
                        {"params", json::object()}
                    };
                    SendIpcJson(stop_msg);

                    if (new_paused) {
                        g_hud_state.store(HudState::Paused);
                        g_hud_label = L"JARVIS // PAUSED";
                    } else {
                        g_hud_state.store(HudState::Ready);
                        g_hud_label = L"JARVIS // READY";
                    }

                    InvalidateRect(hwnd, nullptr, TRUE);
                    return 0;
                } else {
                    // Clicked anywhere on the HUD pill body -> trigger voice listening & speak acknowledgment
                    PostMessageW(hwnd, WM_APP_TRIGGER_SPEAK, 0, 0);
                    return 0;
                }
            }
            return 0;
        }
        case WM_TIMER: {
            if (wParam == ID_TIMER_ANIMATION) {
                g_anim_phase += 0.15f;
                if (g_anim_phase > 62.83f) g_anim_phase = 0.0f;

                // Return to Ready when TTS completes
                if (g_hud_state.load() == HudState::Speaking && g_tts && !g_tts->IsSpeaking()) {
                    g_hud_state.store(g_is_paused.load() ? HudState::Paused : HudState::Ready);
                    g_hud_label = g_is_paused.load() ? L"JARVIS // PAUSED" : L"JARVIS // READY";
                    InvalidateRect(hwnd, nullptr, TRUE);
                } else if (g_hud_state.load() == HudState::Listening ||
                           g_hud_state.load() == HudState::Thinking ||
                           g_hud_state.load() == HudState::Speaking) {
                    InvalidateRect(hwnd, nullptr, FALSE);
                }
            } else if (wParam == ID_TIMER_CONFIRM) {
                ResolveConfirmation(false);
            }
            return 0;
        }
        case WM_APP_TRAY: {
            if (LOWORD(lParam) == WM_RBUTTONUP) {
                POINT pt{};
                GetCursorPos(&pt);
                HMENU menu = CreatePopupMenu();
                AppendMenuW(menu, MF_STRING, 1002, g_is_paused.load() ? L"Resume Jarvis" : L"Pause / Stop Jarvis");
                AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
                AppendMenuW(menu, MF_STRING, ID_TRAY_EXIT, L"Exit Jarvis");
                SetForegroundWindow(hwnd);
                TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, nullptr);
                DestroyMenu(menu);
            }
            return 0;
        }
        case WM_COMMAND: {
            if (LOWORD(wParam) == ID_TRAY_EXIT) {
                DestroyWindow(hwnd);
            } else if (LOWORD(wParam) == 1002) {
                // Tray Pause/Resume toggle
                bool new_paused = !g_is_paused.load();
                g_is_paused.store(new_paused);
                if (g_tts) g_tts->BargeIn();
                if (g_stt) g_stt->ClearQueue();

                if (new_paused) {
                    g_hud_state.store(HudState::Paused);
                    g_hud_label = L"JARVIS // PAUSED";
                } else {
                    g_hud_state.store(HudState::Ready);
                    g_hud_label = L"JARVIS // READY";
                }
                InvalidateRect(hwnd, nullptr, TRUE);
            }
            return 0;
        }
        case WM_PAINT: {
            PAINTSTRUCT ps{};
            HDC hdc = BeginPaint(hwnd, &ps);
            RECT rc{};
            GetClientRect(hwnd, &rc);

            const HudState state = g_hud_state.load();

            if (state == HudState::Confirmation) {
                // Amber Confirmation Card Render
                HBRUSH bg = CreateSolidBrush(RGB(28, 22, 16));
                HPEN border = CreatePen(PS_SOLID, 2, RGB(243, 156, 18));
                SelectObject(hdc, bg);
                SelectObject(hdc, border);
                RoundRect(hdc, rc.left + 1, rc.top + 1, rc.right - 1, rc.bottom - 1, 16, 16);
                DeleteObject(border);
                DeleteObject(bg);

                SetBkMode(hdc, TRANSPARENT);

                // Title
                SetTextColor(hdc, RGB(255, 183, 77));
                RECT title_rc{rc.left + 18, rc.top + 8, rc.right - 18, rc.top + 28};
                DrawTextW(hdc, L"[!] CONFIRMATION REQUIRED", -1, &title_rc, DT_LEFT | DT_SINGLELINE);

                // Command description
                SetTextColor(hdc, RGB(235, 235, 235));
                RECT desc_rc{rc.left + 18, rc.top + 30, rc.right - 18, rc.top + 54};
                std::wstring cmd_text;
                {
                    std::lock_guard<std::mutex> lock(g_confirm_mutex);
                    cmd_text = g_confirm_command;
                }
                DrawTextW(hdc, cmd_text.c_str(), -1, &desc_rc, DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS);

                // Allow Button [Enter]
                RECT allow_rc{rc.left + 24, rc.top + 60, rc.left + 190, rc.top + 88};
                HBRUSH allow_bg = CreateSolidBrush(RGB(20, 56, 32));
                HPEN allow_border = CreatePen(PS_SOLID, 1, RGB(46, 204, 113));
                SelectObject(hdc, allow_bg);
                SelectObject(hdc, allow_border);
                RoundRect(hdc, allow_rc.left, allow_rc.top, allow_rc.right, allow_rc.bottom, 10, 10);
                SetTextColor(hdc, RGB(130, 245, 175));
                DrawTextW(hdc, L"ENTER: Allow", -1, &allow_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                DeleteObject(allow_border);
                DeleteObject(allow_bg);

                // Deny Button [Esc]
                RECT deny_rc{rc.right - 190, rc.top + 60, rc.right - 24, rc.top + 88};
                HBRUSH deny_bg = CreateSolidBrush(RGB(56, 20, 24));
                HPEN deny_border = CreatePen(PS_SOLID, 1, RGB(231, 76, 60));
                SelectObject(hdc, deny_bg);
                SelectObject(hdc, deny_border);
                RoundRect(hdc, deny_rc.left, deny_rc.top, deny_rc.right, deny_rc.bottom, 10, 10);
                SetTextColor(hdc, RGB(255, 160, 160));
                DrawTextW(hdc, L"ESC: Deny", -1, &deny_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                DeleteObject(deny_border);
                DeleteObject(deny_bg);
            } else {
                // Adaptive Pill Render (270 x 40)
                COLORREF bg_color = RGB(22, 24, 30);
                COLORREF border_color = RGB(90, 100, 120);
                COLORREF text_color = RGB(210, 215, 225);

                switch (state) {
                    case HudState::Ready:
                        bg_color = RGB(10, 26, 18);
                        border_color = RGB(46, 204, 113);
                        text_color = RGB(130, 245, 175);
                        break;
                    case HudState::Listening:
                        bg_color = RGB(12, 22, 34);
                        border_color = RGB(52, 152, 219);
                        text_color = RGB(140, 215, 255);
                        break;
                    case HudState::Thinking:
                        bg_color = RGB(24, 16, 36);
                        border_color = RGB(155, 89, 182);
                        text_color = RGB(220, 175, 245);
                        break;
                    case HudState::Speaking:
                        bg_color = RGB(14, 32, 24);
                        border_color = RGB(26, 188, 156);
                        text_color = RGB(150, 245, 220);
                        break;
                    case HudState::Paused:
                        bg_color = RGB(26, 22, 26);
                        border_color = RGB(190, 70, 70);
                        text_color = RGB(230, 150, 150);
                        break;
                    default:
                        break;
                }

                HBRUSH bg = CreateSolidBrush(bg_color);
                HPEN border = CreatePen(PS_SOLID, 2, border_color);
                SelectObject(hdc, bg);
                SelectObject(hdc, border);
                RoundRect(hdc, rc.left + 1, rc.top + 1, rc.right - 1, rc.bottom - 1, 20, 20);

                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, text_color);

                // Render Left Indicator (Equalizer bars when Listening, glowing dot otherwise)
                if (state == HudState::Listening) {
                    HBRUSH bar_brush = CreateSolidBrush(border_color);
                    for (int i = 0; i < 5; ++i) {
                        float h = 6.0f + 10.0f * (0.5f + 0.5f * std::sin(g_anim_phase + static_cast<float>(i) * 0.9f));
                        int bx = rc.left + 14 + i * 5;
                        int by = (rc.bottom - static_cast<int>(h)) / 2;
                        RECT b_rc{bx, by, bx + 3, by + static_cast<int>(h)};
                        FillRect(hdc, &b_rc, bar_brush);
                    }
                    DeleteObject(bar_brush);

                    RECT text_rc{rc.left + 44, rc.top, rc.right - 38, rc.bottom};
                    DrawTextW(hdc, g_hud_label.c_str(), -1, &text_rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                } else if (state == HudState::Ready || state == HudState::Paused) {
                    COLORREF dot_color = (state == HudState::Paused) ? RGB(231, 76, 60) : border_color;
                    HBRUSH dot_brush = CreateSolidBrush(dot_color);
                    SelectObject(hdc, dot_brush);
                    Ellipse(hdc, rc.left + 14, (rc.bottom - 8) / 2, rc.left + 22, (rc.bottom + 8) / 2);
                    DeleteObject(dot_brush);

                    RECT text_rc{rc.left + 28, rc.top, rc.right - 38, rc.bottom};
                    DrawTextW(hdc, g_hud_label.c_str(), -1, &text_rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                } else {
                    RECT text_rc{rc.left + 18, rc.top, rc.right - 38, rc.bottom};
                    DrawTextW(hdc, g_hud_label.c_str(), -1, &text_rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                }

                // Render Stop / Pause / Resume Button on Right Edge of Pill
                RECT stop_btn_rc{rc.right - 32, rc.top + 9, rc.right - 9, rc.bottom - 9};
                const bool paused = g_is_paused.load();

                COLORREF btn_bg = paused ? RGB(16, 46, 26) : RGB(46, 18, 22);
                COLORREF btn_border = paused ? RGB(46, 204, 113) : RGB(231, 76, 60);
                COLORREF btn_fg = paused ? RGB(130, 245, 175) : RGB(255, 140, 140);

                HBRUSH s_bg = CreateSolidBrush(btn_bg);
                HPEN s_pen = CreatePen(PS_SOLID, 1, btn_border);
                SelectObject(hdc, s_bg);
                SelectObject(hdc, s_pen);
                RoundRect(hdc, stop_btn_rc.left, stop_btn_rc.top, stop_btn_rc.right, stop_btn_rc.bottom, 8, 8);
                DeleteObject(s_pen);
                DeleteObject(s_bg);

                SetTextColor(hdc, btn_fg);
                DrawTextW(hdc, paused ? L"▶" : L"■", -1, &stop_btn_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

                DeleteObject(border);
                DeleteObject(bg);
            }

            EndPaint(hwnd, &ps);
            return 0;
        }
        case WM_DESTROY: {
            g_running.store(false);
            if (g_hotkey) {
                g_hotkey->Stop();
                g_hotkey.reset();
            }
            KillTimer(hwnd, ID_TIMER_ANIMATION);
            KillTimer(hwnd, ID_TIMER_CONFIRM);
            RemoveTrayIcon(hwnd);

            // Send graceful shutdown RPC to daemon
            json shutdown_msg = {
                {"jsonrpc", "2.0"},
                {"method", "system.shutdown"},
                {"params", json::object()},
                {"id", 9999}
            };
            SendIpcJson(shutdown_msg);

            // Wait up to 2000 ms for daemon to cleanly flush SQLite WAL and exit
            if (g_daemon_process) {
                WaitForSingleObject(g_daemon_process, 2000);
                CloseHandle(g_daemon_process);
                g_daemon_process = nullptr;
            }

            if (g_job) {
                CloseHandle(g_job); // Triggers JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                g_job = nullptr;
            }
            PostQuitMessage(0);
            return 0;
        }
        default:
            return DefWindowProcW(hwnd, msg, wParam, lParam);
    }
}

} // namespace

int WINAPI wWinMain(HINSTANCE hInstance, HINSTANCE, PWSTR lpCmdLine, int) {
    CoInitialize(nullptr);

    const std::wstring args = lpCmdLine ? lpCmdLine : L"";
    bool no_daemon = (args.find(L"--no-daemon") != std::wstring::npos);

    int argc = 0;
    LPWSTR* argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    if (argv) {
        for (int i = 1; i < argc; ++i) {
            if (std::wcscmp(argv[i], L"--no-daemon") == 0) {
                no_daemon = true;
            } else if (std::wcscmp(argv[i], L"--data-dir") == 0 && i + 1 < argc) {
                g_custom_data_dir = argv[++i];
            }
        }
        LocalFree(argv);
    }

    if (!no_daemon) {
        SpawnDaemonInJob();
    }

    WNDCLASSEXW wc{};
    wc.cbSize = sizeof(wc);
    wc.lpfnWndProc = HudWndProc;
    wc.hInstance = hInstance;
    wc.lpszClassName = kHudClassName;
    RegisterClassExW(&wc);

    const int pill_w = 270;
    const int pill_h = 40;
    const int screen_w = GetSystemMetrics(SM_CXSCREEN);
    const int pos_x = (screen_w - pill_w) / 2;
    const int pos_y = 18;

    g_hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
        kHudClassName,
        L"Jarvis HUD",
        WS_POPUP,
        pos_x, pos_y, pill_w, pill_h,
        nullptr, nullptr, hInstance, nullptr);

    SetLayeredWindowAttributes(g_hwnd, 0, 240, LWA_ALPHA);
    ShowWindow(g_hwnd, SW_SHOWNOACTIVATE);
    AddTrayIcon(g_hwnd);

    // Start Dedicated-Thread Raw Input Hotkey Listener (Ctrl+Space)
    g_hotkey = std::make_unique<jarvis::input::HotkeyListener>();
    g_hotkey->SetTargetHwnd(g_hwnd, WM_APP_TRIGGER_SPEAK, WM_APP_RELEASE_SPEAK);
    g_hotkey->Start();

    // Initialize Audio Pipeline
    g_capture = std::make_unique<jarvis::audio::WasapiCapture>(g_raw_mic_buffer);

    // Model paths from LocalAppData
    wchar_t local_app_data[MAX_PATH]{};
    GetEnvironmentVariableW(L"LOCALAPPDATA", local_app_data, MAX_PATH);
    std::filesystem::path models_dir = std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"models";
    std::filesystem::path vad_model = models_dir / L"silero_vad.onnx";
    std::filesystem::path whisper_model = models_dir / L"ggml-base.en.bin";
    std::filesystem::path piper_model = models_dir / L"en_US-ryan-medium.onnx";
    std::filesystem::path piper_config = models_dir / L"en_US-ryan-medium.onnx.json";

    g_vad = std::make_unique<jarvis::audio::SileroVad>(g_clean_ring_buffer, vad_model.wstring());
    g_stt = std::make_unique<jarvis::audio::WhisperStt>(whisper_model.string());
    g_tts = std::make_unique<jarvis::audio::PiperTts>(piper_model.string(), piper_config.string());

    // Wire VAD speech-end callback into STT worker queue & IPC audio forwarding
    g_vad->SetUtteranceCallback([](std::vector<float>&& pcm) {
        if (g_tts) {
            g_tts->BargeIn();
        }
        if (!g_is_paused.load()) {
            g_hud_state.store(HudState::Thinking);
            g_hud_label = L"JARVIS // THINKING...";
            if (g_hwnd) {
                PostMessageW(g_hwnd, WM_APP_SPEECH, 0, 0);
            }

            // Assign monotonic non-zero turn_id for this utterance
            static std::atomic<uint32_t> s_utterance_turn_id{1};
            uint32_t turn_id = s_utterance_turn_id.fetch_add(1);
            if (turn_id == 0) turn_id = s_utterance_turn_id.fetch_add(1);
            g_active_turn_id.store(turn_id, std::memory_order_release);
            g_latency_tracker.RecordVadEnd(turn_id);

            // Forward 16-bit PCM to daemon for Gemini neural transcription
            if (!pcm.empty()) {
                std::vector<int16_t> pcm16(pcm.size());
                for (size_t i = 0; i < pcm.size(); ++i) {
                    float s = std::clamp(pcm[i], -1.0f, 1.0f);
                    pcm16[i] = static_cast<int16_t>(s * 32767.0f);
                }
                const size_t max_bytes = jarvis::ipc::kMaxAudioPayload;
                const uint8_t* pcm_bytes = reinterpret_cast<const uint8_t*>(pcm16.data());
                size_t total_bytes = pcm16.size() * sizeof(int16_t);
                size_t offset = 0;
                while (offset < total_bytes) {
                    size_t chunk_len = std::min(max_bytes, total_bytes - offset);
                    auto frame = jarvis::ipc::EncodeFrame(
                        jarvis::ipc::kMsgTypeAudioPcm,
                        std::string(reinterpret_cast<const char*>(pcm_bytes + offset), chunk_len),
                        turn_id);
                    EnqueueIpcFrame(std::move(frame));
                    offset += chunk_len;
                }
                // Send 0-byte EOF frame to mark utterance completion
                auto eof_frame = jarvis::ipc::EncodeFrame(
                    jarvis::ipc::kMsgTypeAudioPcm,
                    std::string(),
                    turn_id);
                EnqueueIpcFrame(std::move(eof_frame));
                g_latency_tracker.RecordIpcTx(turn_id);
            }

            if (g_stt) {
                g_stt->EnqueueUtterance(std::move(pcm));
            }
        }
    });

    // Wire STT transcription callback into Named Pipe IPC emission
    g_stt->SetTranscriptionCallback([](const std::string& text, uint64_t duration_ms) {
        if (g_is_paused.load()) return;
        g_latency_tracker.RecordSttDone(g_active_turn_id.load(std::memory_order_relaxed));

        json notify = {
            {"jsonrpc", "2.0"},
            {"method", "speech.transcribed"},
            {"params", {
                {"text", text},
                {"duration_ms", duration_ms}
            }}
        };
        SendIpcJson(notify);

        if (g_hwnd) {
            std::wstring wtext(text.begin(), text.end());
            g_hud_label = L"SPEECH: " + wtext;
            PostMessageW(g_hwnd, WM_APP_SPEECH, 0, 0);
        }
    });

    g_capture->Start();
    if (g_tts) {
        g_tts->Start();
    }

    std::thread ipc_thread(IpcWorkerLoop);
    std::thread vad_thread(VadWorkerLoop);

    MSG msg{};
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    g_running.store(false);

    if (g_tts) {
        g_tts->Stop();
    }

    if (g_capture) {
        g_capture->Stop();
    }

    if (vad_thread.joinable()) {
        vad_thread.join();
    }

    if (ipc_thread.joinable()) {
        ipc_thread.detach();
    }

    CoUninitialize();
    return 0;
}
