# ADR 0026: Activate macOS suppression with revalidation-only dispatch and replay

- Status: Accepted
- Date: 2026-08-08
- Reviewed base: `01a157c`

## Context

P04b2 installed a listen-only Event Tap, P04b3a added prompt-free context
resolution, and P04b3b connected the existing input kernel to a bounded
keyboard-action executor. Those phases deliberately returned every physical
event to macOS. The kernel could produce `Suppress`, `Activate`, and `Replay`
effects, but the macOS adapter did not yet apply them.

P04b3c-a must make suppression real without moving context queries, event
creation, posting, allocation, blocking work, or shutdown joins into the Event
Tap callback. Once a trigger has been suppressed, the kernel must retain its
single terminal action-or-replay decision within the existing bounded lanes.
Passing that trigger after the callback has returned is no longer possible.

The portable effect name `Activate` is broader than the macOS operation needed
here. P04b3c-a needs to prove that the resolved target is still current before
dispatch; it does not need to bring an application or window to the foreground.

## Decision

### Suppress-capable Event Tap and callback boundary

The Event Tap changes from `ListenOnly` to the suppress-capable default option
while retaining the exact P04b2 mouse-event mask. After readiness is published,
the run-loop owner explicitly enables active input. Before that point, and
after shutdown begins, the callback is pass-through.

The callback performs only these ordered operations:

1. compare `EventSourceUserData` with the process-instance marker and pass a
   match unchanged;
2. read and normalize the supported event fields;
3. enqueue the normalized observation in the existing 64-entry SPSC queue and
   record whether that fixed-capacity reservation succeeded;
4. evaluate the already-owned kernel state using that result and the other
   fixed-capacity reservations;
5. update atomic counters; and
6. return null only for an explicit `Disposition::Suppress`, otherwise return
   the original borrowed event pointer.

An idle trigger cannot start suppression when the observation reservation
fails. Unsupported input, an absent owner, inactive input, unavailable or stale
context, disabled configuration, and every other non-suppress result return the
original event. Active sessions retain their previously reserved terminal
capacity; no new retry or unbounded fallback queue is introduced.

The callback continues to perform zero heap allocation, lock acquisition,
blocking send, IPC/JSON, file I/O, logging, OS context query, event posting,
thread creation, or Tauri/WebView call.

### Revalidation-only activation

On macOS, `ActionWork::Activate` means an asynchronous validation gate, not
foreground activation. The run-loop consumer compares the resolver completion
with the pending request id, captured target token, exact point, and the
existing 100 ms freshness limit. Only the same valid target lets the portable
kernel advance to action dispatch.

P04b3c-a does not call `NSRunningApplication` activation, raise an AX window,
write AX focus attributes, synthesize a focus click, or otherwise mutate the
foreground application. A changed, unknown, stale, mismatched, unavailable, or
unfinished target result fails the gate. The configured action is not posted,
and the kernel produces the replay for the already suppressed trigger.

### Mouse replay

Replay is a distinct work kind on the existing eight-entry executor command
and result mailboxes. It carries only the captured trigger button and down/up
points. The private Core Graphics leaf:

- maps the supported trigger to its exact down event, up event, and mouse
  button;
- creates both events before posting either;
- sets the captured point on each event;
- tags both events with the same process-instance
  `EventSourceUserData` marker used by the callback filter; and
- posts down before up at the session Event Tap.

If permission preflight fails or either Create returns null, neither event is
posted and the result is `FailedBeforeInjection`. A successful replay posts one
balanced pair. Marked replay events pass through the callback unchanged and
cannot recursively start a gesture.

### Failure and shutdown ordering

All admission remains nonblocking. Observation-queue exhaustion fails open
before a new session can suppress. Action/replay mailbox rejection or executor
loss is reported to the owner; the callback never waits for capacity or for an
OS operation.

Normal stop, readiness publication failure, and run-loop degradation use the
same teardown order:

1. disable active input so every later callback returns the original event;
2. discard ordinary work still pending in the owner and drive the active
   kernel session through the existing executor-failure/shutdown phases; this
   materializes one replay only when that state machine selects its reserved
   replay path;
3. disable and invalidate the Event Tap and remove its run-loop source;
4. detach the input owner;
5. close the executor command sender so accepted FIFO work, including the
   replay, drains in order, then wait up to the existing 100 ms join bound
   before detaching an in-flight OS call; and
6. shut down the context worker.

An action already accepted by the executor is not removed from its FIFO. If
such an in-flight command prevents replay admission, the owner degrades instead
of dispatching both action and replay. The executor does not use a stop flag
that can overtake accepted FIFO work. Shutdown does not create more than the
kernel's one terminal replay for one active session.

### Platform and phase boundary

No Windows source, callback, queue capacity, renderer, executor, or lifecycle
contract changes in P04b3c-a. Tauri remains outside native input, context,
action, replay, and rendering interfaces.

P04b3c-b Native Overlay remains deferred. Settings permission UX, shell,
autostart, distribution, trusted signing/notarization, and physical acceptance
remain P05m/P06m work.

## Contract and KPI record

P04b3c-a adds nine independently named obligations in
`contracts/p04b3c-a-macos-active-input.json`. Each maps once to one Rust test:
`O = 9`, `O_v = 9`, `U = 0`, `T = 9`, `T_r = 0`, `T_u = 9`, `T_i = 0`,
`T_e = 0`, `P = 0`, `D = 0`, and `F = 0`.

`P04B2-PASS-001` no longer asserts that the current tap is listen-only. It
retains the independently testable P04b2 compatibility boundary: the exact
mouse-event mask is preserved and a non-suppressing self event returns its
original pointer. All 95 inherited P04 obligations therefore remain
registered. With the nine new cases, the current P04 inventory is 104
obligations, all with unique evidence.

The fixed capacities remain: observation SPSC 64, owner action FIFO 16, owner
renderer FIFO 64, executor commands 8, and executor results 8. The callback KPI
remains zero allocations and zero locks, blocking sends, IPC, I/O, logging, OS
queries, event posts, thread creation, and Tauri/WebView calls. No latency,
CPU, memory, or canonical complexity number is invented locally. The
repository-wide cognitive/cyclomatic metrics remain the pinned macOS/quality CI
measurement defined by ADR 0006.

## Verification and limits

Windows-host checks validate formatting, host tests, Clippy, the contract
inventory/evidence mapping, macOS source compilation through the supported
docs.rs configuration, and a zero Windows-source diff for this phase.

The authoritative Apple Silicon job must run macOS Clippy and all library and
contract tests, then build the ad-hoc signed bundle and rerun packaged
process/UDS acceptance. The focused native tests prove callback return values,
queue-full fail-open behavior, changed-target replay selection, tagged balanced
replay creation/post ordering, nullable creation, failure classification, and
shutdown replay ordering.

Noninteractive CI does not prove live Listen Event or Post Event TCC grants,
suppression and replay delivery in another application, foreground changes
caused by another process during the gesture, physical pointer coordinates, or
external imitation of the process marker. Those remain physical/manual
acceptance gates.

## Consequences

macOS now applies the portable kernel's suppress/replay lifecycle while keeping
the callback bounded and fail-open before admission. The activation name is
narrowed at the adapter boundary to target revalidation, avoiding a new focus
mutation subsystem. A suppressed trigger has one bounded terminal path through
action completion or tagged replay, including orderly shutdown.
