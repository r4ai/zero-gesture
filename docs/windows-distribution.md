# Windows distribution and installed acceptance

## Supported artifact

P05c ships one Windows installer shape: Tauri NSIS, current-user scope.
The installer is silent with `/S`, installs below `%LOCALAPPDATA%`, and does not
require elevation.
MSI, per-machine installation, automatic update, and external public control
APIs are outside this phase.

Product identity is fixed at `Zero Gesture`, bundle identifier
`dev.r4ai.zero-gesture`, executable `zero-gesture.exe`, and version `0.1.0`.
Tauri, Cargo, and npm manifests must change version together.
Downgrades are rejected.

## Retention and uninstall

Reinstall and uninstall preserve the application config directory, IPC secret
location, and local log directory.
The live IPC secret is process-owned and is removed by normal Engine shutdown;
the installer never deletes the directory that may contain it.
Only an explicit future reset or uninstall-data command may delete user data.

Uninstall removes the `Zero Gesture` values from current-user `Run` and
`StartupApproved\Run`, preventing a dangling login command after removal.
Quit is different: it stops Engine workers and the process without changing
either autostart value.

| Data | Windows location | Reinstall | Uninstall |
| --- | --- | --- | --- |
| Config and control secret | `%APPDATA%\dev.r4ai.zero-gesture` | Preserve | Preserve |
| Engine and Settings logs | `%LOCALAPPDATA%\dev.r4ai.zero-gesture\logs` | Preserve | Preserve |
| Installed program | `%LOCALAPPDATA%\Zero Gesture` | Replace in place | Remove |
| Login registration | HKCU `Run` and `StartupApproved\Run` | Preserve/reconcile | Remove |

## Disposable CI acceptance

Windows CI creates a one-day self-signed code-signing identity in the disposable
runner's current-user certificate stores, trusts it only on that runner, and
removes it in an unconditional cleanup step.
This proves that Tauri's Authenticode configuration and signed artifact path
work; it is not publisher identity or SmartScreen reputation evidence.

The CI test builds a release NSIS installer and performs:

1. silent install to a path containing a space;
2. signature verification on installer and installed executable;
3. Settings launch, Engine readiness, exact quoted HKCU Run and
   StartupApproved observation;
4. production process single-instance, WebView tree, Settings close, Engine
   survival, and authenticated Engine status/Quit;
5. same-version reinstall as the upgrade/reinstall compatibility case;
6. exact config and log retention after reinstall and uninstall;
7. uninstall of program files and autostart values followed by disposable
   runner data cleanup; and
8. a local JSON KPI artifact for startup, close, quit, working set, threads,
   handles, and WebView count.

The production acceptance control is disabled unless
`ZG_P05C_INSTALLED_ACCEPTANCE=disposable-runner` is exact.
It only reads authenticated Engine status into an explicit file or requests the
existing typed Engine shutdown.
It does not expose configuration, input, capture, or mutation operations.

## Authenticode release gate

The repository and CI workflows contain no Windows publisher certificate
secret.
A release is therefore blocked until a real code-signing identity is provisioned.
Do not commit or generate a publisher private key in the repository.

For a release, install the organization-issued code-signing certificate in the
build user's certificate store and merge its `certificateThumbprint` into
`bundle.windows`; keep `digestAlgorithm` as `sha256` and configure the
certificate authority's RFC 3161 timestamp URL.
Build with `pnpm tauri build --bundles nsis --ci`, then require
`Get-AuthenticodeSignature` to report `Valid` for both the installer and the
installed executable.
The signer subject and timestamp must match the release policy.

## Real GUI and physical-input gate

CI uses no physical mouse and cannot prove Explorer foreground behavior,
overlay fidelity, action delivery, replay, or hardware ordering.
Run the non-destructive operator harness on a Windows 11 machine with a real
installed release:

```powershell
./scripts/windows/p05c-real-gui-acceptance.ps1 `
  -InstalledExecutable "$env:LOCALAPPDATA\Zero Gesture\zero-gesture.exe" `
  -OutputPath "$env:TEMP\zero-gesture-p05c-gui.json" `
  -InputSource physical
```

Use a non-destructive binding and the Explorer/Settings targets named by the
harness.
The harness never calls `SendInput` and never labels injected input as
physical.
If an external injection tool is used, select `-InputSource injected`; that run
cannot close the physical-hardware gate.
