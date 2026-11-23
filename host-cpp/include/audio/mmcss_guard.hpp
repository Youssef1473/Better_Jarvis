#pragma once

#include <windows.h>
#include <avrt.h>
#include <utility>

namespace jarvis::audio {

/// RAII Guard for Windows Multimedia Class Scheduler Service (MMCSS)
/// and Intel P-Core thread affinity.
class MmcssGuard {
public:
    /// Initializes MMCSS scheduling and pins thread to specified affinity mask (default: cores 0-3 / 0x0F).
    explicit MmcssGuard(LPCWSTR task_name = L"Pro Audio", DWORD_PTR affinity_mask = 0x0F)
        : task_handle_(nullptr), prev_affinity_(0) {
        
        // Pin audio thread to Performance cores (P-cores) to prevent E-core throttle underruns
        if (affinity_mask != 0) {
            prev_affinity_ = SetThreadAffinityMask(GetCurrentThread(), affinity_mask);
        }

        DWORD task_index = 0;
        task_handle_ = AvSetMmThreadCharacteristicsW(task_name, &task_index);
        if (!task_handle_ && wcscmp(task_name, L"Audio") != 0) {
            // Fallback to "Audio" profile if "Pro Audio" is unavailable
            task_handle_ = AvSetMmThreadCharacteristicsW(L"Audio", &task_index);
        }
    }

    ~MmcssGuard() {
        Reset();
    }

    // Non-copyable
    MmcssGuard(const MmcssGuard&) = delete;
    MmcssGuard& operator=(const MmcssGuard&) = delete;

    // Movable
    MmcssGuard(MmcssGuard&& other) noexcept
        : task_handle_(other.task_handle_), prev_affinity_(other.prev_affinity_) {
        other.task_handle_ = nullptr;
        other.prev_affinity_ = 0;
    }

    MmcssGuard& operator=(MmcssGuard&& other) noexcept {
        if (this != &other) {
            Reset();
            task_handle_ = other.task_handle_;
            prev_affinity_ = other.prev_affinity_;
            other.task_handle_ = nullptr;
            other.prev_affinity_ = 0;
        }
        return *this;
    }

    /// Whether MMCSS priority was successfully granted by the OS.
    bool IsActive() const noexcept {
        return task_handle_ != nullptr;
    }

    /// Explicit cleanup before object destruction.
    void Reset() noexcept {
        if (task_handle_) {
            AvRevertMmThreadCharacteristics(task_handle_);
            task_handle_ = nullptr;
        }
        if (prev_affinity_ != 0) {
            SetThreadAffinityMask(GetCurrentThread(), prev_affinity_);
            prev_affinity_ = 0;
        }
    }

private:
    HANDLE    task_handle_{nullptr};
    DWORD_PTR prev_affinity_{0};
};

} // namespace jarvis::audio
