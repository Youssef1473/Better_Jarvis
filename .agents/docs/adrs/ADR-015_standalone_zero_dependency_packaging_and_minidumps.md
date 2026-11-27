# ADR-015: Standalone Zero-Dependency Packaging & Minidump Crash Diagnostics
**Status:** Accepted  
**Date:** 2026-11-23  
**Context:** TICKET-15 (Packaging, Crash Diagnostics & Standalone Deployment)  
**Governed Subsystems:** `host-cpp/src/diagnostics/crash_handler.cpp`, `scripts/package-release.ps1`

---

## 1. Context & Problem Statement
Better_Jarvis consists of native C++20 and Rust binaries compiled against MinGW-w64 UCRT64 toolchains with dependencies on ONNX Runtime, Whisper.cpp, and eSpeak-ng. Deploying to standard Windows 11 client environments without a developer SDK installed will fail with "missing DLL" errors if libraries are not staged. Furthermore, unhandled native crashes (e.g. access violations in real-time audio threads) leave zero actionable diagnostic artifacts if the host lacks a native crash handler.

## 2. Decision Drivers
- Zero external runtime installer requirements on Windows 11 target machines.
- Immediate diagnostic capture for post-mortem analysis of native crashes.
- Zero external telemetry or automatic cloud upload of user crash dumps.
- Strict disk space bounding for diagnostic dumps.

## 3. Decision
1. **Self-Contained Bundle Packaging:** `scripts/package-release.ps1` produces a completely standalone directory in `dist/` containing:
   - `jarvis-host.exe` and `jarvis-daemon.exe`
   - All runtime DLLs: `onnxruntime.dll`, `libespeak-ng.dll`, `whisper.dll`, `ggml.dll`, `ggml-base.dll`, `ggml-cpu-alderlake.dll`, `ggml-blas.dll`
   - Verified models from `manifest.sha256` in `dist/models/`
   - Default configuration files in `dist/config/`
2. **Native Win32 Crash Handler:** Install a top-level unhandled exception filter (`SetUnhandledExceptionFilter`) in `jarvis-host.exe` that invokes `MiniDumpWriteDump` (`dbghelp.dll`) with `MiniDumpNormal` flag upon native fault.
3. **Local-Only FIFO Dump Retention:** Write minidumps exclusively to `%LOCALAPPDATA%\Better_Jarvis\logs\crashes\crash_<timestamp>.dmp`. Maintain a strict 5-file FIFO rotation: upon writing a new dump, prune older dumps to ensure disk usage never exceeds 5 dumps (~5 MB total).
4. **Zero Cloud Telemetry:** Dumps remain 100% local on the user's filesystem; zero crash data is transmitted across the network.

## 4. Consequences
- **Positive:** Better_Jarvis can be extracted and run on any clean Windows 11 machine; developers can diagnose crashes using WinDbg or Visual Studio without requiring reproduction steps.
- **Trade-off:** Release distribution bundle size is approximately 210 MB due to bundled models and runtimes.

## 5. Verification Evidence
- `host-cpp/tests/test_crash_handler.cpp`:
  - `TestDirectoryCreationAndDumpHeader`: proves `MiniDumpWriteDump` creates valid files with standard `MDMP` signature.
  - `TestRetentionPruning`: creates 8 mock dumps and proves FIFO pruning retains exactly the 5 newest dumps.
