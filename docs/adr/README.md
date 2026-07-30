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

## Reading order

まずADR 0001でprocess境界を確認し、ADR 0002で常駐process内部の所有権と障害時挙動を確認する。
ADR 0003はdomainとplatformの境界、ADR 0004はprocess間の契約と永続化、ADR 0005はそれらを検証する方法を定める。
ADR 0006はADR 0005の契約を再定義せず、P01aの静的測定範囲と後続P01b/P01cへの分割を定める。
ADR 0007はP01bをP02の直前に必要な実行可能characterizationへ限定し、全project contract inventoryとの境界を定める。
ADR 0008はP02aで実装したportable gesture moduleの所有権、interface、Windows effect適用境界を記録する。
ADR 0009はP02bで実装したconfig document、legacy migration、現在利用可能な永続化、Windows compile seamを記録する。
ADR 0010はP02cで実装したInput owner policy、generation pin、accepted-action completion、allocation-free handle seamを記録する。

## Status policy

- `Accepted`: 後続実装が守る決定である。
- `Superseded`: 新しいADRへのリンクを残し、履歴として保持する。
- 実装上の制約でAcceptedな決定を変更する場合、コードだけで例外を作らず、新しいADRで根拠と影響を記録する。
