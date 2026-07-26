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
`Failed`またはowner deathの通知はactionをacceptせずsessionを終了し、trigger down抑止済みなら`Replay(Trigger)`、未抑止なら`Cancel`とする。
action-producing event時にまだ`Pending`またはaction credit不足でもactionをacceptしない。
`ContinueWithAction` eventはpassし、trigger down抑止済みなら`Replay(Trigger)`、未抑止なら`Cancel`とする。
ただしtrigger down抑止済みで、対応するphysical trigger upが`FinishWithAction`になるeventでは、そのupも抑止して直ちに`Replay(Trigger)`する。
trigger downを抑止していない`FinishWithAction` eventだけはupをpassして`Cancel`する。
activationとaction以外へ再利用する汎用ack frameworkは作らない。

## Backpressure

backpressure policyはmessage種別ごとに固定する。

| Flow | Policy when full or slow |
| --- | --- |
| Input to Renderer points | intermediate pointをcoalesceし、latest pointを優先する |
| Input to Renderer lifecycle | lossless control laneへ送る。enqueue failureまたはRenderer actor deathではcurrent sessionを必要かつ可能ならterminal `Replay`、それ以外は`Cancel`とし、Inputをbypassへ移す。新規gestureを開始せず、SupervisorがEngineをclean terminate/restartしてOS overlay resourceを破棄する |
| Input to Executor | target `Ready`後もgesture開始時に長期action creditを取らない。`ContinueWithAction`または`FinishWithAction`で抑止する各eventの直前に一枠reserveする。取れなければ原則eventをpassして`Replay|Cancel`へ遷移するが、down抑止済みのphysical trigger upだけは抑止して直ちに`Replay`する。reserve後にacceptedとなったactionはsame-session FIFOのreserved slotへinfallibleに送り、silent lossを0にする |
| Input replay | preallocated emergency slotへ保持する。schedule不能なら新規抑止を停止し、terminal degraded stateとして報告する |
| Config to Input | disk更新前にrevisionのdelivery slotをprepare/reserveしてackを得る。予約は必ずterminal `Commit`または`Abort`で閉じる。atomic replace後はreserved slotへinfallibleな`Commit`を送り、actor invariant違反はprocessをterminate/restartしてinputをfail-openにする |
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
- Executor障害はactive sessionをtrigger抑止状態に応じて`Replay|Cancel`し、新しいgesture captureを停止してstatusをdegradedにする。
- safety timeout、panic、event tap timeoutでは現在sessionを終了し、以後のeventを通す。
- FFI callbackからRust panicやforeign exceptionを越境させない。
- injected eventにはself tagを付け、同じgestureとして再捕捉しない。

trigger downを抑止済みでsessionを完了できない場合は`Replay(Trigger)`を選び、固定上限の緊急経路で元のdown/upを一度だけreplayする。
physical triggerがまだdownならInputはrecognition sessionを終了して新規gestureを開始せず、他eventをpassしながら対応するphysical upを待つ。
そのupを抑止してからsynthetic down/upを一度だけ送り、button pairをbalanceしてbypassへ移る。
failureを対応するphysical upのcallbackで検出した場合も、そのupをOSへpassせず抑止してから直ちに同じreplayを送る。
trigger downを抑止していない`Cancel`ではphysical upを含む後続eventを変更せずpassする。
replayも保証できない状態では、新規抑止を即時停止してdiagnostic faultを残す。
無限retryやsilent fallbackは行わない。

## State invariants

- 一つのmutable factを複数ownerへ保存しない。
- Engine全体を包むglobal mutable stateを作らない。
- owner間で共有するconfigはimmutable snapshotであり、revisionとgenerationは一つの値から導出する。
- gesture transitionは`Continue`、`ContinueWithAction`、`Complete`、`FinishWithAction`、`Replay`、`Cancel`のclosed enum一つで表し、一eventで一variantだけを選ぶ。
- accepted済みhold actionはsessionを継続し、trigger upは`Complete`してreplayしない。
- session-bound targetが`Ready`になる前にactionをacceptせず、activationとactionの順序は同じExecutor FIFOが所有する。
- Renderer generationはInput generationより進まず、終了済みgenerationを再表示しない。
- prepared config revisionは最大一つで、`Commit`、`Abort`、またはprocess終了によってだけ解放する。
- accepted action、render lifecycle、replay、committed config、shutdownをsilent dropしない。
- shutdownはidempotentで、hook/event tapを先にpass-through状態へ移してからownerをjoinする。

## Consequences

遅いcontrol処理とhot pathのfailure domainを分離できる。
mailbox capacity、overflow、health transitionを明示的にtestする必要がある。
Actor frameworkのsupervisionやroutingは得られないが、必要な契約だけを小さなenumとowner loopで実装できる。
