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
$logRoot = Split-Path -Parent $logDirectory
$configPath = Join-Path $configDirectory "zero-gesture.config.json"
$secretPath = Join-Path $configDirectory "engine-control.secret"
$sentinelPath = Join-Path $configDirectory "p05c-installer-retention.sentinel"
$unrelatedLogRootSentinelPath = Join-Path $logRoot "p05c-unrelated-runner-data.sentinel"
$unrelatedLogRootSentinelBytes = [byte[]]@(0x50, 0x05, 0x0C, 0x02, 0x55, 0xAA)
$statusPath = Join-Path $env:RUNNER_TEMP "p05c-engine-status.json"
$enabledStartupApproved = [byte[]]@(0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
$measurements = [ordered]@{}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class P05cWindowState
{
    private delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll")]
    private static extern bool IsWindow(IntPtr window);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr window);

    [DllImport("user32.dll")]
    private static extern bool IsIconic(IntPtr window);

    [DllImport("user32.dll")]
    private static extern bool ShowWindowAsync(IntPtr window, int command);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr window, StringBuilder text, int capacity);

    public static void Hide(long handle)
    {
        ShowWindowAsync(new IntPtr(handle), 0);
    }

    public static bool IsExistingVisibleWindow(long handle)
    {
        var window = new IntPtr(handle);
        return IsWindow(window) && IsWindowVisible(window);
    }

    public static bool IsMinimized(long handle)
    {
        return IsIconic(new IntPtr(handle));
    }

    public static uint VisibleTopLevelWindowCount(uint expectedProcessId)
    {
        uint count = 0;
        EnumWindows((window, _) =>
        {
            uint processId;
            GetWindowThreadProcessId(window, out processId);
            var title = new StringBuilder(256);
            GetWindowText(window, title, title.Capacity);
            if (processId == expectedProcessId &&
                IsWindowVisible(window) &&
                title.ToString() == "Zero Gesture")
            {
                count += 1;
            }
            return true;
        }, IntPtr.Zero);
        return count;
    }
}
"@

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

function Get-FileSnapshot {
    param([string]$Root)
    $snapshot = [ordered]@{}
    foreach ($file in @(Get-ChildItem -LiteralPath $Root -File -Recurse | Sort-Object FullName)) {
        $bytes = [IO.File]::ReadAllBytes($file.FullName)
        $relativePath = [IO.Path]::GetRelativePath($Root, $file.FullName).Replace("\", "/")
        $snapshot[$relativePath] = [ordered]@{
            byte_count = $bytes.Length
            sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes))
            base64 = [Convert]::ToBase64String($bytes)
        }
    }
    $snapshot
}

function Get-SnapshotEvidence {
    param([Collections.IDictionary]$Snapshot)
    $evidence = [ordered]@{}
    foreach ($relativePath in $Snapshot.Keys) {
        $evidence[$relativePath] = [ordered]@{
            byte_count = $Snapshot[$relativePath].byte_count
            sha256 = $Snapshot[$relativePath].sha256
        }
    }
    $evidence
}

function Assert-FileSnapshot {
    param(
        [string]$Root,
        [Collections.IDictionary]$Expected,
        [string]$Operation
    )
    foreach ($relativePath in $Expected.Keys) {
        $path = Join-Path $Root $relativePath
        Assert-Condition (Test-Path -LiteralPath $path -PathType Leaf) `
            "$Operation removed retained file: $relativePath"
        $bytes = [IO.File]::ReadAllBytes($path)
        Assert-Condition (([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes))) -ceq $Expected[$relativePath].sha256) `
            "$Operation changed retained file hash: $relativePath"
        Assert-Condition (([Convert]::ToBase64String($bytes)) -ceq $Expected[$relativePath].base64) `
            "$Operation changed retained file bytes: $relativePath"
    }
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

function Get-EngineDescendantSample {
    param([uint32]$EngineProcessId)
    @(
        Get-Descendants -RootProcessId $EngineProcessId |
            ForEach-Object {
                [ordered]@{
                    process_id = [uint32]$_.ProcessId
                    name = [string]$_.Name
                }
            }
    )
}

function Assert-SignedByAcceptanceIdentity {
    param([string]$Path, [string]$ExpectedThumbprint)
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    Assert-Condition ($null -ne $signature.SignerCertificate) "Authenticode signer is absent: $Path"
    Assert-Condition ($signature.SignerCertificate.Thumbprint -ieq $ExpectedThumbprint) `
        "Authenticode signer thumbprint does not match the disposable identity: $Path"
    Assert-Condition ($signature.Status -notin @("NotSigned", "HashMismatch", "NotSupportedFileFormat", "Incompatible")) `
        "Authenticode signature is absent, invalid, or unsupported: $Path ($($signature.Status))"
    [ordered]@{
        status = $signature.Status.ToString()
        status_message = $signature.StatusMessage
        chain_trusted = $signature.Status -eq "Valid"
        thumbprint = $signature.SignerCertificate.Thumbprint
        subject = $signature.SignerCertificate.Subject
    }
}

function Invoke-RejectedAcceptanceMode {
    param(
        [string]$Executable,
        [AllowNull()]
        [string]$Token,
        [string[]]$Arguments,
        [AllowNull()]
        [string]$OutputPath
    )
    if ($OutputPath) {
        Remove-Item -LiteralPath $OutputPath -Force -ErrorAction SilentlyContinue
    }
    $prior = $env:ZG_P05C_INSTALLED_ACCEPTANCE
    try {
        if ($null -eq $Token) {
            Remove-Item Env:\ZG_P05C_INSTALLED_ACCEPTANCE -ErrorAction SilentlyContinue
        } else {
            $env:ZG_P05C_INSTALLED_ACCEPTANCE = $Token
        }
        $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -PassThru -Wait
        Assert-Condition ($process.ExitCode -ne 0) "Rejected installed acceptance mode exited successfully."
        if ($OutputPath) {
            Assert-Condition (-not (Test-Path -LiteralPath $OutputPath)) `
                "Rejected installed acceptance status created an artifact."
        }
    } finally {
        $env:ZG_P05C_INSTALLED_ACCEPTANCE = $prior
    }
}

function Resolve-Uninstaller {
    param($UninstallEntry)
    $command = [string]$UninstallEntry.UninstallString
    Assert-Condition (-not [string]::IsNullOrWhiteSpace($command)) "NSIS did not register an uninstaller."
    $path = if ($command.StartsWith('"')) {
        [regex]::Match($command, '^"([^"]+)"').Groups[1].Value
    } else {
        $command.Split(" ", 2)[0]
    }
    Assert-Condition (Test-Path -LiteralPath $path -PathType Leaf) "Registered NSIS uninstaller was not found."
    $path
}

function Invoke-CancelledUninstall {
    param([string]$Uninstaller)
    $priorAcceptance = $env:ZG_P05C_INSTALLED_ACCEPTANCE
    $priorAbort = $env:ZG_P05C_ABORT_UNINSTALL
    try {
        $env:ZG_P05C_INSTALLED_ACCEPTANCE = "disposable-runner"
        $env:ZG_P05C_ABORT_UNINSTALL = "disposable-runner"
        $process = Start-Process -FilePath $Uninstaller -ArgumentList "/S" -PassThru -Wait
        $process.ExitCode
    } finally {
        $env:ZG_P05C_INSTALLED_ACCEPTANCE = $priorAcceptance
        $env:ZG_P05C_ABORT_UNINSTALL = $priorAbort
    }
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
$relativeArtifactPath = [IO.Path]::GetRelativePath($runnerRoot, $artifactFullPath)
$artifactEscapesRunner = [IO.Path]::IsPathRooted($relativeArtifactPath) -or
    $relativeArtifactPath -eq "." -or
    $relativeArtifactPath -eq ".." -or
    $relativeArtifactPath.StartsWith("..$([IO.Path]::DirectorySeparatorChar)", [StringComparison]::Ordinal)
Assert-Condition (-not $artifactEscapesRunner) `
    "Artifact path must stay below RUNNER_TEMP."
$installerFullPath = [System.IO.Path]::GetFullPath($InstallerPath)
Assert-Condition (Test-Path -LiteralPath $installerFullPath -PathType Leaf) "NSIS installer was not found."
Assert-Condition ((Get-UninstallEntry) -eq $null) "A prior Zero Gesture installation exists on the runner."
Assert-Condition (-not (Test-Path -LiteralPath $configDirectory)) "A prior Zero Gesture config directory exists on the runner."
Assert-Condition (-not (Test-Path -LiteralPath $logDirectory)) "A prior Zero Gesture log directory exists on the runner."
Assert-Condition (-not (Test-Path -LiteralPath $unrelatedLogRootSentinelPath)) `
    "The acceptance-owned unrelated-data sentinel already exists on the runner."
$runKeyExistedBeforeInstall = Test-Path -LiteralPath $runKey
Assert-Condition ($null -eq (Get-ItemProperty -LiteralPath $runKey -Name $productName -ErrorAction SilentlyContinue)) `
    "A prior Zero Gesture Run value exists on the runner."
Assert-Condition ($null -eq (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName -ErrorAction SilentlyContinue)) `
    "A prior Zero Gesture StartupApproved value exists on the runner."
New-Item -Path $startupApprovedKey -Force | Out-Null
New-Item -ItemType Directory -Path $logRoot -Force | Out-Null
[IO.File]::WriteAllBytes($unrelatedLogRootSentinelPath, $unrelatedLogRootSentinelBytes)

$expectedThumbprint = $env:P05C_CERT_THUMBPRINT
Assert-Condition (-not [string]::IsNullOrWhiteSpace($expectedThumbprint)) `
    "P05C_CERT_THUMBPRINT must identify the disposable acceptance signer."
$installerSignature = Assert-SignedByAcceptanceIdentity `
    -Path $installerFullPath `
    -ExpectedThumbprint $expectedThumbprint

$installWatch = [Diagnostics.Stopwatch]::StartNew()
Invoke-Installer -Path $installerFullPath
$installWatch.Stop()
$measurements.install_ms = $installWatch.ElapsedMilliseconds

$uninstallEntry = Get-UninstallEntry
Assert-Condition ($null -ne $uninstallEntry) "NSIS did not register the current-user installation."
$uninstaller = Resolve-Uninstaller -UninstallEntry $uninstallEntry
$installLocation = [string]$uninstallEntry.InstallLocation
Assert-Condition (-not [string]::IsNullOrWhiteSpace($installLocation)) "NSIS did not publish InstallLocation."
$installDirectory = [IO.Path]::GetFullPath($installLocation.Trim().Trim('"'))
Assert-Condition ([IO.Path]::IsPathFullyQualified($installDirectory)) `
    "NSIS InstallLocation must resolve to an absolute path."
Assert-Condition ($installDirectory -match "\s") "Installed path must include whitespace: $installDirectory"
$installedExecutable = Join-Path $installDirectory $executableName
Assert-Condition (Test-Path -LiteralPath $installedExecutable -PathType Leaf) "Installed executable was not found."
$binarySignature = Assert-SignedByAcceptanceIdentity `
    -Path $installedExecutable `
    -ExpectedThumbprint $expectedThumbprint

New-Item -ItemType Directory -Path $configDirectory -Force | Out-Null
Set-Content -LiteralPath $configPath -Encoding utf8NoBOM -NoNewline -Value '{"enabled":false}'

$startupWatch = [Diagnostics.Stopwatch]::StartNew()
$settings = Start-Settings -Executable $installedExecutable
Wait-Condition { Test-Path -LiteralPath $runKey } "Run registry key"
Wait-Condition {
    (Get-ItemProperty -LiteralPath $runKey -Name $productName -ErrorAction SilentlyContinue).$productName
} "exact autostart registration"
Wait-Condition {
    (Test-Path -LiteralPath $startupApprovedKey) -and
        $null -ne (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName -ErrorAction SilentlyContinue).$productName
} "StartupApproved registration"
Wait-Condition {
    $engines = @(
        Get-CimInstance Win32_Process |
            Where-Object {
                $_.ExecutablePath -ieq $installedExecutable -and
                    $_.CommandLine -match '(?:^|\s)--engine(?:\s|$)'
            }
    )
    if ($engines.Count -eq 1) {
        $script:engineProcessId = [uint32]$engines[0].ProcessId
        $true
    } else {
        $false
    }
} "installed Engine process"

$expectedRun = "`"$installedExecutable`" --engine"
$actualRun = (Get-ItemProperty -LiteralPath $runKey -Name $productName).$productName
Assert-Condition ($actualRun -ceq $expectedRun) "HKCU Run command is not exactly quoted."
$startupApproved = (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName).$productName
Assert-Condition ($startupApproved -is [byte[]]) "StartupApproved must be a binary registry value."
Assert-Condition (Test-BytesEqual -Actual $startupApproved -Expected $enabledStartupApproved) `
    "StartupApproved does not contain the exact enabled value."
$rejectedStatusPath = Join-Path $env:RUNNER_TEMP "p05c-rejected-status.json"
foreach ($token in @($null, "wrong-token")) {
    Invoke-RejectedAcceptanceMode `
        -Executable $installedExecutable `
        -Token $token `
        -Arguments @("--installed-acceptance-status", "`"$rejectedStatusPath`"") `
        -OutputPath $rejectedStatusPath
    Invoke-RejectedAcceptanceMode `
        -Executable $installedExecutable `
        -Token $token `
        -Arguments @("--installed-acceptance-quit") `
        -OutputPath $null
    Assert-Condition ($null -ne (Get-Process -Id $engineProcessId -ErrorAction SilentlyContinue)) `
        "Rejected installed acceptance mode changed the Engine process."
    Assert-Condition (((Get-ItemProperty -LiteralPath $runKey -Name $productName).$productName) -ceq $actualRun) `
        "Rejected installed acceptance mode changed HKCU Run."
    Assert-Condition (Test-BytesEqual `
            -Actual (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName).$productName `
            -Expected $startupApproved) `
        "Rejected installed acceptance mode changed StartupApproved."
}
$engineStatus = Invoke-AcceptanceStatus -Executable $installedExecutable
Assert-Condition ($engineStatus.process_id -eq $engineProcessId) `
    "Authenticated status did not report the observed installed Engine PID."
$startupWatch.Stop()
$measurements.engine_startup_ms = $startupWatch.ElapsedMilliseconds
$measurements.engine_startup_readiness = [ordered]@{
    condition = "authenticated_status"
    observed_pid = $engineProcessId
    authenticated_status_pid = $engineStatus.process_id
}
Assert-Condition ($engineStatus.webview_count -eq 0) "Engine reported a managed WebView."
$engineDescendantSamples = @()
for ($sample = 0; $sample -lt 3; $sample += 1) {
    $engineDescendantSamples += , @(Get-EngineDescendantSample -EngineProcessId $engineProcessId)
    Start-Sleep -Milliseconds 50
}
$engineDescendantWebViewCount = @(
    $engineDescendantSamples |
        ForEach-Object { $_ } |
        Where-Object { $_.name -ieq "msedgewebview2.exe" }
).Count
Assert-Condition ($engineDescendantWebViewCount -eq 0) `
    "Observed a real WebView2 descendant below the installed Engine PID."
Assert-Condition ($engineStatus.thread_count -le 32) "Engine thread gate exceeded: $($engineStatus.thread_count)."
Assert-Condition ($engineStatus.handle_count -le 512) "Engine handle gate exceeded: $($engineStatus.handle_count)."
Assert-Condition ($engineStatus.working_set_bytes -le 134217728) "Engine working-set gate exceeded: $($engineStatus.working_set_bytes)."
$measurements.engine = [ordered]@{
    pid = $engineStatus.process_id
    working_set_bytes = $engineStatus.working_set_bytes
    thread_count = $engineStatus.thread_count
    handle_count = $engineStatus.handle_count
    webview_count = $engineStatus.webview_count
    descendant_webview_count = $engineDescendantWebViewCount
    descendant_samples = $engineDescendantSamples
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

$settings.Refresh()
Assert-Condition ($settings.WaitForInputIdle(10000)) `
    "First Settings did not reach an idle GUI message loop."
$existingSettingsWindow = [int64]$settings.MainWindowHandle
Assert-Condition ($existingSettingsWindow -ne 0) "First Settings has no observable window to forward."
[P05cWindowState]::Hide($existingSettingsWindow)
Wait-Condition {
    -not [P05cWindowState]::IsExistingVisibleWindow($existingSettingsWindow)
} "first Settings window hide"
$second = Start-Settings -Executable $installedExecutable
Assert-Condition ($second.WaitForExit(10000)) `
    "Second Settings did not complete single-instance forwarding."
Assert-Condition ($second.ExitCode -eq 0) "Second Settings instance failed."
Assert-Condition (-not $settings.HasExited) "First Settings instance did not survive forwarding."
Wait-Condition {
    [P05cWindowState]::IsExistingVisibleWindow($existingSettingsWindow) -and
        -not [P05cWindowState]::IsMinimized($existingSettingsWindow)
} "existing Settings window show and unminimize"
$settings.Refresh()
$forwardedSettingsWindow = [int64]$settings.MainWindowHandle
Assert-Condition ($forwardedSettingsWindow -eq $existingSettingsWindow) `
    "Settings forwarding replaced the existing window."
$liveSettingsProcesses = @(
    Get-CimInstance Win32_Process |
        Where-Object {
            $_.ExecutablePath -ieq $installedExecutable -and
                $_.CommandLine -match '(?:^|\s)--settings(?:\s|$)'
        }
)
Assert-Condition ($liveSettingsProcesses.Count -eq 1) `
    "Settings forwarding left an extra installed Settings process."
Assert-Condition ([uint32]$liveSettingsProcesses[0].ProcessId -eq [uint32]$settings.Id) `
    "Settings forwarding did not preserve the original Settings process."
$visibleSettingsWindowCount = [P05cWindowState]::VisibleTopLevelWindowCount([uint32]$settings.Id)
Assert-Condition ($visibleSettingsWindowCount -eq 1) `
    "Settings forwarding left an extra visible top-level Settings window."
$settingsForwardingEvidence = [ordered]@{
    existing_process_id = $settings.Id
    second_process_id = $second.Id
    existing_window_handle = $existingSettingsWindow
    forwarded_window_handle = $forwardedSettingsWindow
    visible_settings_process_count = $liveSettingsProcesses.Count
    visible_top_level_window_count = $visibleSettingsWindowCount
    hidden_before_forward = $true
    visible_after_forward = $true
    minimized_after_forward = $false
}

$closeWatch = [Diagnostics.Stopwatch]::StartNew()
Close-Settings -Process $settings
$closeWatch.Stop()
$measurements.settings_close_ms = $closeWatch.ElapsedMilliseconds
$afterClose = Invoke-AcceptanceStatus -Executable $installedExecutable
Assert-Condition ($afterClose.process_id -eq $engineStatus.process_id) "Settings close replaced or stopped Engine."

$retainedConfig = [IO.File]::ReadAllBytes($configPath)
$runBeforeQuit = (Get-ItemProperty -LiteralPath $runKey -Name $productName).$productName
$approvedBeforeQuit = (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName).$productName
Assert-Condition (Test-Path -LiteralPath $secretPath -PathType Leaf) `
    "The running installed Engine did not own its control secret."
$cancelledUninstallExitCode = Invoke-CancelledUninstall -Uninstaller $uninstaller
Assert-Condition (Test-Path -LiteralPath $installedExecutable -PathType Leaf) `
    "Cancelled uninstall removed the installed executable."
Assert-Condition ($null -ne (Get-UninstallEntry)) `
    "Cancelled uninstall removed the package registration."
Assert-Condition ($null -ne (Get-Process -Id $engineProcessId -ErrorAction SilentlyContinue)) `
    "Cancelled uninstall stopped the running Engine."
Assert-Condition (((Get-ItemProperty -LiteralPath $runKey -Name $productName).$productName) -ceq $runBeforeQuit) `
    "Cancelled uninstall changed HKCU Run before successful uninstall."
Assert-Condition (Test-BytesEqual `
        -Actual (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName).$productName `
        -Expected $approvedBeforeQuit) `
    "Cancelled uninstall changed StartupApproved before successful uninstall."
Assert-Condition (Test-BytesEqual -Actual ([IO.File]::ReadAllBytes($configPath)) -Expected $retainedConfig) `
    "Cancelled uninstall changed retained config bytes."
$afterCancelledUninstall = Invoke-AcceptanceStatus -Executable $installedExecutable
Assert-Condition ($afterCancelledUninstall.process_id -eq $engineProcessId) `
    "Cancelled uninstall replaced the running Engine."

$quitWatch = [Diagnostics.Stopwatch]::StartNew()
Invoke-AcceptanceQuit -Executable $installedExecutable
Wait-Condition { -not (Get-Process -Id $engineStatus.process_id -ErrorAction SilentlyContinue) } "Engine worker/process quit"
Wait-Condition { -not (Test-Path -LiteralPath $secretPath) } "normal Engine secret cleanup"
$quitWatch.Stop()
$measurements.engine_quit_ms = $quitWatch.ElapsedMilliseconds
Assert-Condition (((Get-ItemProperty -LiteralPath $runKey -Name $productName).$productName) -ceq $runBeforeQuit) `
    "Quit mutated HKCU Run."
Assert-Condition (Test-BytesEqual `
        -Actual (Get-ItemProperty -LiteralPath $startupApprovedKey -Name $productName).$productName `
        -Expected $approvedBeforeQuit) `
    "Quit mutated StartupApproved."

$sentinelBytes = [byte[]]@(0x50, 0x05, 0x0C, 0x00, 0xFF, 0x5A, 0x47)
[IO.File]::WriteAllBytes($sentinelPath, $sentinelBytes)
$logsBeforeReinstall = Get-FileSnapshot -Root $logDirectory
Assert-Condition ($logsBeforeReinstall.Count -gt 0) "No stopped-Engine logs were available before reinstall."

$reinstallWatch = [Diagnostics.Stopwatch]::StartNew()
Invoke-Installer -Path $installerFullPath
$reinstallWatch.Stop()
$measurements.reinstall_ms = $reinstallWatch.ElapsedMilliseconds
Assert-Condition (Test-BytesEqual -Actual ([IO.File]::ReadAllBytes($configPath)) -Expected $retainedConfig) `
    "Reinstall changed retained config bytes."
Assert-Condition (Test-BytesEqual -Actual ([IO.File]::ReadAllBytes($sentinelPath)) -Expected $sentinelBytes) `
    "Reinstall changed the installer-unowned config sentinel."
Assert-FileSnapshot -Root $logDirectory -Expected $logsBeforeReinstall -Operation "Reinstall"
$reinstalledBinarySignature = Assert-SignedByAcceptanceIdentity `
    -Path $installedExecutable `
    -ExpectedThumbprint $expectedThumbprint

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
Wait-Condition { -not (Test-Path -LiteralPath $secretPath) } "reinstalled Engine secret cleanup"

$uninstallEntry = Get-UninstallEntry
$uninstaller = Resolve-Uninstaller -UninstallEntry $uninstallEntry
$logsBeforeUninstall = Get-FileSnapshot -Root $logDirectory
Assert-Condition ($logsBeforeUninstall.Count -gt 0) "No stopped-Engine logs were available before uninstall."
$uninstallWatch = [Diagnostics.Stopwatch]::StartNew()
$uninstall = Start-Process -FilePath $uninstaller -ArgumentList "/S" -PassThru -Wait
$uninstallWatch.Stop()
Assert-Condition ($uninstall.ExitCode -eq 0) "NSIS uninstaller exited with $($uninstall.ExitCode)."
$measurements.uninstall_ms = $uninstallWatch.ElapsedMilliseconds
Wait-Condition { -not (Test-Path -LiteralPath $installedExecutable) } "installed binary removal"
Wait-Condition { $null -eq (Get-UninstallEntry) } "uninstall registration removal"
Wait-Condition { -not (Test-Path -LiteralPath $uninstaller) } "registered uninstaller removal"
Wait-Condition { -not (Test-Path -LiteralPath $installDirectory) } "installer-owned directory removal"
Assert-Condition (Test-BytesEqual -Actual ([IO.File]::ReadAllBytes($configPath)) -Expected $retainedConfig) `
    "Uninstall changed retained config bytes."
Assert-Condition (Test-BytesEqual -Actual ([IO.File]::ReadAllBytes($sentinelPath)) -Expected $sentinelBytes) `
    "Uninstall changed the installer-unowned config sentinel."
Assert-FileSnapshot -Root $logDirectory -Expected $logsBeforeUninstall -Operation "Uninstall"
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
    engine_descendant_webview_count_max = 0
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
    authenticode = [ordered]@{
        expected_thumbprint = $expectedThumbprint
        installer = $installerSignature
        installed_executable = $binarySignature
        reinstalled_executable = $reinstalledBinarySignature
    }
    autostart_exact_quoted = $true
    run_key_existed_before_install = $runKeyExistedBeforeInstall
    startup_approved_observed = $true
    rejected_acceptance_modes_preserved_state = $true
    engine_settings_coexisted = $true
    settings_single_instance = $true
    settings_forwarded_show_and_unminimized_existing_window = $true
    settings_forwarding_evidence = $settingsForwardingEvidence
    settings_close_removed_webview_tree = $true
    settings_close_kept_engine = $true
    cancelled_uninstall_exit_code = $cancelledUninstallExitCode
    cancelled_uninstall_preserved_install_and_autostart = $true
    quit_stopped_engine = $true
    quit_preserved_autostart = $true
    clean_shutdown_removed_control_secret = $true
    uninstall_removed_autostart = $true
    uninstall_removed_package_registration = $true
    uninstall_removed_registered_uninstaller = $true
    uninstall_removed_install_directory = $true
    config_retained_after_reinstall = $true
    config_retained_after_uninstall = $true
    sentinel_retained_after_reinstall = $true
    sentinel_retained_after_uninstall = $true
    logs_retained_after_reinstall = $true
    logs_retained_after_uninstall = $true
    retained_log_evidence = [ordered]@{
        before_reinstall = (Get-SnapshotEvidence -Snapshot $logsBeforeReinstall)
        before_uninstall = (Get-SnapshotEvidence -Snapshot $logsBeforeUninstall)
    }
    kpi_gates_passed = $true
    measurements = $measurements
}

Remove-Item -LiteralPath $configDirectory -Recurse -Force
Remove-Item -LiteralPath $logDirectory -Recurse -Force
if (Test-Path -LiteralPath $logRoot) {
    $logRootChildren = @(Get-ChildItem -LiteralPath $logRoot -Force)
    if ($logRootChildren.Count -eq 0) {
        Remove-Item -LiteralPath $logRoot -Force
    }
}
Assert-Condition (Test-BytesEqual `
        -Actual ([IO.File]::ReadAllBytes($unrelatedLogRootSentinelPath)) `
        -Expected $unrelatedLogRootSentinelBytes) `
    "Disposable cleanup changed unrelated data beside the exact log directory."
$result["cleanup_preserved_unrelated_log_root_data"] = $true
$result["cleanup_evidence"] = [ordered]@{
    exact_log_directory_removed = -not (Test-Path -LiteralPath $logDirectory)
    unrelated_parent_file = [ordered]@{
        path = $unrelatedLogRootSentinelPath
        byte_count = $unrelatedLogRootSentinelBytes.Length
        sha256 = [Convert]::ToHexString(
            [Security.Cryptography.SHA256]::HashData($unrelatedLogRootSentinelBytes)
        )
    }
}

New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($artifactFullPath)) -Force | Out-Null
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $artifactFullPath -Encoding utf8NoBOM
