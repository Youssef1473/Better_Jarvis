# Better_Jarvis Language-Specific Verification Infrastructure
**Location:** `.agents/docs/language_verification.md`
**Status:** Canonical Quality Gates Specification

---

## 1. Overview
Better_Jarvis enforces strict, automated verification standards tailored to each language in its multi-tier technology stack:
- **Rust (Edition 2021):** Async safety, memory ownership, SQLite durability, bounded IPC framing, Clippy linting.
- **C++ (C++20):** MMCSS real-time audio thread purity, lock-free SPSC hygiene, WASAPI hot-swap resilience, UCRT64 toolchain builds.
- **Python (3.12+):** Win32 desktop window station binding, typing discipline, process supervision, standard library confinement.
- **Cross-Language IPC:** Protocol versioning, 256 KiB payload bounds, Logon-SID SDDL DACL, parent PID validation.

---

## 2. Language Verification Standards

### 2.1 Rust Standards (`daemon-rust`)
| Gate | Command | Passing Threshold | Responsibility |
|---|---|---|---|
| **Compilation** | `cargo check -p jarvis-daemon` | Exit code 0 | Type checking, borrow checker, syntax |
| **Linting** | `cargo clippy -p jarvis-daemon --all-targets -- -D warnings` | Zero warnings | Idiomatic Rust, redundant clones, dead code |
| **Testing** | `cargo test -p jarvis-daemon` | 100% pass (109/109) | Positive, negative, security, and async tests |

### 2.2 C++ Standards (`host-cpp`)
| Gate | Command / Tool | Passing Threshold | Responsibility |
|---|---|---|---|
| **Toolchain** | MinGW-w64 UCRT64 `g++` (`C:\msys64\ucrt64\bin\g++.exe`) | Presence & version | GCC C++20 compiler |
| **Compilation** | `-std=c++20 -O2 -Wall -Wextra -Werror` | Zero warnings | Memory safety, RAII, strict typing |
| **Audio Purity** | Static audit of MMCSS audio loop | Zero allocations/locks | Purity of real-time Pro Audio threads |
| **Test Suites** | `build/tests/test_*.exe` | Exit code 0 across 9 suites | Frame codec, SPSC buffer, AEC, VAD, STT |

### 2.3 Python Standards (`scripts/`)
| Gate | Command / Tool | Passing Threshold | Responsibility |
|---|---|---|---|
| **Syntax** | `python -m py_compile <script_path>` | Exit code 0 | Bytecode compilation, syntax check |
| **Desktop Interop** | `scripts/run-desktop.py` | `winsta0\default` binding | Zero `0xC0000142` session errors |
| **Guardrails** | `python .agents/scripts/test_guardrail.py` | 5/5 tests passing | Deterministic hook behavior |

### 2.4 Cross-Language Integration Gate
- Frame format validation: Bit-for-bit serialization match between `daemon-rust/src/ipc/codec.rs` and `host-cpp/include/ipc/frame_codec.hpp`.
- SDDL DACL generation: Verified restricted to Logon SID; rejects unauthorized second instance.
- Verified PID check: Aborts connection if client PID does not match parent host PID.

---

## 3. Progressive Testing Policy

Agents must execute verification progressively:
1. **Focused Test:** Run single unit test exercising immediate change (`cargo test -p jarvis-daemon -- <test_name>`).
2. **Subsystem Test:** Run subsystem test suite (`cargo test -p jarvis-daemon -- <module>::`).
3. **Unified Verification:** Run `python scripts/verify-env.py`.
4. **Full Production Gate:** Run `& scripts/verify-v1.ps1` before any release candidate or major milestone sign-off.
