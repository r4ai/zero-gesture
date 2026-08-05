# ADR 0019: Complete the Windows product before later macOS phases

- Status: Accepted
- Date: 2026-08-06

## Context

P04b3b completed the bounded macOS context and keyboard-action path. The
remaining macOS work starts with active suppression, trigger replay, native
rendering, permission UI, and distribution. None of those tasks makes the
current Windows implementation installable and dependable as a daily product.

The delivery order in ADR 0005 therefore no longer reflects the product
priority. Windows still needs a reliable resident/runtime shell, truthful
Settings control surface, and installed lifecycle before work continues on
P04b3c.

## Decision

Windows completion precedes P04b3c and later macOS permission/distribution
work, in no more than three independently reviewable phases:

1. **P05a — Windows runtime shell.** Fix login autostart, Settings-only
   single-instance behavior, Settings close, tray re-entry, and Engine Quit.
2. **P05b — Engine-owned Settings control.** Connect the UI to the typed
   Engine query/prepare/apply/import/export/enable-disable surface and display
   revision conflicts, durability warnings, and typed failures.
3. **P05c — Windows distribution acceptance.** Fix installer, signing,
   install/upgrade/reinstall preservation, uninstall, and actual installed
   machine gates. Automatic updating remains outside these phases.

Each phase uses its own branch, Draft PR, contract manifest, KPI comparison,
and exit criteria. P05b depends on P05a's stable process lifecycle. P05c
depends on P05a and P05b so the installed artifact exercises the final runtime
and UI contracts.

### P05a runtime-shell contract

The same executable remains the only artifact. Default and `--settings` start
Settings; `--engine` starts Engine. Engine keeps the existing current-user
singleton, input owner, tray, IPC endpoint, and zero content
window/WebView2 topology. Settings and Engine must coexist.

Only the Settings builder installs the maintained Tauri autostart and
single-instance plugins. Tauri-first is selected because these plugins track
the Tauri v2 application identity and Windows lifecycle, while a second
registry/mutex coordinator would duplicate platform behavior and increase the
state space. No plugin is added to Engine, its callback, or its resident
workers.

On every successful Settings setup, autostart `enable` is called for the
current executable with exactly `--engine`, then `is_enabled` verifies the
result. The Windows plugin's single current-user Run value makes repeated
setup idempotent and repairs the command for the same application identity.
Enable errors, verification errors, and a still-disabled result fail Settings
setup; none is reported as success. Debug process tests explicitly bypass the
OS registration and use the same production wrapper with deterministic fakes,
so CI never writes HKCU.

The single-instance plugin is registered first and only in Settings mode.
Consequently it cannot exclude Engine. A second Settings process forwards to
the first, exits, and schedules show/unminimize/focus on the Tauri main thread.
The Windows plugin callback arrives inside a synchronous `WM_COPYDATA`;
scheduling is initiated from a short-lived Settings activation thread so the
callback can return before the main-thread task runs. No resident Engine
thread or general process coordinator is introduced.

Closing the Settings window follows Tauri's last-window lifecycle and exits
that Settings process; it is never converted to hide or close prevention.
The Engine tray left-click and Open Settings action continue to spawn the same
executable with exactly `--settings`. Repeated requests converge through the
Settings-only plugin on one process and one window. Quit requests Engine
worker shutdown before process exit. It does not call autostart disable, so a
later login may start Engine again.

Engine/Settings log files have separate names because two process modes now
run concurrently. Both use the same application config directory.

## Verification

`contracts/p05a-windows-runtime-shell.json` maps twelve detectable obligations
to twelve unique Cargo tests: `O = 12`, `O_v = 12`, `U = 0`, `T = 12`,
`T_u = 8`, `T_i = 4`, and `T_e = 0`.

Deterministic tests cover the exact autostart argument, repeated enable,
enable/verification failures, repeated tray launch arguments, and
shutdown-before-exit ordering. Actual Windows child-process tests prove
Engine/Settings coexistence, Engine window/WebView2 zero while Settings is
alive, second-Settings exit plus existing-window reactivation, and an explicit
post-window-creation Settings exit seam plus Engine survival. The production
CloseRequested-to-exit leaf is unit-tested; a real user close gesture remains
an installed P05c acceptance gate.

The Windows gate runs formatting, lint, all Rust tests, rustdoc, the frontend,
Tauri debug build, and every contract manifest. P05a may not increase the
Engine callback budget, content-window count, or WebView2 count. BCA 2.0.0
reports base-to-head Product/Test PLOC and function, cognitive, and cyclomatic
metrics.

## Deferrals

P05b owns typed UI error and warning presentation and any remaining
Engine-owned capture/config wiring. P05c owns real HKCU login verification in
an installed bundle, Explorer/tray interaction including an actual Settings
close gesture and process/WebView2 observation, installer and uninstaller
behavior, signing, upgrade/reinstall preservation, and actual-machine
performance. P04b3c and later macOS permission/distribution work follow those
Windows phases. Automatic updating is not added.

## Consequences

Windows gains a bounded, Tauri-native runtime shell without changing the
input callback, gesture algorithm, renderer, config schema, or publication
protocol. CI proves only detectable process and wrapper behavior; it does not
claim installed-bundle or Explorer evidence before P05c.
