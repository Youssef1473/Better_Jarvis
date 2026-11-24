<#
.SYNOPSIS
    Installs Better_Jarvis as a silent, zero-console-flash logon autostart task via Windows Task Scheduler.
.DESCRIPTION
    Registers a Task Scheduler task 'Better_Jarvis_Autostart' configured strictly at Medium Integrity Level
    (RunLevel=Limited, non-elevated) running jarvis-host.exe at user logon with zero execution time limits.
.PARAMETER InstallDir
    Target directory containing jarvis-host.exe. Defaults to parent folder of scripts/ or LocalAppData.
.PARAMETER DryRun
    Tests task registration XML generation without modifying the Windows Task Scheduler.
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [Parameter(Mandatory = $false)]
    [string]$InstallDir = "",

    [Parameter(Mandatory = $false)]
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $parentDir = Split-Path -Parent $scriptDir
    if (Test-Path "$parentDir\jarvis-host.exe") {
        $InstallDir = $parentDir
    } else {
        $InstallDir = "$env:LOCALAPPDATA\Programs\Better_Jarvis"
    }
}

$hostExe = Join-Path $InstallDir "jarvis-host.exe"
$taskName = "Better_Jarvis_Autostart"

Write-Host "=================================================" -ForegroundColor Cyan
Write-Host "   Better_Jarvis Task Scheduler Autostart Setup " -ForegroundColor Cyan
Write-Host "=================================================" -ForegroundColor Cyan
Write-Host "Target Host: $hostExe"
Write-Host "Task Name:   $taskName"
Write-Host "User Logon:  $env:USERNAME"
Write-Host "Integrity:   Medium IL (RunLevel = Limited)"

if (-not (Test-Path $hostExe) -and -not $DryRun) {
    Write-Warning "Target binary not found at $hostExe. Ensure Better_Jarvis is installed or specify -InstallDir."
}

# Construct task definition
$action = New-ScheduledTaskAction -Execute $hostExe -WorkingDirectory $InstallDir
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME

# Principals: RunLevel Limited ensures non-elevated Medium Integrity Level
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited

# Settings: ExecutionTimeLimit PT0S prevents 72-hour task kill cutoff; allow running on battery
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit (New-TimeSpan -Seconds 0) `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -MultipleInstances IgnoreNew

$taskDef = New-ScheduledTask `
    -Action $action `
    -Trigger $trigger `
    -Principal $principal `
    -Settings $settings `
    -Description "Better_Jarvis C++20/Rust Voice Assistant User Autostart Task"

if ($DryRun) {
    Write-Host "`n[DRY RUN] Generated Scheduled Task Definition successfully." -ForegroundColor Yellow
    Write-Host "Action:       $($action.Execute) in $($action.WorkingDirectory)"
    Write-Host "Trigger:      AtLogOn for $env:USERNAME"
    Write-Host "RunLevel:     $($principal.RunLevel)"
    Write-Host "TimeLimit:    $($settings.ExecutionTimeLimit)"
    Write-Host "[DRY RUN] Task Scheduler was not modified." -ForegroundColor Green
    exit 0
}

# Register the task
try {
    # Unregister any existing task first
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
    Register-ScheduledTask -TaskName $taskName -InputObject $taskDef -Force | Out-Null
    Write-Host "`n[SUCCESS] Registered '$taskName' in Windows Task Scheduler." -ForegroundColor Green
} catch {
    Write-Error "Failed to register scheduled task: $_"
}
