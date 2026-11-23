$ErrorActionPreference = "Stop"

# Automatically load .env if present in workspace root
if (Test-Path ".env") {
    Get-Content ".env" | ForEach-Object {
        $line = $_.Trim()
        if ($line -and -not $line.StartsWith("#")) {
            $parts = $line.Split("=", 2)
            if ($parts.Length -eq 2) {
                $varName = $parts[0].Trim()
                $varVal = $parts[1].Trim().Trim('"').Trim("'")
                [System.Environment]::SetEnvironmentVariable($varName, $varVal, "Process")
            }
        }
    }
}

# Production supervisor mode: C++ host spawns jarvis-daemon.exe inside its Job Object
& ".\build\host-cpp\Release\jarvis-host.exe"
