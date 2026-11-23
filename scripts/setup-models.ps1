# scripts/setup-models.ps1
$ErrorActionPreference = "Stop"

$ModelDir = Join-Path $env:LOCALAPPDATA "Better_Jarvis\models"
New-Item -ItemType Directory -Force -Path $ModelDir | Out-Null

$Assets = @(
    @{
        Name   = "silero_vad.onnx"
        Url    = "https://huggingface.co/snakers4/silero-vad/resolve/master/files/silero_vad.onnx"
        MinBytes = 1500000
    },
    @{
        Name   = "ggml-base.en.bin"
        Url    = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
        MinBytes = 140000000
    },
    @{
        Name   = "en_US-ryan-medium.onnx"
        Url    = "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/ryan/medium/en_US-ryan-medium.onnx"
        MinBytes = 60000000
    },
    @{
        Name   = "en_US-ryan-medium.onnx.json"
        Url    = "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/ryan/medium/en_US-ryan-medium.onnx.json"
        MinBytes = 1000
    }
)

$ManifestPath = Join-Path $ModelDir "manifest.sha256"
$PinnedHashes = @{}
if (Test-Path $ManifestPath) {
    Get-Content $ManifestPath | ForEach-Object {
        if ($_ -match '^([0-9a-fA-F]{64})\s+\*?(.+)$') {
            $PinnedHashes[$Matches[2].Trim()] = $Matches[1].ToLowerInvariant()
        }
    }
}

$NewManifestLines = @()

foreach ($Asset in $Assets) {
    $Dest = Join-Path $ModelDir $Asset.Name
    $Tmp  = "$Dest.tmp"

    if (Test-Path $Dest) {
        $ActualHash = (Get-FileHash -Path $Dest -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($PinnedHashes.ContainsKey($Asset.Name) -and $PinnedHashes[$Asset.Name] -eq $ActualHash) {
            Write-Host "[OK] $($Asset.Name) (SHA-256 verified)" -ForegroundColor Green
            $NewManifestLines += "$ActualHash  $($Asset.Name)"
            continue
        } elseif (-not $PinnedHashes.ContainsKey($Asset.Name) -and (Get-Item $Dest).Length -ge $Asset.MinBytes) {
            Write-Host "[OK] $($Asset.Name) (present, pinning SHA-256)" -ForegroundColor Green
            $NewManifestLines += "$ActualHash  $($Asset.Name)"
            continue
        }
        Remove-Item $Dest -Force
    }

    Write-Host "[FETCH] Downloading $($Asset.Name)..." -ForegroundColor Cyan
    if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
        & curl.exe -L --retry 3 --retry-delay 2 --fail --progress-bar $Asset.Url -o $Tmp
        if ($LASTEXITCODE -ne 0) { throw "curl failed for $($Asset.Name)" }
    } else {
        $ProgressPreference = "SilentlyContinue"
        Invoke-WebRequest -Uri $Asset.Url -OutFile $Tmp -UseBasicParsing
    }

    $Size = (Get-Item $Tmp).Length
    if ($Size -lt $Asset.MinBytes) {
        Remove-Item $Tmp -Force
        throw "Downloaded $($Asset.Name) is too small ($Size bytes)"
    }

    $Hash = (Get-FileHash -Path $Tmp -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($PinnedHashes.ContainsKey($Asset.Name) -and $PinnedHashes[$Asset.Name] -ne $Hash) {
        Remove-Item $Tmp -Force
        throw "SHA-256 mismatch for $($Asset.Name): expected $($PinnedHashes[$Asset.Name]), got $Hash"
    }

    Move-Item -Path $Tmp -Destination $Dest -Force
    $NewManifestLines += "$Hash  $($Asset.Name)"
    Write-Host "[VERIFIED] $($Asset.Name) ($Hash)" -ForegroundColor Green
}

$NewManifestLines | Set-Content -Path $ManifestPath -Encoding UTF8
Write-Host "`nAll models ready in $ModelDir" -ForegroundColor Green
