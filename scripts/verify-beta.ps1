# scripts/verify-beta.ps1
# Better_Jarvis Beta Release Candidate Verification Gate
$ErrorActionPreference = "Stop"

Write-Host "=================================================================" -ForegroundColor Cyan
Write-Host "   BETTER_JARVIS BETA RELEASE CANDIDATE VERIFICATION GATE        " -ForegroundColor Cyan
Write-Host "=================================================================" -ForegroundColor Cyan

# -----------------------------------------------------------------------------
# 1. Release Build Verification
# -----------------------------------------------------------------------------
Write-Host "`n[1/5] Release Build Verification..." -ForegroundColor Cyan
& "$PSScriptRoot/build.ps1"
if ($LASTEXITCODE -ne 0) {
    throw "Release build failed with exit code $LASTEXITCODE"
}

$hostExe = "build/host-cpp/Release/jarvis-host.exe"
$daemonExe = "build/host-cpp/Release/jarvis-daemon.exe"
if (-not (Test-Path $daemonExe)) {
    $daemonExe = "target/release/jarvis-daemon.exe"
}

if (-not (Test-Path $hostExe)) {
    throw "Missing release artifact: $hostExe"
}
$hostItem = Get-Item $hostExe
if ($hostItem.Length -le 0) {
    throw "Empty release artifact: $hostExe"
}
Write-Host "  [PASS] Verified C++ host release artifact ($($hostItem.Length) bytes): $hostExe" -ForegroundColor Green

if (-not (Test-Path $daemonExe)) {
    throw "Missing release artifact: $daemonExe"
}
$daemonItem = Get-Item $daemonExe
if ($daemonItem.Length -le 0) {
    throw "Empty release artifact: $daemonExe"
}
Write-Host "  [PASS] Verified Rust daemon release artifact ($($daemonItem.Length) bytes): $daemonExe" -ForegroundColor Green

# -----------------------------------------------------------------------------
# 2. Static Security Invariant Audit
# -----------------------------------------------------------------------------
Write-Host "`n[2/5] Static Security Invariant Audit..." -ForegroundColor Cyan

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

# -----------------------------------------------------------------------------
# 3. Rust Full Test & Lint Gate
# -----------------------------------------------------------------------------
Write-Host "`n[3/5] Rust Full Test & Lint Gate..." -ForegroundColor Cyan

Write-Host "  -> Running cargo test --manifest-path daemon-rust/Cargo.toml..." -ForegroundColor Yellow
cargo test --manifest-path daemon-rust/Cargo.toml
if ($LASTEXITCODE -ne 0) {
    throw "Rust tests failed with exit code $LASTEXITCODE"
}
Write-Host "     [PASS] All Rust unit and integration tests passed" -ForegroundColor Green

Write-Host "  -> Running cargo clippy --manifest-path daemon-rust/Cargo.toml --all-targets -- -D warnings..." -ForegroundColor Yellow
cargo clippy --manifest-path daemon-rust/Cargo.toml --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) {
    throw "Rust clippy failed with exit code $LASTEXITCODE"
}
Write-Host "     [PASS] Zero Rust compiler or Clippy warnings" -ForegroundColor Green

# -----------------------------------------------------------------------------
# 4. C++ Full Test Suite Gate
# -----------------------------------------------------------------------------
Write-Host "`n[4/5] C++ Full Test Suite Gate..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path "build/tests" | Out-Null

$cppTests = @(
    @{ Name = "test_frame_codec"; Src = "host-cpp/tests/test_frame_codec.cpp"; ExtraSrc = ""; Libs = "" },
    @{ Name = "test_ring_buffer"; Src = "host-cpp/tests/test_ring_buffer.cpp"; ExtraSrc = ""; Libs = "" },
    @{ Name = "test_resampler"; Src = "host-cpp/tests/test_resampler.cpp"; ExtraSrc = ""; Libs = "" },
    @{ Name = "test_aec"; Src = "host-cpp/tests/test_aec.cpp"; ExtraSrc = "host-cpp/src/audio/aec_processor.cpp host-cpp/src/audio/piper_tts.cpp"; Libs = "-lole32 -lavrt -luser32" },
    @{ Name = "test_hotkey_listener"; Src = "host-cpp/tests/test_hotkey_listener.cpp"; ExtraSrc = "host-cpp/src/input/hotkey_listener.cpp"; Libs = "-luser32" },
    @{ Name = "test_latency_tracker"; Src = "host-cpp/tests/test_latency_tracker.cpp"; ExtraSrc = ""; Libs = "" },
    @{ Name = "test_vad"; Src = "host-cpp/tests/test_vad.cpp"; ExtraSrc = "host-cpp/src/audio/silero_vad.cpp"; Libs = "" },
    @{ Name = "test_piper_tts"; Src = "host-cpp/tests/test_piper_tts.cpp"; ExtraSrc = "host-cpp/src/audio/piper_tts.cpp"; Libs = "-lole32 -lavrt -luser32" },
    @{ Name = "test_whisper_stt"; Src = "host-cpp/tests/test_whisper_stt.cpp"; ExtraSrc = "host-cpp/src/audio/whisper_stt.cpp"; Libs = "" }
)

foreach ($t in $cppTests) {
    $exe = "build/tests/$($t.Name).exe"
    Write-Host "  -> Compiling and running C++ test: $($t.Name)..." -ForegroundColor Yellow
    $compileCmd = "g++ -std=c++20 -O2 -Wall -Wextra -Werror $($t.Src) $($t.ExtraSrc) -I host-cpp/include $($t.Libs) -o $exe"
    Invoke-Expression $compileCmd
    if ($LASTEXITCODE -ne 0) {
        throw "C++ test compilation failed for $($t.Name)"
    }
    & $exe
    if ($LASTEXITCODE -ne 0) {
        throw "C++ test execution failed for $($t.Name) with exit code $LASTEXITCODE"
    }
    Write-Host "     [PASS] $($t.Name) passed cleanly" -ForegroundColor Green
}

# -----------------------------------------------------------------------------
# 5. Live Binary Security & Crash-Safety Smoke Tests
# -----------------------------------------------------------------------------
Write-Host "`n[5/5] Live Binary Security & Crash-Safety Smoke Tests..." -ForegroundColor Cyan

$smokeDataDir = Join-Path $env:TEMP ("jarvis_smoke_" + (Get-Random))
New-Item -ItemType Directory -Force -Path $smokeDataDir | Out-Null

try {
    # Test A: Unauthorized Parent-PID Rejection with isolated --data-dir
    Write-Host "  -> Verifying --parent-pid 999999 unauthorized startup rejection..." -ForegroundColor Yellow
    $proc = Start-Process -FilePath $daemonExe -ArgumentList "--parent-pid 999999 --data-dir `"$smokeDataDir`"" -Wait -PassThru -NoNewWindow
    if ($proc.ExitCode -eq 0) {
        throw "SECURITY VIOLATION: jarvis-daemon accepted mismatched --parent-pid 999999 with exit code 0"
    }
    Write-Host "     [PASS] Mismatched parent-PID correctly aborted startup with exit code $($proc.ExitCode)" -ForegroundColor Green

    # Test B: Real Host -> Daemon Job Object Cascade & Clean Kill
    Write-Host "  -> Verifying real Host -> Daemon Job Object cascade termination..." -ForegroundColor Yellow

    # Set scoped dummy key so missing API key never halts host startup
    $prevKey = [System.Environment]::GetEnvironmentVariable("GEMINI_API_KEY", "Process")
    [System.Environment]::SetEnvironmentVariable("GEMINI_API_KEY", "smoke-test-dummy", "Process")

    $hostProc = Start-Process -FilePath $hostExe -ArgumentList "--data-dir `"$smokeDataDir`"" -PassThru -NoNewWindow

    # Poll for child daemon process spawned by host
    $childDaemon = $null
    for ($attempt = 0; $attempt -lt 25; $attempt++) {
        Start-Sleep -Milliseconds 200
        $daemons = Get-CimInstance Win32_Process -Filter "Name='jarvis-daemon.exe'" -ErrorAction SilentlyContinue
        foreach ($d in $daemons) {
            if ($d.ParentProcessId -eq $hostProc.Id) {
                $childDaemon = $d
                break
            }
        }
        if ($childDaemon) { break }
        if ($hostProc.HasExited) {
            throw "SUPERVISION FAILURE: jarvis-host exited prematurely with code $($hostProc.ExitCode)"
        }
    }

    if (-not $childDaemon) {
        Stop-Process -Id $hostProc.Id -Force -ErrorAction SilentlyContinue
        throw "SUPERVISION FAILURE: jarvis-host failed to spawn child jarvis-daemon within 5 seconds"
    }

    $daemonPid = $childDaemon.ProcessId
    Write-Host "     [INFO] Detected child jarvis-daemon (PID $daemonPid) under host (PID $($hostProc.Id))" -ForegroundColor Cyan

    # Verify SQLite DB was created strictly inside $smokeDataDir
    $smokeDbPath = Join-Path $smokeDataDir "memory\jarvis_memory.db"
    $dbCreated = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        if (Test-Path $smokeDbPath) {
            $dbCreated = $true
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $dbCreated) {
        Stop-Process -Id $hostProc.Id -Force -ErrorAction SilentlyContinue
        throw "ISOLATION FAILURE: jarvis_memory.db was not created under isolated smoke data-dir: $smokeDbPath"
    }
    Write-Host "     [PASS] SQLite memory DB cleanly initialized under isolated smoke data-dir" -ForegroundColor Green

    # Terminate host ONLY via taskkill /F
    Write-Host "     -> Executing taskkill /F on host process $($hostProc.Id) only..." -ForegroundColor Yellow
    & taskkill /F /PID $hostProc.Id | Out-Null

    # Wait up to 1,500 ms and assert BOTH host and daemon are dead
    $reaped = $false
    for ($attempt = 0; $attempt -lt 15; $attempt++) {
        Start-Sleep -Milliseconds 100
        $hAlive = Get-Process -Id $hostProc.Id -ErrorAction SilentlyContinue
        $dAlive = Get-Process -Id $daemonPid -ErrorAction SilentlyContinue
        if (-not $hAlive -and -not $dAlive) {
            $reaped = $true
            break
        }
    }

    if (-not $reaped) {
        Stop-Process -Id $daemonPid -Force -ErrorAction SilentlyContinue
        throw "JOB OBJECT CASCADE FAILURE: Child daemon PID $daemonPid survived host kill"
    }
    Write-Host "     [PASS] Job Object cascade successfully reaped child daemon PID $daemonPid with zero orphans" -ForegroundColor Green

    # Restore environment variable
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

Write-Host "`n=================================================================" -ForegroundColor Green
Write-Host "   [SUCCESS] ALL BETA RELEASE CANDIDATE GATES PASSED!            " -ForegroundColor Green
Write-Host "=================================================================" -ForegroundColor Green
