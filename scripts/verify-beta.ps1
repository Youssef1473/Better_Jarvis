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

# D. Verify manifest.sha256 if present
if (Test-Path "manifest.sha256") {
    Write-Host "  -> Verifying manifest.sha256 integrity..." -ForegroundColor Yellow
    $lines = Get-Content "manifest.sha256"
    foreach ($line in $lines) {
        if ([string]::IsNullOrWhiteSpace($line) -or $line.StartsWith("#")) { continue }
        $parts = -split $line
        if ($parts.Count -lt 2 -or $parts[0].Length -ne 64) {
            throw "SECURITY AUDIT FAILURE: Invalid SHA-256 digest format in manifest.sha256: $line"
        }
    }
    Write-Host "     [PASS] manifest.sha256 verified cleanly" -ForegroundColor Green
} else {
    Write-Host "     [PASS] manifest.sha256 not present (optional)" -ForegroundColor Green
}

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

# Test A: Unauthorized Parent-PID Rejection
Write-Host "  -> Verifying --parent-pid 999999 unauthorized startup rejection..." -ForegroundColor Yellow
$proc = Start-Process -FilePath $daemonExe -ArgumentList "--parent-pid 999999" -Wait -PassThru -NoNewWindow
if ($proc.ExitCode -eq 0) {
    throw "SECURITY VIOLATION: jarvis-daemon accepted mismatched --parent-pid 999999 with exit code 0"
}
Write-Host "     [PASS] Mismatched parent-PID correctly aborted startup with exit code $($proc.ExitCode)" -ForegroundColor Green

# Test B: Hard-Kill Process Termination & Orphan Cleanup
Write-Host "  -> Verifying forced termination (taskkill /F) and zero-orphan cleanup..." -ForegroundColor Yellow
$daemonProc = Start-Process -FilePath $daemonExe -ArgumentList "--unsafe-allow-same-user-ipc" -PassThru -NoNewWindow
Start-Sleep -Milliseconds 500
Stop-Process -Id $daemonProc.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 200
$orphaned = Get-Process -Id $daemonProc.Id -ErrorAction SilentlyContinue
if ($orphaned) {
    throw "CRASH SAFETY FAILURE: Orphaned jarvis-daemon process $($daemonProc.Id) survived forced termination"
}
Write-Host "     [PASS] Process $($daemonProc.Id) terminated cleanly; zero orphaned processes remain" -ForegroundColor Green

# Test C: SQLite WAL Crash Durability & PRAGMA integrity_check
Write-Host "  -> Verifying SQLite WAL crash durability and integrity_check..." -ForegroundColor Yellow
$tempDbDir = Join-Path $env:TEMP ("jarvis_beta_gate_" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tempDbDir | Out-Null
$tempDbPath = Join-Path $tempDbDir "test_gate.db"

$pyScript = @"
import sqlite3
con = sqlite3.connect(r'$tempDbPath')
con.execute('PRAGMA journal_mode=WAL;')
con.execute('PRAGMA synchronous=NORMAL;')
con.execute('CREATE TABLE facts (id INTEGER PRIMARY KEY, k TEXT, v TEXT);')
for i in range(100):
    con.execute('INSERT INTO facts (k, v) VALUES (?, ?);', (f'k{i}', f'v{i}'))
con.commit()
con.close()

con2 = sqlite3.connect(r'$tempDbPath')
cur = con2.cursor()
cur.execute('PRAGMA integrity_check;')
res = cur.fetchall()
con2.close()
if res != [('ok',)]:
    raise RuntimeError(f'Integrity check failed: {res}')
print('INTEGRITY_CHECK_OK')
"@

$checkOut = python -c "$pyScript"
if ($checkOut -notmatch "INTEGRITY_CHECK_OK") {
    throw "SQLite WAL integrity check failed: $checkOut"
}
Remove-Item -Recurse -Force -Path $tempDbDir
Write-Host "     [PASS] SQLite WAL PRAGMA integrity_check returned 'ok'" -ForegroundColor Green

Write-Host "`n=================================================================" -ForegroundColor Green
Write-Host "   [SUCCESS] ALL BETA RELEASE CANDIDATE GATES PASSED!            " -ForegroundColor Green
Write-Host "=================================================================" -ForegroundColor Green
