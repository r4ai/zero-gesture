# ADR 0005: Gate delivery on contracts, privacy, performance, and complexity

- Status: Accepted
- Date: 2026-07-26

## Context

マルチプラットフォーム化はprocess、configuration、input、rendering、packagingを置き換える。
既存testやcoverageだけをcontract全体と見なすと、Win32 integration、Settings workflow、failure behavior、常駐性能の回帰を見落とす。
一方、foundation ADRだけでrepository全体の外部predicateを手作業でatomizeし、完全性を数値で主張することも再現できない。

P00で決定、既知のpreservation requirement、測定条件を記録する。
機械可読なcontract inventory、evidence mapping、再現可能な測定基盤はP01で導入する。

## P00 scope

このPRはarchitecture decisionと高レベルrequirementを記録するdocs-only foundationである。
contract metricのbaselineやverification completenessは確定しない。

- `O`、`O_v`、`U`を数えない。
- runner case数をlogical test数`T`へ読み替えない。
- 手書きtableから「全contractを列挙した」「mappingが100%である」と主張しない。
- product/test source、manifest、measurement script、raw artifactを追加しない。

既存behaviorと新architectureの高レベル領域は以下に列挙する。
P00で意図的に除外したfeature領域はないが、各領域内のatomic predicateとevidence completenessはP01まで未測定である。

## Logging and privacy contract

通常logは問題の層と時刻を切り分けるため次を含む。

- process mode、executable/Engine/protocol/config version
- ownerのstart、stop、unexpected exit、health transition
- permission状態の変化
- config revision、migration結果、validation error category
- queue high-water mark、coalesced render point数、overflow/fail-open遷移
- callback latencyとaction dispatch latencyのhistogram
- event tap/hook disable reason、OS API error code、renderer/executor/IPCのdegraded reason

通常logへraw mouse coordinate、trail point、押されたkey、完全なshortcut、window title、config document、IPC payloadを含めない。
user file pathも診断に不要な部分を記録しない。
logはlocal、size-bounded、rotatingとし、外部送信しない。
crash report/telemetry uploadは初期実装へ含めない。

詳細diagnostic modeはuserが明示的に時間制限付きで有効化する。
diagnostic modeでもraw key、coordinate、window title、config/IPC bodyは記録しない。
Input callbackはcounter/histogram sampleだけを更新し、formatとI/Oを別ownerで行う。

## Performance acceptance

測定はrelease build、Settings closed、default config、debugger/profilerなしで行う。
Windows 11 x64代表機と、release時点の最新stable macOSを搭載したApple Silicon実機の両方を対象とする。
idle値は60秒warm-up後の10分間を測る。
CPUは一つのlogical coreを完全使用した値を100%としてprocess CPU timeから正規化し、memoryは1秒sampleのRSS/working set p95を使う。

| Metric | Acceptance |
| --- | --- |
| Settings closed WebView processes | `0` |
| Engine idle CPU mean | `< 0.2%` |
| Engine memory p95 | Windows `< 20 MiB`; macOS `< 30 MiB` |
| Input callback own elapsed time | p99 `< 100 us`; p99.9 `< 500 us` |
| Terminal input event to OS injection API call | p99 `< 2 ms` |
| Callback allocations, waits, IPC, file I/O, context queries | `0` |
| App受領済みinput、accepted action、replay、render lifecycle、committed config、shutdownのsilent loss | `0`; 中間render pointとmetrics sampleだけcoalesce/drop可 |

callback timeはentryからpass/suppress return直前までのapp own elapsed timeとし、OS scheduling delayを混ぜない。
action latencyはrelease/hold terminal eventから`SendInput`/`CGEventPost`呼び出し直前までとする。

Renderer停止、full queue、IPC flood/disconnect、blocked log sink、slow config persistence、Executor failure、permission revoke、sleep/wakeをfault-injectionする。
どのcaseでもInput callbackは上限を守るか新規抑止を停止し、Zero Gestureがpointer/keyboard operationを重くしない。
budgetを守れない場合はvisual qualityとgesture機能をdegradeさせ、OS inputを優先する。

## Provisional complexity snapshot

次の値はP00作成時の参考snapshotであり、再現可能なquality gateではない。
repositoryにversioned measurement script、raw analyzer output、集計artifactがないため、第三者が同じ結果を再生成できる状態ではない。

- baseline commit: `c603f0d9530e8426a8d891b1745877d8eee5e154`
- analyzer: `big-code-analysis-cli 2.0.0`
- Ubuntu x86-64 wheel SHA-256: `62316880b772e2be633dccb27773f3bd42b2915376d50f021dd01e38c0405a52`
- product scope: formatted `src-tauri/src/**/*.rs`と`src/**/*.{ts,tsx}`
- exclusions: generated `src/routeTree.gen.ts`、dependencies、build output、documents
- test scope: Rust `#[cfg(test)]`、`*.test.*`、`*.stories.*`
- code lines: blank lineとcomment-only lineを除くphysical line
- nested function: child aggregateを親へ二重加算しない

| Scope | Files | Code lines | Functions/closures | Cognitive max / sum | Cyclomatic max / sum |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rust product | 19 | 3,733 | 208 | 49 / 459 | 28 / 618 |
| TypeScript/TSX product | 35 | 4,231 | 282 | 104 / 495 | 32 / 591 |
| Product total | 54 | 7,964 | 490 | 104 / 954 | 32 / 1,209 |
| Rust tests/helpers | 9 modules | 1,799 | 80 | 2 / 7 | 3 / 87 |
| TypeScript tests/stories | 14 | 1,283 | 117 | 21 / 47 | 8 / 133 |
| Test total | 23 | 3,082 | 197 | 21 / 54 | 8 / 220 |

P01は同じscope、tool version、hash、集計規則をversioned scriptへ移し、raw outputとsummary artifactを保存する。
そのartifactがreview環境で再生成できて初めて、complexity before/afterをmerge gateにする。
P00へ測定scriptを遡及追加しない。

## Observed runner cases and unmeasured metrics

runnerが列挙し実行したcase数は、contract metricではなく観測値`R_runner`としてだけ記録する。

| Runner project | Classification | Executed cases |
| --- | --- | ---: |
| Cargo crate tests | unit-like | 64 |
| Vitest `unit` | unit-like | 39 |
| Vitest `storybook (chromium)` | browser component | 46 |
| Total `R_runner` | unit-like 103、browser component 46、E2E 0 | 149 |

Storybook 46件のうち45件はrender smoke、1件だけが`play` assertionを持つ。
render smokeのgreenをbehavioral contract verificationへ換算しない。
既存caseには複数の独立failure reasonがpackedされているため、runner case数をstrict logical test数へ換算しない。

| KPI | P00 value | Reason |
| --- | --- | --- |
| `O`, `O_v`, `U` | `null` / unmeasured | versioned atomic contract manifestとevidence mappingがない |
| `T`, `T_u`, `T_i`, `T_e` | `null` / unmeasured | logical scenarioとfailure reasonをatomizeしていない |
| `T_r` | `null` / unmeasured | deletionまたはmutation evidenceがない |
| `P`, `D` | `null` / unmeasured | packed/duplicate assertionの全project分類がない |
| `M`, `F` | `null` / unmeasured | mutation operator、repeat-run、retry規則が未固定 |
| `H`, `I`, `R` | `null` / unmeasured | helper/double、indirection、runtimeの分類規則が未固定 |
| runtime CPU/memory/latency | `null` / unmeasured | 分離後Engine processがまだ存在しない |

## Existing Windows and Settings preservation requirements

次は移行で保存対象となる高レベルfeature領域である。
項目数は`O`ではなく、verification済みという意味も持たない。

### Windows Engine

- general `enabled`のload、cold start、enable/disable、worker lifecycle
- trayのenable/disable toggle、enabled label同期、Settings menuとleft-clickによるSettings起動、hook/overlayを停止して終了するgraceful Quit
- validなdefault/v1 configurationのvalidation、save/load、observable behaviorを保つmigration
- invalid fileのread/JSON/validation failureではsilent default/correctionを保存せず、fileを非破壊のままlast known validを維持するかEngineをdisabled/fail-openにしてdiagnostic recoveryへ移る
- application definitionとmatcherのcreate/read/update/delete、label、OR matching、default fallback、app-specific precedence
- matcher target/method/value、process name、window class、title、exact/contains/regexの意味
- gesture bindingのcreate/read/update/delete、stable ID、label、順序
- left/right/middle trigger、release/hold mode、方向・wheel・click sequence、最大長、hold step制約
- unmatched short click replay、travel threshold、movement pass-through、injected-event exclusion、safety timeout
- keyboard shortcut validation、key ordering、`SendInput`実行、partial injection failure
- native trailとlabelのstart/track/finish/clear、appearance。point overflowだけをcoalesce/dropし、lifecycle failure/actor death時は必要なら`Replay`、それ以外は`Cancel`、Input bypass、Engine clean restartとしてheadless継続しない
- window captureのstart/cancel/replace、non-target event pass-through、一回だけのresult、window identity

Windows rendererは現行GDIを維持する。
GDIがperformance acceptanceを満たさないことを測定した場合だけ、別ADR/PRでrenderer変更を検討する。
Direct2D/DirectCompositionをこの移行の先行要件にしない。

### Settings

- general `enabled`とappearance/recognition fieldの表示、draft編集、validation、保存、再読込
- applicationとmatcherのCRUD、cancel、route遷移、default applicationの制約
- gestureのCRUD、並べ替え、label、trigger、release/hold mode、sequence、hold step編集
- shortcut preset、modifier/key catalog、manual shortcut編集、空/不正shortcutの扱い
- initial load、save、stale revision/conflict、backend errorをsuccess表示しないこと
- draft保持と破棄、import apply/error、export内容とfailure
- window captureのstart/cancel、対応するcapture IDのresultだけを対象matcher draftへ反映し、stale/cancelled resultを無視すること
- typed `OpenConfigDirectory`でEngine-owned config pathをOS file managerに開き、arbitrary pathを送らないこと

## New architecture requirements

- 同じTauri executableのEngine/Settings別process mode、Engineのwindow/WebView 0、Settings close時のprocess exit
- userごとのEngine単一起動、Settings crash/disconnectからのEngine独立、Engineだけのinput権限とconfig/IPC ownership
- bounded typed message passing、single-owner state、Input callbackのnormalize/evaluate→essential credit reserve/send→best-effort render→return順序、fail-open
- 各抑止対象hold event直前のnonblocking action credit、credit不足eventのpass、trigger down抑止済みなら`Replay`/未抑止なら`Cancel`、hold中physical upでのbalanced replay、accepted actionのsilent loss 0
- callback外Context workerのpointer sample、window/binding/target事前解決、point tolerance/age/generation/handle validityでのfresh判定、session-bound target
- target activationとactionを同じbounded FIFO Executor laneへ載せ、activation result成功後だけactionをacceptし、activation credit不足/pending/failure/owner deathではevent passと`Replay|Cancel`にすること
- fresh Contextでapp-specific matchなしの場合だけdefault bindingを使い、snapshotがmissing/stale/timeout/invalidならtriggerをpassして現行callback内同期query/activationを保存しないこと
- OS/Tauri/IPCから独立したdomainとWindows/macOS native adapter、同じcanonical traceの同じdomain effect
- non-terminal `Continue|ContinueWithAction(ActionId, repeat)`とterminal `Complete|FinishWithAction(ActionId)|Replay(Trigger)|Cancel`のclosed transition enum、独立したrender effect
- schema version、application/binding以外のtyped overrideのwhole-field replacement、record単位`Shared|Windows|Macos` variant、lossless migration、platform capability validation
- authenticated local IPC、protocol/revision conflict、一般frame 1 MiB上限、config専用bounded chunk transfer、Windows pipeのcurrent-user DACLとremote rejection、macOS socketのmode/peer UID
- configのvalidate/compile、terminal `Commit | Abort`付きInput reservation、temp fsync、atomic replace、metadata sync、reserved `Commit`、success responseの順序
- replace前failureの`Abort`、replace後metadata sync failureの`SuccessWithDurabilityWarning`、reserved `Commit` invariant違反時のterminate/restart
- 通常のprocess crashはreplace前=旧/replace後=新active file、replace後metadata sync前のsystem/power crashはold/new不確定としてvalid candidate recoveryまたはdisabled/fail-openにすること
- typed `OpenConfigDirectory` request/responseでEngine-owned pathだけをOS file managerに開くこと
- Engine-owned window capture、replacement/early cancel/stale resultのtyped protocol
- user-run reinstallでconfigを保持し、automatic updater責務を持たないこと
- Apple Siliconとrelease時点の最新macOS、署名/notarization、Input Monitoring/Accessibility、権限不足時のfail-open
- WindowsはGDIを維持し、macOSは別のAppKit/Core Animation native adapterを持つこと。renderer lifecycle failure/actor deathではheadless継続せず、必要なら`Replay`、それ以外は`Cancel`、Input bypass、Engine clean restartとすること
- logging/privacy、performance acceptance、fault injection、diagnostic cause isolation

## P01 machine-readable contract gate

P01はversioned machine-readable manifestを導入し、そこで初めて全project contractをatomizeする。
各entryは一つの独立して反証可能なpredicateだけを持ち、stable ID、scope、source、owner PR、verification evidenceまたは明示gapを記録する。

- compound predicateをschema validationまたはreviewで分割する。
- duplicate IDとduplicate semantic predicateを検出する。
- runner case、source link、test name、system probe、raw artifactをevidenceとして参照する。
- 一つのtestが複数predicateを検証してもよいが、各predicateのassertion/evidenceを個別に示す。
- failing、missing、stale evidenceをgreenへ換算しない。
- preservation requirementとnew architecture requirementを同じversioned scopeで監査する。

P01のexit gateは、manifest schemaとscopeがreview済みで、全entryがevidenceまたは明示gapへmapされ、定義したscope内で`U=0`であることとする。
この時点で初めて`O`、`O_v`、`U`を算出する。
logical test manifestも同じfailure-reason規則で整備し、`T`、`T_u`、`T_i`、`T_e`、`P`、`D`を非nullにする。
versioned measurement scriptとraw artifactの再生成もP01のexit gateに含める。

P01 manifestは少なくとも次を独立predicateへ分ける。

- hold action後のsession継続、連続wheelごとのaction、multi-notch `repeat`、trigger upの`Complete`とno replay
- hold/release action credit成功、credit不足時のevent passと`Replay|Cancel`分岐、hold中physical upのbalanced replay、accepted action loss 0
- Context cacheのpoint tolerance、age、generation、handle validity、fresh時開始、各stale/missing時pass
- same-session FIFOでのactivation attempt/result先行、success後action acceptance、activation credit不足/pending/failure/owner deathのno-acceptと`Replay|Cancel`
- renderer lifecycle enqueue failureとactor death、Input-owned sessionの`Replay|Cancel`、Input bypass、新規gesture禁止、clean restart、point coalescing
- config `Prepare`後の`Commit | Abort`、replace前failure、replace後durability warning、reserved delivery invariant、process crashとsystem/power crash recovery
- config upload/downloadのchunk size、single in-flight、stream ID/revision/order/length/hash、abort/timeout、oversized valid v1の非破壊recovery/export
- invalid config startup、last known valid維持、file非破壊、diagnostic recovery
- typed `OpenConfigDirectory`のEngine-owned pathとarbitrary-path拒否

## Minimum test policy

- pure domain behaviorはshared unit/contract test一層で検証する。
- transport、filesystem、owner boundaryだけintegration testにする。
- actual hook/event tap、native rendering、installer、permission、process lifecycleだけE2E/system testにする。
- 同じfailureをunitとE2Eへ理由なく複製しない。
- one logical caseへ独立scenarioやfailure reasonを詰め込まない。
- retryでflaky testをgreenにしない。
- property testではgenerated input数でなくcontract propertyとboundaryを`T`として数える。

## PR dependency order

各実装は統合branch `codex/multiplatform-engine`をbaseにした独立draft PRとし、依存PRが統合branchへ入った後にrebaseする。

| Order | PR scope | Depends on | Exit gate |
| --- | --- | --- | --- |
| P00 | ADR foundation (this PR) | none | decision、高レベルrequirement、未測定項目がreview済み |
| P01 | contract manifest、test/performance/complexity harness | P00 | versioned manifest、`U=0`、logical test分類、再現可能script/raw artifact |
| P02 | platform-neutral domainとschema v2 migration | P01 | shared trace/config tests、valid v1 behavior parity、invalid-config recovery exception |
| P03 | same-executable process modes、IPC、Engine config owner | P02 | process/IPC/config/capture evidence、WebView 0、reinstall preservation |
| P04 | Windows owners/adaptersとfail-open移行 | P03 | 影響するmanifest requirement、fault matrix、Windows budgets |
| P05 | macOS same-binary packaging spike | P03 | ADR 0001 spike gate、signed/notarized artifact |
| P06 | macOS input/context/action/renderer/permission adapters | P05 | shared parity、Apple Silicon system tests、macOS budgets |
| P07 | distribution hardening and final acceptance | P04 + P06 | required manifest evidence、privacy、installer、performance report |

P04とP05はP03後に並行できる。
P06はpackaging spikeの結果を先に必要とする。
同一binary方式が不可能だった場合、P05はfallback ADRだけを先に提出し、承認前にhelper実装へ進まない。

## Per-PR quality gate

P01以後の各product PRは次を満たす。

1. 影響するmanifest entryとexternal pre/post condition、invariant、failure conditionを示す。
2. versioned scriptと同じscope/tool/configでbefore/after artifactを生成する。
3. formatter、static analysis、unit、integration、relevant E2Eを通す。
4. product complexityをtest、config、type、crate、thread、queueへ移して隠さない。
5. 未対応evidence、dependency cycle、暗黙event名を残さない。
6. cache、lock-free structure、parallelismは代表measurementが必要性を示した場合だけ採用する。

正しさ、data/state、dependency、cognitive、cyclomatic、code量、実測性能の順でtrade-offを判断する。
