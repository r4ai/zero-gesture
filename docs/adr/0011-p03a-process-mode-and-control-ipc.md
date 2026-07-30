# ADR 0011: Fix P03a process bootstrap and authenticated Windows control IPC

- Status: Accepted
- Date: 2026-07-30

## Context

ADR 0001 requires one executable with separate Engine and Settings process
modes.
ADR 0004 fixes the local transport, current-user access control, frame ceiling,
and eventual config-owner protocol.
P03a needs only the smallest control seam required to start, identify, observe,
and explicitly stop the Engine.
Config transfer, config ownership, capture, and native owner wiring remain
later work.

The ADR 0004 JSON envelope would keep method names and payload keys as dynamic
strings even though this P03a surface has four fixed messages.
The implementation request for P03a instead requires closed Rust enums and no
string event names.

## Decision

### Same-executable bootstrap

The executable accepts exactly these process modes:

| Invocation | Mode |
| --- | --- |
| `zero-gesture` | Settings |
| `zero-gesture --settings` | Settings |
| `zero-gesture --engine` | Engine |

Unknown arguments and multiple mode arguments fail before either mode starts.
Engine and Settings use the existing executable, version, Tauri identifier
`dev.r4ai.zero-gesture`, and Tauri application config directory.

Engine synchronously claims its per-user singleton, creates and flushes the
current-user-only secret, and binds the first ACL-protected named-pipe handle
before loading config or starting input workers.
`EngineServer` owns this prepared first handle and passes it into the IPC owner
loop; later listener handles are created by that same owner.
Failure anywhere through the first bind drops the prepared resources, leaves
pre-existing config bytes unchanged, and never installs the input hook or
overlay.
A second Engine observes the existing singleton, changes no running state, and
exits successfully.
The Engine Tauri builder initializes the native tray and local logging but no
Settings WebView, frontend, dialog plugin, or opener plugin.
Its tray starts the same executable with `--settings`.

Settings initializes the WebView and Settings-only plugins.
It starts no gesture, renderer, or executor worker.
At setup it tries the Engine endpoint, starts the same executable with
`--engine` once when unavailable, and retries only within the fixed startup
deadline.
Concurrent Settings launches serialize only this decision with a short-lived,
current-user-only SID-named launch mutex.
After acquiring it, each caller checks the endpoint again; only the caller that
still observes absence spawns.
Wait, recheck, spawn, and connect all share the same three-second deadline.
Authentication, protocol, security, and version errors never trigger a spawn.
Closing Settings does not send shutdown.

P03a keeps the existing Settings config commands in process as a temporary
transition.
They are not part of this IPC surface and do not claim Engine config ownership,
revision control, or live config delivery.

### Windows endpoint and authentication

The Windows adapter uses one byte-mode named pipe whose name contains the
current user SID.
It creates both the per-user singleton mutex and named pipe with a protected
DACL containing one full-access ACE for that SID.
The Settings launch mutex uses the same DACL.
The pipe always sets `PIPE_REJECT_REMOTE_CLIENTS`.

At each Engine start, Windows CNG generates a 32-byte random authentication
secret.
The Engine writes it to `engine-control.secret` below the unchanged Tauri
application config directory using the same current-user-only DACL and flushes
it before accepting a connection.
A stale file is replaced only after this Engine owns the singleton.
Settings reads exactly 32 bytes and sends them in the first `Hello`.
The Engine compares all bytes without an early exit.
Wrong authentication closes only that connection.
Normal or explicit shutdown removes the secret file and closes the pipe.

The secret, frame body, and payload are never logged.
The SID-derived endpoint name is not a secret.

### Closed protocol and bounds

This ADR amends only ADR 0004's JSON body choice.
The little-endian `u32` length prefix and encoded-frame ceiling remain
unchanged, but the P03 protocol body is a closed binary envelope:

```text
protocol_version: u16
message_tag: u8
reserved_zero: u8
request_id: u64
typed payload
```

The complete prefix plus body is strictly less than 1 MiB.
The receiver checks zero, checked prefix-plus-body length, and the ceiling
before allocating the body once.
Version strings are UTF-8 and at most 64 bytes.
Request IDs are non-zero.
Unknown versions, tags, reserved bits, invalid UTF-8, trailing fields,
truncation, duplicate request IDs, mismatched response IDs, and connection
request-budget exhaustion have deterministic typed failures.

The request enum is exactly:

```text
Hello(auth_secret, executable_version)
Ping
GetStatus
Shutdown
```

The response enum is exactly:

```text
Hello(engine_version, config_schema_version, capabilities)
Pong
Status(process_role, webview_count, process_id, uptime_ms,
       thread_count, handle_count, working_set_bytes)
Shutdown(already_requested)
Error(closed_error_code)
```

`Hello` is mandatory before every other request.
The capability bitmap explicitly advertises Ping, status, and shutdown.
An executable-version mismatch retains `Ping` and `GetStatus` but rejects
`Shutdown`; it does not fall back to file access.
Shutdown is idempotent on the authenticated connection.

One IPC owner thread serves one connection at a time and at most eight requests
per connection.
There is no client thread per connection, async runtime, channel, generic RPC
dispatcher, transport trait, or unbounded request table.
Pipe read and write operations each have a 750 ms deadline.
Settings auto-start has one 3 second deadline with a 40 ms retry interval.
A terminal error response keeps the pipe alive for at most 100 ms so Windows
does not discard the response before disconnect.

`EngineControl` is the Settings-facing seam.
It hides the named pipe and returns typed status or control errors.

### P03a evidence

`contracts/p03-process-ipc.json` contains 45 atomic runnable cases.
For this P03a boundary, `O = 45`, `O_v = 45`, `U = 0`, and
`O_v / O = 100%`.
The 18 pure process/codec cases run at unit level and the 27 Windows
transport/process cases run at integration/process level, so
`T = 45`, `T_u = 18`, `T_i = 27`, and `T_e = 0`.
Pure codec rejection cases are not duplicated as executable E2E cases unless
the Windows server has an independent typed-response obligation.

An actual same-executable `--engine` child runs in a unique test namespace and
temporary config/log root.
Windows process and window APIs verify that it starts no WebView2 descendant
and owns no content window; the only observed top-level classes are Tauri tray,
event-target, and IME infrastructure (`tray_icon_app`,
`Tao Thread Event Target`, and `IME`).
The typed status snapshot separately verifies the serving process identifier,
role, uptime, thread count, handle count, and working set.
This is topology evidence, not the P06 CPU, RSS, or latency acceptance claim.

Tests query the security descriptors of the created singleton mutex, launch
mutex, first pipe handle, and secret file through Windows APIs.
Each has a protected DACL with exactly one allow ACE for the current user SID.
`GetNamedPipeInfo` also verifies that the prepared handle is a kernel server
endpoint.
CI cannot reliably test a remote-form connection: on the current Windows
environment even a control pipe without `PIPE_REJECT_REMOTE_CLIENTS` fails
before reaching NPFS with `ERROR_BAD_NET_NAME`.
Therefore the manifest does not misclassify the creation-flag constant as
runtime proof of remote rejection; the implementation still sets
`PIPE_REJECT_REMOTE_CLIENTS`, while runnable evidence claims only the actual
API observations it can distinguish.

## Deferrals

P03a does not implement config upload or mutation, Config owner/RCU, capture
IPC, InputKernel hook wiring, Context, Renderer, Executor, macOS socket or
adapter, updater, telemetry upload, or UI redesign.

## Consequences

The first real process seam is authenticated, bounded, and observable without
creating a reusable RPC framework.
Sequential connection service intentionally favors a small state space over
parallel control throughput.
P03b can add the already-decided config surface behind the same framing and
access-control rules without changing the Settings-facing transport seam.
