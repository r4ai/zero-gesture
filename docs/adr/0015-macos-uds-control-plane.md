# ADR 0015: Secure the macOS control plane with a per-user Unix socket

- Status: Accepted
- Date: 2026-08-01

## Context

ADR 0004 selects an authenticated Unix Domain Socket for macOS and keeps the
Engine as the only configuration writer. ADR 0014 proves that the packaged
macOS application can run Engine and Settings modes from one executable
without a managed WebView in Engine mode, but deliberately leaves control IPC
unimplemented.

The Windows control implementation already has the closed protocol, bounded
framing and request count, request correlation, authentication, version
handling, configuration-owner transaction, and Settings start/connect policy.
Those rules do not depend on Named Pipes. Its transport file also contains the
Windows DACL, mutex, secret-file, pipe, and process-resource mechanics, which
must not become a generic public transport framework.

P04b1 needs only the macOS control-plane prerequisite for later native input
adapters. There is no macOS input callback in this phase.

## Decision

### Endpoint placement and objects

The production endpoint lives below
`std::env::temp_dir()/dev.r4ai.zero-gesture`. On macOS this is the current
user's temporary runtime location, not the application bundle or configuration
directory. The directory must be a real directory owned by the current
effective UID with an exact permission mode of `0700`.

The directory contains only fixed-purpose internal objects:

- the control socket;
- an Engine singleton lock;
- a bounded-start launch lock; and
- the existing application-level authentication secret.

Existing symlinks, wrong object types, wrong owners, non-exact modes, or
multiply linked secret/lock files are rejected. The Engine does not repair an
unsafe object and does not expose the endpoint after an access-control
failure. Lock and secret files are regular files with exact mode `0600`.
The socket is restricted to mode `0600` in addition to the containing
directory.

The stable configuration remains
`tauri::path::BaseDirectory::AppConfig` as resolved by
`app.path().app_config_dir()`. Runtime IPC placement does not move, duplicate,
or make Settings a writer of `zero-gesture.config.json`.

### Singleton, launch, and stale socket handling

The Engine takes a nonblocking exclusive BSD `flock` on the singleton file
before it examines or removes a stale socket. Failure to take that lock means
an Engine already owns the user endpoint, so the second Engine exits
successfully without changing the running Engine.

Only the lock owner may remove a stale socket. It first rejects a symlink,
non-socket, wrong-owner, or wrong-mode object. A safe socket left by a dead
Engine is removed before bind. The bound server records the socket device and
inode and removes the path on drop only when the path still identifies that
same owned socket. It never removes a replacement or arbitrary path.

Settings first probes the socket. Only an absent endpoint enters the existing
bounded start path. Settings serializes concurrent starts with a separate
exclusive launch lock, probes again, starts the same executable with
`--engine`, and waits only until the fixed connection deadline. Secret access,
peer credential, protocol, or other security failures are terminal and do not
spawn another Engine.

### Peer identity and application authentication

Every accepted connection is checked with the macOS/BSD `getpeereid` API
before protocol bytes are read. A peer effective UID different from the
Engine's current effective UID is disconnected and affects no other client.
Settings performs the same UID check on the connected server before reading
the secret.

The UID check supplements rather than replaces the existing random secret and
constant-time secret comparison. Protocol versioning, executable-version
policy, request IDs, duplicate detection, frame and configuration limits,
timeouts, request-count ceiling, and closed request/response types remain
identical on Windows and macOS.

There is no localhost TCP, HTTP, WebSocket, public endpoint, React socket
client, generic method registry, or raw payload dispatch.

### Shared core seam

The protocol/session lifecycle, Engine control operations, authenticated
request dispatch, configuration-owner calls, and failure classification move
once into an internal cfg-selected core. Windows Named Pipe/DACL/Win32
resource mechanics stay in the Windows leaf. Unix path/permission/flock/socket
and peer-credential mechanics stay in the macOS leaf.

The core imports one concrete platform leaf at compile time. It does not add a
transport trait, runtime polymorphism, Linux implementation, actor, RPC
framework, or public API. Platform leaves expose only the concrete endpoint,
listener/accepted stream, launch lock, secret generation, and process-status
operations required by that core.

The closed protocol adds the already-decided `SetEnabled` operation from ADR
0004. It normalizes to the same `ConfigOwner` prepare/commit path as a full
apply and does not introduce a second configuration writer or mutation state.
Apply and import continue to transfer validated configuration bytes; export is
a snapshot read followed by a Settings-owned write to the user-selected
destination.

### Engine lifecycle and failures

macOS Engine setup preserves this order:

1. validate the runtime directory and acquire the singleton;
2. create the authentication secret and bind the secured socket;
3. load the stable application configuration into `ConfigOwner`;
4. create the native status item without a WebView; and
5. start the control owner.

Endpoint or access-control failure is a startup failure before configuration
mutation or later native input installation. A malformed, slow, or
disconnected client releases only its session candidate and connection.
Configuration projection invariant failure remains fatal after durable commit,
with no rollback of committed truth.

The P04a managed-WebView inventory check remains active during setup and every
run event. Engine and Settings remain process modes of the same packaged main
executable and application identity.

No control connection, filesystem operation, allocation, lock, logging, or
retry is placed on an input callback. P04b1 has no macOS input adapter.

Logs identify the operation category and failure class. They do not contain
the authentication secret, frame or configuration body, user input, or a raw
runtime/configuration path beyond a bounded diagnostic need.

## Verification

`contracts/p04b1-macos-uds-control.json` maps each independently falsifiable
P04b1 obligation to one independently named test.

The manifest records `O = 23`, `O_v = 23`, `U = 0`, and `O_v / O = 100%`.
Its logical-case inventory is `T = 23`, `T_u = 2`, `T_i = 19`, `T_e = 2`,
`T_r = 0`, `P = 0`, `D = 0`, and `F = 0`. The process-helper test entry is a
fixture entry point and is not counted as a logical case when its helper
environment is absent.

The official `macos-26` Apple Silicon job keeps all eight P04a packaged cases
and additionally runs:

- macOS library/process tests over real Unix sockets and configuration files;
- endpoint mode, owner, symlink/object-type, stale cleanup, singleton, and
  actual `getpeereid` checks; and
- a packaged same-executable Engine/Settings connection proof.

The CI runner cannot create a second login UID without privileged test
backdoors. It therefore verifies actual peer credentials for same-UID
connections and tests the closed mismatch decision at the credential boundary.
It does not claim a privileged cross-user end-to-end case.

Windows tests remain the regression evidence for Named Pipe DACLs, remote
rejection, Windows process behavior, and the unchanged control/config
semantics.

## Deferrals

P04b1 does not implement CGEventTap, input suppression, context lookup,
Accessibility or Input Monitoring permission requests/UI, action injection,
AppKit/Core Animation rendering, login items, autostart, updater behavior, a
helper process, Developer ID credentials, notarization evidence, or final
latency/RSS acceptance.

It also does not implement the larger chunked configuration-transfer surface
from ADR 0004. The existing bounded configuration document limit and closed
prepare/commit protocol remain unchanged.

## Consequences

macOS Settings can use the same Tauri-only internal control surface and the
same Engine-owned durable configuration semantics as Windows. Compromise of a
different local UID is rejected by directory permissions and verified peer
credentials while same-UID accidental clients still need the random secret.

The shared control logic has one implementation, while each OS security
boundary remains concrete and reviewable. Later macOS input adapters can rely
on a ready resident Engine without expanding this control slice into a
framework.
