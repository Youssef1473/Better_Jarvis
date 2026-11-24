$ErrorActionPreference = "Stop"

Write-Host "[1/2] Building Rust daemon..." -ForegroundColor Cyan
cargo build --release -p jarvis-daemon
if ($LASTEXITCODE -ne 0) { throw "Rust daemon build failed" }

Write-Host "[2/2] Building C++ host..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path "build/host-cpp/Release" | Out-Null

if (Get-Command cmake -ErrorAction SilentlyContinue) {
    cmake -B build -S . -DCMAKE_BUILD_TYPE=Release
    cmake --build build --config Release
    if ($LASTEXITCODE -ne 0) { throw "C++ host build failed" }
    if (Test-Path "build/host-cpp/jarvis-host.exe") {
        Copy-Item "build/host-cpp/jarvis-host.exe" -Destination "build/host-cpp/Release/jarvis-host.exe" -Force
    }
} elseif (Get-Command g++ -ErrorAction SilentlyContinue) {
    Write-Host "Using MinGW g++ compiler..." -ForegroundColor Yellow
    g++ -std=c++20 -O2 -Wall -Wextra -Werror -DJARVIS_HAS_ONNXRUNTIME=1 -DJARVIS_HAS_WHISPER=1 -DJARVIS_HAS_PIPER=1 host-cpp/src/main.cpp host-cpp/src/audio/wasapi_capture.cpp host-cpp/src/audio/aec_processor.cpp host-cpp/src/audio/aec_delay_estimator.cpp host-cpp/src/audio/silero_vad.cpp host-cpp/src/audio/whisper_stt.cpp host-cpp/src/audio/piper_tts.cpp host-cpp/src/input/hotkey_listener.cpp host-cpp/src/diagnostics/crash_handler.cpp -I host-cpp/include -I C:/msys64/ucrt64/include -I C:/msys64/ucrt64/include/onnxruntime -I C:/msys64/ucrt64/include/espeak-ng C:/msys64/ucrt64/bin/onnxruntime.dll -L C:/msys64/ucrt64/lib -lwhisper -lggml -lespeak-ng -mwindows -municode -luser32 -lgdi32 -ldwmapi -lshell32 -lavrt -lole32 -luuid -ladvapi32 -o build/host-cpp/Release/jarvis-host.exe
    if ($LASTEXITCODE -ne 0) { throw "C++ host build failed" }
} else {
    throw "Neither cmake nor g++ found in PATH. Please install CMake or MinGW-w64."
}

Write-Host "Build complete: build/host-cpp/Release/jarvis-host.exe & target/release/jarvis-daemon.exe" -ForegroundColor Green

# Stage daemon beside host for unified execution
Copy-Item "target/release/jarvis-daemon.exe" -Destination "build/host-cpp/Release/jarvis-daemon.exe" -Force

# Stage UCRT64 runtime inference DLLs & data
$ucrtBin = "C:\msys64\ucrt64\bin"
$stageDirs = @("build/host-cpp/Release", "build/host-cpp")
foreach ($sDir in $stageDirs) {
    if (Test-Path $sDir) {
        Copy-Item "$ucrtBin\onnxruntime.dll" -Destination "$sDir\onnxruntime.dll" -Force -ErrorAction SilentlyContinue
        Get-ChildItem -Path $ucrtBin -Filter "*whisper*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "*ggml*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "*espeak*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "*pcaudio*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "*gomp*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "*winpthread*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "libstdc++-6.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "libgcc_s_seh-1.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "libopenblas.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "libgfortran-5.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        Get-ChildItem -Path $ucrtBin -Filter "libquadmath-0.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $sDir -Force }
        
        $espeakDataSrc = "C:\msys64\ucrt64\share\espeak-ng-data"
        if (Test-Path $espeakDataSrc) {
            Copy-Item -Recurse -Force $espeakDataSrc -Destination "$sDir\espeak-ng-data"
        }
    }
}
