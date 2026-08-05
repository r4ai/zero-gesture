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
against the observed current revision. Applied continues to replace cache,
base, and draft with the Engine-returned observation.

### Engine-owned window capture

Internal protocol version 3 adds one capability and six messages:

```text
BeginWindowCapture(capture_id) -> WindowCaptureStarted(capture_id, epoch)
PollWindowCapture(capture_id, epoch) -> Pending | Captured(info)
CancelWindowCapture(capture_id, epoch) -> WindowCaptureCancelled
```

Only one capture is active. Engine assigns a monotonically increasing epoch;
`capture_id` identifies the Settings request. Every poll/cancel/result carries
both values. Replaced, cancelled, disconnected, or shut-down epochs return a
typed stale/unavailable error and cannot update the UI.

Settings keeps one authenticated capture pipe session open from begin through
poll/cancel. Closing Settings closes that session; the existing connection
cleanup invalidates its capture within the bounded pipe timeout. New begin on
the same session replaces the prior epoch. A stale cancel keeps that session
and replacement alive; only the matching successful cancel drops it. Engine
shutdown invalidates the mailbox before worker teardown. No transport, secret,
endpoint, singleton, or external API is added.

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
removed. React polls the typed command and applies a captured value only when
the effect is still active and a production ref still names the requested id
and epoch. Cleanup or replacement therefore rejects a captured poll that
finishes later. Each metadata field is checked at the 4 KiB UTF-8 byte
boundary before response encoding; overflow returns typed
`capture-backend-failed` without disconnecting the pipe.

Only Windows advertises the capture capability. macOS compiles the shared
protocol but omits the capability and returns `capture-unavailable` for
begin/poll/cancel until its native input owner is connected in a later phase.

## Verification

`contracts/p05b-windows-settings-control.json` maps fourteen independent Rust
obligations to fourteen unique tests: `O = 14`, `O_v = 14`, `U = 0`, `T = 14`,
`T_u = 10`, `T_i = 4`, and `T_e = 0`.

The integration case drives the same crate-private native capture function
called by the production callback, the Engine atomic state, authenticated
persistent Named Pipe session, injected Win32 metadata system boundary, closed
codec, and client observation. Unit cases cover replace/stale, cancel,
disconnect, shutdown, duplicate/overload fail-open, capture codec identity,
typed error categories, conflict current revision, Commit persistence, and
enable-disable operation identity. Regression cases cover stale cancel versus
replacement, the exact/overflow metadata boundary and typed server rejection,
plus macOS capability omission. Frontend unit tests additionally cover
code-only error presentation, conflict cache adoption, dirty-draft
preservation, and post-cleanup stale result rejection. The Settings actions Storybook
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
internal cases and a short-lived persistent session, but no general actor/RPC
framework, trait hierarchy, global coordinator, transport, dependency, or
resident WebView.
