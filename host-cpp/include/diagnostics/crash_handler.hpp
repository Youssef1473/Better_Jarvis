#pragma once

#include <windows.h>
#include <string>

namespace jarvis::diagnostics {

/// Initializes unhandled exception filter writing minidumps to crash_dir.
void InitCrashHandler(const std::wstring& crash_dir);

/// Prunes older .dmp files in crash_dir keeping strictly the max_keep newest.
void PruneCrashDumps(const std::wstring& crash_dir, size_t max_keep = 5);

/// Writes a manual diagnostic dump without crashing (returns written path).
std::wstring WriteDiagnosticDump(const std::wstring& crash_dir);

/// Unhandled exception filter callback.
LONG WINAPI UnhandledCrashFilter(EXCEPTION_POINTERS* exception_ptrs);

} // namespace jarvis::diagnostics
