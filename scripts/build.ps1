$ErrorActionPreference = "Stop"

Write-Host "[1/2] Building Rust daemon..." -ForegroundColor Cyan
cargo build --release -p jarvis-daemon

Write-Host "[2/2] Building C++ host..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path "build/host-cpp/Release" | Out-Null

if (Get-Command cmake -ErrorAction SilentlyContinue) {
    cmake -B build -S .
    cmake --build build --config Release
} elseif (Get-Command g++ -ErrorAction SilentlyContinue) {
    Write-Host "Using MinGW g++ compiler..." -ForegroundColor Yellow
    g++ -std=c++20 -O2 -Wall -Wextra -Werror host-cpp/src/main.cpp host-cpp/src/audio/wasapi_capture.cpp host-cpp/src/audio/aec_processor.cpp host-cpp/src/audio/silero_vad.cpp host-cpp/src/audio/whisper_stt.cpp host-cpp/src/audio/piper_tts.cpp host-cpp/src/input/hotkey_listener.cpp -I host-cpp/include -mwindows -municode -luser32 -lgdi32 -ldwmapi -lshell32 -lavrt -lole32 -luuid -lsapi -ladvapi32 -o build/host-cpp/Release/jarvis-host.exe
    if ($LASTEXITCODE -ne 0) { throw "C++ host build failed" }
} else {
    throw "Neither cmake nor g++ found in PATH. Please install CMake or MinGW-w64."
}

Write-Host "Build complete: build/host-cpp/Release/jarvis-host.exe & target/release/jarvis-daemon.exe" -ForegroundColor Green

# Stage daemon beside host for unified execution
Copy-Item "target/release/jarvis-daemon.exe" -Destination "build/host-cpp/Release/jarvis-daemon.exe" -Force
