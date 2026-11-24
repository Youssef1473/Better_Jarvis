<#
.SYNOPSIS
    Uninstalls Better_Jarvis Task Scheduler autostart task and terminates any running instances.
.PARAMETER DryRun
    Tests task unregistration without modifying the Windows Task Scheduler.
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [Parameter(Mandatory = $false)]
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$taskName = "Better_Jarvis_Autostart"

Write-Host "=================================================" -ForegroundColor Cyan
Write-Host "   Better_Jarvis Task Scheduler Autostart Removal" -ForegroundColor Cyan
Write-Host "=================================================" -ForegroundColor Cyan

if ($DryRun) {
    Write-Host "[DRY RUN] Would terminate running jarvis-host instances and remove '$taskName'." -ForegroundColor Yellow
    exit 0
}

# 1. Gracefully stop running host process first
$procs = Get-Process jarvis-host -ErrorAction SilentlyContinue
if ($procs) {
    Write-Host "Stopping running jarvis-host process..." -ForegroundColor Yellow
    foreach ($p in $procs) {
        $p.CloseMainWindow() | Out-Null
    }
    Start-Sleep -Milliseconds 500
    # Kill if still alive
    Get-Process jarvis-host -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

# 2. Unregister task
try {
    $existing = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    if ($existing) {
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
        Write-Host "[SUCCESS] Unregistered '$taskName' from Windows Task Scheduler." -ForegroundColor Green
    } else {
        Write-Host "[INFO] Scheduled task '$taskName' was not registered." -ForegroundColor Gray
    }
} catch {
    Write-Warning "Could not unregister scheduled task '$taskName': $_"
}
