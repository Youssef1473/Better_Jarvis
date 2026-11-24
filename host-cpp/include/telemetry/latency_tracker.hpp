#pragma once

#include <windows.h>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <deque>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <mutex>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace jarvis::telemetry {

struct StagePercentiles {
    double p50_ms = 0.0;
    double p95_ms = 0.0;
    size_t sample_count = 0;
};

struct LatencySummary {
    StagePercentiles vad_to_stt;
    StagePercentiles stt_to_ipc;
    StagePercentiles ipc_to_first_audio;
    StagePercentiles end_to_end_ttfa;
    size_t total_completed_turns = 0;
};

struct CompletedTurnMetrics {
    uint32_t turn_id = 0;
    double vad_to_stt_ms = 0.0;
    double stt_to_ipc_ms = 0.0;
    double ipc_to_first_audio_ms = 0.0;
    double end_to_end_ttfa_ms = 0.0;
};

class LatencyTracker {
public:
    static constexpr size_t kMaxRollingWindow = 200;

    LatencyTracker() {
        LARGE_INTEGER freq;
        QueryPerformanceFrequency(&freq);
        qpc_frequency_ = freq.QuadPart > 0 ? freq.QuadPart : 1;
    }

    void SetLogFilePath(const std::wstring& path) {
        std::lock_guard<std::mutex> lock(mutex_);
        log_file_path_ = path;
    }

    void RecordPttUp(uint32_t turn_id) {
        if (turn_id == 0) return;
        std::lock_guard<std::mutex> lock(mutex_);
        if (cancelled_turns_.count(turn_id)) return;
        auto& turn = in_flight_[turn_id];
        turn.turn_id = turn_id;
        turn.t_ptt_up = GetCurrentQpc();
    }

    void RecordVadEnd(uint32_t turn_id) {
        if (turn_id == 0) return;
        std::lock_guard<std::mutex> lock(mutex_);
        if (cancelled_turns_.count(turn_id)) return;
        auto& turn = in_flight_[turn_id];
        turn.turn_id = turn_id;
        turn.t_vad_end = GetCurrentQpc();
    }

    void RecordSttDone(uint32_t turn_id) {
        if (turn_id == 0) return;
        std::lock_guard<std::mutex> lock(mutex_);
        if (cancelled_turns_.count(turn_id)) return;
        auto& turn = in_flight_[turn_id];
        turn.turn_id = turn_id;
        turn.t_stt_done = GetCurrentQpc();
    }

    void RecordIpcTx(uint32_t turn_id) {
        if (turn_id == 0) return;
        std::lock_guard<std::mutex> lock(mutex_);
        if (cancelled_turns_.count(turn_id)) return;
        auto& turn = in_flight_[turn_id];
        turn.turn_id = turn_id;
        turn.t_ipc_tx = GetCurrentQpc();
    }

    void RecordTtsFirstAudio(uint32_t turn_id) {
        if (turn_id == 0) return;
        std::lock_guard<std::mutex> lock(mutex_);
        if (cancelled_turns_.count(turn_id)) return;

        auto it = in_flight_.find(turn_id);
        if (it == in_flight_.end()) return;

        int64_t t_first = GetCurrentQpc();
        auto& turn = it->second;
        turn.t_tts_first_audio = t_first;

        CompletedTurnMetrics m{};
        m.turn_id = turn_id;

        if (turn.t_vad_end > 0 && turn.t_stt_done > turn.t_vad_end) {
            m.vad_to_stt_ms = TicksToMs(turn.t_stt_done - turn.t_vad_end);
        }
        if (turn.t_stt_done > 0 && turn.t_ipc_tx > turn.t_stt_done) {
            m.stt_to_ipc_ms = TicksToMs(turn.t_ipc_tx - turn.t_stt_done);
        }
        if (turn.t_ipc_tx > 0 && turn.t_tts_first_audio > turn.t_ipc_tx) {
            m.ipc_to_first_audio_ms = TicksToMs(turn.t_tts_first_audio - turn.t_ipc_tx);
        }

        int64_t start_time = turn.t_ptt_up > 0 ? turn.t_ptt_up : turn.t_vad_end;
        if (start_time > 0 && turn.t_tts_first_audio > start_time) {
            m.end_to_end_ttfa_ms = TicksToMs(turn.t_tts_first_audio - start_time);
        }

        if (completed_turns_.size() >= kMaxRollingWindow) {
            completed_turns_.pop_front();
        }
        completed_turns_.push_back(m);
        in_flight_.erase(it);

        // Asynchronously persist completed turn edge timings to host JSONL metrics
        if (!log_file_path_.empty()) {
            AppendMetricsToFile(m);
        }
    }

    void CancelTurn(uint32_t turn_id) {
        if (turn_id == 0) return;
        std::lock_guard<std::mutex> lock(mutex_);
        cancelled_turns_.insert(turn_id);
        in_flight_.erase(turn_id);
        if (cancelled_turns_.size() > 500) {
            cancelled_turns_.clear();
        }
    }

    bool IsTurnCancelled(uint32_t turn_id) const {
        if (turn_id == 0) return true;
        std::lock_guard<std::mutex> lock(mutex_);
        return cancelled_turns_.count(turn_id) > 0;
    }

    LatencySummary GetSummary() const {
        std::lock_guard<std::mutex> lock(mutex_);
        LatencySummary summary{};
        summary.total_completed_turns = completed_turns_.size();
        if (completed_turns_.empty()) {
            return summary;
        }

        std::vector<double> v_vad_to_stt;
        std::vector<double> v_stt_to_ipc;
        std::vector<double> v_ipc_to_first;
        std::vector<double> v_ttfa;

        v_vad_to_stt.reserve(completed_turns_.size());
        v_stt_to_ipc.reserve(completed_turns_.size());
        v_ipc_to_first.reserve(completed_turns_.size());
        v_ttfa.reserve(completed_turns_.size());

        for (const auto& m : completed_turns_) {
            if (m.vad_to_stt_ms > 0.0) v_vad_to_stt.push_back(m.vad_to_stt_ms);
            if (m.stt_to_ipc_ms > 0.0) v_stt_to_ipc.push_back(m.stt_to_ipc_ms);
            if (m.ipc_to_first_audio_ms > 0.0) v_ipc_to_first.push_back(m.ipc_to_first_audio_ms);
            if (m.end_to_end_ttfa_ms > 0.0) v_ttfa.push_back(m.end_to_end_ttfa_ms);
        }

        summary.vad_to_stt = ComputePercentiles(v_vad_to_stt);
        summary.stt_to_ipc = ComputePercentiles(v_stt_to_ipc);
        summary.ipc_to_first_audio = ComputePercentiles(v_ipc_to_first);
        summary.end_to_end_ttfa = ComputePercentiles(v_ttfa);

        return summary;
    }

    void Reset() {
        std::lock_guard<std::mutex> lock(mutex_);
        in_flight_.clear();
        completed_turns_.clear();
        cancelled_turns_.clear();
    }

    // Helper for synthetic testing
    void InjectCompletedTurn(double vad_stt, double stt_ipc, double ipc_audio, double ttfa) {
        std::lock_guard<std::mutex> lock(mutex_);
        CompletedTurnMetrics m{};
        m.turn_id = 99999;
        m.vad_to_stt_ms = vad_stt;
        m.stt_to_ipc_ms = stt_ipc;
        m.ipc_to_first_audio_ms = ipc_audio;
        m.end_to_end_ttfa_ms = ttfa;

        if (completed_turns_.size() >= kMaxRollingWindow) {
            completed_turns_.pop_front();
        }
        completed_turns_.push_back(m);
    }

    size_t GetCompletedCount() const {
        std::lock_guard<std::mutex> lock(mutex_);
        return completed_turns_.size();
    }

private:
    struct TurnRawTimestamps {
        uint32_t turn_id = 0;
        int64_t t_ptt_up = 0;
        int64_t t_vad_end = 0;
        int64_t t_stt_done = 0;
        int64_t t_ipc_tx = 0;
        int64_t t_tts_first_audio = 0;
    };

    static int64_t GetCurrentQpc() {
        LARGE_INTEGER val;
        QueryPerformanceCounter(&val);
        return val.QuadPart;
    }

    double TicksToMs(int64_t ticks) const {
        return static_cast<double>(ticks) * 1000.0 / static_cast<double>(qpc_frequency_);
    }

    static StagePercentiles ComputePercentiles(std::vector<double>& values) {
        StagePercentiles p{};
        p.sample_count = values.size();
        if (values.empty()) return p;

        std::sort(values.begin(), values.end());
        size_t idx50 = static_cast<size_t>(std::floor(values.size() * 0.50));
        if (idx50 >= values.size()) idx50 = values.size() - 1;

        size_t idx95 = static_cast<size_t>(std::floor(values.size() * 0.95));
        if (idx95 >= values.size()) idx95 = values.size() - 1;

        p.p50_ms = values[idx50];
        p.p95_ms = values[idx95];
        return p;
    }

    void AppendMetricsToFile(const CompletedTurnMetrics& m) {
        try {
            std::filesystem::path p(log_file_path_);
            if (p.has_parent_path()) {
                std::filesystem::create_directories(p.parent_path());
            }
            std::ofstream ofs(p, std::ios::app);
            if (ofs.is_open()) {
                ofs << "{\"turn_id\":" << m.turn_id
                    << ",\"vad_to_stt_ms\":" << m.vad_to_stt_ms
                    << ",\"stt_to_ipc_ms\":" << m.stt_to_ipc_ms
                    << ",\"ipc_to_first_audio_ms\":" << m.ipc_to_first_audio_ms
                    << ",\"end_to_end_ttfa_ms\":" << m.end_to_end_ttfa_ms
                    << "}\n";
            }
        } catch (...) {}
    }

    int64_t qpc_frequency_ = 1;
    std::wstring log_file_path_;
    mutable std::mutex mutex_;
    std::unordered_map<uint32_t, TurnRawTimestamps> in_flight_;
    std::deque<CompletedTurnMetrics> completed_turns_;
    std::unordered_set<uint32_t> cancelled_turns_;
};

} // namespace jarvis::telemetry
