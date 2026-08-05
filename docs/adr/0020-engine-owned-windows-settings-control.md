# ADR 0020: Keep Windows Settings control typed and Engine-owned

- Status: Accepted
- Date: 2026-08-06

## Context

P05a fixed the Windows process lifecycle, but Settings still flattened most
command failures to strings and installed a second `WH_MOUSE_LL` hook inside
the Settings process for window capture. String errors could not distinguish a
revision conflict from an unavailable Engine, invalid input, persistence, or
filesystem/platform failure. The second hook also bypassed the resident
Engine's native-input ownership and delivered an unversioned Tauri string
event.

P05b is the second of the three Windows-first phases in ADR 0019. It must keep
the schema-v2 success path, Engine config ownership, current-user Named Pipe
authentication, and Windows input semantics. It must not add blocking work to
the callback or make Engine own a WebView.

## Decision

### Typed Settings failures

Every normal Settings command rejects with one serializable internal object:

```text
{ code, operation, message, retryable, current? }
```

`code` is one of revision-conflict, engine-unavailable,
engine-disconnected, validation-failed, request-rejected,
filesystem-failed, platform-failed, backend-failed, capture-stale, or
capture-unavailable. `operation` identifies query, prepare, commit,
enable-disable, import, export, open-config-dir, or window-capture. The UI
classifies only `code`; it never searches `message`.

Prepare-side revision/validation rejection and Commit-side token/persistence
failure remain distinguishable. A successful commit still returns the existing
`durability_warning` boolean. Import-file reads and export/open-directory work
are Settings-owned filesystem/platform boundaries; config mutation and
enable-disable remain Engine-owned operations.

On revision conflict, the Rust bridge queries the Engine for its current
observation and includes it in `current` when available. TanStack Query adopts
that observation. The draft state advances its retry revision and base without
replacing a dirty draft. A retry therefore submits the user's unchanged draft
against the observed current revision. Applied, including Import, explicitly
replaces cache, base, draft, and revision with the Engine-returned observation;
an older dirty draft cannot overwrite an imported document on the next Save.

### Engine-owned window capture

Internal protocol version 3 adds one capability and six messages:

```text
BeginWindowCapture(capture_id) -> WindowCaptureStarted(capture_id, epoch)
PollWindowCapture(capture_id, epoch) -> Pending | Captured(info)
CancelWindowCapture(capture_id, epoch) -> WindowCaptureCancelled
```

Only one capture is active. Engine assigns a monotonically increasing epoch;
`capture_id` identifies the Settings request. Every poll/cancel/result carries
both values. Replaced, cancelled, lease-expired, or shut-down epochs return a
typed stale/unavailable error and cannot update the UI.

Each Begin, Poll, and Cancel uses a short authenticated current-user pipe
session. Capture ownership is the unforgeable authenticated request plus the
`capture_id`/Engine epoch pair, not the lifetime of a transport connection.
Pending capture therefore never occupies the single pipe instance and cannot
block GetConfig, SetEnabled, or another control operation. It also cannot
accumulate the per-connection request budget. A terminal Begin failure drops
its short client session before the next attempt.

Each successful Begin or matching Poll establishes or refreshes a two-second
Engine lease. A 50 ms Engine worker sweep cancels an expired matching epoch
outside the callback, while normal dialog cleanup sends an explicit Cancel.
This preserves bounded cleanup for Settings close/disconnect without keeping a
pipe open. A stale cancel cannot invalidate a replacement. Engine shutdown
invalidates the mailbox before worker teardown. No transport, secret, endpoint,
singleton, or external API is added.

The existing Engine `WH_MOUSE_LL` callback is the only Windows capture input
source. On a real left-button down it performs one fixed state load/CAS, stores
the raw point in one atomic word, and publishes the captured phase with a
second CAS. It performs no lock, blocking send, IPC, JSON, file I/O,
allocation, logging, or OS query. A replace race, non-capture input, or already
filled mailbox returns false and normal input stays fail-open. Window lookup,
process/class/title collection, protocol encoding, and capture-id/phase logging
run later on the Engine IPC thread. Ordinary logs never include title, path,
config contents, or IPC secret.

The old Settings-owned hook, mutex handle, and `window-captured` Tauri event are
removed. The App edit route creates one route-private capture controller and
shares it with ConditionsList, PickDialog, and SelectDialog. One open therefore
issues one Begin, and Captured metadata remains available to SelectDialog.
React applies a captured value only when the effect is still active and a
production ref still names the requested id and epoch. Cleanup or replacement
therefore rejects a captured poll that finishes later. Each metadata field is
checked at the 4 KiB UTF-8 byte boundary before response encoding; overflow
returns typed `capture-backend-failed` without disconnecting the pipe.

Only Windows advertises the capture capability. macOS compiles the shared
protocol but omits the capability and returns `capture-unavailable` for
begin/poll/cancel until its native input owner is connected in a later phase.

## Verification

`contracts/p05b-windows-settings-control.json` maps seventeen independent Rust
obligations to seventeen unique tests: `O = 17`, `O_v = 17`, `U = 0`, `T = 17`,
`T_u = 10`, `T_i = 7`, and `T_e = 0`.

The integration cases drive the same crate-private native capture function
called by the production callback, the Engine atomic state, authenticated
short Named Pipe sessions, injected Win32 metadata system boundary, closed
codec, and client observation. Unit cases cover replace/stale, cancel/lease
expiry, shutdown, duplicate/overload fail-open, capture codec identity, typed
error categories, conflict current revision, Commit persistence, and
enable-disable operation identity. Regression cases cover stale cancel versus
replacement, Pending capture concurrent with GetConfig/SetEnabled, polling
beyond one connection budget, the exact/overflow metadata boundary and typed
server rejection, macOS capability omission, and repeated macOS unavailable
Begin after idle timeout. Frontend tests additionally cover code-only error
presentation, conflict cache adoption, dirty-draft preservation, dirty Import
replacement, post-cleanup stale result rejection, and one page-owned controller
carrying Captured metadata into SelectDialog. The Settings actions Storybook
interaction verifies that a retryable failure is visible and Save remains
retryable.

Windows formatting, Clippy, all unit/process/integration tests, rustdoc,
frontend checks/unit/Storybook/typecheck/build, Tauri debug, every contract,
and BCA 2.0.0 remain gates. Apple Silicon CI must continue to compile and
package the same source without claiming Windows capture execution.

## Deferrals

P05c owns installed Explorer/tray behavior, an actual physical capture click,
installed Settings close, installer/signing, upgrade/reinstall preservation,
uninstall, and distribution acceptance. P04b3c and later macOS
permission/distribution work remain after the Windows phases. Automatic
updating and any public capture/config framework are outside scope.

## Consequences

Settings failures are actionable without string parsing, conflicts preserve
user work, and window capture now follows the same Engine ownership and
fail-open callback budget as normal Windows input. The protocol adds explicit
internal cases, short authenticated requests, and one bounded lease sweep, but
no general actor/RPC framework, trait hierarchy, global coordinator, transport,
dependency, or resident WebView.
