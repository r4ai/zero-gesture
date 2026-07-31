# ADR 0012: Make Engine the config owner and publish through two fixed slots

- Status: Accepted
- Date: 2026-07-30

## Context

ADR 0004 assigns configuration mutation to Engine and fixes the durable
Prepare/Commit/Applied ordering.
ADR 0009 provides strict decode, migration, validation, immutable compile, and
same-directory atomic replacement.
ADR 0011 provides the authenticated, versioned, bounded Windows IPC owner but
temporarily leaves Settings writing configuration in its own process.

P03b must remove that second writer and prove the publication primitive needed
by P03c without wiring the Win32 hook or introducing a general actor, RCU, RPC,
or transaction framework.

## Decision

### Ownership and interface

`ConfigOwner` is moved into the existing single IPC owner thread.
It alone owns the active document, revision, generation, one optional prepared
candidate, persistence, and the writer side of publication.
Settings `get_config`, `update_config`, and `import_config`, plus the Engine
tray enable toggle, use `EngineControl`; none writes the active file or a live
config cache.
Export still writes the user-selected export destination after reading the
current document from Engine and is not an active-config writer.

The protocol body remains the closed binary envelope from ADR 0011 and advances
to version 2.
It adds only:

```text
GetConfig
PrepareConfig(expected_revision, bounded_config_bytes)
CommitConfig(token, base_revision, base_generation)

Config(revision, generation, optional_bounded_config_bytes)
Prepared(token, base_revision, base_generation)
Applied(revision, generation, durability_warning)
```

The capability bitmap has explicit config-read and config-transaction bits.
Config errors use a closed enum for payload-too-large, busy,
revision-conflict, validation, token, missing-candidate, generation-exhaustion,
and persistence failures. Unavailable startup is data in the typed Config
observation, not an error code.
An executable-version mismatch may read status and config but may not prepare
or commit.
The existing eight-request connection capacity is unchanged.
Settings retains the revision returned by `GetConfig` with its edit or import
draft and sends that revision to Prepare. It does not fetch a new revision
immediately before Prepare. An unavailable startup is therefore represented
as the typed `Config(0, 0, none)` observation and a valid full document can
repair it with expected revision zero.

P03b permits at most 512 KiB of decoded config bytes in a request or response;
the existing complete encoded-frame ceiling remains strictly below 1 MiB.
The codec checks the inner byte length before copying it.
This is the deliberately narrow P03b editing surface rather than ADR 0004's
future 64 MiB chunk-transfer surface.
An already persisted valid document above this edit bound can still be loaded
by Engine, but IPC read/mutation returns `ConfigPayloadTooLarge`; Settings does
not fall back to direct file mutation.
Chunked lossless recovery/export remains deferred and this bound may change
only through another ADR.

### Prepare, Commit, and Applied

Revisions and generations are process-epoch monotonic values.
A valid startup snapshot begins at revision/generation 1; an unavailable
startup begins at 0/0 and can be repaired by a successful full candidate.
Generation is encoded with the slot index in one `AtomicU64`, so the maximum is
`2^63 - 1`.
Preparing beyond that value fails with `ConfigGenerationExhausted`; values
never wrap.

Prepare performs these steps without changing disk or the active publication:

1. expire the prior candidate, then reject if one still exists;
2. check the 512 KiB payload bound and exact expected revision;
3. decode or explicitly migrate, validate, and compile once;
4. reserve and populate the inactive publication slot only when its reader
   count is zero;
5. return a non-zero opaque process-local token and the exact base
   revision/generation.

The candidate is bound to the authenticated connection.
There is exactly one candidate Engine-wide.
A connection close, its existing 750 ms read timeout, or a fixed two-second
candidate deadline aborts it and clears the inactive slot.
There is no retry task, timer thread, rollback message, or candidate table.
Wrong connection, token, base revision, or base generation returns
`ConfigTokenMismatch` without consuming the valid candidate.
A completed token deterministically returns the same error when replayed.

Commit first verifies that exact tuple, then serializes to an app-owned
same-directory temporary file, writes and flushes the bytes, and performs the
platform atomic replacement.
The inactive publication slot was already reserved by Prepare.
Temporary-create, write, flush, or replacement failure deletes the safe
temporary file when possible, clears the candidate slot, and leaves disk and
active publication unchanged.
Replacement is the logical commit point.
On Windows the temporary file data is flushed, then
`MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)` performs the same-directory
replacement. This codebase has no locally proven Windows directory-handle
flush that guarantees metadata durability, so it does not make that claim:
directory metadata durability is unconfirmed and Applied carries a typed
warning. Other platforms attempt a directory `sync_all`; failure has the same
warning behavior.
After replacement, publication is one non-fallible sequentially consistent
store of the combined generation/index state, followed by owner field
replacement.
There is no rollback after replacement.

### Two fixed publication slots

The publication module contains exactly two `UnsafeCell<Option<T>>` slots, two
atomic reader counters, and one atomic combined generation/index.
The sole writer populates only the inactive slot after its counter is zero.
A reader:

1. sequentially-consistent-loads combined generation/index;
2. sequentially-consistent-increments that slot counter, failing open instead
   of wrapping at `usize::MAX`;
3. sequentially-consistent-loads and compares the combined state again;
4. forms a reference only when both observations match, otherwise decrements
   and retries.

The state loads/store and reader-counter operations participate in one
sequentially consistent order. If a writer observes zero before a delayed
reader increments, the intervening publication precedes that reader's second
state load, so it rejects the selected slot. If the increment precedes the
writer's zero check, the writer cannot reuse the slot. Because generation
never wraps, the encoded state cannot recur.
A writer can therefore never overwrite a slot referenced by a validated
reader.
The guard path performs atomic loads/increments only: no lock, allocation,
refcount clone, IPC, logging, filesystem access, or OS query.
The small unsafe implementation is local to this module and documents both
the writer and reader invariants.

P03b publishes the compiled snapshot and proves concurrent use, but the
existing Windows hook and `InputKernel` do not consume the reader yet.
Until P03c, the Engine's Applied callback preserves existing live behavior by
stopping and recreating the current bounded hook/overlay worker pair from the
exact committed `ActiveConfig`; this is a projection from ConfigOwner and is
not another config or disk writer. Direct snapshot consumption and
old-gesture generation release remain P03c.

This compatibility projection is fallible and runs after durable replacement
and owner publication but before the Applied response is built. Runtime lock
poison, a worker stop panic, or replacement thread spawn failure is a typed
fatal projection error. The server first clears the committed session's
candidate and disconnects it, then returns the fatal error to the top-level
Engine lifecycle. That lifecycle attempts to stop and join the runtime and
exits the whole Engine nonzero. It never rolls back the already committed
file or publication; the next bounded Engine start reloads that truth.

Tray reconciliation is deliberately outside the worker projection. The IPC
owner only enqueues one label task with Tauri's non-waiting main-thread
scheduler and never calls the synchronous menu API. A tray-originated toggle
also reconciles from the returned observation after its synchronous IPC call
has completed on the main thread. Settings-originated commits use the queued
task. This is one local UI scheduling seam, not an event or callback framework.

### Session and Settings behavior

The first ACL-protected pipe instance is reused across authenticated sessions,
so there is no absent-endpoint window between clients. A connection read,
decode, or response-write failure disconnects only that session, clears its
candidate, and returns to accept. Prepared listener/owner invariant failures
and the post-commit live-projection failure above terminate Engine. Thus
losing the Applied response cannot undo the already replaced file or published
owner truth.

Settings probes the pipe before reading the secret. A missing pipe is the only
condition that enters bounded launch-lock startup. Once the pipe exists,
secret ACL, sharing, read, or shape failures are terminal security/I/O errors
and never spawn or retry another Engine.

After Applied, the Tauri adapter returns the exact config observation and
durability-warning bit. The frontend replaces its TanStack Query entry with
that observation, so the next edit uses the applied revision, and presents the
Windows durability warning. Tray changes use the revision they observed.

### Startup and recovery

P03a ordering remains mandatory: singleton, secret file flush, and the first
ACL-protected pipe bind complete before orphan cleanup, config
load/migration/write, or input worker startup.
After that readiness point ConfigOwner removes only regular UTF-8 names exactly
matching `.zero-gesture.config.<u32>.<u32>.tmp` from the unchanged application
config directory.
It then loads, explicitly migrates, validates, and compiles the unchanged
`zero-gesture.config.json` path.
Other files and near-matching names are untouched.
Cleanup or load failure starts the owner at unavailable 0/0 and keeps input
fail-open.
The stable Tauri identifier and normal reinstall/update retention are
unchanged.

## Evidence and complexity

`contracts/p03b-config-owner-rcu.json` maps each independent P03b obligation to
one runnable Rust case.
The manifest maintains `O = 45`, `O_v = 45`, `U = 0`, no evidence reused
within or across phase manifests, and no source-constant claim as runtime
evidence.
The fixed-slot evidence includes a deterministic delayed-reader/reused-slot
interleaving, counter-exhaustion behavior, and repeated writer/reader stress.
Fault tests independently cover temporary create, write, flush, replace,
post-replace metadata durability warning behavior, fatal worker projection,
and restart from committed truth. The real `ThreadRuntime` test starts,
reconfigures, and stops the existing Hook/Overlay worker pair; separate process
tests cover poisoned state, replacement-spawn failure, whole-Engine
termination, no rollback, and candidate-free restart.
The current P02 manifest has 52 independent obligations: two owner-recovery
claims formerly counted there are now exclusively P03b evidence. The P03
manifest keeps 45 independent obligations; inherited first-pipe readiness and
absent-endpoint bounded startup are not recounted in P03b.

No dependency, generic transport trait, actor runtime, transaction framework,
method registry, event string, retry queue, or background task is added.

## Deferrals

P03b does not wire the Windows hook, `InputKernel`, Context, Renderer, or
Executor owners; add macOS transport or adapters; redesign Settings; add
updater/distribution behavior; implement the larger chunk-transfer surface; or
claim final latency/RSS acceptance.

## Consequences

There is one live config writer and one small interface for all existing
mutation callers.
Prepare is pure with respect to active state and disk, persistence failure is
simple before replacement, and owner publication is non-fallible afterward.
The publication implementation pays for one bounded reader counter operation
instead of allocation or reference-count traffic in the eventual callback.
The temporary 512 KiB edit bound is a known narrower capability until the
lossless transfer phase is implemented.

## P03c implementation note

ADR 0013 wires the actual Windows owner to this reader protocol and replaces
the temporary Hook/Overlay compatibility recreation. The publication,
Prepare/Commit/Applied ordering, post-commit no-rollback rule, and fatal owner
observation remain unchanged; config replacement no longer restarts the hook.
