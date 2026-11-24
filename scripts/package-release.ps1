# scripts/package-release.ps1
# Better_Jarvis v1.0.0 Release Packaging Script
[CmdletBinding()]
param (
    [switch]$SkipBuild,
    [string]$Version = "v1.0.0",
    [string]$DistRoot = "dist",
    [string]$StageDir = "dist/Better_Jarvis-v1.0.0-win64",
    [string]$ZipFile = "dist/Better_Jarvis-v1.0.0-win64.zip"
)

$ErrorActionPreference = "Stop"

Write-Host "=================================================================" -ForegroundColor Cyan
Write-Host "   BETTER_JARVIS v1.0.0 RELEASE PACKAGING                        " -ForegroundColor Cyan
Write-Host "=================================================================" -ForegroundColor Cyan

# 1. Build release binaries if not skipped
if (-not $SkipBuild) {
    Write-Host "`n[1/5] Building release artifacts..." -ForegroundColor Cyan
    & "$PSScriptRoot/build.ps1"
    if ($LASTEXITCODE -ne 0) {
        throw "Release build failed with exit code $LASTEXITCODE"
    }
} else {
    Write-Host "`n[1/5] Skipping build step (-SkipBuild specified)..." -ForegroundColor Yellow
}

# 2. Prepare clean staging directory
Write-Host "`n[2/5] Staging standalone release layout..." -ForegroundColor Cyan
if (Test-Path $StageDir) {
    Remove-Item -Recurse -Force $StageDir
}
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null
New-Item -ItemType Directory -Force -Path "$StageDir/config" | Out-Null
New-Item -ItemType Directory -Force -Path "$StageDir/scripts" | Out-Null
New-Item -ItemType Directory -Force -Path "$StageDir/models" | Out-Null

# Copy stripped binaries
$hostSrc = "build/host-cpp/Release/jarvis-host.exe"
if (-not (Test-Path $hostSrc)) {
    $hostSrc = "build/host-cpp/jarvis-host.exe"
}
$daemonSrc = "build/host-cpp/Release/jarvis-daemon.exe"
if (-not (Test-Path $daemonSrc)) {
    $daemonSrc = "target/release/jarvis-daemon.exe"
}

if (-not (Test-Path $hostSrc)) { throw "Missing host executable: $hostSrc" }
if (-not (Test-Path $daemonSrc)) { throw "Missing daemon executable: $daemonSrc" }

Copy-Item $hostSrc -Destination "$StageDir/jarvis-host.exe" -Force
Copy-Item $daemonSrc -Destination "$StageDir/jarvis-daemon.exe" -Force

# Optional binary stripping via MSYS2 strip if available
$stripExe = "C:\msys64\ucrt64\bin\strip.exe"
if (Test-Path $stripExe) {
    & $stripExe --strip-all "$StageDir/jarvis-host.exe" 2>$null
    & $stripExe --strip-all "$StageDir/jarvis-daemon.exe" 2>$null
    Write-Host "  [INFO] Stripped debug symbols from release executables" -ForegroundColor Yellow
}

# 3. Stage runtime DLLs & eSpeak-NG data
Write-Host "`n[3/5] Copying runtime dependencies (UCRT64 & inference DLLs)..." -ForegroundColor Cyan
$ucrtBin = "C:\msys64\ucrt64\bin"

$requiredDllPatterns = @(
    "onnxruntime.dll",
    "*whisper*.dll",
    "*ggml*.dll",
    "*espeak*.dll",
    "*pcaudio*.dll",
    "*gomp*.dll",
    "*winpthread*.dll",
    "libstdc++-6.dll",
    "libgcc_s_seh-1.dll",
    "libopenblas.dll",
    "libgfortran-5.dll",
    "libquadmath-0.dll"
)

foreach ($pattern in $requiredDllPatterns) {
    $matched = Get-ChildItem -Path $ucrtBin -Filter $pattern -ErrorAction SilentlyContinue
    if (-not $matched -or $matched.Count -eq 0) {
        Write-Warning "No DLL matching pattern: $pattern in $ucrtBin"
    } else {
        foreach ($dll in $matched) {
            Copy-Item $dll.FullName -Destination $StageDir -Force
            Write-Host "  -> Staged DLL: $($dll.Name)" -ForegroundColor Green
        }
    }
}

# Copy eSpeak-NG phoneme tables
$espeakDataSrc = "C:\msys64\ucrt64\share\espeak-ng-data"
if (Test-Path $espeakDataSrc) {
    Copy-Item -Recurse -Force $espeakDataSrc -Destination "$StageDir/espeak-ng-data"
    Write-Host "  -> Staged eSpeak-NG phoneme data directory" -ForegroundColor Green
} else {
    Write-Warning "eSpeak-NG data directory not found at $espeakDataSrc"
}

# Copy configs, scripts, and release documentation
Copy-Item "config/config.toml" -Destination "$StageDir/config/config.toml" -Force
Copy-Item "manifest.sha256" -Destination "$StageDir/manifest.sha256" -Force
Copy-Item "scripts/setup-models.ps1" -Destination "$StageDir/scripts/setup-models.ps1" -Force
Copy-Item "scripts/install-startup.ps1" -Destination "$StageDir/scripts/install-startup.ps1" -Force
Copy-Item "scripts/uninstall-startup.ps1" -Destination "$StageDir/scripts/uninstall-startup.ps1" -Force
if (Test-Path "README_RELEASE.txt") {
    Copy-Item "README_RELEASE.txt" -Destination "$StageDir/README.txt" -Force
}

# 4. Generate Zip Archive & Cryptographic Checksums (BOM-free)
Write-Host "`n[4/5] Packaging standalone zip archive and generating SHA256SUMS.txt..." -ForegroundColor Cyan
if (Test-Path $ZipFile) {
    Remove-Item -Force $ZipFile
}
Compress-Archive -Path "$StageDir/*" -DestinationPath $ZipFile -Force
$zipItem = Get-Item $ZipFile
Write-Host "  [PASS] Created standalone zip archive ($([math]::Round($zipItem.Length / 1MB, 2)) MB): $ZipFile" -ForegroundColor Green

# Generate SHA256SUMS.txt (UTF-8 without BOM)
$distResolved = (Resolve-Path $DistRoot).Path
$sumsLines = @()
$zipHash = (Get-FileHash -Path $ZipFile -Algorithm SHA256).Hash.ToLowerInvariant()
$zipLeaf = Split-Path $ZipFile -Leaf
$sumsLines += "$zipHash  $zipLeaf"

Get-ChildItem -Path $StageDir -Recurse -File | ForEach-Object {
    $relPath = $_.FullName.Substring($distResolved.Length + 1).Replace("\", "/")
    $fileHash = (Get-FileHash -Path $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $sumsLines += "$fileHash  $relPath"
}

$sumsContent = ($sumsLines -join "`r`n") + "`r`n"
$sumsPath = Join-Path $DistRoot "SHA256SUMS.txt"
[System.IO.File]::WriteAllText($sumsPath, $sumsContent, (New-Object System.Text.UTF8Encoding($false)))
Write-Host "  [PASS] Written cryptographic manifest (UTF-8 without BOM): $sumsPath" -ForegroundColor Green

# 5. Check for Installer Compilers (Inno Setup & WiX)
Write-Host "`n[5/5] Checking for installer compilers..." -ForegroundColor Cyan
$isccCandidates = @(
    "ISCC.exe",
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 6\ISCC.exe",
    "C:\Program Files (x86)\Inno Setup 7\ISCC.exe",
    "C:\Program Files\Inno Setup 7\ISCC.exe"
)

$isccPath = $null
foreach ($cand in $isccCandidates) {
    if (Get-Command $cand -ErrorAction SilentlyContinue) {
        $isccPath = (Get-Command $cand).Source
        break
    } elseif (Test-Path $cand) {
        $isccPath = $cand
        break
    }
}

if ($isccPath) {
    Write-Host "  -> Compiling Inno Setup installer via $isccPath..." -ForegroundColor Yellow
    & $isccPath "packaging/better_jarvis.iss"
    if ($LASTEXITCODE -eq 0) {
        Write-Host "     [PASS] Inno Setup installer compiled successfully into dist/" -ForegroundColor Green
    } else {
        Write-Warning "Inno Setup compilation failed with exit code $LASTEXITCODE"
    }
} else {
    Write-Host "  [NOTICE] Inno Setup compiler (ISCC.exe) not found on system." -ForegroundColor Yellow
    Write-Host "           To build installer manually: ISCC.exe packaging\better_jarvis.iss" -ForegroundColor Yellow
}

$wixCandidates = @("wix.exe", "candle.exe")
$wixFound = $false
foreach ($w in $wixCandidates) {
    if (Get-Command $w -ErrorAction SilentlyContinue) {
        $wixFound = $true
        break
    }
}

if ($wixFound) {
    Write-Host "  -> Compiling WiX Toolset installer..." -ForegroundColor Yellow
    & wix build packaging/better_jarvis.wxs -o "dist/Better_Jarvis-v1.0.0-win64.msi"
    if ($LASTEXITCODE -eq 0) {
        Write-Host "     [PASS] WiX MSI installer compiled successfully into dist/" -ForegroundColor Green
    } else {
        Write-Warning "WiX compilation failed with exit code $LASTEXITCODE"
    }
} else {
    Write-Host "  [NOTICE] WiX Toolset (wix.exe) not found on system." -ForegroundColor Yellow
    Write-Host "           To build MSI installer manually: wix build packaging\better_jarvis.wxs -o dist\Better_Jarvis-v1.0.0-win64.msi" -ForegroundColor Yellow
}

Write-Host "`n=================================================================" -ForegroundColor Green
Write-Host "   [SUCCESS] v1.0.0 RELEASE PACKAGING COMPLETE                   " -ForegroundColor Green
Write-Host "=================================================================" -ForegroundColor Green
