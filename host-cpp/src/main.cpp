#include <windows.h>
#include <dwmapi.h>
#include <shellapi.h>

#include <atomic>
#include <chrono>
#include <cmath>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <mutex>
#include <string>
#include <thread>
#include <vector>
#include <iostream>
#include <iomanip>

#include <nlohmann/json.hpp>
#include "ipc/frame_codec.hpp"
#include "audio/mmcss_guard.hpp"
#include "audio/ring_buffer.hpp"
#include "audio/aec_processor.hpp"
#include "audio/aec_delay_estimator.hpp"
#include "audio/wasapi_capture.hpp"
#include "audio/silero_vad.hpp"
#include "audio/whisper_stt.hpp"
#include "audio/piper_tts.hpp"
#include "input/hotkey_listener.hpp"
#include "telemetry/latency_tracker.hpp"
#include "diagnostics/crash_handler.hpp"

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

// Vision Capture State
std::atomic<bool>     g_vision_active{false};
std::mutex            g_vision_mutex;
std::wstring          g_vision_app;

// Push-to-Talk and Listening Watchdog State
std::atomic<bool>     g_ptt_active{false};
std::mutex            g_ptt_mutex;
std::vector<float>    g_ptt_audio_buffer;
std::chrono::steady_clock::time_point g_listening_start_time = std::chrono::steady_clock::now();

// Audio Subsystems
jarvis::audio::SpscRingBuffer<32768>           g_raw_mic_buffer;
jarvis::audio::SpscRingBuffer<32768>           g_clean_ring_buffer;
jarvis::audio::AecProcessor                    g_aec;
jarvis::audio::AecDelayEstimator              g_delay_estimator;
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
                        } else if (method == "hud.vision_capture" && resp.contains("params")) {
                            const bool active = resp["params"].value("active", false);
                            const std::string app = resp["params"].value("app", "");
                            g_vision_active.store(active);
                            {
                                std::lock_guard<std::mutex> lock(g_vision_mutex);
                                g_vision_app = std::wstring(app.begin(), app.end());
                            }
                            if (g_hwnd) {
                                InvalidateRect(g_hwnd, nullptr, TRUE);
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
                if (g_ptt_active.load(std::memory_order_relaxed)) {
                    std::lock_guard<std::mutex> lock(g_ptt_mutex);
                    if (g_ptt_audio_buffer.size() < 240000) {
                        g_ptt_audio_buffer.insert(g_ptt_audio_buffer.end(), mic_block, mic_block + 160);
                    }
                }
            } else {
                g_aec.ProcessFrame(mic_block, ref_block, clean_block, is_speaking);
                g_clean_ring_buffer.PushBatch(clean_block, 160);
                if (g_ptt_active.load(std::memory_order_relaxed)) {
                    std::lock_guard<std::mutex> lock(g_ptt_mutex);
                    if (g_ptt_audio_buffer.size() < 240000) {
                        g_ptt_audio_buffer.insert(g_ptt_audio_buffer.end(), clean_block, clean_block + 160);
                    }
                }

                // Run real-time acoustic lag auto-calibration on non-MMCSS VadWorkerLoop thread
                const bool is_double_talk = g_aec.IsDoubleTalk();
                g_delay_estimator.UpdateFrame(mic_block, ref_block, is_speaking, is_double_talk);

                size_t new_bulk = 0;
                float delay_ms = 0.0f;
                float peak_ncc = 0.0f;
                if (g_delay_estimator.CheckAndClearJustLocked(&new_bulk, &delay_ms, &peak_ncc)) {
                    g_aec.SetBulkDelaySamples(new_bulk);
                    std::cout << "[AecCalibrator] Locked acoustic delay: "
                              << std::fixed << std::setprecision(1) << delay_ms
                              << " ms (bulk_delay=" << new_bulk
                              << " samples, NCC=" << std::setprecision(2) << peak_ncc << ")" << std::endl;
                }
            }
        }

        // Automatic recalibration on device hot-swap or discontinuity burst
        static uint64_t s_last_discontinuity = 0;
        const uint64_t current_discontinuity = g_capture ? g_capture->DiscontinuityCount() : 0;
        if (current_discontinuity > s_last_discontinuity) {
            s_last_discontinuity = current_discontinuity;
            g_delay_estimator.Reset();
            g_aec.Reset();
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
                    g_listening_start_time = std::chrono::steady_clock::now();
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
                g_tts->StopPlayback();
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

            // True PTT Start:
            // 1. Mark PTT active and record start time for watchdog
            g_ptt_active.store(true, std::memory_order_release);
            g_listening_start_time = std::chrono::steady_clock::now();

            // 2. Snapshot 300 ms pre-roll (4,800 samples @ 16 kHz) from clean ring buffer
            constexpr size_t kPttPreRollSamples = 4800;
            std::vector<float> pre_roll(kPttPreRollSamples);
            size_t peeked = g_clean_ring_buffer.PeekRecent(pre_roll.data(), kPttPreRollSamples);
            pre_roll.resize(peeked);

            {
                std::lock_guard<std::mutex> lock(g_ptt_mutex);
                g_ptt_audio_buffer = std::move(pre_roll);
            }

            // 3. Instant silent HUD flip (NO "Yes, sir?" playback to avoid tripping barge-in gates)
            g_hud_state.store(HudState::Listening);
            g_hud_label = L"JARVIS // LISTENING...";
            InvalidateRect(hwnd, nullptr, TRUE);

            return 0;
        }
        case WM_APP_RELEASE_SPEAK: {
            if (!g_ptt_active.exchange(false, std::memory_order_acq_rel)) {
                return 0;
            }

            const uint32_t turn_id = g_active_turn_id.load(std::memory_order_relaxed);
            g_latency_tracker.RecordPttUp(turn_id);

            std::vector<float> ptt_audio;
            {
                std::lock_guard<std::mutex> lock(g_ptt_mutex);
                ptt_audio = std::move(g_ptt_audio_buffer);
            }

            const size_t num_samples = ptt_audio.size();
            const float duration_ms = static_cast<float>(num_samples) * 1000.0f / 16000.0f;

            double sum_sq = 0.0;
            for (float s : ptt_audio) {
                sum_sq += static_cast<double>(s) * static_cast<double>(s);
            }
            const float rms = (num_samples > 0) ? static_cast<float>(std::sqrt(sum_sq / static_cast<double>(num_samples))) : 0.0f;

            // Threshold: >= 250 ms (4000 samples @ 16 kHz) and non-silent (RMS >= 0.0001f)
            if (duration_ms >= 250.0f && rms >= 0.0001f) {
                g_hud_state.store(HudState::Thinking);
                g_hud_label = L"JARVIS // THINKING...";
                InvalidateRect(hwnd, nullptr, TRUE);

                static std::atomic<uint32_t> s_ptt_turn_id{1};
                uint32_t ptt_turn = s_ptt_turn_id.fetch_add(1);
                if (ptt_turn == 0) ptt_turn = s_ptt_turn_id.fetch_add(1);
                g_active_turn_id.store(ptt_turn, std::memory_order_release);
                g_latency_tracker.RecordVadEnd(ptt_turn);

                if (g_stt && g_stt->IsModelLoaded()) {
                    g_stt->EnqueueUtterance(std::move(ptt_audio));
                } else if (!ptt_audio.empty()) {
                    std::vector<int16_t> pcm16(ptt_audio.size());
                    for (size_t i = 0; i < ptt_audio.size(); ++i) {
                        float s = std::clamp(ptt_audio[i], -1.0f, 1.0f);
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
                            ptt_turn);
                        EnqueueIpcFrame(std::move(frame));
                        offset += chunk_len;
                    }
                    auto eof_frame = jarvis::ipc::EncodeFrame(
                        jarvis::ipc::kMsgTypeAudioPcm,
                        std::string(),
                        ptt_turn);
                    EnqueueIpcFrame(std::move(eof_frame));
                    g_latency_tracker.RecordIpcTx(ptt_turn);
                }
            } else {
                // Short or silent release: return cleanly to Ready immediately
                if (!g_is_paused.load()) {
                    g_hud_state.store(HudState::Ready);
                    g_hud_label = L"JARVIS // READY";
                    InvalidateRect(hwnd, nullptr, TRUE);
                }
            }
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
                    // Clicked anywhere on HUD pill body: Toggle push-to-talk listening state cleanly
                    if (g_ptt_active.load(std::memory_order_acquire)) {
                        PostMessageW(hwnd, WM_APP_RELEASE_SPEAK, 0, 0);
                    } else if (g_hud_state.load() == HudState::Ready) {
                        PostMessageW(hwnd, WM_APP_TRIGGER_SPEAK, 0, 0);
                    }
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
                } else if (g_hud_state.load() == HudState::Listening) {
                    // Stuck-Listening Safety Watchdog (15-second ceiling)
                    auto now = std::chrono::steady_clock::now();
                    auto elapsed_ms = std::chrono::duration_cast<std::chrono::milliseconds>(now - g_listening_start_time).count();
                    if (elapsed_ms >= 15000) {
                        if (g_ptt_active.load()) {
                            PostMessageW(hwnd, WM_APP_RELEASE_SPEAK, 0, 0);
                        } else {
                            if (g_vad) g_vad->ResetState();
                            if (!g_is_paused.load()) {
                                g_hud_state.store(HudState::Ready);
                                g_hud_label = L"JARVIS // READY";
                                InvalidateRect(hwnd, nullptr, TRUE);
                            }
                        }
                    } else {
                        InvalidateRect(hwnd, nullptr, FALSE);
                    }
                } else if (g_hud_state.load() == HudState::Thinking) {
                    // Thinking Safety Watchdog (20-second ceiling) prevents permanent freeze on network/API failure
                    auto now = std::chrono::steady_clock::now();
                    auto elapsed_ms = std::chrono::duration_cast<std::chrono::milliseconds>(now - g_listening_start_time).count();
                    if (elapsed_ms >= 20000) {
                        if (!g_is_paused.load()) {
                            g_hud_state.store(HudState::Ready);
                            g_hud_label = L"JARVIS // READY";
                            InvalidateRect(hwnd, nullptr, TRUE);
                        }
                    } else {
                        InvalidateRect(hwnd, nullptr, FALSE);
                    }
                } else if (g_hud_state.load() == HudState::Speaking) {
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

                // Render Purple Vision Dot Indicator if Active Window OCR/Vision is Running
                if (g_vision_active.load()) {
                    COLORREF vis_color = RGB(168, 85, 247); // Vivid Purple
                    HBRUSH vis_brush = CreateSolidBrush(vis_color);
                    SelectObject(hdc, vis_brush);
                    Ellipse(hdc, rc.right - 50, (rc.bottom - 8) / 2, rc.right - 42, (rc.bottom + 8) / 2);
                    DeleteObject(vis_brush);
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

            // Log host edge latency summary on graceful shutdown
            auto summary = g_latency_tracker.GetSummary();
            std::cout << "[Host Latency Summary] Total Turns: " << summary.total_completed_turns
                      << " | VAD->STT p50=" << summary.vad_to_stt.p50_ms << "ms p95=" << summary.vad_to_stt.p95_ms << "ms"
                      << " | STT->IPC p50=" << summary.stt_to_ipc.p50_ms << "ms p95=" << summary.stt_to_ipc.p95_ms << "ms"
                      << " | IPC->Audio p50=" << summary.ipc_to_first_audio.p50_ms << "ms p95=" << summary.ipc_to_first_audio.p95_ms << "ms"
                      << " | TTFA p50=" << summary.end_to_end_ttfa.p50_ms << "ms p95=" << summary.end_to_end_ttfa.p95_ms << "ms"
                      << std::endl;

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

struct AudioConfig {
    std::wstring input_device = L"default";
    std::wstring output_device = L"default";
};

AudioConfig LoadAudioConfig(const std::wstring& custom_dir) {
    AudioConfig cfg;
    std::vector<std::filesystem::path> search_paths;
    if (!custom_dir.empty()) {
        search_paths.push_back(std::filesystem::path(custom_dir) / L"config" / L"config.toml");
        search_paths.push_back(std::filesystem::path(custom_dir) / L"config.toml");
    }
    wchar_t exe_path[MAX_PATH]{};
    GetModuleFileNameW(nullptr, exe_path, MAX_PATH);
    std::filesystem::path exe_dir = std::filesystem::path(exe_path).parent_path();
    search_paths.push_back(exe_dir / L"config" / L"config.toml");
    search_paths.push_back(std::filesystem::current_path() / L"config" / L"config.toml");

    wchar_t local_app_data[MAX_PATH]{};
    GetEnvironmentVariableW(L"LOCALAPPDATA", local_app_data, MAX_PATH);
    search_paths.push_back(std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"config" / L"config.toml");

    for (const auto& p : search_paths) {
        if (std::filesystem::exists(p)) {
            std::ifstream file(p);
            if (!file.is_open()) continue;
            std::string line;
            bool in_audio_section = false;
            while (std::getline(file, line)) {
                size_t first = line.find_first_not_of(" \t\r\n");
                if (first == std::string::npos) continue;
                size_t last = line.find_last_not_of(" \t\r\n");
                std::string trimmed = line.substr(first, last - first + 1);
                if (trimmed.empty() || trimmed[0] == '#') continue;

                if (trimmed[0] == '[') {
                    in_audio_section = (trimmed == "[audio]");
                    continue;
                }

                if (in_audio_section) {
                    auto eq_pos = trimmed.find('=');
                    if (eq_pos != std::string::npos) {
                        std::string key = trimmed.substr(0, eq_pos);
                        std::string val = trimmed.substr(eq_pos + 1);

                        key.erase(0, key.find_first_not_of(" \t"));
                        if (!key.empty()) key.erase(key.find_last_not_of(" \t") + 1);

                        val.erase(0, val.find_first_not_of(" \t"));
                        if (!val.empty()) val.erase(val.find_last_not_of(" \t") + 1);
                        if (val.size() >= 2 && val.front() == '"' && val.back() == '"') {
                            val = val.substr(1, val.size() - 2);
                        }

                        if (key == "input_device") {
                            cfg.input_device = std::wstring(val.begin(), val.end());
                        } else if (key == "output_device") {
                            cfg.output_device = std::wstring(val.begin(), val.end());
                        }
                    }
                }
            }
            break;
        }
    }
    return cfg;
}

} // namespace

int WINAPI wWinMain(HINSTANCE hInstance, HINSTANCE, PWSTR lpCmdLine, int) {
    CoInitialize(nullptr);

    const std::wstring args = lpCmdLine ? lpCmdLine : L"";
    bool no_daemon = (args.find(L"--no-daemon") != std::wstring::npos);
    bool probe_mic = (args.find(L"--probe-mic") != std::wstring::npos);
    std::wstring cli_input_device;
    std::wstring cli_output_device;

    int argc = 0;
    LPWSTR* argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    if (argv) {
        for (int i = 1; i < argc; ++i) {
            if (std::wcscmp(argv[i], L"--no-daemon") == 0) {
                no_daemon = true;
            } else if (std::wcscmp(argv[i], L"--probe-mic") == 0) {
                probe_mic = true;
            } else if (std::wcscmp(argv[i], L"--data-dir") == 0 && i + 1 < argc) {
                g_custom_data_dir = argv[++i];
            } else if (std::wcscmp(argv[i], L"--input-device") == 0 && i + 1 < argc) {
                cli_input_device = argv[++i];
            } else if (std::wcscmp(argv[i], L"--output-device") == 0 && i + 1 < argc) {
                cli_output_device = argv[++i];
            }
        }
        LocalFree(argv);
    }

    AudioConfig audio_cfg = LoadAudioConfig(g_custom_data_dir);
    if (!cli_input_device.empty()) {
        audio_cfg.input_device = cli_input_device;
    }
    if (!cli_output_device.empty()) {
        audio_cfg.output_device = cli_output_device;
    }

    if (probe_mic) {
        HANDLE hOut = GetStdHandle(STD_OUTPUT_HANDLE);
        if (!hOut || hOut == INVALID_HANDLE_VALUE || GetFileType(hOut) == FILE_TYPE_UNKNOWN) {
            if (AttachConsole(ATTACH_PARENT_PROCESS)) {
                FILE* dummy = nullptr;
                freopen_s(&dummy, "CONOUT$", "w", stdout);
                freopen_s(&dummy, "CONOUT$", "w", stderr);
                freopen_s(&dummy, "CONIN$", "r", stdin);
            }
        }
        std::ios_base::sync_with_stdio(true);

        std::wcout << L"[ProbeMic] Initializing WASAPI capture probe on device: \""
                   << audio_cfg.input_device << L"\"..." << std::endl;

        jarvis::audio::SpscRingBuffer<32768> probe_ring;
        auto probe_capture = std::make_unique<jarvis::audio::WasapiCapture>(probe_ring, audio_cfg.input_device);
        if (!probe_capture->Start()) {
            std::wcerr << L"[ProbeMic] Failed to start WASAPI capture device." << std::endl;
            CoUninitialize();
            return 1;
        }

        // Locate Silero VAD model if available
        std::filesystem::path models_dir;
        wchar_t exe_path_buf[MAX_PATH]{};
        GetModuleFileNameW(nullptr, exe_path_buf, MAX_PATH);
        std::filesystem::path exe_dir = std::filesystem::path(exe_path_buf).parent_path();
        if (!g_custom_data_dir.empty() && std::filesystem::exists(std::filesystem::path(g_custom_data_dir) / L"models")) {
            models_dir = std::filesystem::path(g_custom_data_dir) / L"models";
        } else if (std::filesystem::exists(exe_dir / L"models" / L"silero_vad.onnx")) {
            models_dir = exe_dir / L"models";
        } else if (std::filesystem::exists("models/silero_vad.onnx")) {
            models_dir = "models";
        } else {
            wchar_t local_app_data[MAX_PATH]{};
            GetEnvironmentVariableW(L"LOCALAPPDATA", local_app_data, MAX_PATH);
            models_dir = std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"models";
        }
        std::filesystem::path vad_model = models_dir / L"silero_vad.onnx";
        std::unique_ptr<jarvis::audio::SileroVad> probe_vad;
        if (std::filesystem::exists(vad_model)) {
            probe_vad = std::make_unique<jarvis::audio::SileroVad>(probe_ring, vad_model.wstring());
        }

        std::wcout << L"[ProbeMic] Probing live audio input for 3.0 seconds..." << std::endl;

        float peak_sample = 0.0f;
        float peak_rms = 0.0f;
        float max_vad_prob = 0.0f;
        uint64_t total_samples = 0;

        auto t_start = std::chrono::steady_clock::now();
        while (true) {
            auto elapsed_ms = std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t_start).count();
            if (elapsed_ms >= 3000) break;

            float buf[512];
            while (probe_ring.AvailableRead() >= 512) {
                probe_ring.PopBatch(buf, 512);
                double sum_sq = 0.0;
                for (size_t i = 0; i < 512; ++i) {
                    float s = std::abs(buf[i]);
                    if (s > peak_sample) peak_sample = s;
                    sum_sq += static_cast<double>(buf[i]) * static_cast<double>(buf[i]);
                }
                float frame_rms = static_cast<float>(std::sqrt(sum_sq / 512.0));
                if (frame_rms > peak_rms) peak_rms = frame_rms;

                if (probe_vad) {
                    float prob = probe_vad->EvaluateFrame(buf);
                    if (prob > max_vad_prob) max_vad_prob = prob;
                }
                total_samples += 512;
            }
            std::this_thread::sleep_for(std::chrono::milliseconds(15));
        }

        probe_capture->Stop();

        std::wcout << L"\n--- MIC PROBE DIAGNOSTIC SCORECARD ---" << std::endl;
        std::wcout << L"Active Endpoint Friendly Name: " << probe_capture->ActiveDeviceFriendlyName() << std::endl;
        std::wcout << L"Hardware Mix Format: " << probe_capture->NativeSampleRate() << L" Hz, "
                   << probe_capture->NativeChannels() << L" channel(s)" << std::endl;
        std::wcout << L"Negotiated Period: " << std::fixed << std::setprecision(1) << probe_capture->NegotiatedPeriodMs() << L" ms" << std::endl;
        std::wcout << L"Discontinuities: " << probe_capture->DiscontinuityCount() << std::endl;
        std::wcout << L"Total Samples Captured (3.0s): " << total_samples << std::endl;
        std::wcout << L"Peak Amplitude: " << std::fixed << std::setprecision(6) << peak_sample << std::endl;
        std::wcout << L"Peak RMS: " << std::fixed << std::setprecision(6) << peak_rms << std::endl;
        std::wcout << L"Max Silero VAD Probability: " << std::fixed << std::setprecision(4) << max_vad_prob << std::endl;
        std::wcout << L"VAD Engine: " << (probe_vad && probe_vad->IsOnnxLoaded() ? L"Silero VAD ONNX Runtime" : L"RMS/ZCR Fallback") << std::endl;
        std::wcout << L"--------------------------------------" << std::endl;

        CoUninitialize();
        return 0;
    }

    std::filesystem::path crash_dir;
    if (!g_custom_data_dir.empty()) {
        crash_dir = std::filesystem::path(g_custom_data_dir) / L"logs" / L"crashes";
    } else {
        wchar_t local_app_data[MAX_PATH]{};
        GetEnvironmentVariableW(L"LOCALAPPDATA", local_app_data, MAX_PATH);
        crash_dir = std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"logs" / L"crashes";
    }
    jarvis::diagnostics::InitCrashHandler(crash_dir.wstring());

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

    // Configure host latency telemetry output path
    std::filesystem::path logs_dir;
    if (!g_custom_data_dir.empty()) {
        logs_dir = std::filesystem::path(g_custom_data_dir) / L"logs";
    } else {
        wchar_t local_app_data[MAX_PATH]{};
        GetEnvironmentVariableW(L"LOCALAPPDATA", local_app_data, MAX_PATH);
        logs_dir = std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"logs";
    }
    std::filesystem::path host_metrics_file = logs_dir / L"host_latency_metrics.jsonl";
    g_latency_tracker.SetLogFilePath(host_metrics_file.wstring());

    // Initialize Audio Pipeline
    g_capture = std::make_unique<jarvis::audio::WasapiCapture>(g_raw_mic_buffer, audio_cfg.input_device);

    // Model paths: check custom data dir, exe dir models, local models dir, or LocalAppData
    std::filesystem::path models_dir;
    wchar_t exe_path[MAX_PATH]{};
    GetModuleFileNameW(nullptr, exe_path, MAX_PATH);
    std::filesystem::path exe_dir = std::filesystem::path(exe_path).parent_path();

    if (!g_custom_data_dir.empty() && std::filesystem::exists(std::filesystem::path(g_custom_data_dir) / L"models")) {
        models_dir = std::filesystem::path(g_custom_data_dir) / L"models";
    } else if (std::filesystem::exists(exe_dir / L"models" / L"silero_vad.onnx")) {
        models_dir = exe_dir / L"models";
    } else if (std::filesystem::exists("models/silero_vad.onnx")) {
        models_dir = "models";
    } else {
        wchar_t local_app_data[MAX_PATH]{};
        GetEnvironmentVariableW(L"LOCALAPPDATA", local_app_data, MAX_PATH);
        models_dir = std::filesystem::path(local_app_data) / L"Better_Jarvis" / L"models";
    }
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

            if (g_stt && g_stt->IsModelLoaded()) {
                // Pass directly to local on-device Whisper STT engine
                g_stt->EnqueueUtterance(std::move(pcm));
            } else if (!pcm.empty()) {
                // Fallback to sending 16-bit PCM over IPC to daemon for cloud Gemini transcription
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
        }
    });

    // Wire STT transcription callback into Named Pipe IPC emission
    g_stt->SetTranscriptionCallback([](const std::string& text, uint64_t duration_ms) {
        if (g_is_paused.load()) return;
        const uint32_t turn_id = g_active_turn_id.load(std::memory_order_relaxed);
        g_latency_tracker.RecordSttDone(turn_id);

        json notify = {
            {"jsonrpc", "2.0"},
            {"method", "speech.transcribed"},
            {"params", {
                {"text", text},
                {"duration_ms", duration_ms},
                {"turn_id", turn_id}
            }}
        };
        auto frame = jarvis::ipc::EncodeFrame(jarvis::ipc::kMsgTypeControlJson, notify.dump(), turn_id);
        EnqueueIpcFrame(std::move(frame));
        g_latency_tracker.RecordIpcTx(turn_id);

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
