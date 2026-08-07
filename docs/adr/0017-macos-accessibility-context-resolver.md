# ADR 0017: Keep a bounded macOS context resolver idle until it has a consumer

- Status: Accepted
- Date: 2026-08-01

## Context

ADR 0016 installs a listen-only macOS Event Tap whose callback ends at a
64-entry normalized SPSC queue. P04b3a defines and compiles the concrete
process/window resolver required by the existing application and `ContextView`
contracts, but P04b3b has not connected a snapshot consumer yet. Running AX
queries before that consumer exists would be permanent dead work.

P04b3b subsequently connected that consumer under ADR 0018. Statements below
that describe the worker as dormant record the P04b3a delivery boundary; the
current production connection and its failure rules are defined by ADR 0018.

Accessibility calls may block, fail because trust is absent, race a target or
focus change, or return malformed values. A process identifier can also be
reused. None of those conditions may delay the Event Tap callback or cause
physical input to stop passing through.

## Decision

### Capability boundary

When a later consumer starts the context worker, the worker calls
`AXIsProcessTrustedWithOptions(NULL)` once. Apple's contract defines `NULL` as
no options, so this leaf never supplies `kAXTrustedCheckOptionPrompt` and
cannot request the Accessibility prompt. A denied preflight publishes Unknown
for every submitted request.

P04b3a deliberately does not start that worker from the resident Engine.
Therefore its production behavior performs no Accessibility preflight,
AppKit query, AX query, process query, or context string conversion. P04b3b
must connect the existing crate-private concrete seam and snapshot consumer in
the same change.

The explicit Settings action that may request trust, explain System Settings,
or open the relevant pane remains P05. Accessibility trust is a TCC runtime
capability, not an entitlement invented by this phase. Input Monitoring
preflight and the listen-only degraded owner remain the separate ADR 0016
boundary.

### Threading, ownership, and OS leaf

`hook/macos/context` is one concrete private macOS leaf. It adds no public
API, platform trait, generic context model, observer graph, actor runtime, or
crate. The existing public and serialized `ForegroundWindowInfo` remains its
three-field Windows shape. Bundle identity enters only the private config
matcher call.

The leaf reads `NSWorkspace.shared.frontmostApplication` and immutable
`NSRunningApplication` properties on the dedicated context worker. Apple
documents the shared workspace as safe to access from any thread and running
application properties as atomic. The worker installs its own autorelease
pool. It does not use AppKit view, window, responder, drawing, or event APIs,
so no Tauri main-thread hop is required.

The AX leaf carries only two concrete function pointers for setting the
element timeout and copying one attribute. Production binds them directly to
ApplicationServices; deterministic macOS tests bind recorders to the same
`focused_window` / `window_title` / `copy_timed_ax_attribute` path. This is a
private FFI test seam, not a backend trait or platform abstraction.

The native observation contains only:

- the basename from `proc_pidpath` for the existing Shared `process_name`
  selector; the full path is neither retained nor logged;
- bundle identifier for the macOS `bundle_identifier` selector;
- focused AX window title for the existing Shared `title` selector;
- PID plus `proc_bsdinfo` start time for opaque target identity; and
- an AX window `CFHash` retained only as a non-unique diagnostic fingerprint.

`window_class` remains Windows-only. The immutable config compiler selects
Shared plus Macos application/binding records on macOS and Shared plus Windows
records elsewhere. It lowers macOS modifier names only far enough to retain
the existing action table; P04b3a does not execute them.

Every nullable CF Create or Copy result passes through one `NonNull`
constructor before it can become owned. `CFRelease` therefore never receives
NULL.

### Timeout, consistent focus, and bounded strings

The worker creates the application AX element and sets a 50 ms element-local
`AXUIElementSetMessagingTimeout` before copying `AXFocusedWindow`. It applies
the same element-local timeout to that window before copying `AXTitle`. After
the title read it repeats the timed focused-window query and requires
`CFEqual` with the first window. A focus change becomes Unknown rather than
publishing a title and fingerprint from different windows.

`CFHash` is not used as an equality check or as input to `TargetToken`. The
leaf never changes the process-global AX timeout. `kAXErrorCannotComplete` is
classified as a timeout; every non-success result becomes Unknown.

CF strings are accepted only when their UTF-16 length is at most 512 units and
their complete UTF-8 representation fits a fixed 2,048-byte worker stack
buffer. Partial conversion, wrong Core Foundation type, invalid UTF-8, null
value, or excess length becomes Unknown. No title, bundle identifier, frame,
raw path, or input value is logged.

### Requests, coalescing, and cache

`event_tap_callback` has no resolver reference and continues to perform
exactly the ADR 0016 queue/counter work. The run-loop owner also has no
resolver reference in P04b3a: it drains and discards normalized observations,
so the resident Engine submits zero context requests. The macOS hook
bootstrap drops its `ConfigSnapshotReader` before entering that owner instead
of retaining a publication reader which this phase cannot consume.

The dormant P04b3b seam limits mouse movement requests to one per 25 ms.
Button-down may request immediately, but never waits for a result. Button-up,
wheel, and other normalized events do not request context. One fixed atomic
latest-request slot and a capacity-one wake channel coalesce overload to the
newest point/timestamp while a prior query is in flight. The worker never
builds a request backlog.

The worker publishes one complete numeric latest snapshot containing the
existing `ContextView` fields and its private process/fingerprint facts. The
same production publisher handles both success and error; an error publishes
Unknown and invalidates the preceding value. Consumers accept only an exact
point, at most 100 ms of wrapping-tick age, and one complete publication.

Publication for the same PID with a different process start time replaces the
old identity and produces a different opaque `TargetToken`, so a reused PID
cannot silently inherit prior context. The window fingerprint does not affect
that token and is not claimed to be unique.

The current P04b3a owner neither creates the worker nor feeds a snapshot into
`InputKernel`. P04b2 remains listen-only and every physical event passes. A
later suppression phase must resolve and verify the target again before
activation; this phase does not claim that a cached numeric token identifies
or activates a unique window.

### Lifecycle and failures

The dormant worker publishes thread readiness before entering capability FFI.
If the readiness receiver times out, startup sets stop and detaches rather
than synchronously joining; the delayed thread checks stop immediately after
publishing readiness and exits without an OS query. It therefore self-cleans
its reader, channels, and snapshot without retaining an uninterruptible query.

After capability preflight completes, the worker observes a capacity-one wake
and a 10 ms stop poll. Shutdown sets its atomic stop, wakes it, and joins after
any in-flight AX call is bounded by the 50 ms messaging timeout. A worker-exit
guard invalidates the latest snapshot, including unwinding. P04b3a's resident
Engine never starts this lifecycle.

Preflight completion is one atomic lifecycle fact. Shutdown invalidates the
snapshot immediately and joins only after that fact is published. If
capability preflight is still blocked, shutdown detaches the thread so Engine
teardown remains bounded; after the capability call returns, the worker sees
stop and releases its reader, channels, and snapshot itself. User-space code
cannot kill a thread blocked inside an OS FFI call, so a permanently blocked
call can retain those thread-owned resources until the OS call returns. This
limit is acceptable here because P04b3a never starts the worker in the
resident Engine.

Permission denial, AX error or timeout, focus/target switch or exit,
configuration unavailability, malformed/oversized string, stale cache,
request overload, or worker lag all degrade to Unknown. There is no retry loop
beyond a later coalesced observation. KPI aggregation and its single
category-only stop log run only on the dormant worker.

## Verification

`contracts/p04b3a-macos-context-resolver.json` maps twenty-four independently
falsifiable obligations to twenty-four uniquely named tests:
`O = 24`, `O_v = 24`, `U = 0`, `T = 24`, `T_u = 24`, `T_i = 0`, `T_e = 0`,
`T_r = 0`, `P = 0`, `D = 0`, and `F = 0`.

The Apple Silicon macOS CI job compiles every target with Clippy, runs the
library tests without requiring Accessibility trust, builds the ad-hoc signed
application, and reruns the packaged same-executable/UDS gates. Deterministic
tests cover no-prompt options, bootstrap/owner idle separation, capacity-one
coalescing, accepted and rejected request kinds, stateful production-worker
failure publication, nullable ownership, readiness and bounded shutdown around
blocked preflight, exact production-leaf timed AX query order, focus-switch
rejection, bounded full executable basename, PID reuse, non-unique fingerprint
semantics, freshness, private macOS matching, unchanged Windows serialization,
and worker stop.

The non-interactive runner cannot grant or revoke Accessibility trust and does
not inject a real AX timeout, switch a real focused GUI window during a query,
or kill a real foreground GUI process. The thin FFI leaf is compiled; its
actual invalid-PID `proc_pidinfo`, current-executable `proc_pidpath`, nullable
ownership, and deterministic private query-spec paths are exercised on macOS.
Permission-dependent success remains manual integration evidence.

## Deferrals

P04b3a does not add:

- a Settings permission request, prompt, explanation, or System Settings link;
- event suppression, replay, action injection, target activation, or a
  self-generated-event marker;
- worker startup/request submission and `InputKernel` consumption of macOS
  context until P04b3b connects both sides;
- renderer, status UI, AX notification observer, or app/window activation;
- login items, updater changes, Developer ID credentials, or notarization
  evidence; or
- final P06 latency/RSS acceptance.

## Consequences

macOS now has a compiled, bounded, fail-open context worker/cache seam that is
intentionally idle. The callback budget, resident resource budget, public
Windows payload, and Windows native owner remain unchanged. P04b3b can connect
the crate-private concrete seam when it also supplies the first real snapshot
consumer.
