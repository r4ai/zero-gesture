# ADR 0001: Run two process modes from one Tauri executable

- Status: Accepted
- Date: 2026-07-26

## Context

マウスhook、入力抑止、gesture認識は常時動作する。
設定画面のWebViewは設定時にしか必要なく、WebViewのmemory、failure、update lifecycleを入力経路へ持ち込む理由はない。
一方で、最初から別binaryやnested helper appを作ると、crate、bundle、署名identity、権限主体、installer処理が重複する。

常駐data planeと設定control planeは別processにするが、配布単位まで分割する必要性はまだ実証されていない。

## Decision

一つのTauri executableを、引数で次の二つのprocess modeとして起動する。

| Mode | Invocation | Lifetime | Windows/WebView | Responsibilities |
| --- | --- | --- | --- | --- |
| Engine | `zero-gesture --engine` | loginからlogoutまたは明示終了まで | window 0、WebView 0 | tray/status item、input、recognition、native overlay、action、IPC、config |
| Settings | `zero-gesture`または`zero-gesture --settings` | 設定windowを閉じるまで | 設定window 1 | React UI、file picker、Engine IPC bridge |

Tauriは次を担う。

- executable、asset、installer、macOS app bundleのbuildと配布
- Settings modeのWebViewとReact bridge
- Engine modeのnative tray/status item
- login時起動の登録

Tauri commandへinput hookやgesture state machineを置かない。
Engine modeはTauriをnative shellとして利用しても、WebViewを生成しない。
Settingsのcloseはwindow hideではなくprocess exitである。

EngineはOS userごとに一つだけendpointを所有する。
二つ目のEngine起動は既存Engineを変更せず、成功扱いで短時間に終了する。
trayの「Settings」は同じexecutableを`--settings`で起動する。
SettingsからEngineが見つからない場合も、同じexecutableを`--engine`で起動し、bounded timeout内でIPC接続を再試行する。

login時起動は、Tauriのautostart pluginへ`--engine`を渡す経路を第一候補とする。
登録対象は同一executableのEngine modeであり、別helperを通常経路にしない。
現pluginのmacOS backendがAppleの現行推奨方式であるとは仮定せず、実機spikeで
LaunchAgentの引数、user拒否状態、再インストール後の登録を検証する。

## External contract

- Settingsを閉じた定常状態ではZero Gesture由来のWebView process数が0である。
- Settingsのcrash、hang、再起動、version mismatchは、動作中のEngineとOS inputを停止しない。
- SettingsはOS inputを直接監視、抑止、注入しない。
- Engineは設定の保存先とIPC endpointを一意に所有する。
- 再インストール後も同じapplication identifierと設定保存先を使う。
- Engine modeとSettings modeは同じrelease version、署名identity、bundle/application identifierから配布する。

## Failure conditions

- Engine起動時にsingleton endpointを安全に所有できない場合、hookをinstallせず終了する。
- SettingsがEngineへ接続できない場合、設定編集をoffline成功に見せず、Engine unavailableを表示する。
- Engine processが停止した場合、OS hook/event tapの解放によりinputを通す。
- Settings close後にWebView processが残る実装は不合格とする。

## Packaging spike gate

macOSの最初のpackaging PRは、署名・notarizeしたrelease artifactで次を実機検証する。

1. Tauriのautostart登録が同一executableをEngine modeで起動できる。
2. login後にwindow/WebViewを生成しない。
3. tray/status itemからSettings modeを起動できる。
4. 新versionの再インストール後も登録と設定が保持される。
5. Input MonitoringとAccessibilityの権限主体が安定した署名identityとして認識される。

このgateが満たせないことを再現手順とartifactで実証した場合だけ、nested Login Item/helper appを候補にする。
その変更には、別ADRで次を記録しなければならない。

- 不可能だった同一binary経路
- 追加されるbinary、bundle、署名identity、permission主体
- IPCとversion整合
- installer、notarization、uninstallへの影響

## Rejected alternatives

### One process with a hidden Settings window

WebViewの常駐とfailure propagationを防げない。

### Separate Engine binary from the beginning

同じRust codeを別entrypointへ分けるだけでは外部契約を改善せず、bundleと署名の責務を増やす。
packaging上の必要性が実証されるまで採用しない。

### Headless daemon plus a separate tray launcher

常駐processとIPC境界を一つ増やす。
Engine自身がnative tray/status itemを所有すれば不要である。

## Consequences

process分離のIPCと起動競合は扱う必要がある。
その代わり、同一artifactと署名identityを保ちながら、常駐memoryとfailure domainをSettingsから分離できる。
同じexecutable内でmodeを選ぶため、将来用の重複crateやpublic interfaceは作らない。
