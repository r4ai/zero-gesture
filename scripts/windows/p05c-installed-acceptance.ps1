[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [Parameter(Mandatory = $true)]
    [string]$ArtifactPath
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$productName = "Zero Gesture"
$executableName = "zero-gesture.exe"
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$startupApprovedKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
$uninstallRoots = @(
    "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall",
    "HKCU:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"
)
$configDirectory = Join-Path $env:APPDATA "dev.r4ai.zero-gesture"
$logDirectory = Join-Path $env:LOCALAPPDATA "dev.r4ai.zero-gesture\logs"
$configPath = Join-Path $configDirectory "zero-gesture.config.json"
$statusPath = Join-Path $env:RUNNER_TEMP "p05c-engine-status.json"
$enabledStartupApproved = [byte[]]@(0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
$measurements = [ordered]@{}

function Assert-Condition {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        throw $Message
    }
}

function Test-BytesEqual {
    param([byte[]]$Actual, [byte[]]$Expected)
    [Convert]::ToBase64String($Actual) -ceq [Convert]::ToBase64String($Expected)
}

function Wait-Condition {
    param(
        [scriptblock]$Condition,
        [string]$Description,
        [int]$TimeoutSeconds = 10
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (& $Condition) {
            return
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Description."
}

function Invoke-Installer {
    param([string]$Path)
    $process = Start-Process -FilePath $Path -ArgumentList "/S" -PassThru -Wait
    Assert-Condition ($process.ExitCode -eq 0) "NSIS installer exited with $($process.ExitCode)."
}

function Get-UninstallEntry {
    foreach ($root in $uninstallRoots) {
        if (-not (Test-Path -LiteralPath $root)) {
            continue
        }
        foreach ($entry in Get-ChildItem -LiteralPath $root) {
            $properties = Get-ItemProperty -LiteralPath $entry.PSPath
            if ($properties.DisplayName -eq $productName) {
                return $properties
            }
        }
    }
    return $null
}

function Get-Descendants {
    param([uint32]$RootProcessId)
    $all = @(Get-CimInstance Win32_Process)
    $pending = [System.Collections.Generic.Queue[uint32]]::new()
    $pending.Enqueue($RootProcessId)
    $seen = [System.Collections.Generic.HashSet[uint32]]::new()
    [void]$seen.Add($RootProcessId)
    $result = @()
    while ($pending.Count -gt 0) {
        $parent = $pending.Dequeue()
        foreach ($child in $all.Where({ $_.ParentProcessId -eq $parent })) {
            if ($seen.Add([uint32]$child.ProcessId)) {
                $result += $child
                $pending.Enqueue([uint32]$child.ProcessId)
            }
        }
    }
    @($result)
}

function Invoke-AcceptanceStatus {
    param([string]$Executable)
    Remove-Item -LiteralPath $statusPath -Force -ErrorAction SilentlyContinue
    $prior = $env:ZG_P05C_INSTALLED_ACCEPTANCE
    try {
        $env:ZG_P05C_INSTALLED_ACCEPTANCE = "disposable-runner"
        $process = Start-Process -FilePath $Executable `
            -ArgumentList @("--installed-acceptance-status", "`"$statusPath`"") `
            -PassThru -Wait
        Assert-Condition ($process.ExitCode -eq 0) "Installed acceptance status exited with $($process.ExitCode)."
        Get-Content -LiteralPath $statusPath -Raw | ConvertFrom-Json
    } finally {
        $env:ZG_P05C_INSTALLED_ACCEPTANCE = $prior
    }
}

function Invoke-AcceptanceQuit {
    param([string]$Executable)
    $prior = $env:ZG_P05C_INSTALLED_ACCEPTANCE
    try {
        $env:ZG_P05C_INSTALLED_ACCEPTANCE = "disposable-runner"
        $process = Start-Process -FilePath $Executable `
            -ArgumentList "--installed-acceptance-quit" -PassThru -Wait
        Assert-Condition ($process.ExitCode -eq 0) "Installed acceptance quit exited with $($process.ExitCode)."
    } finally {
        $env:ZG_P05C_INSTALLED_ACCEPTANCE = $prior
    }
}

function Start-Settings {
    param([string]$Executable)
    Start-Process -FilePath $Executable -ArgumentList "--settings" -PassThru
}

function Close-Settings {
    param([System.Diagnostics.Process]$Process)
    $webviewProcessIds = @(
        Get-Descendants -RootProcessId $Process.Id
    ).Where({
        $_.Name -ieq "msedgewebview2.exe"
    }).ProcessId
    Assert-Condition ($webviewProcessIds.Count -gt 0) "Settings has no observed WebView2 process to close."
    $Process.Refresh()
    Assert-Condition $Process.CloseMainWindow() "Settings did not accept WM_CLOSE."
    Wait-Condition { $Process.HasExited } "Settings process exit"
    Wait-Condition {
        @($webviewProcessIds).Where({
            Get-Process -Id $_ -ErrorAction SilentlyContinue
        }).Count -eq 0
    } "Settings WebView2 tree exit"
}

Assert-Condition ($env:GITHUB_ACTIONS -eq "true") "Installed acceptance is restricted to a disposable GitHub Actions runner."
$runnerRoot = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP)
$artifactFullPath = [System.IO.Path]::GetFullPath($ArtifactPath)
Assert-Condition ($artifactFullPath.StartsWith($runnerRoot, [StringComparison]::OrdinalIgnoreCase)) `
    "Artifact path must stay below RUNNER_TEMP."
$installerFullPath = [System.IO.Path]::GetFullPath($InstallerPath)
Assert-Condition (Test-Path -LiteralPath $installerFullPath -PathType Leaf) "NSIS installer was not found."
Assert-Condition ((Get-UninstallEntry) -eq $null) "A prior Zero Gesture installation exists on the runner."
Assert-Condition (-not (Test-Path -LiteralPath $configDirectory)) "A prior Zero Gesture config directory exists on the runner."
Assert-Condition (-not (Test-Path -LiteralPath $logDirectory)) "A prior Zero Gesture log directory exists on the runner."
Assert-Condition ($null -eq (Get-ItemProperty -LiteralPath $runKey -Name $productName -ErrorAction SilentlyContinue)) `
    "A prior Zero Gesture Run value exists on the runner."
Assert-Condition ($null -eq (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName -ErrorAction SilentlyContinue)) `
    "A prior Zero Gesture StartupApproved value exists on the runner."
New-Item -Path $startupApprovedKey -Force | Out-Null

$installerSignature = Get-AuthenticodeSignature -LiteralPath $installerFullPath
Assert-Condition ($installerSignature.Status -eq "Valid") "The disposable self-signed installer signature is not valid."

$installWatch = [Diagnostics.Stopwatch]::StartNew()
Invoke-Installer -Path $installerFullPath
$installWatch.Stop()
$measurements.install_ms = $installWatch.ElapsedMilliseconds

$uninstallEntry = Get-UninstallEntry
Assert-Condition ($null -ne $uninstallEntry) "NSIS did not register the current-user installation."
$installDirectory = [string]$uninstallEntry.InstallLocation
Assert-Condition (-not [string]::IsNullOrWhiteSpace($installDirectory)) "NSIS did not publish InstallLocation."
Assert-Condition ($installDirectory -match "\s") "Installed path must include whitespace: $installDirectory"
$installedExecutable = Join-Path $installDirectory $executableName
Assert-Condition (Test-Path -LiteralPath $installedExecutable -PathType Leaf) "Installed executable was not found."
$binarySignature = Get-AuthenticodeSignature -LiteralPath $installedExecutable
Assert-Condition ($binarySignature.Status -eq "Valid") "The installed binary self-signature is not valid."

New-Item -ItemType Directory -Path $configDirectory -Force | Out-Null
Set-Content -LiteralPath $configPath -Encoding utf8NoBOM -NoNewline -Value '{"enabled":false}'

$startupWatch = [Diagnostics.Stopwatch]::StartNew()
$settings = Start-Settings -Executable $installedExecutable
Wait-Condition { Test-Path -LiteralPath $runKey } "Run registry key"
Wait-Condition {
    (Get-ItemProperty -LiteralPath $runKey -Name $productName -ErrorAction SilentlyContinue).$productName
} "exact autostart registration"
Wait-Condition {
    Test-Path -LiteralPath $startupApprovedKey -and
        $null -ne (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName -ErrorAction SilentlyContinue).$productName
} "StartupApproved registration"
Wait-Condition {
    try {
        $script:engineStatus = Invoke-AcceptanceStatus -Executable $installedExecutable
        $true
    } catch {
        $false
    }
} "installed Engine readiness"
$startupWatch.Stop()
$measurements.engine_startup_ms = $startupWatch.ElapsedMilliseconds

$expectedRun = "`"$installedExecutable`" --engine"
$actualRun = (Get-ItemProperty -LiteralPath $runKey -Name $productName).$productName
Assert-Condition ($actualRun -ceq $expectedRun) "HKCU Run command is not exactly quoted."
$startupApproved = (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName).$productName
Assert-Condition ($startupApproved -is [byte[]]) "StartupApproved must be a binary registry value."
Assert-Condition (Test-BytesEqual -Actual $startupApproved -Expected $enabledStartupApproved) `
    "StartupApproved does not contain the exact enabled value."
Assert-Condition ($engineStatus.webview_count -eq 0) "Engine reported a managed WebView."
Assert-Condition ($engineStatus.thread_count -le 32) "Engine thread gate exceeded: $($engineStatus.thread_count)."
Assert-Condition ($engineStatus.handle_count -le 512) "Engine handle gate exceeded: $($engineStatus.handle_count)."
Assert-Condition ($engineStatus.working_set_bytes -le 134217728) "Engine working-set gate exceeded: $($engineStatus.working_set_bytes)."
$measurements.engine = [ordered]@{
    pid = $engineStatus.process_id
    working_set_bytes = $engineStatus.working_set_bytes
    thread_count = $engineStatus.thread_count
    handle_count = $engineStatus.handle_count
    webview_count = $engineStatus.webview_count
}

Wait-Condition {
    try {
        $settings.Refresh()
        $settings.MainWindowHandle -ne 0
    } catch {
        $false
    }
} "Settings content window"
Wait-Condition {
    @(Get-Descendants -RootProcessId $settings.Id).Where({
        $_.Name -ieq "msedgewebview2.exe"
    }).Count -gt 0
} "Settings WebView2 descendant"

$settings.Refresh()
$settingsTree = @($settings) + @(Get-Descendants -RootProcessId $settings.Id).ForEach({
    Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue
})
$measurements.settings_open = [ordered]@{
    process_count = $settingsTree.Count
    working_set_bytes = ($settingsTree | Measure-Object WorkingSet64 -Sum).Sum
    thread_count = ($settingsTree | ForEach-Object { $_.Threads.Count } | Measure-Object -Sum).Sum
    handle_count = ($settingsTree | Measure-Object HandleCount -Sum).Sum
}
Assert-Condition ($measurements.settings_open.working_set_bytes -le 536870912) "Settings tree working-set gate exceeded."
Assert-Condition ($measurements.settings_open.thread_count -le 128) "Settings tree thread gate exceeded."
Assert-Condition ($measurements.settings_open.handle_count -le 2048) "Settings tree handle gate exceeded."

$second = Start-Settings -Executable $installedExecutable
Wait-Condition { $second.HasExited } "second Settings single-instance forwarding"
Assert-Condition ($second.ExitCode -eq 0) "Second Settings instance failed."
Assert-Condition (-not $settings.HasExited) "First Settings instance did not survive forwarding."

$closeWatch = [Diagnostics.Stopwatch]::StartNew()
Close-Settings -Process $settings
$closeWatch.Stop()
$measurements.settings_close_ms = $closeWatch.ElapsedMilliseconds
$afterClose = Invoke-AcceptanceStatus -Executable $installedExecutable
Assert-Condition ($afterClose.process_id -eq $engineStatus.process_id) "Settings close replaced or stopped Engine."

$retainedConfig = [IO.File]::ReadAllBytes($configPath)
$retainedLogs = @(Get-ChildItem -LiteralPath $logDirectory -File -Recurse)
Assert-Condition ($retainedLogs.Count -gt 0) "Installed release run did not produce local logs."
$runBeforeQuit = (Get-ItemProperty -LiteralPath $runKey -Name $productName).$productName
$approvedBeforeQuit = (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName).$productName
$quitWatch = [Diagnostics.Stopwatch]::StartNew()
Invoke-AcceptanceQuit -Executable $installedExecutable
Wait-Condition { -not (Get-Process -Id $engineStatus.process_id -ErrorAction SilentlyContinue) } "Engine worker/process quit"
$quitWatch.Stop()
$measurements.engine_quit_ms = $quitWatch.ElapsedMilliseconds
Assert-Condition (((Get-ItemProperty -LiteralPath $runKey -Name $productName).$productName) -ceq $runBeforeQuit) `
    "Quit mutated HKCU Run."
Assert-Condition (Test-BytesEqual `
        -Actual (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName).$productName `
        -Expected $approvedBeforeQuit) `
    "Quit mutated StartupApproved."

$reinstallWatch = [Diagnostics.Stopwatch]::StartNew()
Invoke-Installer -Path $installerFullPath
$reinstallWatch.Stop()
$measurements.reinstall_ms = $reinstallWatch.ElapsedMilliseconds
Assert-Condition (Test-BytesEqual -Actual ([IO.File]::ReadAllBytes($configPath)) -Expected $retainedConfig) `
    "Reinstall changed retained config bytes."
foreach ($log in $retainedLogs) {
    Assert-Condition (Test-Path -LiteralPath $log.FullName -PathType Leaf) "Reinstall removed a retained log."
}

$settingsAfterReinstall = Start-Settings -Executable $installedExecutable
Wait-Condition {
    try {
        $script:reinstalledStatus = Invoke-AcceptanceStatus -Executable $installedExecutable
        $reinstalledStatus.config_available
    } catch {
        $false
    }
} "reinstalled Engine config load"
Wait-Condition {
    try {
        $settingsAfterReinstall.Refresh()
        $settingsAfterReinstall.MainWindowHandle -ne 0
    } catch {
        $false
    }
} "reinstalled Settings window"
Wait-Condition {
    @(Get-Descendants -RootProcessId $settingsAfterReinstall.Id).Where({
        $_.Name -ieq "msedgewebview2.exe"
    }).Count -gt 0
} "reinstalled Settings WebView2 descendant"
Close-Settings -Process $settingsAfterReinstall
Invoke-AcceptanceQuit -Executable $installedExecutable
Wait-Condition { -not (Get-Process -Id $reinstalledStatus.process_id -ErrorAction SilentlyContinue) } "reinstalled Engine quit"

$uninstallEntry = Get-UninstallEntry
$uninstallCommand = [string]$uninstallEntry.UninstallString
Assert-Condition (-not [string]::IsNullOrWhiteSpace($uninstallCommand)) "NSIS did not register an uninstaller."
$uninstaller = if ($uninstallCommand.StartsWith('"')) {
    [regex]::Match($uninstallCommand, '^"([^"]+)"').Groups[1].Value
} else {
    $uninstallCommand.Split(" ", 2)[0]
}
Assert-Condition (Test-Path -LiteralPath $uninstaller -PathType Leaf) "Registered NSIS uninstaller was not found."
$uninstallWatch = [Diagnostics.Stopwatch]::StartNew()
$uninstall = Start-Process -FilePath $uninstaller -ArgumentList "/S" -PassThru -Wait
$uninstallWatch.Stop()
Assert-Condition ($uninstall.ExitCode -eq 0) "NSIS uninstaller exited with $($uninstall.ExitCode)."
$measurements.uninstall_ms = $uninstallWatch.ElapsedMilliseconds
Wait-Condition { -not (Test-Path -LiteralPath $installedExecutable) } "installed binary removal"
Assert-Condition (Test-BytesEqual -Actual ([IO.File]::ReadAllBytes($configPath)) -Expected $retainedConfig) `
    "Uninstall changed retained config bytes."
foreach ($log in $retainedLogs) {
    Assert-Condition (Test-Path -LiteralPath $log.FullName -PathType Leaf) "Uninstall removed a retained log."
}
Assert-Condition ($null -eq (Get-ItemProperty -LiteralPath $runKey -Name $productName -ErrorAction SilentlyContinue)) `
    "Uninstall left a dangling HKCU Run value."
Assert-Condition ($null -eq (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName -ErrorAction SilentlyContinue)) `
    "Uninstall left a dangling StartupApproved value."

$measurements.gates = [ordered]@{
    engine_startup_ms_max = 5000
    settings_close_ms_max = 10000
    engine_quit_ms_max = 3000
    engine_working_set_bytes_max = 134217728
    engine_thread_count_max = 32
    engine_handle_count_max = 512
    settings_tree_working_set_bytes_max = 536870912
    settings_tree_thread_count_max = 128
    settings_tree_handle_count_max = 2048
}
Assert-Condition ($measurements.engine_startup_ms -le $measurements.gates.engine_startup_ms_max) "Engine startup KPI gate exceeded."
Assert-Condition ($measurements.settings_close_ms -le $measurements.gates.settings_close_ms_max) "Settings close KPI gate exceeded."
Assert-Condition ($measurements.engine_quit_ms -le $measurements.gates.engine_quit_ms_max) "Engine quit KPI gate exceeded."
$result = [ordered]@{
    result = "passed"
    installer = $installerFullPath
    install_scope = "current-user"
    install_directory = $installDirectory
    signing = "disposable-self-signed"
    autostart_exact_quoted = $true
    startup_approved_observed = $true
    engine_settings_coexisted = $true
    settings_single_instance = $true
    settings_close_removed_webview_tree = $true
    settings_close_kept_engine = $true
    quit_stopped_engine = $true
    quit_preserved_autostart = $true
    uninstall_removed_autostart = $true
    config_retained_after_reinstall = $true
    config_retained_after_uninstall = $true
    logs_retained_after_reinstall = $true
    logs_retained_after_uninstall = $true
    kpi_gates_passed = $true
    measurements = $measurements
}
New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($artifactFullPath)) -Force | Out-Null
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $artifactFullPath -Encoding utf8NoBOM

Remove-Item -LiteralPath $configDirectory -Recurse -Force
$logRoot = Split-Path -Parent $logDirectory
if (Test-Path -LiteralPath $logRoot) {
    Remove-Item -LiteralPath $logRoot -Recurse -Force
}
