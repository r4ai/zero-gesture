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
| App受領済みinput、accepted action、replay、render lifecycle、committed config、shutdownのsilent loss | `0`; 中間render pointとmetrics sampleだけcoalesce/drop可 |

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
- runnerが列挙・実行したcase数を`R_runner`として別に記録する。
- logical `T`は一つのまとまったscenarioとfailure reasonを一件とし、runner function、assertion、parameter数から推測しない。

### Measured baseline

| Scope | Files | Code lines | Functions/closures | Cognitive max / sum | Cyclomatic max / sum |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rust product | 19 | 3,733 | 208 | 49 / 459 | 28 / 618 |
| TypeScript/TSX product | 35 | 4,231 | 282 | 104 / 495 | 32 / 591 |
| Product total | 54 | 7,964 | 490 | 104 / 954 | 32 / 1,209 |
| Rust tests/helpers | 9 modules | 1,799 | 80 | 2 / 7 | 3 / 87 |
| TypeScript tests/stories | 14 | 1,283 | 117 | 21 / 47 | 8 / 133 |
| Test total | 23 | 3,082 | 197 | 21 / 54 | 8 / 220 |

runnerが列挙し実行したcaseは次のとおりである。

| Runner project | Runner classification | Executed runner cases | Evidence |
| --- | --- | ---: | --- |
| Cargo crate tests | unit | 64 | `cargo test --all-targets` |
| Vitest `unit` | unit | 39 | `vitest list/run --project unit` |
| Vitest `storybook (chromium)` | browser component integration | 46 | `vitest list/run --project storybook` |

観測値は`R_runner = 149`であり、runner分類はunit-like 103、browser component 46、E2E 0である。
Storybookの46 caseはdefault exportやhelperではなく、runnerが列挙してgreenにしたnamed storyだけを数えた。
うち1 caseだけが`play`内にbehavior assertionを持ち、残り45 caseはbrowser render smokeである。
render smokeのgreenをbehavioral contractのverificationへ自動換算しない。
既存testには一つのfunction/play内へ複数failure reasonを詰めたものがあるため、
厳密な`T`、`T_u`、`T_i`、`T_e`へrunner case数を代入しない。
direct runtime dependenciesはCargo platform dependencyを含め10、frontend 19である。

docs-onlyのafter値は全行でbeforeと同一であり、product/test complexityを別のsourceへ移していない。

### Not measured in this ADR

| KPI | Status and reason |
| --- | --- |
| `T`, `T_u`, `T_i`, `T_e` | 未測定。`R_runner=149`の層別内訳は測定済みだが、packed failure reasonをatomizeしていないためlogical case数へ読み替えない |
| `T_r` redundant cases | 未測定。case-by-case deletionまたはmutation analysisなしに0と推測しない |
| `P`, `D` packing/duplicate assertions | 未測定。既存のpackingは確認済みだが全caseをatomizeしておらず、件数はP1 test-harness PRで固定する |
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

scopeは移行で変更されるEngine runtime、persisted config、native Windows integrationと、
process境界を越えるSettings workflowである。
obligationは独立して壊れ得る外部predicateごとに一つのIDを付ける。
一つのgreen caseが複数predicateを明示assertする場合は複数obligationをverifiedにできるが、
caseがgreenという事実だけでobligationを一つ増やさない。
`Verified`は対応predicateをbaseline testがassert済み、`Gap`は括弧内のplanned caseへ割当済みである。

### Runtime and control

| ID | Atomic contract | Source | Verification |
| --- | --- | --- | --- |
| W-R01 | disabled cold startはworkerを作らない | [lib.rs](../../src-tauri/src/lib.rs#L141-L154) | Gap (`G-R01`) |
| W-R02 | disabledからenableするとworkerを開始する | [lib.rs](../../src-tauri/src/lib.rs#L210-L254) | Gap (`G-R02`) |
| W-R03 | runningからdisableするとworkerを停止する | [lib.rs](../../src-tauri/src/lib.rs#L210-L254) | Gap (`G-R03`) |
| W-R04 | changed live configはrestart requiredを返す | [lib.rs](../../src-tauri/src/lib.rs#L258-L272) | Verified (`replace_live_config_updates_shared_state`) |
| W-R05 | changed live configはprevious documentを返す | [lib.rs](../../src-tauri/src/lib.rs#L258-L272) | Verified (`replace_live_config_updates_shared_state`) |
| W-R06 | changed live configはnext documentを共有stateへ保存する | [lib.rs](../../src-tauri/src/lib.rs#L258-L272) | Verified (`replace_live_config_updates_shared_state`) |
| W-R07 | unchanged configはworkerをrestartしない | [commands.rs](../../src-tauri/src/commands.rs#L56-L84) | Gap (`G-R04`) |
| W-R08 | concurrent config updateを直列化する | [commands.rs](../../src-tauri/src/commands.rs#L47-L50) | Gap (`G-R05`) |
| W-R09 | worker apply failureはmemoryとworker stateをrollbackする | [commands.rs](../../src-tauri/src/commands.rs#L56-L67) | Gap (`G-R06`) |
| W-R10 | persistence failureはmemoryとworker stateをrollbackする | [commands.rs](../../src-tauri/src/commands.rs#L70-L73) | Gap (`G-R07`) |
| W-R11 | successful updateだけをdiskへ保存する | [commands.rs](../../src-tauri/src/commands.rs#L70-L86) | Gap (`G-R08`) |
| W-R12 | successful updateはtray labelを同期する | [commands.rs](../../src-tauri/src/commands.rs#L75-L75) | Gap (`G-R08`) |
| W-R13 | successful updateは`config-updated`をemitする | [commands.rs](../../src-tauri/src/commands.rs#L77-L78) | Gap (`G-R08`) |
| W-R14 | shutdown後のconfig updateを拒否する | [commands.rs](../../src-tauri/src/commands.rs#L52-L54) | Gap (`G-R09`) |
| W-R15 | shutdownはidempotentにworkerをjoinする | [lib.rs](../../src-tauri/src/lib.rs#L173-L207) | Gap (`G-R10`) |
| W-R16 | tray toggleはenabledを反転して同じupdate pathへ渡す | [tray.rs](../../src-tauri/src/tray.rs#L104-L131) | Gap (`G-R11`) |
| W-R17 | trayのSettings操作は既存windowをfocusまたは生成する | [tray.rs](../../src-tauri/src/tray.rs#L152-L190) | Gap (`G-R12`) |
| W-R18 | trayのQuit操作はworker shutdown後に終了する | [tray.rs](../../src-tauri/src/tray.rs#L68-L82) | Gap (`G-R13`) |

### Persisted configuration

| ID | Atomic contract | Source | Verification |
| --- | --- | --- | --- |
| W-C01 | default `enabled`はtrue | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C02 | default trail colorは`#00BFFF` | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Gap (`G-C01`) |
| W-C03 | default trail thicknessは`3.0` | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Gap (`G-C01`) |
| W-C04 | default safety timeoutは定数値 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C05 | default minimum segmentは定数値 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C06 | default direction confirmationは定数値 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C07 | default axis ambiguityは定数値 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C08 | default replay distanceは定数値 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C09 | default label font familyは定数値 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C10 | default label font sizeは定数値 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C11 | default label font weightは定数値 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C12 | default label paddingは定数値 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C13 | default app definitionsはempty | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Gap (`G-C01`) |
| W-C14 | default binding catalogは既存10 mappingと一致する | [config.rs](../../src-tauri/src/config.rs#L555-L690) | Verified (`default_bindings_match_legacy_defaults`) |
| W-C15 | default catalogのtriggerはright click | [config.rs](../../src-tauri/src/config.rs#L555-L690) | Verified (`default_bindings_match_legacy_defaults`) |
| W-C16 | default binding sequenceはnon-emptyかつ最大step以内 | [config.rs](../../src-tauri/src/config.rs#L692-L710) | Verified (`default_contains_expected_values`) |
| W-C17 | release JSONはtriggerを保持する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_config_with_sequence_bindings`) |
| W-C18 | release JSONはordered sequenceを保持する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_config_with_sequence_bindings`) |
| W-C19 | release JSONはoptional labelの有無を保持する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_config_with_sequence_bindings`) |
| W-C20 | hold JSONはhold modeを保持する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_config_with_hold_binding`) |
| W-C21 | hold JSONはwheel stepを保持する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_config_with_hold_binding`) |
| W-C22 | wildcard hold JSONはempty prefix sequenceを保持する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_config_with_hold_binding`) |
| W-C23 | scoped hold JSONはprefix sequenceを保持する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_config_with_sequence_scoped_hold_binding`) |
| W-C24 | app definitionとmatcherをJSONから復元する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_config_with_apps_and_per_app_bindings`) |
| W-C25 | per-app binding setをJSONから復元する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_config_with_apps_and_per_app_bindings`) |
| W-C26 | explicit `enabled=false`を保持する | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_json_with_enabled_false`) |
| W-C27 | omitted bindingsはdefault catalogを使う | [config.rs](../../src-tauri/src/config.rs#L168-L306) | Verified (`deserialize_json_with_enabled_false`) |
| W-C28 | invalid safety timeoutはdefaultへnormalizeする | [config.rs](../../src-tauri/src/config.rs#L308-L373) | Verified (`validate_normalizes_numeric_thresholds`) |
| W-C29 | invalid minimum segmentはdefaultへnormalizeする | [config.rs](../../src-tauri/src/config.rs#L308-L373) | Verified (`validate_normalizes_numeric_thresholds`) |
| W-C30 | invalid direction confirmationはdefaultへnormalizeする | [config.rs](../../src-tauri/src/config.rs#L308-L373) | Verified (`validate_normalizes_numeric_thresholds`) |
| W-C31 | invalid axis ambiguityはdefaultへnormalizeする | [config.rs](../../src-tauri/src/config.rs#L308-L373) | Verified (`validate_normalizes_numeric_thresholds`) |
| W-C32 | invalid replay distanceはdefaultへnormalizeする | [config.rs](../../src-tauri/src/config.rs#L308-L373) | Verified (`validate_normalizes_numeric_thresholds`) |
| W-C33 | missing default setはempty setとして挿入する | [config.rs](../../src-tauri/src/config.rs#L308-L373) | Verified (`validate_inserts_empty_default_bindings_when_missing`) |
| W-C34 | unknown app IDのbinding setを除去する | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_removes_bindings_for_unknown_apps`) |
| W-C35 | empty release sequenceを除去する | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_filters_invalid_release_bindings_and_deduplicates`) |
| W-C36 | trigger自身を含むrelease sequenceを除去する | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_filters_invalid_release_bindings_and_deduplicates`) |
| W-C37 | consecutive same moveを含むreleaseを除去する | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_filters_invalid_release_bindings_and_deduplicates`) |
| W-C38 | duplicate release patternはfirst bindingを残す | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_filters_invalid_release_bindings_and_deduplicates`) |
| W-C39 | stepなしholdを除去する | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_filters_invalid_hold_bindings_and_deduplicates`) |
| W-C40 | unsupported hold stepを除去する | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_filters_invalid_hold_bindings_and_deduplicates`) |
| W-C41 | trigger自身を含むhold prefixを除去する | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_filters_invalid_hold_bindings_and_deduplicates`) |
| W-C42 | consecutive same moveを含むhold prefixを除去する | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_filters_invalid_hold_bindings_and_deduplicates`) |
| W-C43 | duplicate hold patternはfirst bindingを残す | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_filters_invalid_hold_bindings_and_deduplicates`) |
| W-C44 | empty binding IDを除去する | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_drops_bindings_with_empty_or_duplicate_id`) |
| W-C45 | duplicate binding IDはfirst bindingを残す | [config.rs](../../src-tauri/src/config.rs#L409-L553) | Verified (`validate_drops_bindings_with_empty_or_duplicate_id`) |
| W-C46 | saveはmissing config directoryを作る | [config.rs](../../src-tauri/src/config.rs#L751-L757) | Verified (`save_creates_directory_and_roundtrips_from_config_dir`) |
| W-C47 | save後のloadはdocumentをroundtripする | [config.rs](../../src-tauri/src/config.rs#L725-L757) | Verified (`save_creates_directory_and_roundtrips_from_config_dir`) |
| W-C48 | missing fileはdefault documentへfallbackする | [config.rs](../../src-tauri/src/config.rs#L725-L733) | Gap (`G-C02`) |
| W-C49 | config read errorはdefault documentへfallbackする | [config.rs](../../src-tauri/src/config.rs#L725-L733) | Gap (`G-C03`) |
| W-C50 | invalid JSONはdefault documentへfallbackする | [config.rs](../../src-tauri/src/config.rs#L725-L733) | Gap (`G-C04`) |

### Gesture, action, and application semantics

| ID | Atomic contract | Source | Verification |
| --- | --- | --- | --- |
| W-A01 | keyboard actionはtagged JSONでroundtripする | [executor.rs](../../src-tauri/src/executor.rs#L10-L22) | Verified (`action_keyboard_serialization_roundtrip`) |
| W-A02 | canonical keyboard JSONをordered key listへdecodeする | [executor.rs](../../src-tauri/src/executor.rs#L10-L22) | Verified (`action_keyboard_deserialize_from_json`) |
| W-A03 | base modifier namesをWin32 keyへmapする | [executor.rs](../../src-tauri/src/executor.rs#L34-L87) | Verified (`parse_key_modifiers`) |
| W-A04 | base navigation namesをWin32 keyへmapする | [executor.rs](../../src-tauri/src/executor.rs#L34-L87) | Verified (`parse_key_navigation`) |
| W-A05 | function key domainはexactly F1-F24 | [executor.rs](../../src-tauri/src/executor.rs#L42-L50) | Verified (`parse_key_function_keys`) |
| W-A06 | F1-F24はcontiguous virtual-key codesへmapする | [executor.rs](../../src-tauri/src/executor.rs#L42-L50) | Verified (`parse_key_function_keys_use_contiguous_vk_codes`) |
| W-A07 | lowercase lettersをWin32 keyへmapする | [executor.rs](../../src-tauri/src/executor.rs#L76-L83) | Verified (`parse_key_characters`) |
| W-A08 | digitsをWin32 keyへmapする | [executor.rs](../../src-tauri/src/executor.rs#L76-L83) | Verified (`parse_key_characters`) |
| W-A09 | unknownまたはempty key nameは拒否する | [executor.rs](../../src-tauri/src/executor.rs#L34-L87) | Verified (`parse_key_unknown_returns_none`) |
| W-A10 | key name parsingはASCII case-insensitive | [executor.rs](../../src-tauri/src/executor.rs#L34-L87) | Verified (`parse_key_case_insensitive`) |
| W-A11 | modifier aliasesをcanonical Win32 keyへmapする | [executor.rs](../../src-tauri/src/executor.rs#L53-L59) | Gap (`G-A01`) |
| W-A12 | `space`をWin32 space keyへmapする | [executor.rs](../../src-tauri/src/executor.rs#L74-L74) | Gap (`G-A02`) |
| W-A13 | injectionはkey downをconfig順、key upをreverse順に送る | [executor.rs](../../src-tauri/src/executor.rs#L148-L174) | Gap (`G-A03`) |
| W-A14 | navigation/Win key injectionへextended flagを付ける | [executor.rs](../../src-tauri/src/executor.rs#L210-L261) | Gap (`G-A03`) |
| W-A15 | unknown keyをskipし、valid keyが0ならinjectionしない | [executor.rs](../../src-tauri/src/executor.rs#L148-L164) | Gap (`G-A04`) |
| W-A16 | navigation aliasesをcanonical Win32 keyへmapする | [executor.rs](../../src-tauri/src/executor.rs#L60-L74) | Gap (`G-A01`) |
| W-G01 | straight traceを一方向へrecognizeする | [gesture.rs](../../src-tauri/src/gesture.rs#L72-L126) | Verified (`recognizes_single_direction`) |
| W-G02 | direction changeをordered multi-segmentへrecognizeする | [gesture.rs](../../src-tauri/src/gesture.rs#L72-L126) | Verified (`recognizes_multi_segment_direction_sequence`) |
| W-G03 | movementとwheel inputを一つのordered sequenceへ混在できる | [gesture.rs](../../src-tauri/src/gesture.rs#L128-L174) | Verified (`supports_mixed_movement_and_input_steps`) |
| W-G04 | configured maxを超えたsequenceはinvalidになる | [gesture.rs](../../src-tauri/src/gesture.rs#L134-L174) | Verified (`over_max_steps_invalidates_sequence`) |
| W-G05 | finalizeはvalidなcurrent directionをflushする | [gesture.rs](../../src-tauri/src/gesture.rs#L163-L174) | Verified (`finalize_flushes_current_direction`) |
| W-G06 | resetはcurrent sequenceをemptyにする | [gesture.rs](../../src-tauri/src/gesture.rs#L176-L187) | Verified (`reset_sequence_clears_steps_and_accepts_new_input`) |
| W-G07 | reset後はnew inputをfresh sequenceとして受理する | [gesture.rs](../../src-tauri/src/gesture.rs#L176-L187) | Verified (`reset_sequence_clears_steps_and_accepts_new_input`) |
| W-G08 | minimum segment未満のmovementはstepにしない | [gesture.rs](../../src-tauri/src/gesture.rs#L246-L269) | Gap (`G-G01`) |
| W-G09 | ambiguous diagonalはaxis deadzone内でstepにしない | [gesture.rs](../../src-tauri/src/gesture.rs#L189-L228) | Gap (`G-G01`) |
| W-G10 | direction changeはconfirmation distance到達後だけ確定する | [gesture.rs](../../src-tauri/src/gesture.rs#L100-L123) | Gap (`G-G01`) |
| W-M01 | process exact matchはcase-insensitive equality | [app_match.rs](../../src-tauri/src/hook/app_match.rs#L58-L79) | Verified (`match_app_process_name_exact`) |
| W-M02 | window class exact matchはcase-sensitive equality | [app_match.rs](../../src-tauri/src/hook/app_match.rs#L58-L79) | Verified (`match_app_window_class_exact_case_sensitive`) |
| W-M03 | title containsはcase-insensitive substring | [app_match.rs](../../src-tauri/src/hook/app_match.rs#L58-L79) | Verified (`match_app_title_contains`) |
| W-M04 | title containsはnon-ASCII case foldを扱う | [app_match.rs](../../src-tauri/src/hook/app_match.rs#L58-L79) | Verified (`match_app_title_contains_non_ascii_case_insensitive`) |
| W-M05 | regex matcherはmatchとnon-matchを区別する | [app_match.rs](../../src-tauri/src/hook/app_match.rs#L58-L79) | Verified (`match_app_regex`) |
| W-M06 | 一つのapp内のmatchersはORで評価する | [app_match.rs](../../src-tauri/src/hook/app_match.rs#L90-L104) | Verified (`match_app_or_logic`) |
| W-M07 | 複数appがmatchする場合はdeterministic firstを返す | [app_match.rs](../../src-tauri/src/hook/app_match.rs#L90-L104) | Verified (`match_app_first_match_wins`) |
| W-M08 | target field欠損はpanicせずnon-match | [app_match.rs](../../src-tauri/src/hook/app_match.rs#L58-L69) | Verified (`match_app_none_field_no_panic`) |
| W-M09 | invalid regex matcherはcompile結果から除外する | [app_match.rs](../../src-tauri/src/hook/app_match.rs#L106-L137) | Gap (`G-M01`) |

### Gesture session state

| ID | Atomic contract | Source | Verification |
| --- | --- | --- | --- |
| W-S01 | configured trigger downをsuppressする | [state.rs](../../src-tauri/src/hook/state.rs#L349-L382) | Verified (`idle_starts_gesture_on_configured_trigger`) |
| W-S02 | configured trigger downでGesturingへ遷移する | [state.rs](../../src-tauri/src/hook/state.rs#L349-L382) | Verified (`idle_starts_gesture_on_configured_trigger`) |
| W-S03 | gesture開始時にStart lifecycleを出す | [state.rs](../../src-tauri/src/hook/state.rs#L349-L382) | Verified (`idle_starts_gesture_on_configured_trigger`) |
| W-S04 | gesture開始時にinitial Track pointを出す | [state.rs](../../src-tauri/src/hook/state.rs#L349-L382) | Gap (`G-S01`) |
| W-S05 | Idleでbindingのないtrigger downをpassする | [state.rs](../../src-tauri/src/hook/state.rs#L349-L384) | Verified (`idle_ignores_unconfigured_trigger`) |
| W-S06 | Idleでbindingのないtrigger downはstateを変えない | [state.rs](../../src-tauri/src/hook/state.rs#L349-L384) | Verified (`idle_ignores_unconfigured_trigger`) |
| W-S07 | Idleでbindingのないtrigger downはoverlay effectを出さない | [state.rs](../../src-tauri/src/hook/state.rs#L349-L384) | Verified (`idle_ignores_unconfigured_trigger`) |
| W-S08 | matching trigger upをsuppressする | [state.rs](../../src-tauri/src/hook/state.rs#L456-L487) | Verified (`executes_action_on_trigger_up_when_sequence_matches`) |
| W-S09 | matching trigger upはactionをrepeat 1で要求する | [state.rs](../../src-tauri/src/hook/state.rs#L460-L473) | Verified (`executes_action_on_trigger_up_when_sequence_matches`) |
| W-S10 | matching trigger upはreplayを要求しない | [state.rs](../../src-tauri/src/hook/state.rs#L460-L484) | Verified (`executes_action_on_trigger_up_when_sequence_matches`) |
| W-S11 | matching trigger up後はIdleへ戻る | [state.rs](../../src-tauri/src/hook/state.rs#L460-L487) | Verified (`executes_action_on_trigger_up_when_sequence_matches`) |
| W-S12 | trigger upによる終了はEnd lifecycleを出す | [state.rs](../../src-tauri/src/hook/state.rs#L460-L487) | Verified (`executes_action_on_trigger_up_when_sequence_matches`) |
| W-S13 | zero-travel unmatched clickは元down/up座標でreplayする | [state.rs](../../src-tauri/src/hook/state.rs#L475-L484) | Verified (`trigger_click_without_matching_sequence_requests_replay`) |
| W-S14 | unmatched clickはactionを要求しない | [state.rs](../../src-tauri/src/hook/state.rs#L460-L484) | Verified (`trigger_click_without_matching_sequence_requests_replay`) |
| W-S15 | threshold以内のshort total travelはreplayする | [state.rs](../../src-tauri/src/hook/state.rs#L475-L505) | Verified (`unmatched_sequence_with_short_move_requests_replay`) |
| W-S16 | threshold超過のlong total travelはreplayしない | [state.rs](../../src-tauri/src/hook/state.rs#L475-L505) | Verified (`unmatched_sequence_with_long_move_does_not_request_replay`) |
| W-S17 | replay判定はdisplacementでなくaccumulated travelを使う | [state.rs](../../src-tauri/src/hook/state.rs#L397-L405) | Verified (`unmatched_sequence_with_small_displacement_but_large_travel_does_not_request_replay`) |
| W-S18 | replay distance thresholdはconfigurable | [state.rs](../../src-tauri/src/hook/state.rs#L497-L500) | Verified (`unmatched_sequence_replay_threshold_is_configurable`) |
| W-S19 | replay thresholdはrecognition thresholdsから独立する | [state.rs](../../src-tauri/src/hook/state.rs#L497-L500) | Verified (`replay_threshold_is_not_coupled_to_recognition_thresholds`) |
| W-S20 | wheel inputをrelease sequenceとしてmatchできる | [state.rs](../../src-tauri/src/hook/state.rs#L518-L551) | Verified (`supports_wheel_input_in_sequence`) |
| W-S21 | matching hold wheelはrelease前にactionを要求する | [state.rs](../../src-tauri/src/hook/state.rs#L518-L537) | Verified (`hold_wheel_executes_immediately_with_repeat_count`) |
| W-S22 | hold wheel action repeatはwheel notch数と一致する | [state.rs](../../src-tauri/src/hook/state.rs#L518-L537) | Verified (`hold_wheel_executes_immediately_with_repeat_count`) |
| W-S23 | matching hold wheelをsuppressする | [state.rs](../../src-tauri/src/hook/state.rs#L518-L537) | Verified (`hold_wheel_executes_immediately_with_repeat_count`) |
| W-S24 | hold action使用後のtrigger releaseはclick replayしない | [state.rs](../../src-tauri/src/hook/state.rs#L475-L484) | Verified (`hold_wheel_usage_disables_unmatched_trigger_replay`) |
| W-S25 | hold bindingはspecific prefix sequenceを要求できる | [state.rs](../../src-tauri/src/hook/state.rs#L553-L569) | Verified (`hold_wheel_can_require_specific_sequence_state`) |
| W-S26 | exact hold prefixはwildcardより優先する | [state.rs](../../src-tauri/src/hook/state.rs#L553-L569) | Verified (`hold_wheel_specific_sequence_overrides_wildcard_binding`) |
| W-S27 | hold match後はrecognized sequenceをresetする | [state.rs](../../src-tauri/src/hook/state.rs#L528-L536) | Verified (`hold_wheel_match_resets_recognized_sequence`) |
| W-S28 | gesture中のnon-trigger button upをsuppressする | [state.rs](../../src-tauri/src/hook/state.rs#L456-L487) | Verified (`non_trigger_button_up_does_not_end_gesture`) |
| W-S29 | gesture中のnon-trigger button upはsessionを終了しない | [state.rs](../../src-tauri/src/hook/state.rs#L456-L487) | Verified (`non_trigger_button_up_does_not_end_gesture`) |
| W-S30 | app-specific release bindingをdefaultより優先する | [state.rs](../../src-tauri/src/hook/state.rs#L80-L104) | Verified (`resolve_binding_prefers_app_specific_then_fallback`) |
| W-S31 | app-specific matchなしではdefault release bindingへfallbackする | [state.rs](../../src-tauri/src/hook/state.rs#L80-L104) | Verified (`resolve_binding_prefers_app_specific_then_fallback`) |
| W-S32 | safety timeout predicateはwrapping tickを扱う | [state.rs](../../src-tauri/src/hook/state.rs#L606-L617) | Verified (`safety_timeout_works_with_wrapping_ticks`) |
| W-S33 | gesture中のmovement eventはpassする | [state.rs](../../src-tauri/src/hook/state.rs#L396-L413) | Gap (`G-S02`) |
| W-S34 | gesture中の追加button downをsequence stepへ加える | [state.rs](../../src-tauri/src/hook/state.rs#L444-L455) | Gap (`G-S03`) |
| W-S35 | gesture中の追加button downをsuppressする | [state.rs](../../src-tauri/src/hook/state.rs#L444-L455) | Gap (`G-S03`) |
| W-S36 | label lifecycleはresolved labelが変わった時だけ更新する | [state.rs](../../src-tauri/src/hook/state.rs#L571-L603) | Gap (`G-S04`) |
| W-S37 | safety timeout handlerはsessionをIdleへresetする | [win32.rs](../../src-tauri/src/hook/win32.rs#L307-L331) | Gap (`G-S05`) |
| W-S38 | safety timeout handlerはEnd lifecycleを送る | [win32.rs](../../src-tauri/src/hook/win32.rs#L307-L331) | Gap (`G-S05`) |

### Windows adapter, renderer, logging, and capture

| ID | Atomic contract | Source | Verification |
| --- | --- | --- | --- |
| W-H01 | validated release bindingをcompiled setへ保持する | [hook/mod.rs](../../src-tauri/src/hook/mod.rs#L108-L156) | Verified (`compile_bindings_for_app_compiles_validated_bindings`) |
| W-H02 | validated hold bindingをcompiled setへ保持する | [hook/mod.rs](../../src-tauri/src/hook/mod.rs#L108-L156) | Verified (`compile_bindings_for_app_compiles_validated_bindings`) |
| W-H03 | explicit binding labelをcompiled setへ保持する | [hook/mod.rs](../../src-tauri/src/hook/mod.rs#L116-L147) | Verified (`compile_bindings_for_app_compiles_validated_bindings`) |
| W-H04 | stepなしholdをdefensiveにcompile対象外とする | [hook/mod.rs](../../src-tauri/src/hook/mod.rs#L133-L140) | Verified (`compile_bindings_for_app_defensively_skips_hold_binding_without_step`) |
| W-H05 | negative hook codeをnext hookへpassする | [win32.rs](../../src-tauri/src/hook/win32.rs#L181-L209) | Gap (`G-H01`) |
| W-H06 | self-injected eventをnext hookへpassする | [win32.rs](../../src-tauri/src/hook/win32.rs#L181-L209) | Gap (`G-H02`) |
| W-H07 | left/right/middle button downをcanonical eventへmapする | [win32.rs](../../src-tauri/src/hook/win32.rs#L266-L288) | Gap (`G-H03`) |
| W-H08 | left/right/middle button upをcanonical eventへmapする | [win32.rs](../../src-tauri/src/hook/win32.rs#L266-L288) | Gap (`G-H03`) |
| W-H09 | mouse moveをcanonical movementへmapする | [win32.rs](../../src-tauri/src/hook/win32.rs#L266-L288) | Gap (`G-H03`) |
| W-H10 | positive wheel deltaをcanonical WheelUpへmapする | [win32.rs](../../src-tauri/src/hook/win32.rs#L266-L304) | Gap (`G-H03`) |
| W-H11 | unknown Win32 messageをcanonical Otherへmapする | [win32.rs](../../src-tauri/src/hook/win32.rs#L266-L288) | Gap (`G-H03`) |
| W-H12 | callbackで決定したactionsをmessage loopでFIFO実行する | [win32.rs](../../src-tauri/src/hook/win32.rs#L244-L261) | Gap (`G-H04`) |
| W-H13 | callbackで決定したreplayをmessage loopで実行する | [win32.rs](../../src-tauri/src/hook/win32.rs#L244-L249) | Gap (`G-H05`) |
| W-H14 | trigger point下のtop-level windowをactivateしてidentityを使う | [win32.rs](../../src-tauri/src/hook/win32.rs#L222-L234) | Gap (`G-H06`) |
| W-H15 | point targetを得られない場合はforeground identityへfallbackする | [win32.rs](../../src-tauri/src/hook/win32.rs#L222-L234) | Gap (`G-H07`) |
| W-H16 | overlay windowはclick-throughである | [overlay/window.rs](../../src-tauri/src/overlay/window.rs#L160-L532) | Gap (`G-H08`) |
| W-H17 | Start lifecycleはoverlayを表示・初期化する | [overlay/window.rs](../../src-tauri/src/overlay/window.rs#L454-L495) | Gap (`G-H08`) |
| W-H18 | Track lifecycleはGDI trailへpointを加える | [overlay/window.rs](../../src-tauri/src/overlay/window.rs#L496-L531) | Gap (`G-H08`) |
| W-H19 | Label lifecycleはnative labelを更新する | [overlay/window.rs](../../src-tauri/src/overlay/window.rs#L585-L682) | Gap (`G-H08`) |
| W-H20 | End lifecycleはoverlayをclearしてhideする | [overlay/window.rs](../../src-tauri/src/overlay/window.rs#L532-L584) | Gap (`G-H08`) |
| W-H21 | `#RRGGBB` colorをparseする | [overlay/mod.rs](../../src-tauri/src/overlay/mod.rs#L128-L156) | Verified (`parse_hex_color_6_digit`) |
| W-H22 | `RRGGBB` colorをparseする | [overlay/mod.rs](../../src-tauri/src/overlay/mod.rs#L128-L156) | Verified (`parse_hex_color_6_digit_no_hash`) |
| W-H23 | `#RGB` colorをexpandしてparseする | [overlay/mod.rs](../../src-tauri/src/overlay/mod.rs#L128-L156) | Verified (`parse_hex_color_3_digit`) |
| W-H24 | invalid colorはfallback colorを返す | [overlay/mod.rs](../../src-tauri/src/overlay/mod.rs#L128-L156) | Verified (`parse_hex_color_invalid_fallback`) |
| W-H25 | known log levelsはcase-insensitiveにparseする | [log.rs](../../src-tauri/src/log.rs#L11-L42) | Verified (`parse_log_level_supports_case_insensitive_values`) |
| W-H26 | log levelはouter whitespaceをtrimする | [log.rs](../../src-tauri/src/log.rs#L11-L42) | Verified (`parse_log_level_trims_whitespace_and_rejects_unknown_values`) |
| W-H27 | unknown log levelをrejectする | [log.rs](../../src-tauri/src/log.rs#L11-L42) | Verified (`parse_log_level_trims_whitespace_and_rejects_unknown_values`) |
| W-H28 | negative wheel deltaをcanonical WheelDownへmapする | [win32.rs](../../src-tauri/src/hook/win32.rs#L266-L304) | Gap (`G-H03`) |
| W-H29 | zero wheel deltaをcanonical Otherへmapする | [win32.rs](../../src-tauri/src/hook/win32.rs#L266-L304) | Gap (`G-H03`) |
| W-P01 | capture hookはnon-left eventをpassする | [capture.rs](../../src-tauri/src/capture.rs#L64-L82) | Gap (`G-P01`) |
| W-P02 | capture hookはinjected left eventをpassする | [capture.rs](../../src-tauri/src/capture.rs#L64-L82) | Gap (`G-P01`) |
| W-P03 | first real left downをcapture時にsuppressする | [capture.rs](../../src-tauri/src/capture.rs#L89-L129) | Gap (`G-P02`) |
| W-P04 | capture resultはclick point下のwindow identityを使う | [capture.rs](../../src-tauri/src/capture.rs#L97-L106) | Gap (`G-P02`) |
| W-P05 | captureは成功result eventを一度だけemitする | [capture.rs](../../src-tauri/src/capture.rs#L89-L121) | Gap (`G-P02`) |
| W-P06 | result後はcapture hookを終了する | [capture.rs](../../src-tauri/src/capture.rs#L117-L130) | Gap (`G-P02`) |
| W-P07 | active captureなしのstopは成功no-op | [commands.rs](../../src-tauri/src/commands.rs#L194-L215) | Gap (`G-P03`) |

現行のcapture replacementと即時cancelはpreservation contractではない。
commandは[new captureをstartしてからold handleをcancel](../../src-tauri/src/commands.rs#L176-L190)し、
`tid == 0`のcancelは[quitをpostできず](../../src-tauri/src/capture.rs#L142-L158)、
capture loopは受け取った[cancel flagをhook install前に確認しない](../../src-tauri/src/capture.rs#L194-L219)。
この既知startup raceを「常に一件」「replace可能」という現行factに数えず、修正後のcontractは後述のA-I25からA-I30へ置く。

### Settings workflow

39 frontend unit runner casesはSettings全体ではなく、keyboard-input constants/pure helpersだけを検証する。
次の表はそれらのassert対象と、未検証の実際のSettings workflowを分ける。

| ID | Atomic contract | Source | Verification |
| --- | --- | --- | --- |
| W-U01 | shortcut catalogはlowercase letters a-zを含む | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L29-L124) | Verified (`SHORTCUT_KEYS letters`) |
| W-U02 | shortcut catalogはdigits 0-9を含む | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L29-L124) | Verified (`SHORTCUT_KEYS numbers`) |
| W-U03 | shortcut catalogはF1-F24を含む | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L29-L124) | Verified (`SHORTCUT_KEYS function keys`) |
| W-U04 | shortcut catalogはsupported navigation keysを含む | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L29-L124) | Verified (`SHORTCUT_KEYS navigation`) |
| W-U05 | shortcut catalogはspaceを含む | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L29-L124) | Verified (`SHORTCUT_KEYS space`) |
| W-U06 | shortcut catalogはuppercase lettersを含まない | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L29-L124) | Verified (`SHORTCUT_KEYS uppercase exclusion`) |
| W-U07 | modifier catalogはlowercase ctrl/alt/shift/winだけ | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L15-L15) | Verified (`MODIFIER_KEYS two cases`) |
| W-U08 | missingまたはempty shortcut textはempty listになる | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys empty cases`) |
| W-U09 | modifier aliasesはcanonical lowercaseへnormalizeする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys modifier aliases`) |
| W-U10 | shortcut lettersはlowercaseへnormalizeする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys letters`) |
| W-U11 | shortcut digitsはそのまま保持する | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys numbers`) |
| W-U12 | function keysはlowercase F1-F24だけを受理する | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys function key cases`) |
| W-U13 | navigation aliasesはcanonical nameへnormalizeする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys navigation aliases`) |
| W-U14 | space aliasはcanonical `space`へnormalizeする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys space`) |
| W-U15 | comma separated shortcutはouter whitespaceを除いて順序保持する | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys comma/space cases`) |
| W-U16 | unknown keyとunsupported single characterをfilterする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys invalid cases`) |
| W-U17 | complex shortcut combinationはcanonical orderを保持する | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L126-L203) | Verified (`parseKeys complex combinations`) |
| W-U18 | bare modifier key releaseはmain keyとして確定しない | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey bare modifiers`) |
| W-U19 | pressed spaceをcanonical `space`へnormalizeする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey space`) |
| W-U20 | pressed unknown main keyをrejectする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey unknown`) |
| W-U21 | modifier display labelは先頭大文字を使う | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L268-L299) | Verified (`keyLabel modifier`) |
| W-U22 | modifier labelはkey labelと同じ規則を使う | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L301-L301) | Verified (`modifierLabel alias`) |
| W-U23 | wait-mode shortcut editorはkeydownのcanonical combinationをpreviewする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L323-L397) | Gap (`G-U01`) |
| W-U24 | manual editorはopen時のshortcutを選択stateへ復元する | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L556-L605) | Gap (`G-U02`) |
| W-U25 | manual editorはmodifierを独立にtoggleする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L607-L617) | Gap (`G-U03`) |
| W-U26 | manual editorは一つのmain keyを選択する | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L673-L685) | Gap (`G-U04`) |
| W-U27 | manual editorのAssignはcanonical previewを確定する | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L619-L622) | Gap (`G-U05`) |
| W-U28 | manual editorのCancelは変更を確定しない | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L704-L719) | Gap (`G-U06`) |
| W-U29 | Settingsはbackendのcurrent configをloadする | [api.ts](../../src/lib/api.ts#L14-L14) | Gap (`G-U07`) |
| W-U30 | Settings saveはedited documentをbackend updateへ渡す | [api.ts](../../src/lib/api.ts#L16-L17) | Gap (`G-U08`) |
| W-U31 | Settings importはselected JSONをbackend validation/applyへ渡す | [api.ts](../../src/lib/api.ts#L19-L20) | Gap (`G-U09`) |
| W-U32 | Settings exportはcurrent snapshotをselected pathへ書き出す | [api.ts](../../src/lib/api.ts#L22-L23) | Gap (`G-U10`) |
| W-U33 | backendの`config-updated`はSettings cacheを同じdocumentへ更新する | [use-config.ts](../../src/hooks/use-config.ts#L14-L30) | Gap (`G-U11`) |
| W-U34 | Open Config Directoryはapp config directoryを作成して開く | [commands.rs](../../src-tauri/src/commands.rs#L126-L140) | Gap (`G-U12`) |
| W-U35 | foreground info requestは取得時点のoptional identity fieldsを返す | [api.ts](../../src/lib/api.ts#L27-L34) | Gap (`G-U13`) |
| W-U36 | sidebar initial widthは200px | [sidebar story](../../src/components/ui/sidebar.stories.tsx#L218-L301) | Verified (`StateTransitions.play`) |
| W-U37 | compact thresholdではwidthを72pxにする | [sidebar story](../../src/components/ui/sidebar.stories.tsx#L218-L301) | Verified (`StateTransitions.play`) |
| W-U38 | sidebar resizeは72px未満へ縮まない | [sidebar story](../../src/components/ui/sidebar.stories.tsx#L218-L301) | Verified (`StateTransitions.play`) |
| W-U39 | dragging中だけwidth transitionを無効化する | [sidebar story](../../src/components/ui/sidebar.stories.tsx#L218-L301) | Verified (`StateTransitions.play`) |
| W-U40 | continuous drag deltaをcurrent widthへ反映する | [sidebar story](../../src/components/ui/sidebar.stories.tsx#L218-L301) | Verified (`StateTransitions.play`) |
| W-U41 | compact幅から拡大するとexpanded contentを再表示する | [sidebar story](../../src/components/ui/sidebar.stories.tsx#L218-L301) | Verified (`StateTransitions.play`) |
| W-U42 | sidebar resizeは360pxを上限にする | [sidebar story](../../src/components/ui/sidebar.stories.tsx#L218-L301) | Verified (`StateTransitions.play`) |
| W-U43 | 同一drag中にcompact境界を再び越えられる | [sidebar story](../../src/components/ui/sidebar.stories.tsx#L218-L301) | Verified (`StateTransitions.play`) |
| W-U44 | wait-mode shortcut editorはEscapeで変更を確定せず閉じる | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L357-L380) | Gap (`G-U14`) |
| W-U45 | wait-mode shortcut editorはmain-key releaseを一度だけconfirmする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L374-L388) | Gap (`G-U15`) |
| W-U46 | save failureをsuccessとして表示しない | [config draft](../../src/contexts/config-draft.tsx#L37-L60) | Gap (`G-U16`) |
| W-U47 | save pending中は重複Save操作を無効にする | [form actions](../../src/components/settings-form-actions.tsx#L18-L30) | Gap (`G-U17`) |
| W-U48 | import dialog cancel時はbackend requestを送らない | [advanced route](../../src/routes/advanced/index.tsx#L18-L69) | Gap (`G-U18`) |
| W-U49 | malformed import JSONをsuccessに見せない | [advanced route](../../src/routes/advanced/index.tsx#L18-L69) | Gap (`G-U19`) |
| W-U50 | successful importはcommitted snapshotへ反映する | [advanced route](../../src/routes/advanced/index.tsx#L18-L69) | Gap (`G-U20`) |
| W-U51 | export dialog cancel時はbackend requestを送らない | [advanced route](../../src/routes/advanced/index.tsx#L18-L69) | Gap (`G-U21`) |
| W-U52 | export write failureをsuccessに見せない | [advanced route](../../src/routes/advanced/index.tsx#L18-L69) | Gap (`G-U22`) |
| W-U53 | Settings draftはloaded config snapshotで初期化する | [config draft](../../src/contexts/config-draft.tsx#L37-L60) | Gap (`G-U23`) |
| W-U54 | dirty判定はdraftとcommitted snapshotのsemantic差分を使う | [config draft](../../src/contexts/config-draft.tsx#L37-L60) | Gap (`G-U24`) |
| W-U55 | Cancelはdraftをcommitted snapshotへ戻す | [config draft](../../src/contexts/config-draft.tsx#L37-L60) | Gap (`G-U25`) |
| W-U56 | successful Save後はdraftをcleanとして扱う | [config draft](../../src/contexts/config-draft.tsx#L37-L60) | Gap (`G-U26`) |
| W-U57 | import semantic validation failureをsuccessに見せない | [advanced route](../../src/routes/advanced/index.tsx#L18-L69) | Gap (`G-U19`) |
| W-U58 | import revision conflictをsuccessに見せない | [advanced route](../../src/routes/advanced/index.tsx#L18-L69) | Gap (`G-U19`) |
| W-U59 | import I/O failureをsuccessに見せない | [advanced route](../../src/routes/advanced/index.tsx#L18-L69) | Gap (`G-U19`) |
| W-U60 | successful importはSettings draftをcommitted snapshotへ同期する | [advanced route](../../src/routes/advanced/index.tsx#L18-L69) | Gap (`G-U20`) |
| W-U61 | save failure後もedited draftを保持する | [config draft](../../src/contexts/config-draft.tsx#L37-L60) | Gap (`G-U16`) |
| W-U62 | save pending中はCancel操作を無効にする | [form actions](../../src/components/settings-form-actions.tsx#L18-L30) | Gap (`G-U17`) |
| W-U63 | compact sidebarはexpanded labelを隠す | [sidebar story](../../src/components/ui/sidebar.stories.tsx#L218-L301) | Verified (`StateTransitions.play`) |
| W-U64 | pressed arrow keyをcanonical navigation nameへnormalizeする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey arrows`) |
| W-U65 | pressed navigation aliasをcanonical nameへnormalizeする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey navigation aliases`) |
| W-U66 | pressed letterをlowercaseへnormalizeする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey letters`) |
| W-U67 | pressed digitをそのまま保持する | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey numbers`) |
| W-U68 | pressed F1-F24をlowercaseへnormalizeする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey function keys`) |
| W-U69 | pressed invalid function keyをrejectする | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey invalid function keys`) |
| W-U70 | pressed supported navigation canonical nameを保持する | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L205-L266) | Verified (`normalizePressedKey navigation`) |
| W-U71 | function key display labelはuppercase Fを使う | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L268-L299) | Verified (`keyLabel function keys`) |
| W-U72 | PageUp/PageDown display labelはcanonical camel caseを使う | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L268-L299) | Verified (`keyLabel page keys`) |
| W-U73 | single-letter display labelはuppercaseを使う | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L268-L299) | Verified (`keyLabel letters`) |
| W-U74 | other key display labelは先頭大文字を使う | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L268-L299) | Verified (`keyLabel other`) |
| W-U75 | empty key display labelはempty string | [keyboard-input.tsx](../../src/routes/applications/$appId/gestures/-components/keyboard-input.tsx#L268-L299) | Verified (`keyLabel empty`) |

### Obligation accounting

各`Gap`は行内のplanned case IDへ割り当て済みである。
同じplanned caseが複数のatomic predicateをassertする対応は次のとおりである。

| Planned cases | Scope |
| --- | --- |
| `G-R01`-`G-R13` | cold/enable/disable、unchanged/concurrent update、worker/disk fault rollback、success side effects、shutdown、tray |
| `G-C01`-`G-C04` | unasserted defaults、missing/read-error/invalid-JSON load |
| `G-A01`-`G-A04` | key aliases/space、exact injection flags/order、unknown-key handling |
| `G-G01`, `G-M01` | recognizer threshold boundaries、invalid regex |
| `G-S01`-`G-S05` | initial Track、movement pass-through、additional button、label、safety terminal |
| `G-H01`-`G-H08` | hook pass-through/mapping、deferred action/replay、window targeting、native overlay lifecycle |
| `G-P01`-`G-P03` | capture pass-through、one-shot result、no-active stop |
| `G-U01`-`G-U26` | shortcut dialogs、draft、load/save/import/export/cache、config directory、foreground identity |

table rowをscriptで数えたbaselineは次のとおりである。

| Inventory | `O` atomic obligations | `O_v` verified obligations | `U` unmapped obligations |
| --- | ---: | ---: | ---: |
| Existing Windows + Settings | 252 | 157 | 0 |

64 Rust runner cases、39 frontend unit runner cases、1 browser `play` caseは、明示assertしたpredicateだけを`O_v`へ反映する。
残り45 Storybook render-smoke casesはbrowser componentの`R_runner`へ含めるが、behavior assertionがないため`O_v`を増やさない。
後続product PRをintegration branchへ入れる時点では、影響するobligationについて`O_v / O = 100%`を必須にする。
W-C48からW-C50のsilent fallbackなど意図的に変更するcontractは、旧behavior testを削除して数値を良くせず、migration input testと新failure contract testへ置換する。

## New architecture obligations

### Process and packaging

| ID | Atomic contract | Source | Required verification |
| --- | --- | --- | --- |
| A-P01 | EngineとSettingsは別processで動く | [ADR 0001](./0001-tauri-two-process-modes.md#decision) | `A-T-P01` process E2E |
| A-P02 | EngineとSettingsは同一executableのmodeである | [ADR 0001](./0001-tauri-two-process-modes.md#decision) | `A-T-P02` artifact/process inspection |
| A-P03 | Engine modeはwindow 0かつWebView 0 | [ADR 0001](./0001-tauri-two-process-modes.md#external-contract) | `A-T-P03` process tree probe |
| A-P04 | Settings closeはhideでなくprocess exit | [ADR 0001](./0001-tauri-two-process-modes.md#external-contract) | `A-T-P04` close lifecycle E2E |
| A-P05 | userごとのEngine instanceは一つだけ | [ADR 0001](./0001-tauri-two-process-modes.md#external-contract) | `A-T-P05` concurrent launch |
| A-P06 | Settingsのcrash/hang/disconnectはEngineを停止しない | [ADR 0001](./0001-tauri-two-process-modes.md#external-contract) | `A-T-P06` Settings fault E2E |
| A-P07 | SettingsはOS inputを監視・抑止・注入しない | [ADR 0001](./0001-tauri-two-process-modes.md#external-contract) | `A-T-P07` mode capability audit |
| A-P08 | Engineだけがconfig pathとIPC endpointを所有する | [ADR 0001](./0001-tauri-two-process-modes.md#external-contract) | `A-T-P08` owner/endpoint integration |
| A-P09 | 両modeは同じversion、署名identity、application IDを使う | [ADR 0001](./0001-tauri-two-process-modes.md#external-contract) | `A-T-P09` signed artifact inspection |
| A-P10 | Tauri autostartは同一executableへ`--engine`を渡す | [ADR 0001](./0001-tauri-two-process-modes.md#packaging-spike-gate) | `A-T-P10` LaunchAgent/installer inspection |
| A-P11 | autostartのuser拒否状態を成功登録と誤認しない | [ADR 0001](./0001-tauri-two-process-modes.md#packaging-spike-gate) | `A-T-P11` macOS refusal system test |
| A-P12 | 新versionの再インストール後もautostart登録を維持する | [ADR 0001](./0001-tauri-two-process-modes.md#packaging-spike-gate) | `A-T-P12` reinstall E2E |

### Ownership, queues, and fail-open

| ID | Atomic contract | Source | Required verification |
| --- | --- | --- | --- |
| A-M01 | 一つのmutable factは一ownerだけが変更する | [ADR 0002](./0002-message-passing-and-fail-open.md#state-invariants) | `A-T-M01` ownership architecture check |
| A-M02 | owner間messageはclosed Rust enumである | [ADR 0002](./0002-message-passing-and-fail-open.md#decision) | `A-T-M02` type/dependency check |
| A-M03 | Input callbackはlockまたはwaitを行わない | [ADR 0002](./0002-message-passing-and-fail-open.md#input-callback-exception) | `A-T-M03` lock/wait instrumentation |
| A-M04 | Input callbackはheap allocationを行わない | [ADR 0002](./0002-message-passing-and-fail-open.md#input-callback-exception) | `A-T-M04` allocator instrumentation |
| A-M05 | Input callbackはfile/socket/JSON/IPC/awaitを行わない | [ADR 0002](./0002-message-passing-and-fail-open.md#input-callback-exception) | `A-T-M05` boundary instrumentation |
| A-M06 | Input callbackはwindow queryまたはregex compile/matchを行わない | [ADR 0002](./0002-message-passing-and-fail-open.md#input-callback-exception) | `A-T-M06` platform-call instrumentation |
| A-M07 | Input callbackはaction実行またはlog formattingを行わない | [ADR 0002](./0002-message-passing-and-fail-open.md#input-callback-exception) | `A-T-M07` executor/log instrumentation |
| A-M08 | 中間render pointだけをcoalesceできる | [ADR 0002](./0002-message-passing-and-fail-open.md#backpressure) | `A-T-M08` render queue stress |
| A-M09 | diagnostic metrics sampleだけをdropできる | [ADR 0002](./0002-message-passing-and-fail-open.md#backpressure) | `A-T-M09` metrics overload |
| A-M10 | accepted actionをsilent dropしない | [ADR 0002](./0002-message-passing-and-fail-open.md#backpressure) | `A-T-M10` executor queue fault |
| A-M11 | render lifecycleをsilent dropしない | [ADR 0002](./0002-message-passing-and-fail-open.md#backpressure) | `A-T-M11` lifecycle queue fault |
| A-M12 | replay operationをsilent dropしない | [ADR 0002](./0002-message-passing-and-fail-open.md#backpressure) | `A-T-M12` replay queue fault |
| A-M13 | committed config deliveryをsilent dropしない | [ADR 0002](./0002-message-passing-and-fail-open.md#backpressure) | `A-T-M13` config queue fault |
| A-M14 | shutdown messageをsilent dropしない | [ADR 0002](./0002-message-passing-and-fail-open.md#backpressure) | `A-T-M14` shutdown queue fault |
| A-M15 | essential messageを保持不能ならterminal fail-openへ遷移する | [ADR 0002](./0002-message-passing-and-fail-open.md#backpressure) | `A-T-M15` full-queue state test |
| A-M16 | 進行中gestureは開始時のimmutable snapshotを使い切る | [ADR 0002](./0002-message-passing-and-fail-open.md#input-callback-exception) | `A-T-M16` concurrent config trace |
| A-M17 | Renderer障害はvisualだけを停止する | [ADR 0002](./0002-message-passing-and-fail-open.md#fail-open-invariant) | `A-T-M17` renderer fault |
| A-M18 | Executor障害後は新しいgesture captureを停止する | [ADR 0002](./0002-message-passing-and-fail-open.md#fail-open-invariant) | `A-T-M18` executor fault |
| A-M19 | 抑止済みtriggerのreplayは一度だけboundedに試す | [ADR 0002](./0002-message-passing-and-fail-open.md#fail-open-invariant) | `A-T-M19` replay failure trace |
| A-M20 | Rust panicやforeign exceptionをFFI callback外へ越境させない | [ADR 0002](./0002-message-passing-and-fail-open.md#fail-open-invariant) | `A-T-M20` callback panic injection |
| A-M21 | injected eventはself tagで再捕捉しない | [ADR 0002](./0002-message-passing-and-fail-open.md#fail-open-invariant) | `A-T-M21` injection loop test |
| A-M22 | shutdownはinputをpass-throughへ移してからownerをjoinする | [ADR 0002](./0002-message-passing-and-fail-open.md#state-invariants) | `A-T-M22` shutdown order test |
| A-M23 | hook/event tap install失敗時はgestureを開始しない | [ADR 0002](./0002-message-passing-and-fail-open.md#fail-open-invariant) | `A-T-M23` install failure |
| A-M24 | safety timeoutまたはevent tap timeout後は新規eventをpassする | [ADR 0002](./0002-message-passing-and-fail-open.md#fail-open-invariant) | `A-T-M24` timeout state test |

### Portable domain

| ID | Atomic contract | Source | Required verification |
| --- | --- | --- | --- |
| A-D01 | domain型はOS/Tauri/IPC/thread/renderer型に依存しない | [ADR 0003](./0003-portable-domain-and-native-platforms.md#platform-neutral-domain-contract) | `A-T-D01` dependency check |
| A-D02 | Windows/macOSの同じcanonical traceは同じdomain effectを返す | [ADR 0003](./0003-portable-domain-and-native-platforms.md#platform-neutral-domain-contract) | `A-T-D02` shared trace suite |
| A-D03 | platformで利用不能なselectorはvalidation errorになる | [ADR 0003](./0003-portable-domain-and-native-platforms.md#configuration-schema) | `A-T-D03` capability contract |
| A-D04 | logical `primary/secondary/shift`をplatform keyへmapする | [ADR 0003](./0003-portable-domain-and-native-platforms.md#configuration-schema) | `A-T-D04` dual-platform mapping |
| A-D05 | legacy Windows `ctrl`はphysical Ctrlとしてmigrationする | [ADR 0003](./0003-portable-domain-and-native-platforms.md#configuration-schema) | `A-T-D05` migration fixture |
| A-D06 | Windows 11 x64で既存behaviorを維持する | [ADR 0003](./0003-portable-domain-and-native-platforms.md#supported-platforms) | `A-T-D06` Windows acceptance |
| A-D07 | 最新stable macOSのApple Silicon arm64で動く | [ADR 0003](./0003-portable-domain-and-native-platforms.md#supported-platforms) | `A-T-D07` Apple Silicon acceptance |
| A-D08 | Engine rendererはWebView/Canvas/Skiaを使わない | [ADR 0003](./0003-portable-domain-and-native-platforms.md#native-adapters) | `A-T-D08` process/dependency inspection |
| A-D09 | app contextを期限内に解決不能ならdefault bindingを使う | [ADR 0003](./0003-portable-domain-and-native-platforms.md#failure-conditions) | `A-T-D09` context timeout |
| A-D10 | context解決不能時にstaleな別app identityを使わない | [ADR 0003](./0003-portable-domain-and-native-platforms.md#failure-conditions) | `A-T-D10` stale-context fault |
| A-D11 | platformで表現不能なkey/actionは保存境界で拒否する | [ADR 0003](./0003-portable-domain-and-native-platforms.md#failure-conditions) | `A-T-D11` capability validation |

### IPC, configuration, and window capture

| ID | Atomic contract | Source | Required verification |
| --- | --- | --- | --- |
| A-I01 | Windows Named Pipeはcurrent user SIDだけを許可する | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#decision) | `A-T-I01` peer authorization |
| A-I02 | macOS socket directoryはuser-onlyかつmode `0700` | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#decision) | `A-T-I02` filesystem permission |
| A-I03 | macOS IPC connectionはpeer UIDを検証する | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#decision) | `A-T-I03` peer credential integration |
| A-I04 | frameはlittle-endian `u32` lengthを使う | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#framing-and-envelope) | `A-T-I04` framing vectors |
| A-I05 | frame bodyは1 MiBを超えられない | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#framing-and-envelope) | `A-T-I05` boundary/fuzz |
| A-I06 | malformed frameはconnectionだけを閉じる | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#framing-and-envelope) | `A-T-I06` malformed corpus |
| A-I07 | handshakeはprotocol/executable/schema/capability/revisionを交換する | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#framing-and-envelope) | `A-T-I07` handshake integration |
| A-I08 | executable version mismatchではread-only requestだけを許可する | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#framing-and-envelope) | `A-T-I08` mixed-version test |
| A-I09 | protocol methodを境界でtyped requestへdecodeする | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#minimal-request-surface) | `A-T-I09` exhaustive decode test |
| A-I10 | IPCはarbitrary command/file read/input injectionを公開しない | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#minimal-request-surface) | `A-T-I10` surface allowlist check |
| A-I11 | Config ownerだけがconfigをwriteする | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#config-transaction) | `A-T-I11` writer ownership integration |
| A-I12 | stale expected revisionはconflictとして拒否する | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#config-transaction) | `A-T-I12` concurrent client test |
| A-I13 | accepted documentはsemantic validationを一度だけ通る | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#config-transaction) | `A-T-I13` validation instrumentation |
| A-I14 | accepted documentはimmutable runtime snapshotへcompileする | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#config-transaction) | `A-T-I14` snapshot contract |
| A-I15 | temporary configは同じuser config directoryへ置く | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#config-transaction) | `A-T-I15` filesystem probe |
| A-I16 | config saveはflush後にatomic replaceする | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#config-transaction) | `A-T-I16` filesystem fault test |
| A-I17 | commit前failureはdiskとrunning snapshotを変えない | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#config-transaction) | `A-T-I17` staged fault matrix |
| A-I18 | successful commitだけrevisionを進めてInput deliveryをackする | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#config-transaction) | `A-T-I18` revision/publish integration |
| A-I19 | update中のgestureはold snapshotで完了する | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#config-transaction) | `A-T-I19` concurrent gesture trace |
| A-I20 | legacy Windows configをschema v2へ一度だけ前方migrationする | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#migration-and-reinstall-preservation) | `A-T-I20` migration fixture |
| A-I21 | migration前fileを同じdirectoryへbackupする | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#migration-and-reinstall-preservation) | `A-T-I21` backup inspection |
| A-I22 | migration failureは元fileを保持してdiagnosticにする | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#migration-and-reinstall-preservation) | `A-T-I22` migration fault |
| A-I23 | 新versionの再インストールはuser configを保持する | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#migration-and-reinstall-preservation) | `A-T-I23` reinstall E2E |
| A-I24 | config deletionは明示的Reset/Deleteだけが行う | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#migration-and-reinstall-preservation) | `A-T-I24` installer/reset E2E |
| A-I25 | window capture startはEngine-owned typed requestである | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#minimal-request-surface) | `A-T-I25` protocol test |
| A-I26 | window capture cancelはcapture ID付きtyped requestである | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#minimal-request-surface) | `A-T-I26` protocol test |
| A-I27 | window capture resultはcapture ID付きtyped eventである | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#minimal-request-surface) | `A-T-I27` protocol/event test |
| A-I28 | replacementはold capture停止後にnew captureを開始する | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#minimal-request-surface) | `A-T-I28` startup-race test |
| A-I29 | cancelはhook install前後のどちらでも観測される | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#minimal-request-surface) | `A-T-I29` immediate-cancel test |
| A-I30 | cancelled/replaced capture IDはsuccess resultを送らない | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#minimal-request-surface) | `A-T-I30` stale-result test |
| A-I31 | IPC disconnectはEngine gesture operationを停止しない | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#availability-and-failure-conditions) | `A-T-I31` disconnect fault |
| A-I32 | malformed clientは他connectionやEngineを停止しない | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#availability-and-failure-conditions) | `A-T-I32` multi-client fault |
| A-I33 | endpoint access control失敗時はserverを公開しない | [ADR 0004](./0004-internal-ipc-and-engine-owned-config.md#availability-and-failure-conditions) | `A-T-I33` authorization setup fault |

### macOS distribution and permissions

| ID | Atomic contract | Source | Required verification |
| --- | --- | --- | --- |
| A-X01 | macOS artifactはDeveloper ID、Hardened Runtime、notarizationを満たす | [ADR 0003](./0003-portable-domain-and-native-platforms.md#macos-permissions-and-distribution) | `A-T-X01` signed artifact inspection |
| A-X02 | macOS配布はMac App Store/App Sandboxを使わない | [ADR 0003](./0003-portable-domain-and-native-platforms.md#macos-permissions-and-distribution) | `A-T-X02` entitlement/distribution check |
| A-X03 | root daemon/kernel extension/system extensionを要求しない | [ADR 0003](./0003-portable-domain-and-native-platforms.md#macos-permissions-and-distribution) | `A-T-X03` bundle/privilege audit |
| A-X04 | Input Monitoring/Accessibility不足またはrevoke時はinputをpassする | [ADR 0003](./0003-portable-domain-and-native-platforms.md#macos-permissions-and-distribution) | `A-T-X04` permission system test |
| A-X05 | event tap timeout後はinputをpassして安全に再構築する | [ADR 0002](./0002-message-passing-and-fail-open.md#fail-open-invariant) | `A-T-X05` tap timeout system test |
| A-X06 | sleep/wake後もinputを塞がない | [performance fault matrix](#performance-acceptance) | `A-T-X06` sleep/wake system test |
| A-X07 | Screen Recording permissionを要求しない | [ADR 0003](./0003-portable-domain-and-native-platforms.md#macos-permissions-and-distribution) | `A-T-X07` permission audit |

### Diagnostics and performance

| ID | Atomic contract | Source | Required verification |
| --- | --- | --- | --- |
| A-Q01 | normal logはversion/owner/permission/config lifecycleを含む | [logging contract](#logging-and-privacy-contract) | `A-T-Q01` diagnostic fault matrix |
| A-Q02 | normal logはqueue/latency/OS error/degraded reasonを含む | [logging contract](#logging-and-privacy-contract) | `A-T-Q02` diagnostic fault matrix |
| A-Q03 | normal/diagnostic logはcoordinate/key/title/config/IPC bodyを除外する | [logging contract](#logging-and-privacy-contract) | `A-T-Q03` redaction contract |
| A-Q04 | logはlocal、bounded、rotatingで外部送信しない | [logging contract](#logging-and-privacy-contract) | `A-T-Q04` rotation/network integration |
| A-Q05 | detailed diagnosticはopt-inかつtime-boundedである | [logging contract](#logging-and-privacy-contract) | `A-T-Q05` enable/expiry test |
| A-Q06 | Settings closed時のWebView process数は0 | [performance table](#performance-acceptance) | `A-T-Q06` process tree benchmark |
| A-Q07 | Engine idle CPU meanは0.2%未満 | [performance table](#performance-acceptance) | `A-T-Q07` pinned release benchmark |
| A-Q08 | Windows Engine memory p95は20 MiB未満 | [performance table](#performance-acceptance) | `A-T-Q08` Windows RSS benchmark |
| A-Q09 | macOS Engine memory p95は30 MiB未満 | [performance table](#performance-acceptance) | `A-T-Q09` macOS RSS benchmark |
| A-Q10 | callback own elapsed p99は100 us未満 | [performance table](#performance-acceptance) | `A-T-Q10` callback benchmark |
| A-Q11 | callback own elapsed p99.9は500 us未満 | [performance table](#performance-acceptance) | `A-T-Q11` callback benchmark |
| A-Q12 | terminal eventからinjection API直前のp99は2 ms未満 | [performance table](#performance-acceptance) | `A-T-Q12` action latency benchmark |
| A-Q13 | callback allocations/waits/IPC/I/O/context queriesは0 | [performance table](#performance-acceptance) | `A-T-Q13` callback instrumentation |
| A-Q14 | app受領済みinputとessential messageのsilent lossは0 | [performance table](#performance-acceptance) | `A-T-Q14` loss counters/fault matrix |

新contractは`O_A = 101`、`O_Av = 0`であり、全項目をtest obligationへ対応済みなのでinventory内の`U_A = 0`である。
実装前のためverification済みとは数えず、owner PRがtestを追加してから`O_Av`を進める。
foundation時点のinventory totalは`O = 353`、`O_v = 157`、`U = 0`である。
ここで`U = 0`は、このADRが列挙した353 contract rowすべてにverification先があるという
mapping completenessだけを表し、repositoryに未知の外部contractが存在しないという主張ではない。

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
| P1 | contract test/performance harness | P0 | existing inventoryの影響範囲が100% verified、packed casesをatomizeして`T/T_u/T_i/T_e/P/D`を測定、measurement repeatable |
| P2 | platform-neutral domainとschema v2 migration | P1 | shared trace/config tests、Windows behavior parity |
| P3 | same executable two-process modes、IPC、Engine config owner | P2 | A-P/A-I obligations、WebView 0、reinstall preservation |
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
