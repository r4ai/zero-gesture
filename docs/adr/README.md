# Architecture Decision Records

このディレクトリは、Zero Gestureのマルチプラットフォーム化で採用した意思決定を記録する。
ADRは実装の詳細設計ではなく、後続PRが守る外部契約、不変条件、失敗条件、検証ゲートを定める。

| ADR | Status | Decision |
| --- | --- | --- |
| [0001](./0001-tauri-two-process-modes.md) | Accepted | 同一Tauri executableをEngine modeとSettings modeの別processとして起動する |
| [0002](./0002-message-passing-and-fail-open.md) | Accepted | 単一所有者と型付きmessage passingを使い、入力経路はfail-openにする |
| [0003](./0003-portable-domain-and-native-platforms.md) | Accepted | platform-neutral domainとOS固有adapterを分離し、Apple Silicon/macOSを直接配布で支える |
| [0004](./0004-internal-ipc-and-engine-owned-config.md) | Accepted | 内部local IPCを境界にし、Engineを設定のsingle writerにする |
| [0005](./0005-quality-contracts-and-delivery-plan.md) | Accepted | Windows互換、privacy、性能、複雑度、テスト義務、PR依存順を品質ゲートにする |
| [0006](./0006-reproducible-static-quality-kpis.md) | Accepted | P01を分割し、canonical CIで再現可能な静的quality/KPIを先に測定する |
| [0007](./0007-p02-characterization-baseline.md) | Accepted | P02直前に現行Windowsのportable化対象だけを実行可能なcharacterization baselineとして固定する |
| [0008](./0008-portable-gesture-decision-core.md) | Accepted | gesture認識とsession判断を一つのportable moduleへ移し、Windowsを唯一のcallerとして切り替える |
| [0009](./0009-config-schema-v2-migration-and-compile.md) | Accepted | strict schema v2、legacy分類移行、非破壊upgrade、immutable Windows compileを固定する |
| [0010](./0010-bounded-input-owner-kernel.md) | Accepted | boundedなplatform-neutral Input owner kernelと固定ID/effect境界を追加する |
| [0011](./0011-p03a-process-mode-and-control-ipc.md) | Accepted | 同一executableのprocess bootstrapと認証済みWindows control IPCを固定する |
| [0012](./0012-engine-config-owner-and-two-slot-publication.md) | Accepted | Engine config single writer、bounded Prepare/Commit/Applied、二固定slot publicationを固定する |
| [0013](./0013-windows-native-input-owner.md) | Accepted | Windows native input ownerをInputKernel、二slot snapshot、bounded context/action/renderer laneへ接続する |
| [0014](./0014-macos-same-binary-packaging-spike.md) | Accepted | macOS arm64の同一bundle/executable、署名、window/WebViewなしEngine起動をpackaging gateにする |
| [0015](./0015-macos-uds-control-plane.md) | Accepted | macOSのuser-only runtime directory、UDS、peer UID検証、共有control coreを固定する |
| [0016](./0016-macos-listen-only-event-tap-owner.md) | Accepted | macOS Engineの専用threadでlisten-only CGEventTap、bounded SPSC、fail-open lifecycleを所有する |
| [0017](./0017-macos-accessibility-context-resolver.md) | Accepted | promptなしAccessibility preflightとbounded macOS context worker/cacheを固定する |
| [0018](./0018-macos-action-executor-and-event-tagging.md) | Accepted | bounded macOS action worker、self-generated event marker、最小context consumer接続を固定する |
| [0019](./0019-windows-first-runtime-shell.md) | Accepted | Windows完成を先行し、P05a runtime shell・P05b Settings control・P05c distributionの順序と契約を固定する |
| [0020](./0020-engine-owned-windows-settings-control.md) | Accepted | typed Settings error、conflict-safe draft、Engine-owned Windows capture protocolを固定する |
| [0021](./0021-windows-nsis-installed-acceptance.md) | Accepted | current-user NSIS、retention、installed release acceptance、KPI、Authenticode blockerを固定する |
| [0022](./0022-objc2-macos-library-foundation.md) | Accepted | P04R0 foundationからcontext・Event Tap・action移行、Active Input、Native Overlay、shell、distributionへ進むmacOS順序とobjc2不変条件を固定する |

## Reading order

まずADR 0001でprocess境界を確認し、ADR 0002で常駐process内部の所有権と障害時挙動を確認する。
ADR 0003はdomainとplatformの境界、ADR 0004はprocess間の契約と永続化、ADR 0005はそれらを検証する方法を定める。
ADR 0006はADR 0005の契約を再定義せず、P01aの静的測定範囲と後続P01b/P01cへの分割を定める。
ADR 0007はP01bをP02の直前に必要な実行可能characterizationへ限定し、全project contract inventoryとの境界を定める。
ADR 0008はP02aで実装したportable gesture moduleの所有権、interface、Windows effect適用境界を記録する。
ADR 0009はP02bで実装したconfig document、legacy migration、現在利用可能な永続化、Windows compile seamを記録する。
ADR 0010はP02cで実装したInput owner policy、generation pin、accepted-action completion、allocation-free handle seamを記録する。
ADR 0011はP03aで実装したEngine/Settings bootstrap、Windows current-user endpoint、閉じたcontrol protocol、bounded retryを記録する。
ADR 0012はP03bで実装したEngine-owned config transaction、Settings/tray mutation adapter、二固定slot publicationを記録する。
ADR 0013はP03cで実装した実Windows callback、generation pin、事前解決context、action/renderer lane、owner lifecycleを記録する。
ADR 0014はP04a（既存計画のP05）で固定したmacOS arm64 bundle、同一署名identity、ad-hoc CI、Developer ID/notarization release gateと未検証条件を記録する。
ADR 0015はP04b1で固定したmacOS user-only UDS endpoint、singleton/stale cleanup、peer UID検証、共有control core seamを記録する。
ADR 0016はP04b2で固定したmacOS listen-only Event Tap、allocation-free callback、bounded normalization、degraded lifecycleを記録する。
ADR 0017はP04b3aで固定したconsumer接続前のidle境界、promptなしAccessibility seam、worker threading、AX timeout、context identity/cache、fail-open semanticsを記録する。
ADR 0018はP04b3bで接続したrun-loop context consumer、bounded keyboard action worker、self-generated event marker、failure/defer境界を記録する。
ADR 0019はP04b3b後の順序をWindows-firstへ変更し、P05aのautostart、Settings-only single instance、close/tray/Quit境界とP05b/P05cへの分割を記録する。
ADR 0020はP05bで接続したtyped Settings failure、revision conflict時のdraft保持、既存native callbackからEngine IPCへ至るcapture id/epoch境界を記録する。
ADR 0021はP05cで固定したcurrent-user NSIS、設定/log保持、installed production lifecycle/KPI、self-signed CIと実署名blocker、実GUI/physical gateを記録する。
ADR 0022はP04R0 Foundationで固定したmacOS-only objc2依存とcallback safety、
P04R1 context、P04R2 listen-only Event Tap、P04R3 action executorの分割移行、
P04b3c-a Active Input、P04b3c-b Native Overlay、P05m shell/permissions/autostart、
P06m distribution/physical acceptanceの順序を記録する。UDS分割は任意の後続作業とし、
R0はruntime contractを追加せず既存5 manifestの95 obligationsを継承する。

## Status policy

- `Accepted`: 後続実装が守る決定である。
- `Superseded`: 新しいADRへのリンクを残し、履歴として保持する。
- 実装上の制約でAcceptedな決定を変更する場合、コードだけで例外を作らず、新しいADRで根拠と影響を記録する。
