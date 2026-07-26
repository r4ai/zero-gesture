# ADR 0005: Gate delivery on contracts, privacy, performance, and complexity

- Status: Accepted
- Date: 2026-07-26

## Context

マルチプラットフォーム化はprocess、configuration、input、rendering、packagingを置き換える。
既存testやcoverageだけをcontract全体と見なすと、Win32 integration、failure behavior、常駐性能の回帰を見落とす。
また、性能を理由にstate、thread、queue、crate、testを増やすと、複雑さを別の場所へ移しただけになる。

このADRは、変更前の測定条件、外部contractから導くtest obligation、performance/privacy acceptance、PR依存順を固定する。

## Logging and privacy contract

通常logは、問題の層と時刻を切り分けるため次を含む。

- process mode、executable/Engine/protocol/config version
- ownerのstart、stop、unexpected exit、health transition
- permission状態の変化
- config revision、migration結果、validation error category
- queue high-water mark、coalesced render point数、overflow/fail-open遷移
- callback latency histogramとaction dispatch latency histogram
- event tap/hook disable reasonとOS API error code
- renderer、executor、IPCのdegraded reason

通常logへ次を含めない。

- raw mouse coordinateまたはtrail point
- 押されたkeyまたは注入した完全なshortcut
- window title
- config documentまたはIPC payload
- user file pathの不要な全体

logはlocal、size-bounded、rotatingとし、外部送信しない。
crash report/telemetry uploadは初期実装へ含めない。
詳細diagnostic modeはuserが明示的に時間制限付きで有効化する。
diagnostic modeでもraw key、coordinate、window title、config/IPC bodyは記録しない。
Input callbackはcounter/histogram sampleだけを更新し、formatとI/Oを別ownerで行う。

## Performance acceptance

測定はrelease build、Settings closed、default config、他のdebugger/profilerを外した状態で行う。
Windows 11 x64代表機と、release時点の最新stable macOSを搭載したApple Silicon実機の両方が対象である。
idle値は60秒warm-up後の10分間を測る。
CPUは「一つのlogical coreを完全使用した値を100%」としてprocess CPU timeから正規化し、memoryは1秒sampleのRSS/working set p95を使う。

| Metric | Acceptance |
| --- | --- |
| Settings closed WebView processes | `0` |
| Engine idle CPU mean | `< 0.2%` |
| Engine memory p95 | Windows `< 20 MiB`; macOS `< 30 MiB` |
| Input callback own elapsed time | p99 `< 100 us`; p99.9 `< 500 us` |
| Terminal input event to OS injection API call | p99 `< 2 ms` |
| Callback allocations, waits, IPC, file I/O, context queries | `0` |
| Raw input/action event loss | `0`; render intermediate points may coalesce |

callback timeはcallback entryからpass/suppress return直前までのapp own elapsed timeとし、OS scheduling delayを混ぜない。
action latencyはrelease/hold terminal eventを受けた時刻から`SendInput`/`CGEventPost`呼び出し直前までとする。

Renderer停止、full render queue、IPC flood/disconnect、blocked log sink、slow config persistence、Executor failure、permission revoke、sleep/wakeをfault-injectionする。
どのcaseでもInput callbackは上限を守るか新規抑止を停止し、Zero Gestureが原因でpointer/keyboard operationを重くしない。
performance budgetを守れない場合、visual qualityとgesture機能をdegradeさせ、OS inputを優先する。

## Baseline measurement contract

### Scope and tools

baseline commitは`c603f0d9530e8426a8d891b1745877d8eee5e154`である。
このADRだけを追加したbranchはproduct/test sourceを変更しないため、before/afterは同値である。

- formatter適用済みの`src-tauri/src/**/*.rs`と`src/**/*.{ts,tsx}`を対象にする。
- generated `src/routeTree.gen.ts`、dependencies、build output、documentsをproduct metricsから除外する。
- Rust `#[cfg(test)] mod tests`、`#[cfg(test)]` helper、`*.test.*`、`*.stories.*`をtest metricsへ分類する。
- code linesはblank lineとcomment-only lineを除くphysical lineとする。
- function/closure、cognitive、cyclomatic、PLOCは`big-code-analysis-cli 2.0.0`のstandard scopeで測る。
- analyzerはproject dependencyにせず一時environmentへ固定する。
  baselineで使用したUbuntu x86-64 wheelのSHA-256は
  `62316880b772e2be633dccb27773f3bd42b2915376d50f021dd01e38c0405a52`である。
- nested functionのaggregate値を二重加算しないよう、各nodeの値からdirect child aggregateを引いて一functionの値とする。
- logical test caseは実行runnerが列挙したcaseを数え、assertion数やparameter内の入力数をcase数にしない。

### Measured baseline

| Scope | Files | Code lines | Functions/closures | Cognitive max / sum | Cyclomatic max / sum |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rust product | 19 | 3,733 | 208 | 49 / 459 | 28 / 618 |
| TypeScript/TSX product | 35 | 4,231 | 282 | 104 / 495 | 32 / 591 |
| Product total | 54 | 7,964 | 490 | 104 / 954 | 32 / 1,209 |
| Rust tests/helpers | 9 modules | 1,799 | 80 | 2 / 7 | 3 / 87 |
| TypeScript tests/stories | 14 | 1,283 | 117 | 21 / 47 | 8 / 133 |
| Test total | 23 | 3,082 | 197 | 21 / 54 | 8 / 220 |

runnerで確認したlogical unit casesはRust 64、TypeScript 39、合計`T_u = 103`である。
automated integrationとE2Eは現在`T_i = 0`、`T_e = 0`である。
direct runtime dependenciesはCargo platform dependencyを含め10、frontend 19である。

docs-onlyのafter値は全行でbeforeと同一であり、product/test complexityを別のsourceへ移していない。

### Not measured in this ADR

| KPI | Status and reason |
| --- | --- |
| `T_r` redundant cases | 未測定。case-by-case deletionまたはmutation analysisなしに0と推測しない |
| `P`, `D` packing/duplicate assertions | 未測定。obligationごとのfailure injectionを後続test PRで固定してから数える |
| `M` surviving mutants | 未測定。mutation tool/operator/exclusion ruleがrepositoryで固定されていない |
| `F` flaky/retry cases | observed retryは0だが、繰り返しrunをしていないため未測定 |
| `H`, `I`, `R` fixture/helper/double、indirection、runtime | helper数の分類規則が未固定。後続test-harness PRで測定する |
| semantic states/transitions | current `GestureState`は2、`RuntimeState`は3だが、全owner横断の到達可能状態を形式計測していない |
| dependency edges、fan-out、cycles | Rust `use`、TS import、Tauri event名を統合するpinned analyzerがない。direct dependency数だけを記録した |
| public symbols/contracts | Rust visibilityとTauri/React/IPC boundaryの分類を後続module split後に固定する |
| runtime CPU/memory/latency | docs-only branchではrelease Engine processがまだ分離されておらず、将来条件と同じ対象を測れない |

測定不能な値をmanual estimateで埋めない。
最初のtest-harness PRで規則とtool versionを固定し、以後のproduct PRは同じscopeでbefore/afterをPR本文へ載せる。

## Current Windows contract to test obligations

このinventoryのscopeは、移行で変更されるEngine runtime、persisted config、native Windows integrationである。
Settings UI固有の39 unit casesはbaselineの`T_u`とtest complexityへ含めるが、このruntime preservation inventoryの`O`へ重複計上しない。
`Verified`はbaselineで対応するlogical Rust test caseが実行済み、`Gap`はobligationを定義済みだがintegration testが未実装である。
contract group内の独立して壊れ得るbehaviorは、後続のobligation accountingで別IDにしている。

| ID | Existing Windows contract and code evidence | Test obligation | Baseline evidence | Status |
| --- | --- | --- | --- | --- |
| W01 | `enabled=false`ではworkerを起動しない | disabled cold start、enable、disableを別caseで確認する | `ThreadRuntime::start/apply_worker_state`; G01-G03 | Gap |
| W02 | configured trigger downだけgestureを開始し抑止する | configured downがIdleからGesturing、Start/Track、suppressを返す | `idle_starts_gesture_on_configured_trigger` | Verified |
| W03 | unconfigured trigger/eventはOSへ通す | bindingなしのdownがstate/effectを変えない | `idle_ignores_unconfigured_trigger` | Verified |
| W04 | direction、mixed input、finalize、reset、最大8 step | representative boundary traceがcanonical sequenceまたはinvalidを返す | `gesture::tests::*` 6 cases | Verified |
| W05 | release matchは同じtrigger upで一度actionを決定する | match、End、suppress、execute once、Idleを同時に確認する | `executes_action_on_trigger_up_when_sequence_matches` | Verified |
| W06 | non-trigger button upはsessionを終えず抑止する | state継続、no execute/replay、suppressを確認する | `non_trigger_button_up_does_not_end_gesture` | Verified |
| W07 | wheelをrelease sequenceへ含められる | wheel stepがsequenceへ入りrelease matchする | `supports_wheel_input_in_sequence` | Verified |
| W08 | hold wheelはnotch数だけ即時実行する | repeat count、suppress、session継続を確認する | `hold_wheel_executes_immediately_with_repeat_count` | Verified |
| W09 | scoped holdはexactをwildcardより優先し、実行後resetし、click replayしない | four independent hold state transitionsを確認する | `hold_wheel_*` 4 cases | Verified |
| W10 | unmatched short clickだけ元clickをreplayし、total travelと専用thresholdを使う | zero/short/long/loop travelとthreshold独立境界を確認する | replay関連6 cases | Verified |
| W11 | wrapping tickでもsafety timeoutを判定する | wrap境界のbefore/afterを確認する | `safety_timeout_works_with_wrapping_ticks` | Verified |
| W12 | app matcherはfieldごとのcase規則、regex、OR、deterministic first、missing fieldを扱う | 各equivalence classが正しいapp ID/noneを返す | `hook::app_match::tests::*` 8 cases | Verified |
| W13 | app-specific bindingをdefaultより優先する | same traceでspecific、missing specificでdefaultを返す | `resolve_binding_prefers_app_specific_then_fallback` | Verified |
| W14 | keyboard action JSONとWindows key nameをcanonical mappingする | roundtrip、modifier/navigation/character/F1-F24/case/unknownを確認する | `executor::tests::*` 9 cases | Verified |
| W15 | default、release/hold JSON、validation、dedupe、save/loadを維持する | schema equivalence、invalid boundary、roundtripを確認する | `config::tests::*` 14 cases | Verified |
| W16 | overlay color parserは3/6 digitを読み、不正値を既定色へする | accepted formsとinvalid fallbackを確認する | `overlay::tests::*` 4 cases | Verified |
| W17 | live config replacementはprevious valueとrestart flagを返す | changedとunchangedを別caseで確認する | changedは`replace_live_config_updates_shared_state` (V64)、unchangedはG04 | Partial |
| W18 | negative hook codeとself-injected eventを常にpass-throughする | actual callback adapterへ両入力を与えnext-hook pathを確認する | `low_level_mouse_proc`; G05-G06 | Gap |
| W19 | Win32 event mapping、deferred execute/replay、suppression resultがpure effectと一致する | event equivalence classとdeferred orderを別caseで確認する | `hook/win32.rs`; G07-G12 | Gap |
| W20 | Start/Track/Label/Endがclick-through native overlayへ収束する | lifecycleとstalled renderer収束を確認する | `overlay/window.rs`; G13-G16 | Gap |
| W21 | config apply/save failureはruntimeとmemoryをrollbackし、成功時だけrestartする | worker/disk/concurrency/successを別fault caseで確認する | `apply_config_update`; G17-G20 | Gap |
| W22 | tray toggle、Settings open、quitはlifecycleとlabelを同期する | 三つのmenu operationを別caseで確認する | `tray.rs`; G21-G23 | Gap |
| W23 | window captureは一つだけactiveで、replace/cancel/one-shotする | 三つのcapture transitionを別caseで確認する | `capture.rs`; G24-G26 | Gap |
| W24 | keyboard injectionはmodifier down、key press、modifier upの順を守る | fake platform sinkでexact input sequenceを確認する | `execute_keyboard`; G27 | Gap |
| W25 | missing/invalid legacy configは現在defaultへfallbackする | missing、I/O error、不正JSONを別caseで固定し、v2 migrationで意図的変更を示す | `load_or_default`; G28-G30 | Gap |
| W26 | trigger point下のtop-level windowをactivateし、そのidentityでmatchする | point hitとforeground fallbackを別caseで確認する | `activate_window_at_point`; G31-G32 | Gap |
| W27 | `ZG_LOG_LEVEL`はtrim/case-insensitiveで既知levelだけを受理する | accepted classとunknown classを別caseで確認する | `log_config::tests::*` (V58-V59) | Verified |

### Obligation accounting

baselineでgreenになった64 Rust logical casesを、実装codeを確認したうえで一つのindependently breakable obligationとしてV01-V64へ割り当てた。
rangeはtest runnerのmodule順ではなく、次のcode ownership順である。

| IDs | Owner/contract | Count | Green evidence |
| --- | --- | ---: | --- |
| V01-V14 | config default、deserialize、validation、save/load | 14 | `config::tests::*` |
| V15-V23 | action JSONとWindows key parsing | 9 | `executor::tests::*` |
| V24-V29 | direction/mixed input/finalize/reset/max step | 6 | `gesture::tests::*` |
| V30-V37 | app matcher equivalence classesとprecedence | 8 | `hook::app_match::tests::*` |
| V38-V55 | gesture state transitions、hold、replay、timeout | 18 | `hook::state::tests::*` |
| V56-V57 | validated binding compileとdefensive invalid hold skip | 2 | `hook::tests::*` |
| V58-V59 | log level accepted/unknown classes | 2 | `log_config::tests::*` |
| V60-V63 | overlay color accepted/invalid classes | 4 | `overlay::tests::*` |
| V64 | changed live config replacement | 1 | `tests::replace_live_config_updates_shared_state` |

未検証のintegration obligationsはG01-G32である。

| IDs | Independently failing scenarios |
| --- | --- |
| G01-G03 | disabled cold start、enable transition、disable transition |
| G04 | unchanged config replacement does not restart |
| G05-G06 | negative hook code pass-through、self-injected event pass-through |
| G07-G12 | button mapping、move mapping、wheel sign/zero mapping、unknown mapping、deferred execute order、deferred replay order |
| G13-G16 | overlay start、track/label、end/clear、stalled renderer convergence |
| G17-G20 | worker apply rollback、disk save rollback、success restart once、concurrent update serialization |
| G21-G23 | tray enable toggle、Settings launch、quit |
| G24-G26 | capture replace、cancel、one-shot identity |
| G27 | keyboard injection down/press/up order |
| G28-G30 | missing config、config I/O error、invalid JSON |
| G31-G32 | point window activation/identity、foreground fallback |

Windows preservation inventoryでは`O_W = 96`、`O_Wv = 64`、contract項目にtest obligationがない数は`U_W = 0`である。
passing testをcontract全体とは見なさず、G01-G32を移行前のtest-harness PRで実装する。
後続product PRをintegration branchへ入れる時点では、影響するobligationについて`O_v / O = 100%`を必須にする。
W25のsilent fallbackなど意図的に変更するcontractは、旧behavior testを削除して数値を良くせず、migration input testと新failure contract testへ置換する。

## New architecture obligations

| ID | New contract | Required verification |
| --- | --- | --- |
| A01 | Engine/Settingsは別process、同一executable、Settings close後WebView 0 | release process E2Eとprocess tree/RSS probe |
| A02 | same-binary autostartとstable signing identity | signed/notarized macOS packaging spikeとWindows installer E2E |
| A03 | Engine single instance、Settings crash/disconnect非干渉 | concurrent launch/crash fault integration |
| A04 | typed owner message、no global mutable state、callback no-wait/no-allocation | architecture check、allocator/lock instrumentation、queue fault test |
| A05 | overload/failure時fail-open、suppressed clickはbounded replay | renderer/executor/IPC/log/permission fault matrix |
| A06 | Windows/macOSが同じcanonical traceへ同じdomain effectを返す | shared contract suiteを両targetで実行 |
| A07 | IPC frame/version/auth/size/revisionを一度だけ検証する | transport integration、malformed/fuzz corpus、peer authorization |
| A08 | config atomic save、migration backup、reinstall preservation | filesystem fault integrationとinstaller upgrade E2E |
| A09 | macOS permission revoke、event tap timeout、sleep/wakeでinputを塞がない | Apple Silicon実機system test |
| A10 | native renderer degradationはinput latencyへ波及しない | stalled renderer faultとcallback latency benchmark |
| A11 | privacy exclusionとbounded rotating log | redaction contract testとrotation integration |
| A12 | performance tableの全budgetを満たす | pinned release benchmark report for both supported platforms |

新contractは`O_A = 12`、`O_Av = 0`であり、全項目をtest obligationへ対応済みなので`U_A = 0`である。
実装前のためverification済みとは数えず、owner PRがtestを追加してから`O_Av`を進める。
foundation時点のproject totalは`O = 108`、`O_v = 64`、`U = 0`である。

## Minimum test policy

- pure domain behaviorはshared unit/contract test一層で検証する。
- transport、filesystem、owner boundaryだけintegration testにする。
- actual hook/event tap、native rendering、installer、permission、process lifecycleだけE2E/system testにする。
- 同じfailureをunitとE2Eへ理由なく複製しない。
- one logical caseへ独立scenarioを詰め込まない。
- retryでflaky testを緑にしない。
- property testを使う場合、generated input数ではなくcontract propertyとboundaryを`T`として数える。

各PRは`U`, `O`, `O_v`, `T`, `T_r`, `T_u`, `T_i`, `T_e`を更新する。
`T_r`を0と主張するには、case deletionまたはmutation evidenceが必要である。

## PR dependency order

各実装は統合branch `codex/multiplatform-engine`をbaseにした独立draft PRとし、依存PRが統合branchへ入った後にrebaseする。
一つのPRへunrelated layerをまとめない。

| Order | PR scope | Depends on | Exit gate |
| --- | --- | --- | --- |
| P0 | ADR foundation (this PR) | none | decision、contract、KPI、obligation mapping reviewed |
| P1 | contract test/performance harness | P0 | W01-W26の影響範囲が100% verified、measurement repeatable |
| P2 | platform-neutral domainとschema v2 migration | P1 | shared trace/config tests、Windows behavior parity |
| P3 | same executable two-process modes、IPC、Engine config owner | P2 | A01/A03/A07/A08、WebView 0、reinstall preservation |
| P4 | Windows owners/adaptersとfail-open移行 | P3 | W obligations 100%、fault matrix、Windows budgets |
| P5 | macOS same-binary packaging spike | P3 | ADR 0001 spike gate、signed/notarized artifact |
| P6 | macOS input/context/action/renderer/permission adapters | P5 | shared parity、Apple Silicon system tests、macOS budgets |
| P7 | distribution hardening and final cross-platform acceptance | P4 + P6 | full O_v/O 100%、privacy、installer、performance report |

P4とP5はP3後に並行できる。
P6はpackaging spikeの結果を先に必要とする。
同一binary方式が不可能だった場合、P5はfallback ADRだけを先に提出し、承認前にhelper実装へ進まない。

## Per-PR quality gate

1. external pre/post condition、invariant、failure conditionをPR本文へ列挙する。
2. 同じscope/tool/configでbefore/after KPIを測る。
3. cognitive/cyclomaticのmaxとsum、code/test lines、function数、test obligation値を必ず載せる。
4. formatter、static analysis、unit、integration、relevant E2Eを通す。
5. product complexityの増加をtest、config、type、crate、thread、queueへ移して隠さない。
6. 重複fact、dependency cycle、暗黙event名、未対応obligationを0にする。
7. 性能のためのcache、lock-free structure、parallelismは代表measurementが必要性を示した場合だけ採用する。

正しさ、data/state、dependency、cognitive、cyclomatic、code量、実測性能の順でtrade-offを判断する。
