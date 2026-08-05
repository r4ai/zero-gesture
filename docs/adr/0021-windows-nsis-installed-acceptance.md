# ADR 0021: Windows NSIS distribution and installed acceptance

- Status: Accepted
- Date: 2026-08-06
- Phase: P05c, Windows-first 3/3

## Context

P05a fixed the Engine/Settings runtime shell and P05b fixed typed Settings,
Engine-owned configuration, and capture control.
Their process tests used real debug binaries but deliberately bypassed the real
HKCU login registration and did not build or install a Windows bundle.
P05c must prove the same contracts from an installed release binary without
changing the callback or introducing a new resident framework.

The repository has no Windows publisher certificate reference or CI secret.
CI also has no physical mouse and cannot truthfully prove physical input,
Explorer foreground metadata, overlay fidelity, or cross-application action
delivery.

## Decision

### One current-user installer

Windows distribution uses only Tauri NSIS with `installMode: currentUser`.
It needs no elevation, installs below `%LOCALAPPDATA%`, supports `/S`, and fits
the existing per-user Engine singleton, Named Pipe, config, secret, and
autostart ownership.
MSI and per-machine installation add no required capability and are excluded.
Downgrades are refused.

Identity is `Zero Gesture`, `dev.r4ai.zero-gesture`, `zero-gesture.exe`, version
`0.1.0`.
Tauri, Cargo, and npm versions move together.
Automatic update and updater keys are not introduced.

### Retention and cleanup

NSIS replaces program files in place for reinstall.
Neither reinstall nor uninstall deletes `%APPDATA%\dev.r4ai.zero-gesture` or
`%LOCALAPPDATA%\dev.r4ai.zero-gesture\logs`.
This preserves config, migration backups, local diagnostics, and any
process-owned secret file left by an abnormal exit.
Only an explicit reset/uninstall-data operation may remove those files.

The uninstaller does remove the package-derived current-user `Run` and
`StartupApproved\Run` values.
Leaving them would create a dangling command after program removal.
Engine Quit remains unchanged and cannot mutate either value.

### Installed production acceptance seam

The same release executable accepts two internal process modes:

- `--installed-acceptance-status <artifact>` writes the authenticated typed
  Engine status to an explicit path; and
- `--installed-acceptance-quit` requests the existing authenticated typed
  shutdown.

Both fail before Tauri or IPC work unless
`ZG_P05C_INSTALLED_ACCEPTANCE=disposable-runner` is exact.
They do not expose config, capture, action, replay, or input injection.
They are an internal release-test seam, not a supported external API.
Status and Quit use the normal current-user app config path, secret, Named Pipe,
protocol, and Engine worker shutdown.

### Disposable installed scenario

Windows CI builds the actual release NSIS artifact.
On a disposable runner only, it creates and locally trusts a one-day
self-signed code-signing certificate.
The certificate proves Tauri Authenticode wiring and is removed even when the
job fails.
It does not represent a publisher identity, public trust, or SmartScreen
reputation.

One installed scenario directly verifies:

1. valid disposable signature on installer and installed executable;
2. silent current-user install and a whitespace-bearing install path;
3. exact `"absolute path" --engine` HKCU Run quoting and binary
   StartupApproved value;
4. one Engine plus one Settings, second Settings forwarding, a real Settings
   window and WebView2 tree;
5. WM_CLOSE removing Settings and its WebView tree while the same Engine PID
   remains authenticated and alive;
6. typed Quit stopping workers/process without autostart mutation;
7. same-version reinstall as the compatible upgrade/reinstall case;
8. exact config-byte and existing-log retention after reinstall and uninstall;
9. uninstall removing program files and both autostart values; and
10. disposable cleanup after the retention assertions.

The script refuses non-GitHub runners, any pre-existing Zero Gesture install,
config, or log directory, and any KPI artifact path outside `RUNNER_TEMP`.
It therefore never snapshots, overwrites, or deletes a developer's existing
installation.

### KPI and hot-path gate

The installed JSON artifact records actual release measurements.
The gates are:

| Metric | Gate |
| --- | ---: |
| Engine startup | 5,000 ms |
| Settings close/WebView cleanup | 10,000 ms |
| Engine typed Quit | 3,000 ms |
| Engine working set | 128 MiB |
| Engine threads | 32 |
| Engine handles | 512 |
| Settings plus descendants working set | 512 MiB |
| Settings plus descendants threads | 128 |
| Settings plus descendants handles | 2,048 |
| Engine managed WebViews | 0 |

P03c/P05b callback structure remains the stronger safety contract: no lock,
blocking send, IPC/JSON, file I/O, allocation, logging, OS query, WebView, or
thread creation in the synchronous decision path.
A repeated non-capture callback stress test gates bounded fail-open behavior.
P05c adds no callback state, queue, dependency, or telemetry.
Diagnostics stay local; Sentry, PostHog, and other external telemetry are
excluded.

### Signing truth

Tauri keeps SHA-256 Authenticode-ready configuration.
No release certificate is generated, requested, or stored by this phase.
Release remains blocked until an organization-issued certificate is provided
outside the repository, its thumbprint and RFC 3161 timestamp service are
configured, and both installer and installed executable verify as `Valid`.
The Draft PR and CI artifact must call the current result
`disposable-self-signed`, never release-signed.

### Manual GUI and physical input

The repository keeps a non-destructive operator harness for Explorer and
Settings.
It records capture/foreground metadata, overlay/action/replay ordering,
fail-open responsiveness, Settings close, and tray Quit as pass/fail/blocked.
The harness generates no input.
Injected input must be labeled injected and cannot satisfy the physical
hardware gate.

## Consequences

Windows-first work is complete as P05a/P05b/P05c (3/3) with an installed
release acceptance gate.
Real publisher signing and the physical/GUI checklist remain release blockers,
not fabricated automated successes.

macOS feature work, M-series distribution changes after P04b3c, automatic
updating, and external public APIs remain deferred.
The existing macOS Apple Silicon compile, test, ad-hoc signing, and packaging
jobs remain unchanged.
