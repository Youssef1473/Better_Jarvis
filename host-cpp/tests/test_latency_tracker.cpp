#include <cassert>
#include <chrono>
#include <cmath>
#include <iostream>
#include <thread>
#include <vector>

#include "telemetry/latency_tracker.hpp"

using namespace jarvis::telemetry;

void TestQpcMonotonicAndDurations() {
    LatencyTracker tracker;

    const uint32_t turn_id = 42;
    tracker.RecordPttUp(turn_id);
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
    tracker.RecordVadEnd(turn_id);
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
    tracker.RecordSttDone(turn_id);
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
    tracker.RecordIpcTx(turn_id);
    std::this_thread::sleep_for(std::chrono::milliseconds(15));
    tracker.RecordTtsFirstAudio(turn_id);

    auto summary = tracker.GetSummary();
    assert(summary.total_completed_turns == 1);
    assert(summary.vad_to_stt.sample_count == 1);
    assert(summary.vad_to_stt.p50_ms >= 5.0 && summary.vad_to_stt.p50_ms <= 100.0);
    assert(summary.stt_to_ipc.p50_ms >= 2.0 && summary.stt_to_ipc.p50_ms <= 100.0);
    assert(summary.ipc_to_first_audio.p50_ms >= 5.0 && summary.ipc_to_first_audio.p50_ms <= 100.0);
    assert(summary.end_to_end_ttfa.p50_ms >= 20.0 && summary.end_to_end_ttfa.p50_ms <= 200.0);
}

void TestPercentilesAndRollingWindow() {
    LatencyTracker tracker;

    // Inject 100 synthetic turns with known linear values 1.0ms through 100.0ms
    for (int i = 1; i <= 100; ++i) {
        double val = static_cast<double>(i);
        tracker.InjectCompletedTurn(val, val, val, val);
    }

    auto summary100 = tracker.GetSummary();
    assert(summary100.total_completed_turns == 100);
    assert(summary100.end_to_end_ttfa.sample_count == 100);

    // Sorted indices: 100 items -> idx 50 is item 51 (value 51.0), idx 95 is item 96 (value 96.0)
    assert(std::fabs(summary100.end_to_end_ttfa.p50_ms - 51.0) < 0.1);
    assert(std::fabs(summary100.end_to_end_ttfa.p95_ms - 96.0) < 0.1);

    // Now inject 150 more turns (total 250) to test 200-item rolling window eviction
    for (int i = 101; i <= 250; ++i) {
        double val = static_cast<double>(i);
        tracker.InjectCompletedTurn(val, val, val, val);
    }

    assert(tracker.GetCompletedCount() == 200);
    auto summary200 = tracker.GetSummary();
    assert(summary200.total_completed_turns == 200);

    // Items now in window: 51 through 250 (200 items)
    // idx 50 in 200 items: 51 + 100 = 151
    // idx 95 in 200 items (0.95 * 200 = 190): 51 + 190 = 241
    assert(std::fabs(summary200.end_to_end_ttfa.p50_ms - 151.0) < 0.1);
    assert(std::fabs(summary200.end_to_end_ttfa.p95_ms - 241.0) < 0.1);
}

void TestCancellationAndStaleTurnRejection() {
    LatencyTracker tracker;

    const uint32_t active_turn = 101;
    const uint32_t cancelled_turn = 102;

    tracker.RecordVadEnd(active_turn);
    tracker.RecordVadEnd(cancelled_turn);

    // Cancel turn 102 (e.g. barge-in occurred)
    tracker.CancelTurn(cancelled_turn);
    assert(tracker.IsTurnCancelled(cancelled_turn));
    assert(!tracker.IsTurnCancelled(active_turn));

    // Stale/cancelled chunk arrival for 102 should be ignored
    tracker.RecordSttDone(cancelled_turn);
    tracker.RecordIpcTx(cancelled_turn);
    tracker.RecordTtsFirstAudio(cancelled_turn);

    assert(tracker.GetCompletedCount() == 0);

    // Active turn 101 completes normally
    tracker.RecordSttDone(active_turn);
    tracker.RecordIpcTx(active_turn);
    tracker.RecordTtsFirstAudio(active_turn);

    assert(tracker.GetCompletedCount() == 1);
}

int main() {
    std::cout << "[TEST] Running TestQpcMonotonicAndDurations..." << std::endl;
    TestQpcMonotonicAndDurations();

    std::cout << "[TEST] Running TestPercentilesAndRollingWindow..." << std::endl;
    TestPercentilesAndRollingWindow();

    std::cout << "[TEST] Running TestCancellationAndStaleTurnRejection..." << std::endl;
    TestCancellationAndStaleTurnRejection();

    std::cout << "[PASS] All LatencyTracker tests passed successfully!" << std::endl;
    return 0;
}
