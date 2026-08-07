# ADR 0023: Isolate the macOS context native leaf and use objc2 ownership

- Status: Accepted
- Date: 2026-08-08
- Reviewed base: `3556f24234f25933b8698d87bceac1ffc1462e96`

## Context

The P04b3a context resolver combined worker/cache policy, deterministic seams,
raw Objective-C message dispatch, handwritten Accessibility and Core
Foundation declarations, retain/release ownership, and libc process lookup in
one file. ADR 0022 selected objc2 for maintained Apple framework bindings, but
P04R1 must migrate that implementation without moving OS queries into the
Event Tap callback or changing the existing context contract.

The migration must preserve the prompt-free trust preflight, capacity-one
latest request mailbox, bounded shutdown, 50 ms timeout before every
Accessibility read, focused-window/title/focused-window sequence, strict
type/string limits, process-start identity, freshness, and fail-open Unknown
publication.

## Decision

### Module ownership

`hook::macos::context` remains the only context owner and is private to the
macOS input owner:

- `context/mod.rs` owns requests, worker lifecycle, mailbox coalescing,
  snapshot publication/invalidation, freshness, identity policy, string
  bounds, and deterministic contract seams.
- `context/native.rs` owns all context references to AppKit,
  ApplicationServices, and Core Foundation types and functions.
- the parent `hook::macos` module sees only `ContextWorker` and numeric/domain
  values. Apple objects never cross that boundary.

No new public interface, trait, adapter hierarchy, or pass-through wrapper is
introduced. The Event Tap callback is unchanged and still performs no OS
query, allocation, retain/release, autorelease-pool work, lock, blocking send,
IPC, file I/O, or logging.

### Generated bindings and ownership

The native leaf uses:

- `NSWorkspace` and retained `NSRunningApplication` for the frontmost process;
- generated `AXUIElement` methods and `AXError` for timeout, attribute copy,
  and PID checks;
- `CFRetained`, `ConcreteType` downcasts, `CFString`, `CFEqual`, and `CFHash`
  for owned values, strict types, strings, equality, and the non-unique window
  fingerprint; and
- `objc2::rc::autoreleasepool` around one complete observation.

The old `OwnedCf`, manual autorelease pool, `objc_msgSend` signature
transmutes, selector lookup, retain/release calls, and handwritten framework
declaration blocks are removed. `libc::proc_pidinfo` and
`libc::proc_pidpath` remain because they provide the process start identity
and complete executable path required by the existing contract.

### One nullable raw leaf

The generated `AXUIElement::new_application` wrapper models
`AXUIElementCreateApplication` as non-null and panics when the framework
returns NULL. The context contract instead treats a disappearing target as a
normal fail-open failure. Therefore `native.rs` retains one typed raw
`AXUIElementCreateApplication` declaration returning
`Option<NonNull<AXUIElement>>`.

That function is the only handwritten Apple framework call in the context
module. It immediately converts a non-null Create result to
`CFRetained<AXUIElement>` and maps NULL to `ResolveFailure::TargetExited`.
This exception is narrower than catching a panic and preserves ownership at
the point where the framework transfers it.

### Query and failure contract

The trust preflight passes `None` as the options dictionary, so Engine startup
cannot request the Accessibility prompt. Each focused-window or title copy
passes through one timed-query function that sets exactly `0.05` seconds
before copying `AXFocusedWindow` or `AXTitle`.

One observation reads focused window, reads its title, reads focused window
again, and publishes only when generated `CFEqual` says both window objects
are equal. Generated Core Foundation downcasts reject an unexpected
attribute type. CF strings retain the prior 512 UTF-16-unit and 2048-byte
UTF-8 limits and require complete, non-lossy conversion.

`AXError::CannotComplete` remains Timeout. Other AX errors remain
Accessibility failures. Permission denial, timeout, malformed/oversized
data, focus change, target exit, process identity change, config
unavailability, delayed results, worker loss, and shutdown continue to
invalidate rather than reuse a stale context.

## Contract and KPI record

P04R1 adds no external behavior and no manifest. All 24 P04b3a obligations
remain mapped once (`O = 24`, `O_v = 24`, `U = 0`, `T = 24`, `T_r = 0`).
Evidence paths move only where the owning test moved to `native.rs`; the
action phase's inherited context evidence follows the move to `context/mod.rs`.

The migration-local source comparison uses fixed base `3556f242...` and the
production portions of the old `hook/macos_context.rs` versus
`hook/macos/context/{mod,native}.rs`. It counts physical/nonblank lines,
function declarations, and exact source tokens; generated dependency sources
are excluded.

| Migrated context scope | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Production physical lines | 1009 | 818 | -191 |
| Production nonblank lines | 926 | 750 | -176 |
| Production functions | 71 | 45 | -26 |
| Production `unsafe` tokens | 29 | 13 | -16 |
| Raw Apple extern blocks | 4 | 1 | -3 |
| `objc_msgSend` tokens | 4 | 0 | -4 |
| `transmute` tokens | 3 | 0 | -3 |
| Handwritten Apple function declarations | 17 | 1 | -16 |
| Test physical lines | 532 | 530 | -2 |
| Test functions | 35 | 34 | -1 |
| `#[test]` attributes in the context module | 22 | 23 | +1 |

The added test is the native AX error/nullability ownership check; obsolete
raw-message and function-pointer implementation tests are removed. Contract
tests remain at the worker/context query seam. The repository-wide canonical
cognitive, cyclomatic, function, and PLOC comparison remains the macOS CI
measurement defined by ADR 0006; this Windows implementation environment
does not have `big-code-analysis-cli 2.0.0`, so no canonical value is
fabricated here.

## Verification and limits

Windows-host checks validate dependency target scoping, disabled default
features, the unchanged contract inventory, and all host tests. A
Windows-host cross-target `cargo check` can validate the Apple Silicon Rust
and generated-binding types when the Objective-C exception helper build is
skipped as on docs.rs; it does not link an application.

The authoritative macOS 26 arm64 job must run Clippy, all library and contract
tests, symbol smoke checks, bundle packaging, and packaged process/UDS
acceptance. Noninteractive CI still cannot prove a live TCC grant,
cross-application Accessibility observation, or a target disappearing at the
exact Create boundary. Those remain physical/manual acceptance evidence.

## Consequences

The context worker/cache contract is smaller and framework ownership is
localized to one native file. Maintained generated types now enforce most
nullability, retain/release, Objective-C signature, and Core Foundation type
rules. One documented raw Create leaf remains intentionally visible instead
of converting a recoverable target exit into a panic.

P04R2 can split and migrate the Event Tap owner independently. It must not
reuse context Apple objects, move context queries into the callback, or widen
this private seam.
