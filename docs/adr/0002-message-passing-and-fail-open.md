# ADR 0002: Use message passing and fail open on the input path

- Status: Accepted
- Date: 2026-07-26

## Context

低level input callbackは、OSへeventを通すか抑止するかを同期的に返す。
遅い描画、config I/O、IPC、window query、action実行をcallbackへ置くと、このappがsystem全体のinput latencyを悪化させる。
共有`Arc<Mutex<AppState>>`は所有権を曖昧にし、lock contentionとpoisoningをhot pathへ持ち込む。

一方で、汎用Actor frameworkや独自schedulerを導入すると、今回必要な少数のownerより概念とfailure modeが増える。

## Decision

常駐Engineはactor-likeな単一所有者と型付きmessage passingを使う。
ここでいうactorはlibraryやruntimeではなく、stateを一つのloopだけが変更するという設計規則である。

| Owner | Exclusively owned state | Input |
| --- | --- | --- |
| Supervisor | lifecycle、health、shutdown reason | owner exit、fatal/degraded report |
| Input | OS hook/event tap、gesture session、active config generation | OS event、control snapshot |
| Renderer | overlay window、native drawing resources | render state、coalesced points |
| Executor | injection order、platform key mapping | validated `Action` |
| Context | active app/window identity、compiled match result | OS context notifications、bounded query |
| Config | schema document、revision、compiled immutable snapshot | validated control request |
| IPC | connection framing、request correlation | authenticated local connection |
| Status | tray/status item state | health and permission snapshot |

OS thread/run-loop affinityがあるInput、Renderer、Statusはそのthreadにstateを固定する。
I/O中心のConfig、IPC、diagnosticsは、所有権が混ざらない限り同じcontrol runtime上で実行してよい。
owner数とthread数を同一にすること自体は目標にしない。

内部messageはRustのclosed enumで表す。
JSON、string event名、汎用mapをowner間の正準表現にしない。
通常のmailboxはboundedとし、blocking sendをInputから呼ばない。
request/responseが必要なcontrol pathだけ、messageへoneshot replyを含める。

## Input callback exception

OS callbackのeventをmailboxへ送り、別ownerの回答を待つ設計は禁止する。

```text
OS callback
  -> Input ownerが同じthreadでpure state transitionを評価
  -> pass / suppressを同期return
  -> reserved laneへessential effectを送る
  -> render point / metricsだけをbest-effortで送る
```

callbackは次を行わない。

- mutex、rwlock、condition variable、blocking channelの待機
- heap allocation
- file、socket、JSON、IPC
- async runtimeへの`await`
- Accessibility/Win32 foreground window query、regex compile/match
- action execution、process launch
- log message formattingまたは同期log sink

app/window contextとbindingはcallback前に解決し、Inputは小さなcanonical IDとimmutable compiled snapshotだけを使う。
進行中のgestureは開始時のsnapshotを所有し、config更新は次のgestureから見える。

## Backpressure

backpressure policyはmessage種別ごとに固定する。

| Flow | Policy when full or slow |
| --- | --- |
| Input to Renderer points | intermediate pointをcoalesceし、latest pointを優先する |
| Input to Renderer lifecycle | lossless control laneへ保持する。送れない場合は当該generationの描画を開始せず、rendererを終了状態へ移して新規gestureをfail-openにする |
| Input to Executor | gesture開始前にcapacityをreserveする。accepted actionは保持し、実行不能ならtrigger replayを試みて新規gestureをfail-openにする |
| Input replay | preallocated emergency slotへ保持する。schedule不能なら新規抑止を停止し、terminal degraded stateとして報告する |
| Config to Input | committed revisionをack付きで配送する。配送不能ならcommitを成功扱いにせず、旧snapshotを維持する |
| Supervisor shutdown | 専用control laneへ保持する。送信失敗はowner終了として扱い、supervisorがresource解放とjoinを完了する |
| Metrics | sampleまたは個別eventだけをdropできる。counterはaggregateした値へ収束させる |

lossまたはcoalesceを許すのは中間render pointとdiagnostic metricsだけである。
action、render lifecycle、replay、committed config、shutdownはsilent dropしない。
保持できなければ上表のfail-open terminal transitionを同期的に選び、新規input抑止を開始しない。

custom unsafe lock-free queueは実装しない。
既存の検証済みprimitiveで要求を満たせないことをbenchmarkで示した場合だけ、別ADRで範囲とmemory modelを定める。

## Fail-open invariant

Zero Gestureが正しい抑止とactionを期限内に保証できない場合、機能よりOS inputを優先する。

- hook/event tap install失敗時はgestureを開始しない。
- callback state、queue、permission、essential ownerの異常時は新しいtriggerを抑止しない。
- Renderer障害はtrail/labelだけを無効化し、inputとactionを継続できる。
- Executor障害は新しいgesture captureを停止し、statusをdegradedにする。
- safety timeout、panic、event tap timeoutでは現在sessionを終了し、以後のeventを通す。
- FFI callbackからRust panicやforeign exceptionを越境させない。
- injected eventにはself tagを付け、同じgestureとして再捕捉しない。

trigger downを抑止済みでsessionを完了できない場合は、固定上限の緊急経路で元のdown/upを一度だけreplayする。
replayも保証できない状態では、新規抑止を即時停止してdiagnostic faultを残す。
無限retryやsilent fallbackは行わない。

## State invariants

- 一つのmutable factを複数ownerへ保存しない。
- Engine全体を包むglobal mutable stateを作らない。
- owner間で共有するconfigはimmutable snapshotであり、revisionとgenerationは一つの値から導出する。
- gestureのterminal transitionはexecute、replay、cancelのいずれか一つである。
- Renderer generationはInput generationより進まず、終了済みgenerationを再表示しない。
- accepted action、render lifecycle、replay、committed config、shutdownをsilent dropしない。
- shutdownはidempotentで、hook/event tapを先にpass-through状態へ移してからownerをjoinする。

## Consequences

遅いcontrol処理とhot pathのfailure domainを分離できる。
mailbox capacity、overflow、health transitionを明示的にtestする必要がある。
Actor frameworkのsupervisionやroutingは得られないが、必要な契約だけを小さなenumとowner loopで実装できる。
