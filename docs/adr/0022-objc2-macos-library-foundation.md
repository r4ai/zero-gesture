# ADR 0022: Adopt minimal objc2 framework crates for macOS leaves

- Status: Accepted
- Date: 2026-08-07
- Reviewed base: `8e72da8`

## Context

P04a through P04b3b proved the Apple Silicon package and the macOS input,
context, and action ownership model with hand-written Core Graphics, Core
Foundation, Accessibility, AppKit, and Objective-C FFI. That implementation
kept the callback bounded, but it also duplicates framework declarations,
ownership rules, nullable-result handling, and Objective-C message signatures
that maintained bindings can encode.

The Tauri shell already depends transitively on objc2 framework crates on
macOS. P04R0 chooses the library policy and compile surface before migrating
any production leaf. This phase must not change input, action, context,
rendering, process, IPC, or Windows behavior.

## Decision

### Library policy

macOS framework access uses the maintained objc2 family when the required
symbol and ownership contract are represented correctly. Direct framework
dependencies live only under
`target.'cfg(target_os = "macos")'.dependencies`, disable default features,
and enable only features exercised by the next migration seam. P04R0 selects:

- `objc2-core-graphics`: `CGEvent`, `CGEventTypes`, and
  `CGRemoteOperation`;
- `objc2-application-services`: `AXError`, `AXUIElement`, and the parent
  `HIServices` re-export feature;
- `objc2-app-kit`: `NSApplication` and its `NSResponder` superclass; and
- `objc2-quartz-core`: `CALayer`.

`objc2-core-foundation` is not a direct dependency because no P04R0 production
code imports it. Framework feature edges supply it transitively. `libc`
remains direct because the existing macOS UDS endpoint, peer credential,
polling, and filesystem implementation still calls libc. A later phase may
change an explicit feature set when its production seam and tests require
more symbols; it must retain target scoping and disabled default features.

Tauri continues to own process bootstrap, Settings WebView lifecycle,
commands, tray integration, and packaging. It is not an input-callback,
Accessibility-query, action-posting, or native-rendering interface.

### Deep module seams

Migration replaces implementation inside the existing ownership modules:

- `hook::macos` owns Event Tap installation, callback context, run loop, and
  normalized-input queue;
- `hook::macos_context` owns prompt-free Accessibility/AppKit queries,
  timeout, identity, and cache policy;
- `executor::macos` owns tagged Core Graphics event creation and posting; and
- the later macOS renderer owner will own its AppKit/Core Animation objects.

Their existing crate-private interfaces remain the seams. Callers continue to
see normalized input, context snapshots, and bounded action/results rather
than framework objects. P04R0 adds no adapter, trait, wrapper, public symbol,
or production use site. A one-implementation trait or pass-through wrapper
would enlarge the interface without adding leverage or locality.

### Rejected alternatives

- `rdev` is rejected because a cross-platform callback abstraction does not
  expose Zero Gesture's listen-only tap ownership, source-user-data
  self-filter, synchronous fail-open disposition, or bounded callback KPI
  contract precisely enough.
- `enigo` is rejected because action simulation alone cannot share the
  process marker and ownership/order seam with the Event Tap, and it does not
  cover AX or native rendering.
- A Swift sidecar is rejected because it introduces another executable,
  signing identity, IPC protocol, lifecycle, serialization, and failure mode.
  It would violate the same-executable package and deepen no existing module.
- Keeping all hand-written FFI permanently is rejected because generated,
  reviewed bindings can remove duplicated declarations and localize
  retain/release and Objective-C type rules. A raw leaf remains acceptable
  only for a required symbol or semantic that objc2 does not represent; that
  exception requires evidence in the migration PR.

### Safety and callback invariants

Dependency adoption does not relax ADRs 0016 through 0018:

- the Event Tap callback performs the self-marker comparison, fixed field
  normalization, fixed SPSC enqueue, and atomic counters only;
- it performs no allocation, lock, blocking send, IPC/JSON, file I/O, log,
  Tauri/WebView call, OS query, object construction, retain/release, or
  autorelease-pool work;
- it remains listen-only and always returns the original event;
- overload, disable, permission loss, nullable creation, and worker loss
  remain bounded and fail open;
- context and action work remain on their existing owner workers; and
- every `unsafe` call is kept in the smallest framework leaf with its
  ownership, thread, nullability, and callback-lifetime preconditions stated
  at that call.

Objective-C main-thread-only types may be created or used only while holding
the objc2 main-thread marker. Retained framework objects do not cross an
existing numeric/message seam unless that seam explicitly owns their
lifetime. Callback function ABI and context lifetime remain compatible with
Core Graphics for the entire tap lifetime.

## Phased migration

1. **P04R0 — Foundation:** add target-scoped dependencies, this ADR, support
   checks, a representative symbol smoke, and the Apple Silicon
   compile/package gate; change no production behavior or runtime contract.
2. **P04R1 — Context split and migration:** split the existing context owner
   into `hook/macos/context/{mod,native}` and migrate its
   Accessibility/AppKit leaf to objc2. Preserve prompt-free preflight,
   timeout, identity, freshness, and Unknown failure semantics.
3. **P04R2 — Event Tap split and migration:** split the input owner into
   `hook/macos/{mod,callback,run_loop,consumer}` and migrate its Core Graphics
   leaf to objc2. Preserve callback ABI, bounded work, lifecycle, and the
   listen-only behavior proven by P04b2/P04b3b.
4. **P04R3 — Action executor split and migration:** split the executor into
   `executor/macos/{mod,native,keymap}` and migrate tagged Core Graphics event
   creation/posting to objc2. Preserve marker, ordering, bounded queue, and
   fail-open behavior.
5. **P04b3c-a — Active Input:** add active suppression, trigger replay, and
   target revalidation/activation only after the migrated input/action leaves
   retain their existing contracts.
6. **P04b3c-b — Native Overlay:** add the deferred native renderer behind its
   real owner seam using AppKit/Core Animation. Define the second concrete
   renderer behavior before introducing any shared renderer seam.
7. **P05m — Shell and permissions:** complete the macOS runtime shell,
   permission UI, and login autostart.
8. **P06m — Distribution and physical acceptance:** complete signing,
   notarization, installation lifecycle, physical-input, permission, and
   cross-application acceptance.

A UDS module split is optional later work. It is not on the critical path for
the native-leaf migrations above and does not move into P04R0-R3 implicitly.

Each behavior-changing migration phase has its own contract manifest, static
quality comparison, Apple Silicon compile/test/package evidence, and manual
TCC limitations. P04R0 adds no runtime behavior, failure mode, or interface,
so it adds no contract manifest.

## Contract and KPI record

P04R0 adds no external runtime contract and therefore adds no obligation.
The five unchanged P04 manifests remain the single source of truth:
`8 + 38 + 8 + 24 + 17 = 95`, so the inherited inventory remains `O = 95`,
`O_v = 95`, `U = 0`, `T = 95`, and `T_r = 0`. The inherited phase records
also retain `P = 0`, `D = 0`, and `F = 0`; those values describe that
inherited contract inventory only. The Cargo target-policy checks and
representative-symbol compile smoke are support checks, not external
obligations, and are not added to `O` or `T`.

The P04R0 review fixes `8e72da8` as the comparison base. The measurement scope
and analyzer remain ADR 0006's tracked Rust/TypeScript product and test
sources with `big-code-analysis-cli 2.0.0`; dependency and generated sources
are excluded.

- Production Rust behavior delta: `0`
- Production cognitive complexity delta: max `0`, sum `0`
- Production cyclomatic complexity delta: max `0`, sum `0`
- Production function delta: `0`
- Production PLOC delta: `0`
- Production unsafe-token delta: `0`

The source/unsafe claims are established by an empty
`8e72da8..P04R0` diff under `src-tauri/src`; the cognitive, cyclomatic,
function, and PLOC claims follow from that unchanged canonical product scope
and are confirmed by the PR's canonical base/head quality comparison. The new
support checks are test-only, so their test PLOC/function and complexity
increase is reported by that comparison rather than hidden.
These are historical P04R0 measurements, not ceilings that prohibit P04R1+
production migration.

Targets for every migration phase are: inherited obligation coverage 100%,
unmapped obligations `U = 0`, redundant tests `T_r = 0`, callback
allocation/blocking/lock/I/O counts `0`, production unsafe-site count
non-increasing for the migrated leaf, and no unexplained increase in maximum
or summed cognitive/cyclomatic complexity. Dependency fan-out must not extend
beyond the macOS leaf modules.

## Verification and limits

Windows-host support checks verify target scoping and disabled default
features. The existing contract checker continues to validate the inherited
five P04 manifests without a P04R0 registration.
The macOS 26 arm64 job compiles and runs representative `CGEvent`,
`AXUIElement`, `NSApplication`, and `CALayer` type references, runs Clippy for
every target, runs the existing macOS library tests, creates the ad-hoc signed
application bundle, and reruns packaged process/UDS acceptance.

The symbol smoke does not call an OS function and therefore changes no
permission or runtime state. Noninteractive CI still does not prove TCC
grants, cross-application event observation/posting, Objective-C main-thread
runtime use, or Core Animation presentation. Those remain phase-specific
physical/manual release evidence.

## Consequences

The next migration has a reviewed, minimal binding surface and can replace
raw implementation locally without changing the module interfaces. Windows
does not resolve or compile these dependencies. P04R0 adds dependency compile
cost and three support checks, but no resident process, state, thread, queue,
callback work, or runtime failure mode.
