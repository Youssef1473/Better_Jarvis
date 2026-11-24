#include "diagnostics/crash_handler.hpp"

#include <dbghelp.h>
#include <atomic>
#include <chrono>
#include <filesystem>
#include <iomanip>
#include <mutex>
#include <sstream>
#include <vector>
#include <algorithm>

namespace jarvis::diagnostics {

namespace {

std::mutex    g_dir_mutex;
std::wstring  g_configured_crash_dir;
std::atomic_flag g_crash_in_progress = ATOMIC_FLAG_INIT;

typedef BOOL (WINAPI *MiniDumpWriteDumpFn)(
    HANDLE hProcess,
    DWORD ProcessId,
    HANDLE hFile,
    MINIDUMP_TYPE DumpType,
    PMINIDUMP_EXCEPTION_INFORMATION ExceptionParam,
    PMINIDUMP_USER_STREAM_INFORMATION UserStreamParam,
    PMINIDUMP_CALLBACK_INFORMATION CallbackParam
);

MiniDumpWriteDumpFn GetMiniDumpWriteDump() {
    static HMODULE hDbgHelp = LoadLibraryW(L"dbghelp.dll");
    if (!hDbgHelp) {
        return nullptr;
    }
    auto proc = GetProcAddress(hDbgHelp, "MiniDumpWriteDump");
    return reinterpret_cast<MiniDumpWriteDumpFn>(reinterpret_cast<void*>(proc));
}

std::wstring GenerateDumpFilename(const std::wstring& crash_dir) {
    auto now = std::chrono::system_clock::now();
    std::time_t tt = std::chrono::system_clock::to_time_t(now);
    std::tm tm_buf{};
#if defined(_WIN32)
    localtime_s(&tm_buf, &tt);
#else
    localtime_r(&tt, &tm_buf);
#endif

    std::wostringstream oss;
    oss << crash_dir << L"\\jarvis-host-"
        << std::put_time(&tm_buf, L"%Y%m%d-%H%M%S")
        << L"-pid" << GetCurrentProcessId() << L".dmp";
    return oss.str();
}

struct CrashThreadContext {
    EXCEPTION_POINTERS* exception_ptrs;
    DWORD               faulting_thread_id;
    std::wstring        crash_dir;
};

DWORD WINAPI CrashWriterThread(LPVOID param) {
    auto* ctx = static_cast<CrashThreadContext*>(param);
    if (!ctx) return 1;

    MiniDumpWriteDumpFn pfnDump = GetMiniDumpWriteDump();
    if (!pfnDump) {
        delete ctx;
        return 1;
    }

    try {
        std::filesystem::create_directories(ctx->crash_dir);
    } catch (...) {}

    std::wstring dump_path = GenerateDumpFilename(ctx->crash_dir);
    HANDLE hFile = CreateFileW(
        dump_path.c_str(),
        GENERIC_READ | GENERIC_WRITE,
        0,
        nullptr,
        CREATE_ALWAYS,
        FILE_ATTRIBUTE_NORMAL,
        nullptr
    );

    if (hFile != INVALID_HANDLE_VALUE) {
        MINIDUMP_EXCEPTION_INFORMATION mei{};
        mei.ThreadId = ctx->faulting_thread_id;
        mei.ExceptionPointers = ctx->exception_ptrs;
        mei.ClientPointers = FALSE;

        const MINIDUMP_TYPE dump_flags = static_cast<MINIDUMP_TYPE>(
            MiniDumpNormal | MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules
        );

        pfnDump(
            GetCurrentProcess(),
            GetCurrentProcessId(),
            hFile,
            dump_flags,
            ctx->exception_ptrs ? &mei : nullptr,
            nullptr,
            nullptr
        );

        CloseHandle(hFile);
        PruneCrashDumps(ctx->crash_dir, 5);
    }

    delete ctx;
    return 0;
}

} // namespace

void InitCrashHandler(const std::wstring& crash_dir) {
    {
        std::lock_guard<std::mutex> lock(g_dir_mutex);
        g_configured_crash_dir = crash_dir;
    }

    try {
        std::filesystem::create_directories(crash_dir);
    } catch (...) {}

    SetUnhandledExceptionFilter(UnhandledCrashFilter);
}

void PruneCrashDumps(const std::wstring& crash_dir, size_t max_keep) {
    namespace fs = std::filesystem;
    std::error_code ec;
    if (!fs::exists(crash_dir, ec) || !fs::is_directory(crash_dir, ec)) {
        return;
    }

    struct DumpEntry {
        fs::path path;
        fs::file_time_type mtime;
    };

    std::vector<DumpEntry> dumps;
    for (const auto& entry : fs::directory_iterator(crash_dir, ec)) {
        if (ec) break;
        if (entry.is_regular_file(ec) && entry.path().extension() == L".dmp") {
            dumps.push_back({entry.path(), entry.last_write_time(ec)});
        }
    }

    if (dumps.size() <= max_keep) {
        return;
    }

    // Sort newest first; break ties with path name descending
    std::sort(dumps.begin(), dumps.end(), [](const DumpEntry& a, const DumpEntry& b) {
        if (a.mtime != b.mtime) {
            return a.mtime > b.mtime;
        }
        return a.path > b.path;
    });

    for (size_t i = max_keep; i < dumps.size(); ++i) {
        fs::remove(dumps[i].path, ec);
    }
}

std::wstring WriteDiagnosticDump(const std::wstring& crash_dir) {
    MiniDumpWriteDumpFn pfnDump = GetMiniDumpWriteDump();
    if (!pfnDump) {
        return L"";
    }

    try {
        std::filesystem::create_directories(crash_dir);
    } catch (...) {}

    std::wstring dump_path = GenerateDumpFilename(crash_dir);
    HANDLE hFile = CreateFileW(
        dump_path.c_str(),
        GENERIC_READ | GENERIC_WRITE,
        0,
        nullptr,
        CREATE_ALWAYS,
        FILE_ATTRIBUTE_NORMAL,
        nullptr
    );

    if (hFile == INVALID_HANDLE_VALUE) {
        return L"";
    }

    const MINIDUMP_TYPE dump_flags = static_cast<MINIDUMP_TYPE>(
        MiniDumpNormal | MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules
    );

    BOOL ok = pfnDump(
        GetCurrentProcess(),
        GetCurrentProcessId(),
        hFile,
        dump_flags,
        nullptr,
        nullptr,
        nullptr
    );

    CloseHandle(hFile);

    if (ok) {
        PruneCrashDumps(crash_dir, 5);
        return dump_path;
    }

    std::error_code ec;
    std::filesystem::remove(dump_path, ec);
    return L"";
}

LONG WINAPI UnhandledCrashFilter(EXCEPTION_POINTERS* exception_ptrs) {
    if (g_crash_in_progress.test_and_set(std::memory_order_acquire)) {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    std::wstring crash_dir;
    {
        std::lock_guard<std::mutex> lock(g_dir_mutex);
        crash_dir = g_configured_crash_dir;
    }

    if (crash_dir.empty()) {
        wchar_t temp_dir[MAX_PATH]{};
        GetTempPathW(MAX_PATH, temp_dir);
        crash_dir = std::wstring(temp_dir) + L"Better_Jarvis_Crashes";
    }

    auto* ctx = new CrashThreadContext{
        exception_ptrs,
        GetCurrentThreadId(),
        crash_dir
    };

    HANDLE hThread = CreateThread(
        nullptr,
        0,
        CrashWriterThread,
        ctx,
        0,
        nullptr
    );

    if (hThread) {
        WaitForSingleObject(hThread, 5000);
        CloseHandle(hThread);
    } else {
        delete ctx;
    }

    return EXCEPTION_EXECUTE_HANDLER;
}

} // namespace jarvis::diagnostics
