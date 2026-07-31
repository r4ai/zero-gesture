# ADR 0014: Gate the macOS same-binary bundle before native adapters

- Status: Accepted
- Date: 2026-07-31

## Context

ADR 0001 requires Settings and Engine to remain two process modes of one Tauri
executable and makes a signed, notarized macOS packaging spike the gate before
adding a helper.
ADR 0003 limits macOS support to Apple Silicon arm64 on the latest stable macOS
and selects direct Developer ID distribution.
P04a is the repository phase corresponding to P05 in ADR 0005.

The Windows Engine already has its native input owner and authenticated local
control plane.
The macOS UDS, CGEventTap, Accessibility integration, action posting, and
AppKit renderer are not prerequisites for determining whether Tauri can package
and start the same executable in a windowless Engine mode.
Adding those adapters to a packaging spike would mix independently reviewable
failure boundaries.

## Decision

### One bundle, executable, and application identity

The macOS application remains the Tauri bundle `Zero Gesture.app` with
identifier `dev.r4ai.zero-gesture`.
`Contents/MacOS` contains one main executable.
That executable selects Settings for its default or `--settings` invocation and
Engine for `--engine`.
Both modes therefore inherit the same bundle version, code signature, Team ID
when Developer ID signed, and macOS privacy permission identity.

The P04a Engine builds only a native Tauri status item.
It does not create a Tauri webview window and launches Settings with
`current_exe --settings`.
Because macOS control IPC and native adapters are deferred, its status item
contains only Open Settings and Quit.
It does not present gesture enablement as functional.

This is a packaging runtime, not a second Engine architecture.
The Windows Engine path is unchanged.
No helper executable, nested login item, external API, generic platform trait,
actor, RPC dispatcher, plugin framework, updater, or automatic update path is
added.

### Supported artifact

P04a fixes its release target to `aarch64-apple-darwin`.
At the decision date, macOS 26 is the latest stable macOS generation used by
the official `macos-26` GitHub-hosted Apple Silicon runner.
The bundle records `LSMinimumSystemVersion=26.0`.
Intel macOS and Linux are outside the support matrix.
The minimum version must be reviewed when the latest stable macOS generation
changes; ADR 0003's moving release target is not amended into a permanent
macOS 26 promise.

Tauri owns `.app` and `.dmg` construction.
Hardened Runtime is enabled.
P04a adds no entitlement file: App Sandbox is not enabled, and P04a does not
call Accessibility, Input Monitoring, Screen Recording, CGEventTap, or event
posting APIs.
The later native-adapter slice must add only entitlements proven necessary by
the actual APIs and repeat signature and permission-identity validation.

### Two signing gates

Pull requests run on the official `macos-26` arm64 runner.
They build an ad-hoc signed debug `.app` with Tauri, verify the complete
signature, inspect the bundle and Mach-O target, and launch the packaged main
executable as `--engine`.
The checked-in Tauri configuration declares `app.windows: []`.
After creating the native status item, macOS Engine setup enforces that Tauri's
stable managed WebView-window inventory remains empty and fails startup on a
violation.
The window-only manager API is feature-gated behind Tauri's `unstable` feature;
that feature is not enabled, so raw window-only objects are outside this
application's compile surface.
One bounded startup loop proves that the packaged Engine survives the enforced
invariant, while repeated descendant-process inspection fails on any WebKit
process, including a startup-only process.
The release executable contains no marker-file or arbitrary-path test hook.
This evidence validates bundle and process topology only.
Ad-hoc signing is not evidence of Developer ID trust, Gatekeeper acceptance, or
notarization.

The manually dispatched `macOS Developer ID Release Gate` is the distribution
path.
It requires these GitHub Actions secret names:

- `APPLE_CERTIFICATE`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_ID`
- `APPLE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`
- `APPLE_TEAM_ID`
- `KEYCHAIN_PASSWORD`

Before its first fallible `security` command, the workflow exports the
deterministic temporary-keychain path so `always()` cleanup also covers partial
certificate-import failures.
It lets Tauri sign, notarize, and staple the arm64 `.app` and `.dmg`, requires
`codesign`, `spctl`, and `stapler` validation of the application and the unique
DMG, then uploads the DMG and a `ditto` metadata-preserving archive of the
application bundle.
The common P04a bundle tests assert signature validity and Hardened Runtime for
both ad-hoc debug and Developer ID release artifacts; they do not reinterpret
those assertions as Developer ID trust.
Only the release workflow's `spctl` and `stapler` observations cover
Gatekeeper and notarization.
It cannot run successfully when any required secret is absent.
No secret value is printed or stored in the repository.

### Reinstall and permission identity

The stable identifier keeps Tauri's application config directory outside the
bundle.
Replacing the `.app` therefore does not intentionally delete or relocate user
configuration, and P04a adds no uninstaller or data-removal behavior.
This is a design precondition, not proof that a real reinstall preserves
configuration or login registration.

Accessibility and Input Monitoring must eventually observe the Developer ID
signed main executable as the same stable permission subject before and after
reinstall.
P04a does not request those permissions and does not claim that result.

## Contract and test accounting

`contracts/p04a-macos-packaging.json` contains eight independent obligations
and eight macOS-only runnable system cases:

- `O = 8`, `O_v = 8`, `U = 0`, and `O_v / O = 100%`;
- `T = 8`, `T_u = 0`, `T_i = 0`, and `T_e = 8`;
- `T_r = 0`, `P = 0`, `D = 0`, and `F = 0`.

Each obligation has one unique test.
The existing Windows Cargo-test listing intentionally does not treat a
macOS-only test as absent evidence.
The macOS packaging job runs all eight tests against the just-built `.app`.
No source test is duplicated at another layer.
That job also runs macOS-target Clippy with all warnings denied except
`dead_code`: P04a intentionally compiles but does not connect the existing
Windows input pipeline or the deferred macOS native adapters.
Other warnings remain failures; removing the exception belongs to the slice
that connects those adapters.

## Packaging spike status and release blockers

The following facts are automated by the P04a pull-request gate:

- native Apple Silicon compilation;
- one bundle identifier and one arm64 main executable;
- Hardened Runtime and a valid ad-hoc signature;
- `app.windows: []`, runtime enforcement of an empty managed WebView-window
  inventory, packaged `--engine` survival through the bounded startup interval,
  and no WebKit descendant observed anywhere in that interval; and
- metadata-preserving archival of the validated application bundle.

The full ADR 0001 packaging spike remains open until all of these are recorded
from the intended release path and a real Apple Silicon user session:

1. Developer ID Application signing, notarization, stapling, and Gatekeeper
   acceptance;
2. login registration of the same executable with `--engine`, including user
   refusal and login restart behavior;
3. status-item interaction that launches Settings;
4. reinstall preservation of configuration and login registration; and
5. stable Accessibility and Input Monitoring permission identity.

The repository had none of the required release secret names when P04a was
implemented, so Developer ID and notarization were not run.
Failure of one of the open gates must be reproduced before proposing a helper
or second signing identity.

## Amendments and deferrals

This ADR does not amend ADR 0001, 0003, 0004, 0005, 0011, or 0013.
It implements the smallest automated portion of ADR 0001's packaging spike and
keeps the remaining gates explicit.

P04a defers macOS autostart/login registration, singleton and UDS control IPC,
configuration ownership over that IPC, CGEventTap, Accessibility/context,
CGEvent posting, native rendering, permission UI, performance acceptance, and
human-device reinstall validation.
Those deferrals do not authorize the packaging runtime to suppress input or
claim functional gesture support.

## Consequences

The same-binary design now has a reproducible Apple Silicon bundle/process
gate without adding the native input implementation early.
Developer ID and permission facts remain honestly blocked by credentials and
real-device validation.
If the release gates later disprove the same-binary design, the helper
alternative still requires the amendment and impact analysis mandated by
ADR 0001.
