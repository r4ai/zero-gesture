# ADR 0009: Fix schema v2 migration and immutable Windows compilation

- Status: Accepted
- Date: 2026-07-30

## Context

ADR 0003 fixes the portable/platform record rules and ADR 0004 fixes the eventual Engine-owned transaction.
P02b must implement the document, one-time legacy upgrade, and current-process snapshot without prematurely implementing that owner protocol.
The former schema-less Rust model also corrected invalid values, discarded records, and compiled application matchers again in the hook path.

## Decision

The persisted and Settings-facing representation is one strict `schema_version: 2` document.
Every record rejects unknown fields and uses closed enums.
Its concrete current shape is:

```text
ConfigDocument {
  schema_version: 2
  shared {
    enabled
    recognition {
      safety_timeout_ms
      min_segment_px
      direction_switch_confirm_px
      axis_ambiguity_deadzone_px
      replay_distance_threshold_px
      max_gesture_steps
    }
    appearance {
      trail_color
      trail_thickness
      label_font_family
      label_font_size
      label_font_weight
      label_padding
    }
  }
  applications: [Shared(Application) | Windows(Application) | Macos(Application)]
  bindings: [Shared(Binding) | Windows(Binding) | Macos(Binding)]
  platforms {
    windows { appearance?: Appearance }
    macos { appearance?: Appearance }
  }
}
```

`enabled` and recognition have only the shared canonical location.
Appearance is the only current non-binding override allowlist because native font and trail presentation may vary.
A missing platform appearance inherits the shared field; a present value replaces the whole appearance field.
There is no element merge or recursive merge.

Shared application selectors are `process_name` and `title`.
Windows additionally permits `window_class`; macOS additionally permits `bundle_identifier`.
Shared bindings permit logical `primary`, `secondary`, and `shift` modifiers plus portable non-modifier keys.
Windows additionally permits physical `ctrl`, `alt`, and `win`; macOS permits physical `ctrl`, `command`, and `option`.
Application references are optional, where missing means the default binding set.
A Shared binding may reference only a Shared application.
A platform binding may reference Shared or its own platform.

## Legacy classification

Schema-less and explicit v1 input decode into one typed legacy record and migrate before persistence.
Application and per-application binding map keys become stable IDs and `application_id` references.
Arrays retain their order; legacy map groups use deterministic default-first then lexical ordering because JSON object member order is not a binding precedence contract.

- `process_name` and `title` applications become whole Shared records.
- Any `window_class` matcher makes the whole application Windows.
- Any physical Windows modifier or reference to a Windows application makes the whole binding Windows.
- Other supported legacy keys remain Shared.
- `ctrl` and its accepted legacy alias compile to physical Windows Ctrl, never logical `primary`.
- A mixed record is never split.
- An unsupported selector, key, reference, or gesture fails with its concrete legacy or v2 field path.

The migration preserves enabled state, IDs, labels, application references, matcher values and methods, binding order, gesture fields, actions, and disabled behavior.
Only documented legacy aliases normalize to their canonical closed key name.

## Compile interface

`ActiveConfig::from_document(ConfigDocument)` is the validation and compile seam.
It returns the canonical document plus one immutable `RuntimeConfig`.
Validation, regex compilation, platform filtering, logical-to-Windows key lowering, binding-set construction, and effective appearance selection occur once there.

Windows compilation walks the ordered collections once, selecting Shared and Windows records and excluding Macos records.
Application matching keeps first-document-match wins.
`GestureMachine` keeps app-specific-first then default-on-fresh-context resolution.
The hook receives only the compiled snapshot; it performs no JSON work, validation, regex construction, or binding compilation.
Win32 window activation and foreground/window acquisition stay in the hook adapter.

## Persistence available in P02b

The application identifier, Tauri application config directory, and active filename remain unchanged.
Startup behavior is:

1. A missing active file uses compiled v2 defaults without writing.
2. A valid v2 file validates and compiles without rewriting.
3. A valid v1/schema-less file migrates and compiles in memory.
4. The original bytes are written to a create-new same-directory migration backup and flushed.
5. Pretty-printed v2 bytes are flushed to a same-directory temporary file and atomically replace the active file.

Migration decode, validation, backup, temporary write, flush, or replace failure leaves the original active bytes untouched.
Invalid v2 and newer versions are non-destructive errors.
At startup such an error leaves the shared configuration unavailable, starts no gesture workers, logs the field-path diagnostic, and makes the existing `get_config` and mutation paths return an error instead of substituting defaults.
An imported or Settings-submitted candidate is compiled and persisted before the in-memory snapshot and workers change.

Windows atomic replacement uses the existing `windows-sys` dependency's `MoveFileExW` with replace-existing and write-through flags.
Other targets use same-directory `rename`.
Directory metadata fsync and crash-recovery arbitration remain unavailable in the current architecture and are not claimed here.

## Deferrals

P02b does not implement the Engine config-owner actor, expected revisions, RCU prepare/commit/applied handshake, retained generations, IPC framing or transfer budgets, dual process modes, native macOS adapter, runtime KPI harness, or updater.
Those remain P02c/P03 and later work under ADR 0004 and ADR 0005.

## Consequences

Invalid input is no longer silently corrected or replaced with defaults.
The document is the only editable representation in Rust and TypeScript.
Windows Settings selectors exclude Macos records before editing; updates preserve their order, IDs, references, content, and platform override.
The config module has one small external compile interface while migration, validation, and selector knowledge remain local.

The existing P02 manifest retains its initial 15 Windows cases and adds 10 one-to-one Rust migration, compile, validation, and persistence cases.
For that bounded manifest, `O = 25`, `O_v = 25`, and `U = 0`.
