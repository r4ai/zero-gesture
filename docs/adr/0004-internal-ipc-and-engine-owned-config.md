# ADR 0004: Use internal local IPC and make Engine the config owner

- Status: Accepted
- Date: 2026-07-26

## Context

process分離後にSettingsとEngineが同じconfig fileを直接更新すると、disk、UI、running snapshotの三つが競合する。
入力hot pathにはIPCやJSONを置けないが、設定control pathではbinary encodingの速度よりdebuggabilityとmigration容易性が重要である。
外部toolへAPIを公開する予定はないが、再インストール直後に旧Engineと新Settingsが短時間共存する可能性はある。

## Decision

Engineをlocal IPC serverとconfigのsingle writerにする。
Settingsはfileを直接read/writeせず、Tauri Rust bridgeを介してEngineへrequestする。
React codeはNamed PipeやUnix Domain Socketを直接扱わない。

transportは次を使う。

| Platform | Transport | Access control |
| --- | --- | --- |
| Windows | Named Pipe | current user SIDだけを許可するDACLと`PIPE_REJECT_REMOTE_CLIENTS` |
| macOS | Unix Domain Socket | user-only runtime directory、directory `0700`、peer UID検証 |

localhost TCP、HTTP、WebSocketは使わない。
network interfaceやfirewallへ露出させない。
Windowsではpipe作成時に`PIPE_REJECT_REMOTE_CLIENTS`を必須にし、同じcredentialでもremote clientを拒否する。
current user以外を拒否するsecurity descriptorとremote接続拒否の両方をintegration testで検証する。

## Framing and envelope

frameはlittle-endian `u32` byte lengthとUTF-8 JSON bodyで構成する。
4 byteのlength prefixを含むencoded frame全体は1 MiB未満とし、checked additionでlengthを検証してからbodyを一度だけallocate/readする。
zero length、上限以上、不正UTF-8、不正JSON、unknown version、unknown requestは明示errorでconnectionを閉じる。

request envelopeは次を持つ。

```text
protocol_version
request_id
method
payload
```

connection開始時に次を交換する。

- protocol version
- executable/Engine version
- config schema version
- platform capabilities
- current config revision

protocolはinternalであり、外部互換性を約束しない。
それでもversionを明示し、同じversion内でsilent semantic changeをしない。
EngineとSettingsのexecutable versionが異なる場合、healthとsnapshotのread-only requestだけを許可し、変更requestはEngine restart requiredで拒否する。

## Minimal request surface

初期requestは次に限定する。

```text
Hello
GetSnapshot
ApplyConfig(expected_revision, config_transfer_id)
SetEnabled(expected_revision, enabled)
ImportConfig(expected_revision, config_transfer_id)
ExportConfig
BeginConfigUpload(Apply | Import, expected_revision, total_bytes, sha256)
AppendConfigChunk(transfer_id, index, chunk_b64)
FinishConfigUpload(transfer_id)
ReadConfigChunk(transfer_id, index)
AbortConfigTransfer(transfer_id)
OpenConfigDirectory
GetDiagnostics
OpenSettingsRequested
StartWindowCapture
CancelWindowCapture(capture_id)
ShutdownEngine
```

EngineからSettingsへのtyped eventは次に限定する。

```text
ConfigChanged(revision)
HealthChanged(snapshot)
WindowCaptureResult(capture_id, outcome, window_identity)
```

`OpenConfigDirectory`はpathを受け取らない。
Engineが所有するuser config directoryをplatformのfile managerで開き、typed response
`OpenConfigDirectoryResult::Opened | Failed(reason)`を返す。
arbitrary pathをSettingsから渡すfile-open APIにはしない。

window captureのhook、active capture ID、cancel stateはEngineが所有する。
`StartWindowCapture`は既存captureを停止してから新しいIDを返し、即時の
`CancelWindowCapture`もhook install前後のどちらで到着しても観測される。
結果はcapture IDと対応し、cancelledまたは置換済みIDから成功eventを送らない。
SettingsはTauri event名やprocess-local mutexをcaptureの正準protocolにしない。

汎用command実行、任意file read/write、raw OS input injectionを公開しない。
methodごとにtyped requestへdecodeし、境界通過後にJSON objectやdynamic mapを持ち回らない。

## Config document transfer

一般frameの1 MiB上限を維持し、config documentを一frameへinlineしない。
`GetSnapshot`、`ApplyConfig`、`ImportConfig`、`ExportConfig`だけが次のtyped transferを使う。

```text
BeginConfigUpload(Apply | Import, expected_revision, total_bytes, sha256) -> transfer_id
AppendConfigChunk(transfer_id, index, chunk_b64) -> next_index
FinishConfigUpload(transfer_id)
GetSnapshot | ExportConfig -> transfer_id, revision, total_bytes, sha256
ReadConfigChunk(transfer_id, index) -> chunk_b64, next_index
AbortConfigTransfer(transfer_id)
```

`chunk_b64`はRFC 4648 standard Base64 alphabetと必須paddingを使うcanonical string一種類に固定し、whitespace、別alphabet、不正padding、非canonical encodingを拒否する。
transport境界はまずlength prefixとenvelopeを含むencoded frame全体が1 MiB未満であることを検証し、次に`chunk_b64`をdecodeしてpayloadが256 KiB以下であることを別々に検証する。
`total_bytes`、chunkの累積length、SHA-256はBase64 textではなくdecoded config bytesを対象とする。
送信側もdecoded payloadを256 KiB以下に切り、length prefixとenvelopeを含むencoded frameが1 MiB未満であることを検証してから送る。

`transfer_id`はconnection-scopedで、一connectionにつきactive config transferは一つ、未acknowledged chunkも一つに限定する。
さらにEngine全体でactive config transferは一つ、upload temporary fileの合計bytesは64 MiB以下に固定する。
`BeginConfigUpload`で宣言値がbudgetを超える場合と、実際のdecoded bytesが宣言値またはbudgetを超える場合は`ResourceLimit`でabortする。
zero-based indexを厳密な順序で処理し、checked arithmeticで累積lengthを検証する。
downloadはrevisionをpinし、uploadはfinish時にも`expected_revision`を再検証する。
`ApplyConfig`/`ImportConfig`はtransfer purposeとexpected revisionが一致しないcompleted uploadを拒否する。

uploadはEngine-owned temporary fileへ逐次writeし、downloadはclientが一chunkずつpullする。
serialized document全体を一bufferへ読むheap allocation、chunk queue、declared total sizeによる事前allocationをしない。
finish時にactual lengthとSHA-256をdescriptorへ照合してからtemporary fileをstreaming JSON decodeし、Config transactionへ進む。
順序違反、duplicate、length超過、不一致hash、idle timeout、明示abort、connection closeではtransferをabortし、不完全temporary fileを削除してactive configを変更しない。

Config ownerはdecode/compileを同時に一件だけ実行し、immutable snapshotはactiveとpreparedの最大二revisionだけを保持する。
Config compiler固有のchecked counterで、両snapshotが所有するheap allocationと現在のdecode/compile scratch allocationを合計64 MiB以下に制限する。
container capacity、decoded string/action payload、index、scratch bufferをallocation前に加算してfallible reserveし、超過は`ResourceLimit`としてcandidateだけを破棄する。
temporary fileの64 MiB disk budgetと、この64 MiB accounted memory budgetは別々のEngine aggregate ceilingであり、通常時allocationまたはidle RSSの目標値ではない。
process全体の汎用resource managerや任意blob用budget frameworkは作らない。

config schema自体にはlogical total-size上限を追加しないが、64 MiBを超える既存valid v1 documentのactive化とcompileは互換性の明示例外とする。
その場合も元fileを変更、削除、truncate、暗黙default化せず、gestureをdisableしてinputをfail-openにする。
`GetSnapshot`/`ExportConfig`はsnapshotへのdecodeや再serializeを行わず、元fileのpinned read-only handleから直接chunkをpullできるため、bytesをlosslessにrecovery/exportできる。
storage、budget、fallible decode failureはactive configを変更しない非破壊errorにする。
このceilingはP04の実測で再評価し、値を変更する場合はADR amendmentを必須にする。
このprotocolを任意blobや複数streamの汎用frameworkへ拡張しない。

## Config transaction

EngineのConfig ownerだけが次の順で更新する。

1. request、schema version、expected revisionを検証する。`ApplyConfig`/`ImportConfig`では完了済みuploadのpurpose、length、hashも検証する。
2. platform capabilityを含むsemantic validationを一度だけ行い、次revisionのimmutable runtime snapshotへcompileする。
3. Inputに`PrepareConfig(revision, snapshot)`を送り、terminal `Commit | Abort`専用delivery slotをreserveしたackを得る。Inputはまだactive snapshotを変更しない。
4. active fileと同じdirectoryのtemporary fileへserializeし、flush/fsyncする。
5. temporary fileをactive fileへatomic replaceする。この成功をlogical commit pointとする。
6. directory metadata syncを試みる。
7. reserved slotへallocationもfailureもない`Commit(revision)`を送り、Inputの次snapshotを切り替える。
8. metadata sync成功時は`Success(revision)`、失敗時は`SuccessWithDurabilityWarning(revision, reason)`をSettingsへ返す。

`PrepareConfig`後、atomic replaceが成功するまでのwrite、flush、fsync、replace failureはreserved slotへ`Abort(revision)`を送り、active fileとrunning snapshotを変更せずrequestを失敗させる。
slotをreserveできなければtemporary fileへ書き始めない。
atomic replace後はrollbackできないため、directory metadata sync failureでも`Commit`を続行し、diagnosticを残してfailure responseへ戻さない。
reserved `Commit` deliveryのfailureはprotocol invariant違反であり、Engineをterminate/restartしてinputをfail-openにする。restart時はnew active fileを正本にする。
通常のprocess crashでは、atomic replace前なら旧active file、replace後なら新active fileをrestart時の正本とし、Input snapshotをdiskから再構築する。
system/power crashがatomic replace成功後かつdirectory metadata sync成功前に起きた場合、filesystem上で旧file、新file、または不完全なmetadataのどれが残るかを保証しない。
restart recoveryはactive/temporary/backupのvalidityとrevisionを検査し、一意に選べるvalid candidateだけを採用する。
一意に復旧できなければfileを上書きせずEngineをdisabled/fail-openにし、diagnostic recoveryを要求する。
metadata sync成功後のsystem/power crash durabilityはplatform/filesystemが提供する保証の範囲とする。
compile後のsnapshotはvalidとして扱い、ownerごとに再validationしない。
Settingsのexpected revisionが古い場合はconflictとして拒否し、last-write-winsにしない。

gesture開始時にInputがsnapshotを保持する。
更新中のgestureはそのsnapshotで終了し、次のgestureが新snapshotを使う。
上記prepareはInput deliveryの一枠予約であり、複数storage ownerを跨ぐ汎用二相commitやgesture中断protocolは追加しない。

新schemaの不正値はdefaultへ黙って補正せず、field pathを含むvalidation errorとして返す。
legacy migrationだけがversionごとの明示変換を行える。

[Config document transfer](#config-document-transfer)のresource budgetへadmitされたvalid legacy v1 documentは、observable behaviorを保ってschema v2へmigrationする。
budget超過documentのactive化とcompileは、同節で定義したbyte-preserving recovery/exportだけを保証する明示的なcompatibility exceptionである。
一方、現行のread/JSON/validation failure時のsilent defaultとsilent correctionはpreservationからの意図的compatibility exceptionとする。

- startupでvalid snapshotが一つもない場合、Engineをdisabled/fail-openにする。
- runtime更新または再読込に失敗しlast known valid snapshotがある場合、そのsnapshotを維持してinvalid documentをactivateしない。
- invalid fileをdefaultで上書き、削除、破壊的修正しない。
- diagnostic stateとvalidation pathをSettingsへ返し、edit/import/resetと`OpenConfigDirectory`によるrecoveryを可能にする。

## Migration and reinstall preservation

configはTauriが解決するuser application config directoryに置き、app bundle、Program Files、executable隣接directoryへ置かない。
stable application identifierを維持する。

- 既存schemaなしWindows configからschema v2へ一度だけ前方migrationする。
- migration前fileを同じuser config directoryへbackupする。
- migration成功後にだけactive fileをatomic replaceする。
- downgrade migrationは実装しない。
- migration不能時は元fileを保持し、gestureをdefaultで成功したように起動せず、diagnostic状態にする。
- 新versionの再インストールとrepairではuser configを削除しない。
- config削除は明示的なReset/Delete user data操作だけが行う。

uninstaller固有のdata removal optionを将来追加する場合も、既定値はkeepである。
自動更新機能は明示的なnon-goalであり、初期architectureにupdater責務や将来用interfaceを置かない。
upgrade経路は新versionのinstallerをuserが実行する再インストールだけである。

## Availability and failure conditions

- IPC disconnectはEngineのgesture operationを停止しない。
- malformed clientはそのconnectionだけを閉じ、Engineをpanicさせない。
- atomic replace前のconfig persistence失敗をmemory-only successとして返さない。
- atomic replace後のmetadata sync failureはrollback不能なのでdurability warning付きsuccessとして返す。
- 通常のprocess crash後はreplace前なら旧、replace後なら新active fileからsnapshotを再構築する。
- replace後かつmetadata sync前のsystem/power crashはold/newを保証せず、valid candidateを一意に選べない場合はdisabled/fail-openでdiagnostic recoveryへ移る。
- stale revisionは現行documentを上書きしない。
- endpoint access controlを設定できない場合、serverを公開せずEngineをdegradedにする。
- protocol mismatch時にSettingsはraw file editへfallbackしない。

## Consequences

configとrunning stateのownerが一つになり、Settings crashや複数起動によるlost updateを防げる。
JSON costはcontrol ownerだけに限定される。
internal APIを必要以上に安定化せず、version mismatchとreinstall境界だけを丁寧に扱える。
