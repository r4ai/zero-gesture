# ADR 0003: Keep the domain portable and use native platform adapters

- Status: Accepted
- Date: 2026-07-26

## Context

現行実装はgesture logicの一部をpure Rustへ分離しているが、configuration、application identity、key representation、rendering、lifecycleはWin32概念と同じTauri processに結合している。
Windows対応を維持しながらmacOSを追加するには、OS APIを条件分岐でdomainへ漏らさず、同じevent traceへ同じ意味を与える必要がある。

最初から多数のcrate、単一実装trait、将来のLinux用extension pointを作ると、現在の二platform契約より構造が大きくなる。

## Decision

依存方向を次の一方向にする。

```text
Tauri process mode / engine runtime
  -> platform facade (one implementation selected at compile time)
    -> platform-neutral domain
```

初期実装は既存Tauri crate内のdeep moduleとして分離する。

```text
src-tauri/src/
  domain/       # config, gesture, action, app selector
  engine/       # owner loops, protocol, lifecycle
  platform/
    windows/    # Win32 input, context, injection, rendering
    macos/      # Core Graphics, Accessibility, AppKit/Core Animation
```

別crateは、independent compilation、dependency isolation、または複数consumerが必要と実測された場合だけ追加する。
platform選択は`cfg`で一つのfacade moduleを公開し、単一実装traitやruntime dynamic dispatchを作らない。

## Platform-neutral domain contract

domainは次を知らない。

- Win32 message、virtual key、`HWND`
- `CGEvent`、AppKit object、Accessibility object
- Tauri、IPC、thread、renderer API
- physical screen coordinate system

canonical inputは、trigger button、button transition、wheel notch、normalized point、monotonic tick、resolved app IDで表す。
outputは、`Pass`/`Suppress`、render delta、optional validated action、optional replay operationで表す。

既存Windowsの次の意味を共通contractとして維持する。

- left/right/middle trigger
- release gestureとhold wheel gesture
- up/down/left/right、wheel、追加clickを混在できる最大8 stepのsequence
- app-specific bindingをdefaultより優先する解決
- unmatched short clickのreplayとtravel-distance threshold
- movement eventを抑止しない
- safety timeout
- labelとnative trail
- keyboard actionのみ

任意shell command、汎用script、IPC経由のprocess executionは追加しない。

## Configuration schema

新schemaはversionを持ち、一つのdocumentにcommon設定と明示的なplatform overrideを置く。

```json
{
  "schema_version": 2,
  "shared": {
    "recognition": {},
    "appearance": {},
    "bindings": []
  },
  "platforms": {
    "windows": {},
    "macos": {}
  }
}
```

logical modifierを次のように定義する。

- `primary`: Windows `Ctrl`、macOS `Command`
- `secondary`: Windows `Alt`、macOS `Option`
- `shift`: 両OSの`Shift`

`ctrl`、`command`、`win`などのphysical keyも明示できる。
legacy Windows configの`ctrl`はphysical `Ctrl`としてmigrationし、暗黙に`primary`へ変えない。
default bindingだけをlogical modifierへ更新できる。

application selectorは共通IDへcompileする。

- Windows: process name、window class、title
- macOS: bundle identifier、process name、title

利用できないselectorはplatform capabilityとしてvalidation errorにし、matchしない値へ黙って変換しない。

platform overrideはschemaで列挙したtyped fieldだけを許可する。
unknown field、汎用JSON map、generic merge、recursive deep mergeは使わない。
effective configはallowlistの各fieldについて次の規則だけで作る。

- overrideがmissingなら`shared`のfieldを使う。
- overrideが`Some(value)`なら`shared`の同じfieldを明示的に置換する。
- arrayとobjectはfield全体を置換し、要素追加、key単位merge、暗黙継承をしない。
- sharedに表現できないWindows固有matcher/keyは`platforms.windows`へ、macOS固有matcher/keyは`platforms.macos`へ移す。

allowlistの具体的なfield mappingはschema実装PRで型として固定するが、この優先順位とwhole-field replacementは変更しない。
legacy v1の両platformで意味が同じ値は`shared`へ移し、Windows固有のmatcher、physical key、bindingだけをWindows overrideへ移す。
移行時に分類できない値は黙って共有せず、field pathを含むmigration errorにする。

## Supported platforms

初期support matrixは次で固定する。

| Platform | Supported target |
| --- | --- |
| Windows | Windows 11 x64、既存behaviorを維持 |
| macOS | release時点の最新stable macOS、Apple Silicon arm64 |
| Intel macOS | 対象外 |
| Linux | 対象外 |

macOS version numberを長期contractに固定しない。
各releaseはCIとApple Silicon実機で、その時点の最新stable macOSを記録する。

## Native adapters

### Windows

- `WH_MOUSE_LL`
- `SendInput`
- Win32 foreground/window identity
- 現行GDI native overlay
- Tauri native tray
- Named Pipe

### macOS

- active `CGEventTap`
- `CGEventPost`
- Accessibilityによるapp/window identityとhit test
- click-through AppKit windowとCore Animation
- Tauri native status item
- Unix Domain Socket

recognitionとconfig compileはsafe Rustを基本とする。
`unsafe`はOS API境界の小さなmoduleに閉じ込め、safe wrapperがpointer lifetime、thread affinity、ownershipを確立する。
Rust bindingで表現が著しく不安定になるAPIだけ、薄いObjective-C/Swift shimを許可する。
shim採用には、直接bindingで満たせないcontractと追加されるbuild/ABI boundaryをADRへ記録する。

native overlayはWebView、Canvas、Skiaを常駐Engineへ入れない。
platform adapterがdisplay scale、multi-display origin、macOSの座標反転を吸収する。
Windowsは移行中も現行GDI rendererを維持する。
performance acceptanceを満たさないことを測定で実証した場合だけ、別ADRと独立PRでrenderer変更を判断する。
macOSのAppKit/Core Animation rendererはWindows renderer変更とは別のadapter実装である。

## macOS permissions and distribution

Mac App StoreとApp Sandboxを初期対象にしない。
Developer ID署名、Hardened Runtime、notarization済みの`.app`を`.dmg`または`.pkg`で直接配布する。
root daemon、kernel extension、system extension、管理者権限を要求しない。

必要なInput Monitoring、Accessibility、event posting権限はEngine modeの安定した署名identityへ紐付ける。
permissionが不足または実行中に剥奪された場合、input eventを抑止せずgesture機能を無効化する。
Screen Recording権限は要求しない。
Settingsはpermission状態とSystem Settingsへの案内を表示するが、承認済みと推測しない。

## Failure conditions

- platform eventをcanonical eventへ変換できない場合はそのeventを通す。
- app contextを期限内に解決できない場合はdefault bindingだけを使用する。staleな別app identityを使わない。
- key/actionをplatformで表現できないconfigは保存境界で拒否する。
- native renderer初期化失敗はheadless operationへdegradeし、inputを停止させない。
- permissionを要求しないAPIを誤って呼ぶ実装はrelease gateで拒否する。

## Consequences

WindowsとmacOSの同じtraceをpure contract testで比較できる。
OS APIの重複はadapterに限定される。
LinuxやIntel対応のための未使用abstractionを持たず、必要になった時点でsupport matrixとcontractを更新する。
