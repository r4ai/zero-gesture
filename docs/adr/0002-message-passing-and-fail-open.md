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
| Executor | session target activation、injection order、platform key mapping | session-bound activationとvalidated `Action` |
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
  -> canonical eventへnormalizeし、Input ownerがpure state transitionを評価
  -> 必要なessential creditをnonblocking reserveし、reserved laneへeffectを送る
  -> render point / metricsだけをbest-effortで送る
  -> pass / suppressを同期return
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

### Pre-resolved trigger context

WindowsのContext workerはcallback外でpointerをsampleし、`WindowFromPoint`、window identity、binding解決、target handle取得を行う。
結果はpreallocated latest-value slotへ次のimmutable snapshotとしてpublishする。

```text
sampled_point
sampled_at
config_generation
binding_set_id
target_handle
```

trigger時のInputはOS queryを行わず、point tolerance、最大age、config generation、cached handle validityだけを検査する。
全条件がfreshならcached `BindingSetId`とtarget handleを持つ`SessionId`を作り、target activationはcallback復帰を待たない`ActivateTarget(SessionId, TargetHandle)`として送る。
snapshotがmissing/stale、pointがtolerance外、generation不一致、handle invalidならtrigger eventをpassする。

これは現行Windowsのcallback内同期window query/app matching/activationを保存しない意図的な安全変更である。
P01 manifestはfresh-cache開始と各fail-open条件を独立predicateとして持つ。

### Session-bound Executor ordering

`ActivateTarget`と`Action(SessionId, ActionId, repeat)`は同じbounded FIFO Executor laneだけを使う。
Inputはtriggerを抑止する前にactivation creditをnonblocking reserveして`ActivateTarget`をenqueueする。
credit不足またはExecutor owner deathならtriggerをpassし、まだ抑止していないsessionを`Cancel`する。

Executorはactivationを試行し、そのsession専用のpreallocated result slotへ`Ready | Failed`をpublishする。
FIFOと`SessionId`によりactivation attempt/resultは同sessionのaction acceptanceより先になり、stale resultは無視する。
Inputはresultを待たず、`Ready`を観測したaction-producing eventだけaction creditをreserveできる。
`Failed`、owner death、`Pending`、action credit不足ではactionをacceptせず、[captured-trigger failure rule](#captured-trigger-failure-rule)を一度だけ適用する。
activationとaction以外へ再利用する汎用ack frameworkは作らない。

### Accepted action delivery

Inputはrecognition transitionと別に、一session一枠のpreallocated action completion recordのlifecycleとcaptured triggerを所有する。
record内のphaseはExecutorだけが書くmonotonic atomicであり、Inputはterminal resultまたはExecutor owner death後に読む。
recordはInputだけが更新する`physical_up: NotObserved | ObservedAndSuppressed`も初期値`NotObserved`で持ち、pending中に対応するphysical upが来た場合だけ抑止して後者へ進め、他eventはpassする。
action-producing eventを抑止する前に、Executor credit、completion record、captured-trigger replay obligationを全てreserveする。
一枠がpendingまたはReplay cleanup中は同sessionの次actionをacceptせず、hold sessionはterminal cleanup後に同じ空き枠を再利用する。

accepted actionは`PendingBeforeInjection`から、Executorのterminal resultで`Completed | FailedBeforeInjection | FailedAfterInjection`のどれか一つへ閉じる。
Executorは最初のOS injection API callへ入る直前にpreallocated result slotを`InjectionStarted`へ進め、この更新とcallの間ではcooperative stopを受け付けない。
Executor owner deathまたはterminal lane closeでは、Inputはstableなphaseが`PendingBeforeInjection`なら`FailedBeforeInjection`、`InjectionStarted`なら`FailedAfterInjection`として同じterminal policyを適用する。
`InjectionStarted`前の停止またはzero-event failureだけを`FailedBeforeInjection`とし、Inputはcompletion recordのcaptured triggerへ[captured-trigger failure rule](#captured-trigger-failure-rule)を適用する。`ObservedAndSuppressed`なら既存reserved emergency slotへsynthetic down/upを即時enqueueしてReplayを完了し、`NotObserved`なら対応upを待って抑止してから同じpairをenqueueする。他eventはpassし、新しいqueueは追加しない。
`InjectionStarted`後の停止、partial injection、結果不明は`FailedAfterInjection`とし、triggerをreplayして二重実行せず、terminal diagnosticを記録してInput bypassとExecutor recoveryへ進む。
`FinishWithAction`がrecognition sessionを閉じても、Inputはこのrecordをterminal resultと必要なReplay cleanupが完了するまで保持する。
汎用journal、複数action ledger、永続queueは追加しない。

## Backpressure

backpressure policyはmessage種別ごとに固定する。

| Flow | Policy when full or slow |
| --- | --- |
| Input to Renderer points | intermediate pointをcoalesceし、latest pointを優先する |
| Input to Renderer lifecycle | lossless control laneへ送る。enqueue failureまたはRenderer actor deathではcurrent sessionを必要かつ可能ならterminal `Replay`、それ以外は`Cancel`とし、Inputをbypassへ移す。新規gestureを開始せず、SupervisorがEngineをclean terminate/restartしてOS overlay resourceを破棄する |
| Input to Executor | target `Ready`後もgesture開始時に長期action creditを取らない。`ContinueWithAction`または`FinishWithAction`で抑止する各eventの直前に[accepted action delivery](#accepted-action-delivery)の一枠をreserveする。取れなければ[captured-trigger failure rule](#captured-trigger-failure-rule)へ遷移する |
| Input replay | preallocated emergency slotへ保持する。schedule不能なら新規抑止を停止し、terminal degraded stateとして報告する |
| Config to Input | disk更新前にrevisionのdelivery slotをprepare/reserveしてackを得る。予約は`Abort`またはinfallibleな`Commit`とInputの`Applied` ackで閉じる。`Commit` deliveryまたは`Applied`不能はprocessをterminate/restartしてinputをfail-openにする |
| Supervisor shutdown | 専用control laneへ保持する。送信失敗はowner終了として扱い、supervisorがresource解放とjoinを完了する |
| Metrics | sampleまたは個別eventだけをdropできる。counterはaggregateした値へ収束させる |

lossまたはcoalesceを許すのは中間render pointとdiagnostic metricsだけである。
action、render lifecycle、replay、committed config、shutdownはsilent dropしない。
保持できなければ上表のfail-open terminal transitionを同期的に選び、新規input抑止を開始しない。
Renderer lifecycle failure後にheadlessでgestureを継続しない。
callbackはSupervisorのterminate/restartやowner cleanupを待たない。

custom unsafe lock-free queueは実装しない。
既存の検証済みprimitiveで要求を満たせないことをbenchmarkで示した場合だけ、別ADRで範囲とmemory modelを定める。

## Fail-open invariant

Zero Gestureが正しい抑止とactionを期限内に保証できない場合、機能よりOS inputを優先する。

- hook/event tap install失敗時はgestureを開始しない。
- callback state、queue、permission、essential ownerの異常時は新しいtriggerを抑止しない。
- Renderer lifecycle enqueue failureまたはactor deathではcurrent sessionを必要かつ可能なら`Replay`、それ以外は`Cancel`とし、Inputをbypassへ移して新規gestureを停止する。SupervisorがEngineをclean terminate/restartする。
- accepted actionがないExecutor障害はactive sessionをtrigger抑止状態に応じて`Replay|Cancel`し、新しいgesture captureを停止してstatusをdegradedにする。accepted actionがある場合はcompletion recordのphaseで分類し、`InjectionStarted`以後はreplayしない。
- safety timeout、panic、event tap timeoutでは現在sessionへ下記の共通ruleを適用し、terminal cleanup後のeventを通す。
- FFI callbackからRust panicやforeign exceptionを越境させない。
- injected eventにはself tagを付け、同じgestureとして再捕捉しない。

### Captured-trigger failure rule

gesture開始後のtimeout、owner failure、backpressure、`InjectionStarted`前の`FinishWithAction` failureは次の一規則だけを使う。
trigger downを抑止していなければ`Cancel`し、現在eventと以後のeventをpassする。
trigger downを抑止済みならterminal `Replay(Trigger)`を選び、recognition sessionを閉じてInput-owned replay待機へ移る。
対応するphysical up以外のeventだけをpassし、新規gestureは開始しない。
対応するupが現在eventならその場で抑止し、まだdownなら到着まで待って抑止してから、reserved emergency slotでsynthetic down/upを一度だけ送りbutton pairをbalanceしてbypassへ移る。
replayも保証できない状態では、新規抑止を即時停止してdiagnostic faultを残す。
無限retryやsilent fallbackは行わない。

## State invariants

- 一つのmutable factを複数ownerへ保存しない。
- Engine全体を包むglobal mutable stateを作らない。
- owner間で共有するconfigはimmutable snapshotであり、revisionとgenerationは一つの値から導出する。
- gesture transitionは`Continue`、`ContinueWithAction`、`Complete`、`FinishWithAction`、`Replay`、`Cancel`のclosed enum一つで表し、一eventで一variantだけを選ぶ。
- accepted済みhold actionはsessionを継続し、trigger upは`Complete`してreplayしない。
- session-bound targetが`Ready`になる前にactionをacceptせず、activationとactionの順序は同じExecutor FIFOが所有する。
- accepted action completion recordは一session一枠で、recognition session終了後もterminal resultと必要なReplay cleanupまでInputが保持する。
- Renderer generationはInput generationより進まず、終了済みgenerationを再表示しない。
- candidate config revisionは最大一つで、`Abort`、`Applied`、またはprocess終了によってだけ解放する。`Commit` deliveryだけでは解放しない。
- accepted action、render lifecycle、replay、committed config、shutdownをsilent dropしない。
- shutdownはidempotentで、hook/event tapを先にpass-through状態へ移してからownerをjoinする。

## Consequences

遅いcontrol処理とhot pathのfailure domainを分離できる。
mailbox capacity、overflow、health transitionを明示的にtestする必要がある。
Actor frameworkのsupervisionやroutingは得られないが、必要な契約だけを小さなenumとowner loopで実装できる。
