# scripts/verify-v1.ps1
# Better_Jarvis v1.0.0 Production Verification Gate
$ErrorActionPreference = "Stop"

Write-Host "=================================================================" -ForegroundColor Cyan
Write-Host "   BETTER_JARVIS v1.0.0 PRODUCTION VERIFICATION GATE             " -ForegroundColor Cyan
Write-Host "=================================================================" -ForegroundColor Cyan

# -----------------------------------------------------------------------------
# 1. Release Packaging & Artifact Verification
# -----------------------------------------------------------------------------
Write-Host "`n[1/6] Release Packaging & Artifact Verification..." -ForegroundColor Cyan
& "$PSScriptRoot/package-release.ps1"
if ($LASTEXITCODE -ne 0) {
    throw "Packaging failed with exit code $LASTEXITCODE"
}

$stageDir = "dist/Better_Jarvis-v1.0.0-win64"
$hostExe = "$stageDir/jarvis-host.exe"
$daemonExe = "$stageDir/jarvis-daemon.exe"
$zipPath = "dist/Better_Jarvis-v1.0.0-win64.zip"
$sumsPath = "dist/SHA256SUMS.txt"

if (-not (Test-Path $hostExe)) { throw "Missing staged host executable: $hostExe" }
if (-not (Test-Path $daemonExe)) { throw "Missing staged daemon executable: $daemonExe" }
if (-not (Test-Path $zipPath)) { throw "Missing release zip package: $zipPath" }
if (-not (Test-Path $sumsPath)) { throw "Missing checksum manifest: $sumsPath" }

# Verify SHA256SUMS.txt is UTF-8 without BOM
$rawBytes = [System.IO.File]::ReadAllBytes((Resolve-Path $sumsPath).Path)
if ($rawBytes.Length -ge 3 -and $rawBytes[0] -eq 0xEF -and $rawBytes[1] -eq 0xBB -and $rawBytes[2] -eq 0xBF) {
    throw "PACKAGING AUDIT FAILURE: $sumsPath contains an illegal UTF-8 BOM"
}
Write-Host "  [PASS] Staged release binaries and BOM-free SHA256SUMS.txt verified" -ForegroundColor Green

# -----------------------------------------------------------------------------
# 2. Static Security & Telemetry Invariant Audit
# -----------------------------------------------------------------------------
Write-Host "`n[2/6] Static Security & Telemetry Invariant Audit..." -ForegroundColor Cyan

# A. Zero powershell / cmd invocations in daemon-rust/src
Write-Host "  -> Auditing daemon-rust/src for forbidden shell invocations..." -ForegroundColor Yellow
$shellMatches = Get-ChildItem -Path "daemon-rust/src" -Recurse -Filter "*.rs" | Select-String 'Command::new\s*\(\s*["''](powershell|cmd)'
if ($shellMatches) {
    throw "SECURITY AUDIT FAILURE: Found shell invocation in daemon-rust/src: $shellMatches"
}
Write-Host "     [PASS] Zero shell invocations (Command::new powershell/cmd)" -ForegroundColor Green

# B. Zero 127.0.0.1:9222 or remote-debugging-port in daemon-rust/src
Write-Host "  -> Auditing daemon-rust/src for unisolated CDP TCP ports..." -ForegroundColor Yellow
$cdpMatches = Get-ChildItem -Path "daemon-rust/src" -Recurse -Filter "*.rs" | Select-String '127\.0\.0\.1:9222|remote-debugging-port'
if ($cdpMatches) {
    throw "SECURITY AUDIT FAILURE: Found unsafe CDP TCP port/flag in daemon-rust/src: $cdpMatches"
}
Write-Host "     [PASS] Zero localhost CDP ports or remote-debugging-port flags" -ForegroundColor Green

# C. Zero SetWindowsHookEx / WH_KEYBOARD_LL in host-cpp/src
Write-Host "  -> Auditing host-cpp/src for low-level keyboard hooks..." -ForegroundColor Yellow
$hookMatches = Get-ChildItem -Path "host-cpp/src" -Recurse -Include "*.cpp", "*.hpp" | Select-String 'SetWindowsHookEx|WH_KEYBOARD_LL'
if ($hookMatches) {
    throw "SECURITY AUDIT FAILURE: Found low-level keyboard hook in host-cpp/src: $hookMatches"
}
Write-Host "     [PASS] Zero functional SetWindowsHookEx / WH_KEYBOARD_LL hooks" -ForegroundColor Green

# D. Verify manifest.sha256 (Mandatory Supply-Chain Model Pinning)
Write-Host "  -> Verifying mandatory manifest.sha256 supply-chain pinning..." -ForegroundColor Yellow
$manifestFile = "manifest.sha256"
if (-not (Test-Path $manifestFile)) {
    throw "SECURITY AUDIT FAILURE: Mandatory manifest.sha256 is missing from repository root"
}
$manifestContent = Get-Content $manifestFile
$manifestLines = $manifestContent | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and -not $_.StartsWith("#") }
if ($manifestLines.Count -eq 0) {
    throw "SECURITY AUDIT FAILURE: manifest.sha256 is empty"
}

$modelSearchDirs = @(
    "models",
    (Join-Path $env:LOCALAPPDATA "Better_Jarvis\models")
)

foreach ($line in $manifestLines) {
    if ($line -notmatch '^[0-9a-fA-F]{64}\s+\S+$') {
        throw "SECURITY AUDIT FAILURE: manifest.sha256 contains invalid entry format: '$line'"
    }
    $parts = -split $line
    $expectedHash = $parts[0].ToLowerInvariant()
    $assetName = $parts[1].Trim()

    foreach ($mDir in $modelSearchDirs) {
        $candidatePath = Join-Path $mDir $assetName
        if (Test-Path $candidatePath) {
            $actualHash = (Get-FileHash -Path $candidatePath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actualHash -ne $expectedHash) {
                throw "SECURITY AUDIT FAILURE: Cryptographic checksum mismatch for $($assetName): expected $expectedHash, got $actualHash"
            }
            Write-Host "     [PASS] Verified cryptographic hash for $assetName" -ForegroundColor Green
        }
    }
}
Write-Host "     [PASS] Mandatory manifest.sha256 validated cleanly" -ForegroundColor Green

# E. Native Inference Definitions
Write-Host "  -> Auditing CMake configuration for mandatory native inference definitions..." -ForegroundColor Yellow
$cmakeContent = Get-Content "host-cpp/CMakeLists.txt" -Raw
if ($cmakeContent -notmatch "JARVIS_HAS_ONNXRUNTIME=1" -or $cmakeContent -notmatch "JARVIS_HAS_WHISPER=1" -or $cmakeContent -notmatch "JARVIS_HAS_PIPER=1") {
    throw "INFERENCE AUDIT FAILURE: host-cpp/CMakeLists.txt missing mandatory JARVIS_HAS_* definitions"
}
Write-Host "     [PASS] Mandatory JARVIS_HAS_ONNXRUNTIME, JARVIS_HAS_WHISPER, JARVIS_HAS_PIPER verified" -ForegroundColor Green

# F. Zero-Telemetry Minidump & Local Crash Log Invariant
Write-Host "  -> Auditing crash and diagnostics handlers for zero telemetry upload..." -ForegroundColor Yellow
$diagCpp = "host-cpp/src/diagnostics/crash_handler.cpp"
$diagRs = "daemon-rust/src/diagnostics.rs"
if (Test-Path $diagCpp) {
    $cppNetMatches = Select-String -Path $diagCpp -Pattern 'WinHttp|InternetOpen|curl|socket|connect|http' -CaseSensitive
    if ($cppNetMatches) {
        throw "TELEMETRY VIOLATION: Crash handler contains unauthorized network calls: $cppNetMatches"
    }
}
if (Test-Path $diagRs) {
    $rsNetMatches = Select-String -Path $diagRs -Pattern 'reqwest|hyper|TcpStream|UdpSocket|http' -CaseSensitive
    if ($rsNetMatches) {
        throw "TELEMETRY VIOLATION: Panic diagnostics contains unauthorized network calls: $rsNetMatches"
    }
}
Write-Host "     [PASS] Zero network/telemetry calls in crash/panic diagnostics (local-only writes confirmed)" -ForegroundColor Green

# G. Least-Privilege Autostart & Installer Privilege Audit (Zero Elevation / HighestAvailable)
Write-Host "  -> Auditing startup scripts and installer manifests for least-privilege (Limited RunLevel)..." -ForegroundColor Yellow
$startupScripts = @("scripts/install-startup.ps1")
$packagingFiles = Get-ChildItem -Path "packaging" -Filter "*.*" -ErrorAction SilentlyContinue

$elevationMatches = @()
foreach ($s in $startupScripts) {
    if (Test-Path $s) {
        $m = Select-String -Path $s -Pattern 'HighestAvailable|-RunLevel\s+Highest'
        if ($m) { $elevationMatches += $m }
    }
}
foreach ($p in $packagingFiles) {
    $m = Select-String -Path $p.FullName -Pattern 'HighestAvailable|RunLevel\s*=\s*"?Highest"?|PrivilegesRequired\s*=\s*admin'
    if ($m) { $elevationMatches += $m }
}
if ($elevationMatches) {
    throw "PRIVILEGE ESCALATION VIOLATION: Found unauthorized elevation/HighestAvailable in startup or packaging manifests: $elevationMatches"
}
$limitedCheck = Select-String -Path "scripts/install-startup.ps1" -Pattern '-RunLevel\s+Limited'
if (-not $limitedCheck) {
    throw "LEAST-PRIVILEGE AUDIT FAILURE: scripts/install-startup.ps1 must explicitly declare -RunLevel Limited"
}
Write-Host "     [PASS] Zero elevation flags; strictly Limited RunLevel (Medium IL) confirmed in autostart & packaging" -ForegroundColor Green

# -----------------------------------------------------------------------------
# 3. Rust Full Test & Lint Gate
# -----------------------------------------------------------------------------
Write-Host "`n[3/6] Rust Full Test & Lint Gate..." -ForegroundColor Cyan
$env:CARGO_INCREMENTAL = "0"

$ucrtBin = "C:\msys64\ucrt64\bin"
if (Test-Path "$ucrtBin\onnxruntime.dll") {
    New-Item -ItemType Directory -Force -Path "target/debug" | Out-Null
    New-Item -ItemType Directory -Force -Path "target/debug/deps" | Out-Null
    Copy-Item "$ucrtBin\onnxruntime.dll" -Destination "target/debug/onnxruntime.dll" -Force
    Copy-Item "$ucrtBin\onnxruntime.dll" -Destination "target/debug/deps/onnxruntime.dll" -Force
}
$desktopRunner = "$PSScriptRoot/run-desktop.py"
Write-Host "  -> Running cargo test --manifest-path daemon-rust/Cargo.toml..." -ForegroundColor Yellow
if (Test-Path $desktopRunner) {
    python $desktopRunner cargo test --manifest-path daemon-rust/Cargo.toml
} else {
    cargo test --manifest-path daemon-rust/Cargo.toml
}
if ($LASTEXITCODE -ne 0) {
    throw "Rust tests failed with exit code $LASTEXITCODE"
}
Write-Host "     [PASS] All Rust unit and integration tests passed" -ForegroundColor Green

Write-Host "  -> Running cargo clippy --manifest-path daemon-rust/Cargo.toml --all-targets -- -D warnings..." -ForegroundColor Yellow
if (Test-Path $desktopRunner) {
    python $desktopRunner cargo clippy --manifest-path daemon-rust/Cargo.toml --all-targets -- -D warnings
} else {
    cargo clippy --manifest-path daemon-rust/Cargo.toml --all-targets -- -D warnings
}
if ($LASTEXITCODE -ne 0) {
    throw "Rust clippy failed with exit code $LASTEXITCODE"
}
Write-Host "     [PASS] Zero Rust compiler or Clippy warnings" -ForegroundColor Green

# -----------------------------------------------------------------------------
# 4. C++ Full Test Suite Gate
# -----------------------------------------------------------------------------
Write-Host "`n[4/6] C++ Full Test Suite Gate..." -ForegroundColor Cyan
$testDir = "build/tests"
New-Item -ItemType Directory -Force -Path $testDir | Out-Null

$ucrtBin = "C:\msys64\ucrt64\bin"
Copy-Item "$ucrtBin\onnxruntime.dll" -Destination "$testDir\onnxruntime.dll" -Force -ErrorAction SilentlyContinue
Get-ChildItem -Path $ucrtBin -Filter "*whisper*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "*ggml*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "*espeak*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "*pcaudio*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "*gomp*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "*winpthread*.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "libstdc++-6.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "libgcc_s_seh-1.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "libopenblas.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "libgfortran-5.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
Get-ChildItem -Path $ucrtBin -Filter "libquadmath-0.dll" | ForEach-Object { Copy-Item $_.FullName -Destination $testDir -Force }
if (Test-Path "C:\msys64\ucrt64\share\espeak-ng-data") {
    Copy-Item -Recurse -Force "C:\msys64\ucrt64\share\espeak-ng-data" -Destination "$testDir\espeak-ng-data"
}
$env:PATH = "$ucrtBin;$env:PATH"

$cppTests = @(
    @{ Name = "test_frame_codec"; Src = "host-cpp/tests/test_frame_codec.cpp"; ExtraSrc = ""; Libs = "" },
    @{ Name = "test_ring_buffer"; Src = "host-cpp/tests/test_ring_buffer.cpp"; ExtraSrc = ""; Libs = "" },
    @{ Name = "test_resampler"; Src = "host-cpp/tests/test_resampler.cpp"; ExtraSrc = ""; Libs = "" },
    @{ Name = "test_aec"; Src = "host-cpp/tests/test_aec.cpp"; ExtraSrc = "host-cpp/src/audio/aec_processor.cpp host-cpp/src/audio/aec_delay_estimator.cpp host-cpp/src/audio/piper_tts.cpp"; Libs = "C:/msys64/ucrt64/bin/onnxruntime.dll -L C:/msys64/ucrt64/lib -lespeak-ng -lole32 -lavrt -luser32" },
    @{ Name = "test_hotkey_listener"; Src = "host-cpp/tests/test_hotkey_listener.cpp"; ExtraSrc = "host-cpp/src/input/hotkey_listener.cpp"; Libs = "-luser32" },
    @{ Name = "test_latency_tracker"; Src = "host-cpp/tests/test_latency_tracker.cpp"; ExtraSrc = ""; Libs = "" },
    @{ Name = "test_vad"; Src = "host-cpp/tests/test_vad.cpp"; ExtraSrc = "host-cpp/src/audio/silero_vad.cpp"; Libs = "C:/msys64/ucrt64/bin/onnxruntime.dll" },
    @{ Name = "test_piper_tts"; Src = "host-cpp/tests/test_piper_tts.cpp"; ExtraSrc = "host-cpp/src/audio/piper_tts.cpp"; Libs = "C:/msys64/ucrt64/bin/onnxruntime.dll -L C:/msys64/ucrt64/lib -lespeak-ng -lole32 -lavrt -luser32" },
    @{ Name = "test_whisper_stt"; Src = "host-cpp/tests/test_whisper_stt.cpp"; ExtraSrc = "host-cpp/src/audio/whisper_stt.cpp"; Libs = "-L C:/msys64/ucrt64/lib -lwhisper -lggml" },
    @{ Name = "test_crash_handler"; Src = "host-cpp/tests/test_crash_handler.cpp"; ExtraSrc = "host-cpp/src/diagnostics/crash_handler.cpp"; Libs = "" },
    @{ Name = "test_hud_watchdog"; Src = "host-cpp/tests/test_hud_watchdog.cpp"; ExtraSrc = ""; Libs = "" },
    @{ Name = "test_operational_throughput"; Src = "host-cpp/tests/test_operational_throughput.cpp"; ExtraSrc = ""; Libs = "" }
)

foreach ($t in $cppTests) {
    $exe = "build/tests/$($t.Name).exe"
    Write-Host "  -> Compiling and running C++ test: $($t.Name)..." -ForegroundColor Yellow
    $compileCmd = "g++ -std=c++20 -O2 -Wall -Wextra -Werror -DJARVIS_HAS_ONNXRUNTIME=1 -DJARVIS_HAS_WHISPER=1 -DJARVIS_HAS_PIPER=1 $($t.Src) $($t.ExtraSrc) -I host-cpp/include -I C:/msys64/ucrt64/include -I C:/msys64/ucrt64/include/onnxruntime -I C:/msys64/ucrt64/include/espeak-ng $($t.Libs) -o $exe"
    Invoke-Expression $compileCmd
    if ($LASTEXITCODE -ne 0) {
        throw "C++ test compilation failed for $($t.Name)"
    }
    if (Test-Path $desktopRunner) {
        python $desktopRunner $exe
    } else {
        & $exe
    }
    if ($LASTEXITCODE -ne 0) {
        throw "C++ test execution failed for $($t.Name) with exit code $LASTEXITCODE"
    }
    Write-Host "     [PASS] $($t.Name) passed cleanly" -ForegroundColor Green
}

# -----------------------------------------------------------------------------
# 5. Live Binary Security & Crash-Safety Smoke Tests
# -----------------------------------------------------------------------------
Write-Host "`n[5/6] Live Binary Security & Crash-Safety Smoke Tests..." -ForegroundColor Cyan

$tempBase = if (Test-Path "$env:LOCALAPPDATA\Temp\Low") { "$env:LOCALAPPDATA\Temp\Low" } else { $env:TEMP }
$smokeDataDir = Join-Path $tempBase ("jarvis_smoke_" + (Get-Random))
New-Item -ItemType Directory -Force -Path $smokeDataDir | Out-Null
icacls $smokeDataDir /setintegritylevel '(OI)(CI)L' | Out-Null

try {
    # Test A: Unauthorized Parent-PID Rejection with isolated --data-dir
    Write-Host "  -> Verifying --parent-pid 999999 unauthorized startup rejection..." -ForegroundColor Yellow
    python $desktopRunner $daemonExe --parent-pid 999999 --data-dir "$smokeDataDir"
    if ($LASTEXITCODE -eq 0) {
        throw "SECURITY VIOLATION: jarvis-daemon accepted mismatched --parent-pid 999999 with exit code 0"
    }
    Write-Host "     [PASS] Mismatched parent-PID correctly aborted startup with exit code $LASTEXITCODE" -ForegroundColor Green

    # Test B: Real Host -> Daemon Job Object Cascade & Clean Kill
    Write-Host "  -> Verifying real Host -> Daemon Job Object cascade termination..." -ForegroundColor Yellow

    $prevKey = [System.Environment]::GetEnvironmentVariable("GEMINI_API_KEY", "Process")
    [System.Environment]::SetEnvironmentVariable("GEMINI_API_KEY", "smoke-test-dummy", "Process")

    $hostPidRaw = python $desktopRunner --detach $hostExe --data-dir "$smokeDataDir"
    $hostPid = [int]($hostPidRaw.Trim())

    $childDaemon = $null
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Milliseconds 200
        try {
            $daemons = Get-CimInstance Win32_Process -Filter "Name='jarvis-daemon.exe'" -ErrorAction Stop
            foreach ($d in $daemons) {
                if ($d.ParentProcessId -eq $hostPid) {
                    $childDaemon = [PSCustomObject]@{ ProcessId = $d.ProcessId; ParentProcessId = $d.ParentProcessId }
                    break
                }
            }
        } catch {
            $dProc = Get-Process -Name "jarvis-daemon" -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($dProc) {
                $childDaemon = [PSCustomObject]@{ ProcessId = $dProc.Id; ParentProcessId = $hostPid }
            }
        }
        if ($childDaemon) { break }
        $hAlive = Get-Process -Id $hostPid -ErrorAction SilentlyContinue
        if (-not $hAlive) {
            throw "SUPERVISION FAILURE: jarvis-host PID $hostPid exited prematurely"
        }
    }

    if (-not $childDaemon) {
        & taskkill /F /PID $hostPid | Out-Null
        throw "SUPERVISION FAILURE: jarvis-host failed to spawn child jarvis-daemon within 6 seconds"
    }

    $daemonPid = $childDaemon.ProcessId
    Write-Host "     [INFO] Detected child jarvis-daemon (PID $daemonPid) under host (PID $hostPid)" -ForegroundColor Cyan

    Start-Sleep -Milliseconds 500
    $hRam = [math]::Round((Get-Process -Id $hostPid -ErrorAction SilentlyContinue).WorkingSet64 / 1MB, 1)
    $dRam = [math]::Round((Get-Process -Id $daemonPid -ErrorAction SilentlyContinue).WorkingSet64 / 1MB, 1)
    $totalRam = [math]::Round($hRam + $dRam, 1)
    Write-Host "     [INFO] Live Working Set RAM: jarvis-host = $hRam MB, jarvis-daemon = $dRam MB (Total: $totalRam MB)" -ForegroundColor Cyan

    $smokeDbPath = Join-Path $smokeDataDir "memory\jarvis_memory.db"
    $dbCreated = $false
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        if (Test-Path $smokeDbPath) {
            $dbCreated = $true
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $dbCreated) {
        & taskkill /F /PID $hostPid | Out-Null
        throw "ISOLATION FAILURE: jarvis_memory.db was not created under isolated smoke data-dir: $smokeDbPath"
    }
    Write-Host "     [PASS] SQLite memory DB cleanly initialized under isolated smoke data-dir" -ForegroundColor Green

    Write-Host "     -> Executing taskkill /F on host process $hostPid only..." -ForegroundColor Yellow
    & taskkill /F /PID $hostPid | Out-Null

    $reaped = $false
    for ($attempt = 0; $attempt -lt 15; $attempt++) {
        Start-Sleep -Milliseconds 100
        $hAlive = Get-Process -Id $hostPid -ErrorAction SilentlyContinue
        $dAlive = Get-Process -Id $daemonPid -ErrorAction SilentlyContinue
        if (-not $hAlive -and -not $dAlive) {
            $reaped = $true
            break
        }
    }

    if (-not $reaped) {
        & taskkill /F /PID $daemonPid | Out-Null
        throw "JOB OBJECT CASCADE FAILURE: Child daemon PID $daemonPid survived host kill"
    }
    Write-Host "     [PASS] Job Object cascade successfully reaped child daemon PID $daemonPid with zero orphans" -ForegroundColor Green

    [System.Environment]::SetEnvironmentVariable("GEMINI_API_KEY", $prevKey, "Process")

    # Test C: SQLite WAL Crash Durability on the Killed Smoke DB
    Write-Host "  -> Verifying SQLite WAL crash durability and integrity_check on smoke DB..." -ForegroundColor Yellow
    $pyScript = @"
import sqlite3
con = sqlite3.connect(r'$smokeDbPath')
cur = con.cursor()
cur.execute('PRAGMA integrity_check;')
res = cur.fetchall()
con.close()
if res != [('ok',)]:
    raise RuntimeError(f'Integrity check failed: {res}')
print('SMOKE_DB_INTEGRITY_OK')
"@
    $checkOut = python -c "$pyScript"
    if ($checkOut -notmatch "SMOKE_DB_INTEGRITY_OK") {
        throw "SQLite WAL integrity check failed on killed smoke DB: $checkOut"
    }
    Write-Host "     [PASS] Killed smoke DB passed PRAGMA integrity_check ('ok')" -ForegroundColor Green

} finally {
    Remove-Item -Recurse -Force -Path $smokeDataDir -ErrorAction SilentlyContinue
}

# -----------------------------------------------------------------------------
# 6. Clean-PATH Standalone Launch Proof (Phase 6)
# -----------------------------------------------------------------------------
Write-Host "`n[6/6] Clean-PATH Standalone Launch Proof (Zero MSYS2 on PATH)..." -ForegroundColor Cyan

$tempBase = if (Test-Path "$env:LOCALAPPDATA\Temp\Low") { "$env:LOCALAPPDATA\Temp\Low" } else { $env:TEMP }
$cleanSmokeDir = Join-Path $tempBase ("jarvis_clean_smoke_" + (Get-Random))
New-Item -ItemType Directory -Force -Path $cleanSmokeDir | Out-Null
icacls $cleanSmokeDir /setintegritylevel '(OI)(CI)L' | Out-Null

$origPath = $env:PATH
$pythonExe = (Get-Command python).Source
try {
    # Isolate PATH strictly to standard Windows directories
    $env:PATH = "C:\Windows\System32;C:\Windows"
    Write-Host "  -> Operating under isolated clean PATH: '$env:PATH'" -ForegroundColor Yellow

    $prevKey = [System.Environment]::GetEnvironmentVariable("GEMINI_API_KEY", "Process")
    [System.Environment]::SetEnvironmentVariable("GEMINI_API_KEY", "clean-path-dummy", "Process")

    $cleanHostExe = (Resolve-Path $hostExe).Path
    Write-Host "  -> Launching staged host binary from clean environment: $cleanHostExe" -ForegroundColor Yellow

    $cleanHostPidRaw = & $pythonExe $desktopRunner --detach $cleanHostExe --data-dir "$cleanSmokeDir"
    $cleanHostPid = [int]($cleanHostPidRaw.Trim())

    $cleanChildDaemon = $null
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Milliseconds 200
        try {
            $daemons = Get-CimInstance Win32_Process -Filter "Name='jarvis-daemon.exe'" -ErrorAction Stop
            foreach ($d in $daemons) {
                if ($d.ParentProcessId -eq $cleanHostPid) {
                    $cleanChildDaemon = [PSCustomObject]@{ ProcessId = $d.ProcessId; ParentProcessId = $d.ParentProcessId }
                    break
                }
            }
        } catch {
            $dProc = Get-Process -Name "jarvis-daemon" -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($dProc) {
                $cleanChildDaemon = [PSCustomObject]@{ ProcessId = $dProc.Id; ParentProcessId = $cleanHostPid }
            }
        }
        if ($cleanChildDaemon) { break }
        $chAlive = Get-Process -Id $cleanHostPid -ErrorAction SilentlyContinue
        if (-not $chAlive) {
            throw "CLEAN-PATH LAUNCH FAILURE: jarvis-host PID $cleanHostPid exited prematurely (likely missing staged DLL)"
        }
    }

    if (-not $cleanChildDaemon) {
        & taskkill /F /PID $cleanHostPid | Out-Null
        throw "CLEAN-PATH LAUNCH FAILURE: jarvis-host failed to spawn child daemon under clean PATH"
    }

    $cleanDaemonPid = $cleanChildDaemon.ProcessId
    Write-Host "     [PASS] Successfully spawned child daemon PID $cleanDaemonPid under clean PATH (All DLLs resolved locally)" -ForegroundColor Green

    $cleanDbPath = Join-Path $cleanSmokeDir "memory\jarvis_memory.db"
    $cleanDbCreated = $false
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        if (Test-Path $cleanDbPath) {
            $cleanDbCreated = $true
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $cleanDbCreated) {
        & taskkill /F /PID $cleanHostPid | Out-Null
        throw "CLEAN-PATH LAUNCH FAILURE: Database not created at $cleanDbPath"
    }
    Write-Host "     [PASS] SQLite DB initialized cleanly under clean PATH" -ForegroundColor Green

    # Reaping test
    & taskkill /F /PID $cleanHostPid | Out-Null
    $cleanReaped = $false
    for ($attempt = 0; $attempt -lt 15; $attempt++) {
        Start-Sleep -Milliseconds 100
        $hAlive = Get-Process -Id $cleanHostPid -ErrorAction SilentlyContinue
        $dAlive = Get-Process -Id $cleanDaemonPid -ErrorAction SilentlyContinue
        if (-not $hAlive -and -not $dAlive) {
            $cleanReaped = $true
            break
        }
    }

    if (-not $cleanReaped) {
        & taskkill /F /PID $cleanDaemonPid | Out-Null
        throw "CLEAN-PATH REAP FAILURE: Child daemon PID $cleanDaemonPid survived host termination"
    }
    Write-Host "     [PASS] Clean-PATH Job Object cascade reaped child daemon cleanly with zero orphans" -ForegroundColor Green

    [System.Environment]::SetEnvironmentVariable("GEMINI_API_KEY", $prevKey, "Process")

    # Restore PATH for python call
    $env:PATH = $origPath

    # Verify DB integrity
    $pyCleanScript = @"
import sqlite3
con = sqlite3.connect(r'$cleanDbPath')
cur = con.cursor()
cur.execute('PRAGMA integrity_check;')
res = cur.fetchall()
con.close()
if res != [('ok',)]:
    raise RuntimeError(f'Integrity check failed: {res}')
print('CLEAN_PATH_DB_INTEGRITY_OK')
"@
    $cleanCheckOut = python -c "$pyCleanScript"
    if ($cleanCheckOut -notmatch "CLEAN_PATH_DB_INTEGRITY_OK") {
        throw "SQLite WAL integrity check failed on clean-PATH DB: $cleanCheckOut"
    }
    Write-Host "     [PASS] Clean-PATH killed DB passed PRAGMA integrity_check ('ok')" -ForegroundColor Green

} finally {
    $env:PATH = $origPath
    Remove-Item -Recurse -Force -Path $cleanSmokeDir -ErrorAction SilentlyContinue
}

Write-Host "`n=================================================================" -ForegroundColor Green
Write-Host "   [SUCCESS] ALL v1.0.0 PRODUCTION GATES PASSED!                 " -ForegroundColor Green
Write-Host "=================================================================" -ForegroundColor Green
