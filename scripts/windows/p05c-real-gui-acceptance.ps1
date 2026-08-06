[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstalledExecutable,
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [ValidateSet("physical", "injected")]
    [string]$InputSource = "physical"
)

$ErrorActionPreference = "Stop"
$executable = [IO.Path]::GetFullPath($InstalledExecutable)
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "Installed Zero Gesture executable was not found."
}
if (Test-Path -LiteralPath $OutputPath) {
    throw "Output already exists; choose a new artifact path."
}

$signature = Get-AuthenticodeSignature -LiteralPath $executable
Write-Host "This harness does not synthesize input and does not modify or delete app data."
Write-Host "Input source recorded for this run: $InputSource"
if ($InputSource -eq "injected") {
    Write-Warning "Injected input is not evidence of a physical hardware click."
}

Start-Process explorer.exe $env:USERPROFILE | Out-Null
$settings = Start-Process -FilePath $executable -ArgumentList "--settings" -PassThru
$checks = @(
    [ordered]@{
        id = "capture-foreground-metadata"
        prompt = "In Settings capture, use the declared input source on Explorer. Confirm process/class/title match the foreground Explorer window."
    },
    [ordered]@{
        id = "overlay-ordering"
        prompt = "Perform a configured gesture in Explorer. Confirm trail points, label, completion, and disappearance preserve input order."
    },
    [ordered]@{
        id = "action-ordering"
        prompt = "Use a non-destructive configured action in Explorer. Confirm activation precedes key-down order and reverse key-up completion."
    },
    [ordered]@{
        id = "replay"
        prompt = "Perform a short/unrecognized trigger in Explorer. Confirm the original click is replayed once and no click is lost or duplicated."
    },
    [ordered]@{
        id = "fail-open"
        prompt = "While Engine is busy or Settings is opening/closing, move and click outside the gesture. Confirm ordinary input remains responsive and unstuck."
    },
    [ordered]@{
        id = "settings-close"
        prompt = "Close Settings with the real window close button. Confirm its WebView tree exits and Engine/tray remains available."
    },
    [ordered]@{
        id = "engine-quit"
        prompt = "Quit from the Engine tray. Confirm Engine exits cleanly and the configured login-start preference is unchanged."
    }
)

$results = foreach ($check in $checks) {
    Write-Host ""
    Write-Host "[$($check.id)] $($check.prompt)"
    do {
        $outcome = (Read-Host "Result (pass/fail/blocked)").Trim().ToLowerInvariant()
    } while ($outcome -notin @("pass", "fail", "blocked"))
    do {
        $evidence = (Read-Host "Evidence note").Trim()
    } while ([string]::IsNullOrWhiteSpace($evidence))
    [ordered]@{
        id = $check.id
        outcome = $outcome
        evidence = $evidence
    }
}

$artifact = [ordered]@{
    executable = $executable
    executable_version = (Get-Item -LiteralPath $executable).VersionInfo.FileVersion
    authenticode_status = $signature.Status.ToString()
    signer_subject = $signature.SignerCertificate.Subject
    declared_input_source = $InputSource
    input_was_synthesized_by_harness = $false
    physical_hardware_release_gate_closed = $false
    settings_process_id = $settings.Id
    recorded_at = [DateTimeOffset]::Now.ToString("O")
    checks = $results
}
$parent = Split-Path -Parent ([IO.Path]::GetFullPath($OutputPath))
New-Item -ItemType Directory -Path $parent -Force | Out-Null
$artifact | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutputPath -Encoding utf8NoBOM
