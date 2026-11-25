#!/usr/bin/env python3
"""
Better_Jarvis Unified Verification Infrastructure Runner
Validates Rust, C++, Python, and static security invariants across the repository.
"""

import subprocess
import sys
import os
import shutil
import time

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
UCRT64_BIN = r"C:\msys64\ucrt64\bin"

def run_cmd(cmd, cwd=REPO_ROOT, env=None, desc=""):
    print(f"[*] Running: {desc or cmd}...")
    start_t = time.time()
    environ = os.environ.copy()
    if env:
        environ.update(env)
    
    # Prepend UCRT64_BIN to PATH if present
    if os.path.isdir(UCRT64_BIN):
        environ["PATH"] = UCRT64_BIN + os.pathsep + environ.get("PATH", "")

    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        env=environ,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        shell=True
    )
    stdout, stderr = proc.communicate()
    elapsed = time.time() - start_t
    
    if proc.returncode != 0:
        print(f"[FAIL] {desc or cmd} (exit code {proc.returncode}, {elapsed:.2f}s)")
        if stdout:
            print("--- STDOUT ---")
            print(stdout[:2000])
        if stderr:
            print("--- STDERR ---")
            print(stderr[:2000])
        return False
    else:
        print(f"[PASS] {desc or cmd} ({elapsed:.2f}s)")
        return True

def verify_rust():
    print("\n=== [1/4] Rust Verification Suite ===")
    ok = True
    ok &= run_cmd("cargo check -p jarvis-daemon", desc="Cargo Check (jarvis-daemon)")
    ok &= run_cmd("cargo clippy -p jarvis-daemon --all-targets -- -D warnings", desc="Cargo Clippy (-D warnings)")
    ok &= run_cmd("cargo test -p jarvis-daemon", desc="Cargo Test (jarvis-daemon)")
    return ok

def verify_cpp():
    print("\n=== [2/4] C++ Verification Suite ===")
    gpp = os.path.join(UCRT64_BIN, "g++.exe")
    if not os.path.isfile(gpp):
        print(f"[WARN] MinGW-w64 UCRT64 g++ not found at {gpp}. Skipping C++ compilation.")
        return True
    
    test_dir = os.path.join(REPO_ROOT, "build", "tests")
    os.makedirs(test_dir, exist_ok=True)
    
    # Stage runtime DLLs
    runtime_dlls = [
        "onnxruntime.dll", "libespeak-ng.dll", "whisper.dll",
        "ggml.dll", "ggml-base.dll", "ggml-cpu-alderlake.dll",
        "ggml-blas.dll", "ggml-rpc.dll", "ggml-vulkan.dll", "ggml-opencl.dll"
    ]
    for dll in runtime_dlls:
        src_dll = os.path.join(UCRT64_BIN, dll)
        if os.path.exists(src_dll):
            shutil.copy(src_dll, test_dir)
            
    cpp_tests = [
        ("test_frame_codec", "host-cpp/tests/test_frame_codec.cpp", "", ""),
        ("test_ring_buffer", "host-cpp/tests/test_ring_buffer.cpp", "", ""),
        ("test_resampler", "host-cpp/tests/test_resampler.cpp", "", ""),
        ("test_latency_tracker", "host-cpp/tests/test_latency_tracker.cpp", "", ""),
        ("test_hotkey_listener", "host-cpp/tests/test_hotkey_listener.cpp host-cpp/src/input/hotkey_listener.cpp", "", ""),
        ("test_crash_handler", "host-cpp/tests/test_crash_handler.cpp host-cpp/src/diagnostics/crash_handler.cpp", "", "-ldbghelp"),
        ("test_vad", "host-cpp/tests/test_vad.cpp host-cpp/src/audio/silero_vad.cpp", "-I C:/msys64/ucrt64/include/onnxruntime", "-L C:/msys64/ucrt64/lib -L C:/msys64/ucrt64/bin -lonnxruntime"),
        ("test_aec", "host-cpp/tests/test_aec.cpp host-cpp/src/audio/aec_processor.cpp host-cpp/src/audio/aec_delay_estimator.cpp host-cpp/src/audio/piper_tts.cpp", "-I C:/msys64/ucrt64/include/onnxruntime -I C:/msys64/ucrt64/include", "-L C:/msys64/ucrt64/lib -L C:/msys64/ucrt64/bin -lonnxruntime -lole32 -lavrt"),
        ("test_piper_tts", "host-cpp/tests/test_piper_tts.cpp host-cpp/src/audio/piper_tts.cpp", "-DJARVIS_HAS_PIPER=1 -I C:/msys64/ucrt64/include/onnxruntime -I C:/msys64/ucrt64/include", "-L C:/msys64/ucrt64/lib -L C:/msys64/ucrt64/bin -lonnxruntime -lespeak-ng -lole32 -lavrt"),
        ("test_whisper_stt", "host-cpp/tests/test_whisper_stt.cpp host-cpp/src/audio/whisper_stt.cpp", "-DJARVIS_HAS_WHISPER=1 -I C:/msys64/ucrt64/include", "-L C:/msys64/ucrt64/lib -L C:/msys64/ucrt64/bin -lwhisper -lggml -lggml-base")
    ]
    
    ok = True
    for name, src, extra, libs in cpp_tests:
        exe = os.path.join(test_dir, f"{name}.exe")
        compile_cmd = f"g++ -std=c++20 -O2 -Wall -Wextra -DJARVIS_HAS_ONNXRUNTIME=1 {src} {extra} -I host-cpp/include {libs} -o {exe}"
        if run_cmd(compile_cmd, desc=f"Compile C++ {name}"):
            ok &= run_cmd(f'"{exe}"', desc=f"Run C++ {name}")
        else:
            ok = False
    return ok

def verify_python():
    print("\n=== [3/4] Python Verification Suite ===")
    scripts_to_check = [
        "scripts/run-desktop.py",
        "scripts/verify-env.py",
        ".agents/scripts/guardrail.py",
        ".agents/scripts/test_guardrail.py"
    ]
    ok = True
    for s in scripts_to_check:
        full_path = os.path.join(REPO_ROOT, s)
        if os.path.isfile(full_path):
            ok &= run_cmd(f'python -m py_compile "{full_path}"', desc=f"Syntax Check: {s}")
    
    # Run guardrail test suite
    guardrail_test = os.path.join(REPO_ROOT, ".agents", "scripts", "test_guardrail.py")
    if os.path.isfile(guardrail_test):
        ok &= run_cmd(f'python "{guardrail_test}"', desc="Execute Guardrail Unit Tests")
    return ok

def verify_static_security():
    print("\n=== [4/4] Static Security Invariant Audit ===")
    ok = True
    
    # 1. Shell checks
    rust_src = os.path.join(REPO_ROOT, "daemon-rust", "src")
    shell_found = False
    for root, _, files in os.walk(rust_src):
        for f in files:
            if f.endswith(".rs"):
                p = os.path.join(root, f)
                with open(p, "r", encoding="utf-8", errors="ignore") as fh:
                    content = fh.read()
                    if 'Command::new("powershell")' in content or 'Command::new("cmd")' in content:
                        print(f"[FAIL] Found forbidden shell invocation in {f}")
                        shell_found = True
    if not shell_found:
        print("[PASS] Zero forbidden shell invocations (Command::new powershell/cmd)")
    else:
        ok = False

    # 2. Manifest check
    manifest = os.path.join(REPO_ROOT, "manifest.sha256")
    if os.path.isfile(manifest) and os.path.getsize(manifest) > 0:
        print("[PASS] Mandatory manifest.sha256 verified present")
    else:
        print("[FAIL] Missing or empty manifest.sha256")
        ok = False

    return ok

def main():
    print("=================================================================")
    print("   BETTER_JARVIS UNIFIED VERIFICATION INFRASTRUCTURE             ")
    print("=================================================================")
    rust_ok = verify_rust()
    cpp_ok = verify_cpp()
    py_ok = verify_python()
    sec_ok = verify_static_security()

    print("\n=================================================================")
    print("   VERIFICATION SUMMARY REPORT                                   ")
    print("=================================================================")
    print(f"Rust Suite:    {'PASS' if rust_ok else 'FAIL'}")
    print(f"C++ Suite:     {'PASS' if cpp_ok else 'FAIL'}")
    print(f"Python Suite:  {'PASS' if py_ok else 'FAIL'}")
    print(f"Security Gate: {'PASS' if sec_ok else 'FAIL'}")
    
    overall = rust_ok and cpp_ok and py_ok and sec_ok
    print(f"Overall Result: {'ALL GATES PASSED' if overall else 'VERIFICATION FAILED'}")
    sys.exit(0 if overall else 1)

if __name__ == "__main__":
    main()
