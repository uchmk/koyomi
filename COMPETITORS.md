# 競合との比較

公式の文書で確かめたものは事実、そうでなければ「推測」と書く。企画役が更新する。

最後に確かめた日: 2026-11-02。比べたのは JP1/AJS3（Version 13、13-50 まで）・Control-M（SaaS と 9.0.22）・Rundeck（6.2）・Airflow（3.3.2）・Windows のタスク スケジューラ。
cron は表 1 だけに置く（表 2・表 3 はほとんど × なので省いた）。

- 印は ○ ある・△ 一部（有償の追加・別の製品・条件つき）・× 無い（文書にそう書いてある、または製品の外）・? 文書に見当たらない。
- 表の印は、下の「出どころ」に書いた公式の文書で確かめた事実。「見当たらない」と ? は、文書に書かれていなかっただけで、無いと確かめたわけではない（推測）。
- koyomi の列は 0.35.0 に入っているもの。途中のものは TODO の番号を書く。
- 行の番号（P・C・H）は「出どころ」から引くためのもの。

## まとめ

行は koyomi の機能から選んだので、○ の数で製品の良し悪しは比べられない。

**koyomi が強いところ**

- 保存の前に予定の変わり方を見せる（P15）のは koyomi だけ。近いのは Control-M の Forecast の What-If（別の画面で模擬する）。
- 眠る PC のための機能（P16・C5・H3）。サーバーの製品（JP1・Control-M・Rundeck・Airflow）は対象の外。
  タスク スケジューラは PC を起こす（`WakeToRun`）と動かす条件（H3）を持つが、予定に印は付けない。
- Windows の PC 1 台に exe 1 つで、無料（H25・H26）。
  - JP1 は Windows では Manager に Server の OS が要り、有償（日立の例で買取 125 万円＋保守 28 万円/年、税抜）。
  - Control-M は RAM 20 GB〜と Java 17。SaaS は月 $2,400〜。
  - Rundeck は Java 17 と、本番では外の DB。
  - Airflow は Windows では直接は動かない（WSL2 かコンテナ）。
- 前の営業日を引数に埋め込める（C1）のは koyomi と Control-M（`%%$WCALC`）。JP1 は予定日だけ。
- メールで起動（H13）。JP1 は 32 bit の Outlook が要り、Control-M・Rundeck には見当たらない。
- ほかに 1 つしか無いか、どれにも無いもの:
  - 対応済みの印（C13）は Control-M だけ（Alerts の Reviewed）。
  - 終わりの見込みで先に知らせる（H2）は Control-M の SLA Management だけ（オンプレは追加の製品）。
  - タスク スケジューラからの取り込み（H23）は Control-M の Conversion Tool だけ。
  - 外への生存の合図（H18）は、どれにも専用のものが見当たらない（Rundeck・Airflow は代わりの方法がある）。
  - 全部のネットを止める枠と、そのために流れない回を先に見せること（N1）。枠は Rundeck の有償の Blackout にあるが、影響する回の一覧はどれにも無い。
  - 何台かの PC に流して「どれか 1 つが通れば正常」（N4）は、どの製品の文書にも無い（Airflow の `one_success` が近い）。
  - PC が空いたら起動（N7）は、ほかにはタスク スケジューラだけ。

**koyomi が弱いところ**

- 多くの PC で流す（H9）。JP1・Control-M はエージェント、Rundeck は SSH・WinRM で、1 か所から多くの PC に流す。
  koyomi も ssh・winrm の部品で koyomi の無い PC に流せ、`[groups]` の PC にまとめて流せる（N4）。
  ただ、相手の PC で予定を持って流すには PC ごとに koyomi を入れ、`[[remote]]` で LAN の koyomi どうしをつなぐ。
- Web UI（H10）。タスク スケジューラ以外は Web が本体。koyomi の本体は窓で、Web は見る・流す・止める・流し直すまで（定義を直すのは窓）。
- つながりの広さ。Control-M は MFT（FTP・SFTP・S3・Azure）、Airflow は provider（SFTP・IMAP・HTTP など）で広げる。
  koyomi が送る・受けるのは SFTP・共有フォルダー・S3（と S3 互換）までで、FTP・Azure Blob は無い。
  古い履歴は `archive = true` で月ごとの zip にしまえる（TODO 92。Control-M の Workload Archiving にあたる）。
- 表 4 の残り: サインイン・ロックで起動（N7。TODO 115 は ito 待ち）と Azure Blob（N9）。
- 権限の細かさ（H12）。JP1 は資源グループごとに 5 つのロール、Rundeck は ACL でジョブごと。koyomi は view・run・edit の 3 つで、`nets` でネット・グループごとに絞れ、`roles` でネット・グループごとに違うロールを持てる（JP1 の 5 つほど細かくはない）。

**差が無くなったところ**

- MCP（H24）は Rundeck（6.1〜、公式）と Control-M（試験中）も持つ。
- 承認の待ち（C10）は JP1（保留）・Control-M（Confirm）・Airflow（3.1 からの HITL）も持つ。
- HA（H16）は、待機系が自動で引き継ぐようになった（2026-11-01 にサービスで確かめた）。JP1 はクラスタ（共有ディスク）、Control-M は二次ホスト、Airflow は scheduler を複数動かす。
  koyomi は共有のディスクも DB も要らない。戻すのは人か自動かを選べ、相手で動いていた回はネットごとに続きから流せる（2026-11-01、TODO 96）。
- 表 4（N1〜N9）は、競合にあって koyomi に無かったもの。2026-11-02 に TODO 106〜114 で入れた。
  添えて送るメール（N2）・共通の値（N3）・日を決めた切り替え（N5）・別の PC に配る（N6）・SQL（N8）・S3（N9）は、持つ製品と並んだ。

## 1. 基本の機能

2 周目（2026-10-29）に確かめた行。Control-M の列は 2026-11-01 に足した。

| | 機能 | koyomi | JP1/AJS3 | Control-M | Rundeck | Airflow | タスク スケジューラ | cron |
|---|---|---|---|---|---|---|---|---|
| P1 | ジョブの定義と手動実行 | ○ TOML とエディター。今すぐ流す | ○ | ○ Folder/Job を定義（UI・JSON）、Run Now で条件を無視して即実行 | ○ | ○ | ○ | △ 今すぐ流すコマンドが無い |
| P2 | 時刻のスケジュール | ○ `[[when]]` の種類をエディターで選ぶ。cron 式は持たない | ○ ルールは 144 個まで | ○ 曜日・月日・特定日、From/To、Cyclic（間隔/特定時刻）、Time zone | ○ Quartz の cron 式 | ○ cron 式・timetable | ○ cron 式は無い | ○ |
| P3 | 逃した回（止まっていた間の分） | ○ 既定は流さない。`missed = "once"` で 1 回だけ | ○ exec・skip・plan | ○ Retroactive Run で停止中に逃した回を流すか選ぶ。Keep Active で最大 98 日待機 | × 有償版で「逃した」と記録するだけ | ○ catchup（3.x の既定は切） | ○ StartWhenAvailable | × anacron は日単位で ○ |
| P4 | 打ち切り時間と強制終了 | ○ 孫のプロセスまで止める | ○ Windows では子プロセスまでは止めない | △ Kill/ctmkilljob は全プロセスを終了。時間超過は通知のみ、自動 Kill は SLA 管理ジョブ | ○ | △ 画面の「失敗にする」で止まるかは文書に見当たらない | ○ 既定 72 時間 | × |
| P5 | リトライ | ○ `retry`（回数・間隔） | ○ 1〜12 回・1〜10 分 | ○ RerunLimit（回数・間隔）、If-Action の Rerun Job、Cyclic | ○ | ○ | ○ | × |
| P6 | 前の回が動いているとき | ○ 飛ばす・`overlap = "queue"`・`max` で並べる | ○ 待つ・飛ばす | △ 専用設定なし。Cyclic は前回後に次、特定時刻は Tolerance 超過で次へ、Lock で直列 | ○ 待たせるのは有償版 | △ 同時の数の上限だけ | ○ 並行・待つ・無視・前を止める | × |
| P7 | 依存（ジョブネット）・分岐 | ○ 線 3 種・分岐・待ち合わせ・子のジョブネット | ○ 判定ジョブ・リカバリー | ○ Events（AND/OR/NOT）で依存、If-Actions（Add Event 等）で分岐 | ○ 条件の分岐は有償版 | ○ | △ 順に流すだけ | × |
| P8 | 失敗したところからの再実行 | ○ 失敗したジョブから・選んだジョブから・次から | ○ 異常終了ジョブから・その次から・指定したジョブから・指定したものだけ | ○ 失敗ジョブを Rerun→後続へ。Rerun from point of failure は FT/DB 等 | △ 失敗したノードだけ流し直す。失敗した手順から続けるのは有償版 | ○ Clear（Downstream・Failed） | 見当たらない | × |
| P9 | 実行履歴とログ | ○ 既定 90 日。`archive = true` で月ごとの zip にしまう | ○ 1〜99 世代、標準出力はファイル名を書いたときだけ | ○ 出力 OUTPUTRETN 既定 1 日・ログ 2 日。長期は Workload Archiving（最長 7 年） | ○ 既定 60 日・50 回 | ○ | △ 最後の 1 回の結果だけ | △ syslog の行だけ |
| P10 | 失敗の通知 | ○ システムの通知・ベル・webhook・メール。ネットごとの宛先 | ○ メール・JP1 イベント | ○ Mail・Alerts・Remedy 等。Teams は Communication Suite | ○ メール・webhook | ○ コールバック | × メールの動作は廃止 | △ 出力があればメール |
| P11 | 休日・営業日のカレンダー | ○ 日本の祝日・振替・第 n 営業日・`shift` | ○ 振替 4 種と猶予日数 | ○ RBC・Confirmation calendar、前後の営業日、Shift By、Dn/Ln | △ 有償版で、飛ばすだけ | △ timetable を自分で書く | × | × |
| P12 | 出来事での起動 | ○ ファイル・起動時・別のネット・ログの行・イベントログ・メール・HTTP | ○ ファイル・イベントログ・メールなど | ○ File Watcher job（作成・削除・最小サイズ・安定確認） | △ webhook だけ | ○ AssetWatcher（ファイル待ちは勧めない） | ○ イベントログ・起動・ログオン（ファイルは無い） | × @reboot だけ |
| P13 | 遅延の監視 | ○ 開始の遅れ・終わりの遅れ・いつもより長い・終わりの見込み | ○ 開始遅延・終了遅延 | ○ Not Submitted/Not Finished by、Execution Time（平均比も） | ○ 平均より長いとき | △ Deadline Alerts は試験中 | × | × |
| P14 | これからの予定の表示 | ○ 次の 10 回・月の予定表 | ○ 日・月の予定の窓 | ○ Forecast（オンプレは Add-on 扱い、SaaS は System Settings で有効化） | △ API と CLI の forecast だけ | △ 次の 1 回 | △ 登録したタスクの GetRunTimes | × |
| P15 | 保存の前に、予定の変わり方を見せる | ○ 保存の前に 7 日分の差分 | 見当たらない | △ Forecast What-If で変更後を模擬（別画面。保存時の差分表示ではない） | 見当たらない | 見当たらない | 見当たらない | × 予定を出さない |
| P16 | 眠っている PC の予定を前もって見せる | ○ 次の 10 回に印。PC を起こす | 見当たらない | △ What-If の Downtime で停止枠にかかる回を確認。PC の起床は記載なし | 見当たらない | 見当たらない | △ タスクごとに起こす（`WakeToRun`）。起こせないときの知らせは見当たらない | × |

## 2. 運用の細かいところ

3 周目の候補。2026-10-29 には記憶による推測だった。2026-11-01 に公式の文書で確かめると、競合 4 つの 72 個の印のうち 36 個が違っていた。

| | 機能 | koyomi | JP1/AJS3 | Control-M | Rundeck | Airflow | タスク スケジューラ |
|---|---|---|---|---|---|---|---|
| C1 | 引数・環境変数への業務日の埋め込み | ○ `{date-1bd}` など（前の営業日も） | △ 環境変数 AJSEXDATE に予定日（yyyy/mm/dd）。前営業日の算出は記載なし | ○ %%ODATE、%%$CALCDATE、%%$WCALC（前後の営業日）、%%PREV | △ `${DATE:fmt}` で日付を埋め込み、日の加減もできる。営業日の扱いは記述なし | △ {{ ds }}・logical_date・macros.ds_add あり。前営業日のマクロはなく plugin で自作 | × 日付の変数なし。使えるのは $(Arg0)… と ValueQueries のみ |
| C2 | 秘密をジョブに渡す・出力で伏せる | ○ `secret:`。出力では `***` | ? ジョブに秘密を渡す仕組みは公式で見つからず | △ Secrets in Code は Connection Profile 等用。出力の伏せ字は記載なし | ○ Key Storage を Secure option で渡し、Mask Passwords で伏せる | ○ Connections・Variables・secrets backend。password 等はログで自動で伏字 | × 実行アカウントのパスワード保管のみ。動作へ秘密を渡す仕組みなし |
| C3 | 出力の文字コード（Shift_JIS） | ○ `encoding` | ? ジョブ出力の文字コードをジョブ単位で指定する機能は公式で見つからず | ○ Agent の CJK Encoding で JAPANESE SHIFT-JIS（I18N=CJK） | △ WinRM ノードの `output-charset` で文字コードを指定（既定 UTF-8） | △ BashOperator の output_encoding（既定 utf-8）を変えられる（standard provider） | × 出力を取り込まない。公式は transcript やスクリプト側の出力を案内 |
| C4 | 実行の前の点検 | ○ `koyomi check` | ○ ajschkdef：実行ファイル・作業パスの有無と権限、順序のループ等を点検 | ○ Site Standards＋ctm build。オンプレは Workload Change Mgr | △ option の必須・正規表現の点検で、合わない値は実行の前に弾く。試し実行の記述なし | ○ DagBag の import_errors のテスト・dag.test()（コードと CLI で点検） | △ TASK_VALIDATE_ONLY は XML の構文だけ確かめる（API のみ） |
| C5 | 実行中は PC を眠らせない | ○ 流している間は眠らせない | ? 公式で見つからず | ? 公式文書で見つからず | × サーバー製品で対象外（スリープを止める機能の記述なし） | × Linux のサーバーが前提。PC を起こしておく機能の記載なし | ○ WakeToRun で起こし、終わるまで起きたままにする |
| C6 | 出力の文字列・ファイルで終了を判定 | ○ 出力の文字列・ファイルで判定 | ○ 判定ジョブ（終了コード・ファイル有無・変数比較）。出力は引き継ぎ情報設定ジョブで抽出 | ○ Specific Statement Output（文字列一致）→Set to OK/Not OK | △ 有償 Conditional Logic で出力を判定。ファイル有無は記述なし | △ exit code で成否・skip_on_exit_code・FileSensor。出力の文字列の判定は自作 | × 終わりの判定は Last Run Result（終了コード）のみ |
| C7 | 次の 1 回だけ飛ばす・時刻をずらす | ○ 次の 1 回を飛ばす・ずらす | ○ 一時変更：次回予定の実行中止、実行日時の一時変更、即時実行 | △ 発注済み（当日）の回のみ Hold/Skip/Bypass。先の回・時刻ずらしは未確認 | △ 有償の Calendars の blackout で日時を除外。1 回だけ動かす機能は記述なし | ? 推測: 次の 1 回だけ飛ばす・ずらす機能なし。pause か分岐の task で代用 | × トリガーに除外日・1 回だけの移動なし |
| C8 | ジョブの間で値を渡す | ○ `KOYOMI_OUT` | ○ 引き継ぎ情報設定ジョブ：先行ジョブの標準出力から値を抽出して後続へ | ○ Named pool/SMART folder 変数、Set Variable、出力から Capture | ○ Key Value Data で取り出し、Global Variable で別ジョブへ | ○ XCom（TaskFlow の戻り値も XCom） | × 動作は最大 32 個を順に実行するだけ。分岐なし |
| C9 | 排他（同じ資源を同時に使わせない） | ○ `lock`（排他と共有 `shared = true`。koyomi.toml の `[locks]` で数） | △ 排他実行リソースはサブミットジョブ（jpqjobsub）だけ。ジョブネットのジョブでは使えない | ○ Lock（Exclusive/Shared）、Resource Pool（数量） | △ 同じジョブは既定で 1 本ずつ。Job Queue は有償。ジョブをまたぐロックは記述なし | ○ pool（slot 1 なら排他）。pool_slots で重さを指定 | × MultipleInstancesPolicy は同じタスクの中だけ |
| C10 | 人の確認待ち | ○ 承認のステップ | ○ 保留属性：人が保留解除するまでそのジョブで待つ | ○ Confirm（Wait User）→Monitoring で承認 | ? 承認のステップは文書に見当たらず。要望の Issue #4288 だけ | ○ HITL operators（3.1〜、standard provider）。UI のフォームで応答 | × 承認の手順なし |
| C11 | 期間を指定してまとめて流す | ○ `run --date A..B` | △ 確定実行登録で回数・期間分の予定を確定。過去日の一括実行は記載なし | △ ctmorder -ODATE で過去日を 1 日ずつ。範囲指定の一括発注は見つからず | ? 記述なし。推測: rd CLI を日ごとに回すスクリプト | ○ Backfill を UI・CLI・REST API から。欠け・失敗・全部を選んで流す | × StartWhenAvailable は逃した分の後追い起動のみ。期間指定なし |
| C12 | ほかのジョブネットの終わりを待つ | ○ `wait_for` | ○ ジョブネットコネクタ（ルートジョブネット間）・待ち合わせ条件 | ○ Events でフォルダ間、Global events でサーバー間 | ○ Job Reference・Job State Conditional・Executions Wait Result | ○ Asset で後続を起動・ExternalTaskSensor（standard provider） | △ 依存設定なし。EventTrigger で他タスクの Operational ログを拾う |
| C13 | 失敗に「対応済み」の印 | ○ 対応済みの印 | △ 状態変更でジョブの状態・戻り値を変える。「対応済み」の印はない | ○ Alerts を Set Reviewed/Set Closed、Comment 付き | ? 記述なし。要望の Issue #9312（acknowledged の状態）だけ | △ Dag run に note を付けられる。専用の「対応済み」状態は記載なし | × 対応済みの印なし |
| C14 | 操作の記録（誰が流した・止めた・変えた） | ○ `koyomi ops` | ○ スケジューラーログに操作した JP1 ユーザー・要求元 IP・操作を記録 | ○ Audit（認証・定義・操作等を選択、既定 90 日）、Audit report | △ 有償の Audit Trail Log（4.6 から）と Audit Stream（5.13 から） | ○ Audit Logs（UI の Browse・REST /eventLogs）。UI/API/CLI の操作を記録 | △ 監査 4698〜4702 で作成・変更・削除（監査ポリシーが要る） |
| C15 | ガントチャート | ○ 今日のガント | ○ View［デイリースケジュール］：世代を時間軸上の四角（開始〜終了）で表示 | ○ Workspaces の Timeline View（予定の Gantt、4 年分） | ? 公式の文書に見当たらず | ○ Gantt を 3.1 で Grid に組み込み（3.1.1 で既定オフ） | × ガント表示なし |
| C16 | 月の予定表と印刷 | ○ 月の予定表・印刷・PDF | △ View［マンスリースケジュール］で表示。印刷は Print Option（別製品）の月次帳票 | △ View Schedule で 1 年分の実行日を表示。印刷は見つからず | ? 月の表示は見当たらず。今後の予定は Forecast API で取れる | △ Calendar view（3.1〜、予定の cron 実行も出す）。印刷は記載なし | × 月のカレンダー表示なし |
| C17 | 常駐のアイコン（タスクトレイ） | ○ トレイ | ? 公式で見つからず | ? 公式文書で見つからず | × サービスとして動く Web アプリで、常駐アイコンはない | × ブラウザの Web UI のみ。常駐アイコンの記載なし（Linux サーバー前提） | × トレイ常駐なし |
| C18 | ジョブネットの間のつながりの図 | ○ つながりの図 | △ ネストジョブネット間は関連線で図示。ルートジョブネット間の図は記載なし | ○ Neighborhood（上流/下流を半径指定）、フォルダ間の Events を線で表示 | △ ui-job-graph プラグイン（DAG）。2025 年に archived | ○ Asset Graph と Graph の「All Dag Dependencies」重ね表示 | × 依存関係の図なし |

## 3. 広げた機能

6 周目（TODO 72〜88）。2026-11-01 に公式の文書で確かめた。

| | 機能 | koyomi | JP1/AJS3 | Control-M | Rundeck | Airflow | タスク スケジューラ |
|---|---|---|---|---|---|---|---|
| H1 | イベントログ・外の監視に書く | ○ イベントログ（ソース `koyomi`、状態ごとの ID） | ○ Windows イベントログ／syslog へ出力、JP1 イベント発行。SNMP は見つからず | ○ Notify 先に SNMP trap・Mail・Remedy・Shout。Event Log は記載なし | △ Email・Webhook、有償で Slack 等。SNMP・イベントログは記述なし | △ metrics を StatsD・OpenTelemetry に送る。Windows イベントログは記載なし | ○ TaskScheduler/Operational に記録。ID 一覧は公式に見当たらず |
| H2 | 終わりの見込みで先に知らせる | ○ `late_end` に間に合わない見込みを先に | △ 開始遅延・終了遅延・所要時間超過の監視のみ。終了の予測は記載なし | △ SLA Management（旧 BIM）が遅れを予測し通知。オンプレは add-on | △ Average Duration Exceeded で超過を通知。締切の予測はなし | △ Deadline Alerts（実験的）。AVERAGE_RUNTIME 基準は可、終わりの予測で先に知らせるのは記載なし | × 遅れの見込み通知なし。ExecutionTimeLimit で止めるのみ |
| H3 | 動かす条件（AC 電源・ネットワーク・アイドル） | ○ `require` | ? 公式で見つからず | △ Agent Restrictions の CPU 上限のみ。AC 電源・ネット条件は記載なし | × サーバー製品で対象外（AC 電源・アイドルの条件の記述なし） | × AC 電源・アイドルの条件の記載なし（Linux サーバー前提） | ○ 電源・ネットワーク・アイドルの条件（DisallowStartIfOnBatteries 等） |
| H4 | ログファイルの行・イベントログで起動 | ○ `[[when]] log`・`eventlog` | ○ ログファイル監視ジョブ・Windows イベントログ監視ジョブ（同梱の JP1/Base のトラップ） | ? File Watcher はファイルの有無・サイズのみ。行・イベントログ起動は未発見 | ? Webhook で外から起動はできる。ログ・イベントログの監視は見当たらず | △ AssetWatcher＋BaseEventTrigger（MQ・ファイル）。ログ行・イベントログは trigger を自作 | △ EventTrigger（XPath）でイベントログは可。ログファイルの行は不可 |
| H5 | 枠が埋まったときの優先度 | ○ `priority = 1..5` | ○ ジョブの実行優先順位、ルートジョブネット優先順位の一時変更、実行多重度 | ○ Priority（Very High〜Very Low / AA〜99）で実行順 | × Quartz の待ちは FIFO。優先度の要望 #1848 は not planned で閉じた | ○ priority_weight（下流の分も足す）で queued の順を決める | △ Priority 0–10 はプロセス・I/O・メモリの優先度。待ち順ではない |
| H6 | よくある作業の部品（消す・zip・写す・空ける） | ○ 消す・zip・写す・空ける | ? ファイル削除・圧縮・コピーの部品は公式で見つからず | △ MFT の File Transfer job に rm・mkdir・rename・元削除。zip 段は未発見 | △ Copy file だけ。削除・zip の部品は見当たらず。File Transfer は有償 | △ 古いファイルの削除・zip・コピーの専用部品は見当たらず。Bash/Python で書く | × Exec・ComHandler のみ。メール・メッセージの動作は廃止 |
| H7 | ファイル転送（SFTP など） | ○ SFTP・共有フォルダー・S3 へ送る・受ける | △ 標準カスタムジョブ JP1FTP。送受両側に JP1/FTP（別製品）が要る | △ Control-M MFT（add-on）で FTP・SFTP・S3・Azure 等 | ○ Copy file ステップ（OSS）。File Transfer のステップは有償 | △ SFTP provider の SFTPOperator（別に入れる）。FTP provider は既定で入る | × FTP/SFTP の部品なし |
| H8 | ジョブごとに実行のアカウント | ○ `run_as` | ○ ジョブごとの実行ユーザー＋JP1 ユーザー→OS ユーザーのユーザーマッピング | ○ RunAs でジョブ・フォルダごとに OS ユーザーを指定 | ○ ノード属性 username に `${option.x}` でジョブごとに指定 | ○ run_as_user（Unix ユーザーへ sudo -u、passwordless sudo が要る） | ○ タスクごとに実行アカウント（利用者・S4U・SYSTEM・グループ） |
| H9 | 複数の PC・エージェント | ○ `[[remote]]`（LAN、TLS） | ○ マネージャーから実行エージェント・エージェントグループで実行 | ○ 各ホストに Agent、Host Group で振り分け | ○ SSH・WinRM で複数のノードに実行。Runner もある | △ Celery・Edge・Kubernetes executor（各 provider）。Edge の Windows は PoC | △ schtasks /s・-CimSession で遠隔の PC を 1 台ずつ。集中管理なし |
| H10 | Web UI | ○ 今日・履歴・ログを見る、流す・止める・流し直す・飛ばす・保留。見た目は窓の今日の画面と同じ（TODO 81）。定義を直すのは窓 | ○ Web GUI（ジョブポータル／マネジメントポータル）で監視・実行・保留。Web Console 要 | ○ Control-M Web（Planning・Monitoring 等）をブラウザで | ○ ブラウザーの UI が本体 | ○ React の Web UI（API server）。17 言語 | △ Windows Admin Center の Scheduled tasks ツール（別製品・履歴なし） |
| H11 | REST API・HTTP で起動 | ○ `POST /runs` など | ○ HTTP の API：ユニットの実行登録と実行情報の取得 | ○ Automation API の REST（run/order・run status） | ○ REST API と Webhook（Run Job）。高度な Webhook は有償 | ○ REST API（v2）で DAG run の起動・照会 | × GUI と COM API・PowerShell・schtasks のみ。HTTP の口なし |
| H12 | 利用者とロール | ○ `[[http.user]]` に view・run・edit。`nets = ["経理/*"]` でネット・グループごとに絞り、`roles = { "経理/*" = "run" }` でネットごとに違うロール | ○ 資源グループごとに 5 種のロール（JP1_AJS_Admin〜Guest） | ○ Role で権限。既定ロール 3 つ、API でも設定可 | ○ ACL ポリシーでプロジェクト・ジョブごとの権限 | △ 標準の Simple auth manager は固定 4 ロールで開発用。本番 RBAC は FAB 等の provider | △ ロールなし。タスクごとの DACL で読む・変える・実行を分ける |
| H13 | メールを受けて起動 | ○ `[[when]] mail`（IMAP） | △ メール受信監視ジョブ。Windows は 32bit 版 Outlook 必須（64bit・365 版不可） | ? メール受信での起動は未発見（Mail は送信の Action のみ） | ? メールでの起動は見当たらず | △ IMAP provider の ImapAttachmentSensor（添付の名前を待つ） | × メールのトリガーなし |
| H14 | 同じフローを並べて動かす（上限つき） | ○ `max` | △ ルートジョブネットごとに多重起動の可否。本数の上限は記載なし | ○ 同じ folder を New で何度も order 可。同時数は Resource:Pool | ○ Multiple Executions と Limit で同時の本数の上限 | ○ DAG の max_active_runs・設定の max_active_runs_per_dag | ○ MultipleInstancesPolicy=Parallel。同時数の上限は指定できない |
| H15 | サービスとして動かす（サインインなし） | ○ 入れるのに管理者のターミナルが要る | ○ Windows サービス。JP1/Base の起動管理で OS 起動時に自動起動 | ○ Windows の Agent は既定で Local System のサービス | ○ Windows サービスとして入れられ、サインインなしで動く | △ Linux 用の systemd unit あり。Windows は WSL2/コンテナのみでネイティブ非対応 | ○ Password・S4U（schtasks /np）でログオンなしに実行 |
| H16 | HA（待機系の引き継ぎ） | ○ 待機系が `/health` を見張り、答えなければ引き継ぐ（共有ディスク・DB は要らない）。戻すのは人か自動（`give_back = "auto"`。1 台目の `[primary]` と受け渡して両方で流さない）。動いていた回はネットごとに続きから流せる（`takeover = "rerun"`） | ○ クラスタ構成（論理ホスト・共有ディスク、jajs_setup_cluster） | ○ EM・Server の二次ホストへ自動 failover。SaaS は HA 込み | △ 有償の Cluster＋Autotakeover。外部 DB と LB が要る | ○ scheduler を複数動かせる（Postgres/MySQL 8 の行ロック） | △ Failover Clustering の clustered scheduled task（AnyNode 等） |
| H17 | HTTP を呼ぶ部品（状態コードで判定） | ○ `kind = "http"` | ○ HTTP 接続ジョブ。ステータスコードごとに終了コード 0〜9 を割当（Win/Linux） | △ Web Services REST plug-in。HTTP コードで成否を決める設定は記載なし | ○ HTTP Request ステップ。5.12 から応答コードの一覧・範囲・2xx で判定 | △ HttpOperator・HttpSensor の response_check（http provider） | × HTTP の動作なし |
| H18 | 外への生存の合図 | ○ 外の URL を決めた間隔で叩く | ? 外部への生存通知は公式で見つからず | ? Watchdog の異常時スクリプトはあるが外への定期の合図は未発見 | △ 専用の機能は見当たらず。成功時の Webhook 通知で代わりにできる | △ scheduler_heartbeat を StatsD/OTel に送る。外の URL への ping は記載なし | × 生存の合図（heartbeat）なし |
| H19 | 成績の報告（成功率・所要） | ○ `koyomi report --month` | △ 13-50 で実行エージェント稼働状況レポート。分析は Job Analysis（別） | ○ Reports: Jobs Executions（平均時間）・Workloads 統計。成功率専用は記載なし | ○ メトリクス API・rd metrics・Prometheus で成功率と時間 | △ Runs の所要時間の列・Grid の task duration。成功率の報告は記載なし | × 成績の報告なし |
| H20 | 定義をまたいだ検索・置換 | ○ 差分を見せてまとめて直す | △ View［検索］で配下を検索。一括編集は Definition Assistant（別製品） | ○ Planning の Find / Update（Replace Substring 等）。Workspace 内 | ? 置換は見当たらず。書き出して rd jobs load で読み直す方法はある | ? 推測: DAG は Python のファイルで、製品の中の横断検索・置換はなし | △ UI になし。Get-/Set-ScheduledTask を組めば可 |
| H21 | 手で流すときに値を渡す | ○ `[params]`・`--set` | ○ 実行登録時にマクロ変数の値（引き継ぎ情報）を指定 | ○ run order の設定ファイルで variables を渡せる | ○ 手で流すと「Choose Execution Options」の画面で値を聞く | ○ Params と Trigger のフォーム（型・既定値・検証） | △ schtasks /run に引数なし。COM の Run(params) で $(Arg0)… |
| H22 | 繰り返しの部品 | ○ `kind = "loop"` | ? ループの部品は公式で見つからず（自動リトライ・サイクルジョブはある） | △ Rerun（cyclic）の Times で N 回。ファイルごとの loop は未発見 | △ Loop Script Plugins（有償） | ○ dynamic task mapping（expand） | × 動作に繰り返しの部品なし（トリガーの Repetition のみ） |
| H23 | タスク スケジューラからの取り込み | ○ XML の取り込み | ? 公式で見つからず | ○ Conversion Tool が MS Task Scheduler に対応（後処理で元を無効化） | ? 取り込みは見当たらず。一覧だけの PS1 Scheduled Jobs List がある | × Linux 前提。タスク スケジューラの取り込みの記載なし | ○ 取り込み元そのもの。XML を schtasks /create /xml で入れ直せる |
| H24 | AI・MCP | ○ MCP | △ AI エージェントは JP1 Cloud Service 側。AJS3 本体の MCP は記載なし | ○ MCP Server（Preview、API 9.0.22.125〜）。生成 AI の Jett も | ○ 公式の MCP サーバー（別に配布、6.1 以上。OSS にも RBA にもつながる） | △ common.ai provider（LLM・MCPHook はクライアント側）。公式 MCP サーバーは記載なし | × AI・MCP なし |
| H25 | 価格・ライセンス | ○ MIT OR Apache-2.0、無料 | △ Mgr1+Agent3+View1：買取 125 万＋保守 28 万/年、サブスク 60 万/年（税抜） | △ SaaS Starter $2,400/月〜（年払い）。実行数課金、オンプレは見積 | ○ OSS は Apache-2.0。有償 SaaS $125/人/月＋基本料、自前版は要問合せ | ○ Apache License 2.0。マネージドは Astro・MWAA・Cloud Composer など | ○ Windows に同梱。別売りなし |
| H26 | 入れる手間・前提（Windows の PC 1 台） | ○ exe 1 つ。DB・Java は要らない | △ Windows 版 Manager は Server OS のみ（Win11 は View）。DB・Base 同梱 | △ EM+Server で 4vCPU・RAM 20GB〜・Java 17 要。小型 PC 1 台は不向き | △ Java 17+、Windows Server 2019+、本番は外部 DB。Win 11 は記述なし | △ Python 3.10〜3.14・Postgres/MySQL・メモリ 4GB〜・本番は Linux のみ。Windows は WSL2/コンテナ | ○ 全ての Windows に自動で入り、OS の起動で動く |
| H27 | 定義の版・差分・戻す | ○ `versions/`・差分・戻す | △ ジョブネットリリースで指定日時に定義を切替。差分・版の管理は記載なし | ○ Changes History で版の一覧・JSON 比較・復元（180 日保持） | △ Git の SCM プラグインで版を管理。差分・戻す操作の記述はなし | △ DAG versioning（run ごとに版を記録、Git bundle なら旧版で再実行）。差分・戻すは記載なし | × RegistrationInfo の Version は文字列のみ。履歴・差分・戻しなし |
| H28 | Excel・CSV への書き出し | ○ CSV・.xlsx | △ ajsshow で書式指定出力。CSV・Excel は別製品（Print Option 等） | ○ Reports を CSV・XLSX・PDF で出力（履歴・定義など） | △ ジョブは YAML/XML/JSON、履歴は API とアーカイブ。CSV は見当たらず | △ UI の CSV 書き出しは記載なし。CLI・REST API（JSON）から自作 | △ schtasks /query /fo CSV でタスク一覧。履歴の CSV は記載なし |

## 4. 9 周目で足した機能

9 周目（TODO 106〜114）。表 1〜表 3 の外で、競合にあって koyomi に無かったもの。2026-11-02 に公式の文書で確かめ、同じ日に入れた。
残りは N7 のサインイン・ロック（TODO 115、ito 待ち）と N9 の Azure Blob。

| | 機能 | koyomi | JP1/AJS3 | Control-M | Rundeck | Airflow | タスク スケジューラ |
|---|---|---|---|---|---|---|---|
| N1 | 計画停止の日時 | ○ koyomi.toml の `[[blackout]]`（`from`・`to`・`why`）の間はどのネットも流さない。止まる回を今日・カレンダー・`koyomi plan` が先に見せる。手で流すのは止めない（TODO 106 で入れた） | △ 一時変更の「実行中止」をジョブネットごとに（ルートジョブネットは複数の予定をまとめて）。全部を止める日時の枠・影響する回の表示は見当たらない | △ What-If の Downtime で停止の枠を入れたときの影響を模擬（実際には止めない）。Workload Policy に日時の期間があるが、0 本にできるかは見当たらない | △ 有償の System Calendars の Blackout（日付の範囲・時間帯、全プロジェクトに当てられる、手での実行は止めない）。止まるジョブには印が出るが、影響する回の一覧は見当たらない | ? 計画停止の枠は文書に見当たらず | × 停止の枠なし（推測: API に項目が無い） |
| N2 | ファイルを添えてメールを送るジョブ | ○ `kind = "mail"`（`to`・`cc`・`subject`・`body`・`files`。20 個・18 MB まで）。知らせと同じ `[[notify.mails]]` の SMTP で送る（TODO 107 で入れた） | ○ メール送信ジョブ。添付は Windows だけで 20 個まで。Outlook 経由（32bit の 2016・2019・2021 のみ、64bit・365 は不可）か SMTP へ直接（SMTPS は不可） | △ If-Action の Mail でジョブの出力を添付（AttachOutput）。任意のファイルを付けて送る job type は見当たらない | △ メールは通知だけで、ログを添付できる。任意のファイルを付けて送るステップは見当たらない | △ smtp provider の EmailOperator の `files` で添付。3.3.2 では smtp provider は既定で入らない（別に入れる） | × メールの動作（添付あり）は Windows 7・2008 R2 までで廃止。代わりに PowerShell の Send-MailMessage を勧める |
| N3 | どのジョブからも読める共通の値 | ○ koyomi.toml の `[vars]` をどのネットのコマンド・パス・`params` の既定からも `{var:名前}` で。秘密は `secret:`（TODO 108 で入れた） | △ マクロ変数は実行登録のときに指定し、配下のユニットで効く。全ジョブで共通の変数は見当たらない | ○ Global（`%%\`、self-hosted の Server の全ジョブ）と Named Pool の変数。ctmvar・Set Variable で書き換え | ○ `project.globals.X` をプロジェクトの全ジョブで `${globals.X}` | ○ Variables（global な key/value、`Variable.get`・`{{ var.value.x }}`） | × 共通の値なし（推測: API に項目が無い） |
| N4 | 何台かの PC にまとめて流す（同時数・全部かどれか） | ○ ssh・winrm の `on = "group:web"` で koyomi.toml の `[groups]` の PC に。同時数は `max`、全部で正常か どれか 1 つで正常かは `pass = "all" \| "any"`（TODO 109 で入れた） | × 実行エージェントグループは 1 台へ振り分けるだけ（優先順位・多重度・使用率）。全台で同時に流す機能は見当たらない | ○ RunOnAllAgentsInGroup（"Run job on all hosts in group"）で、グループの全 Agent に同じジョブ（それぞれ別の Order ID）。同時数の上限・全部かどれかの成功は見当たらない | ○ ノードへ配り、Thread Count で同時数、失敗したら止めるか残りを流すかを選べる。どれか 1 つの成功で正常は見当たらない | △ ホストへ配る機能なし。dynamic task mapping でホストごとに写し（SSH など）を作り、`max_active_tis_per_dag` で上限、後続の trigger rule `all_success`・`one_success` | × `schtasks /s` で 1 台ずつ |
| N5 | 決めた日時から新しい定義に切り替える | ○ エディターの「この日から使う」か `koyomi stage … --from YYYY-MM-DD` で置き、その日の最初の回から新しい定義で流す。置くときに差分を見せて確認する（TODO 110 で入れた） | ○ ジョブネットリリースで、指定した日時から定義を切り替え | △ When の期間（StartDate・EndDate、ActivePeriod=false で止める）で旧・新を受け渡す。定義の版を日時で切り替える機能は見当たらない | ? 指定した日時からの定義の切り替えは見当たらず | △ 旧の DAG に `end_date`、新の DAG に `start_date` を書いて受け渡す。版を日時で切り替える機能は見当たらない | △ StartBoundary・EndBoundary で 2 つのタスクを受け渡す |
| N6 | 定義を別の PC・環境に配る | ○ 窓の「別の PC に送る」か `koyomi remote send` で `[[remote]]` の PC へ、差分を見せて送る（向こうでは未確認で置かれ、向こうで確認するまで流れない）。`export`・`import` と HA の待機への写しも（TODO 111 で入れた） | △ ajsexport→ajsimport（パッケージング機能用のスクリプトの中だけで使える）、ajsprint→ajsdefine。配る・昇格する仕組みは見当たらない | ○ Workspace を JSON に書き出し、別の環境へ取り込む。Automation API の Deploy Descriptor で Dev・Test・Prod ごとに値を置き換え | ○ プロジェクトのアーカイブで別のインスタンスへ移す（`rd projects archives`）、SCM Git の Export・Import | △ Git の DAG bundle が `tracking_ref`（ブランチ・タグ・SHA）を追う。環境の間の昇格の機能は見当たらない | △ XML へ書き出し、`schtasks /create /xml` で取り込む。ドメインなら GPP の Scheduled Task で配る |
| N7 | PC が空いたら起動する（サインイン・ロックも） | △ `[[when]] idle = "10m"` で空いたら起動（TODO 112 で入れた）。サービスのエンジンからは空いたかが見えない。サインイン・ロックは TODO 115（ito 待ち） | × イベントジョブ 8 種にアイドル・ログオン・ロックなし | ? 公式で見つからず | × サーバーの製品で対象外 | × Linux のサーバー前提で、アイドルなどで起動するものなし | ○ Idle トリガー（Windows 8 以降は 4 分・80%）、ログオン、SessionStateChange（ロック・アンロック・接続・切断） |
| N8 | SQL を流すジョブ | ○ `kind = "sql"`（SQL Server・PostgreSQL・SQLite。TODO 113 で入れた） | △ 本体のジョブの種類に DB のジョブなし。別製品 JP1/AJS3 for Cloud Service Applications のサンプルに Athena・BigQuery の SQL | ○ Databases のジョブ（SQL Script・Stored Procedure・Embedded Query。Oracle・MSSQL・DB2・PostgreSQL など、JDBC で足せる）。Agent に plug-in を入れる | △ 有償の SQL Runner（node step）。OSS は MongoDB Command Runner だけ | ○ 既定で入る common.sql の SQLExecuteQueryOperator（DB ごとの provider は別に入れる） | × 動作はプログラムの起動だけ |
| N9 | クラウドのストレージ（S3・Azure Blob）へ送る・受ける | △ `send`・`fetch` の `s3://` で S3 と S3 互換（MinIO など）へ（TODO 114 で入れた）。Azure Blob はまだ | △ 別製品 JP1/AJS3 for Cloud Service Applications のサービス連携ジョブのサンプル（S3・Azure Blob・GCS・OCI の upload・download など、サポート契約者向け） | △ MFT（add-on）の File Transfer ジョブで S3・Azure Storage・GCS など | △ AWS S3 の node step（OSS、Copy・Sync・Move でローカルとの間も）。Azure Blob は見当たらない。File Transfer は file・ftp・sftp・http だけ | △ amazon provider の LocalFilesystemToS3Operator・S3 の operator、microsoft.azure provider の LocalFilesystemToWasbOperator（どちらも別に入れる） | × 動作はプログラムの起動だけ |

表 4 で分かったこと:

- 全部を 1 つの枠で止め、影響する回を一覧するもの（N1）は、どの製品にも見当たらない。影響を先に見せるのは Control-M の What-If だけで、それも模擬。
- 任意のファイルを添えて送るジョブ（N2）は JP1 だけ。Control-M・Rundeck は通知に出力やログを付けるだけ。
- 全台で同時に流せる（N4）のは Control-M と Rundeck。JP1 のエージェントグループは 1 台へ振り分けるだけ。「どれか 1 つが通れば正常」はどの製品の文書にも無い（Airflow の `one_success` が近い）。
- 空いたら・サインイン・ロックで起動（N7）はタスク スケジューラだけ。
- SQL とクラウドのストレージ（N8・N9）は、JP1 では別製品のサンプル、Control-M では add-on、Airflow でも多くは別に入れる provider。

## 出どころ

表 1 の行は 2 周目（2026-10-29）に読んだ文書。表 2・表 3 と Control-M は 2026-11-01、表 4（N）は 2026-11-02 に読んだ文書。

- **JP1/AJS3**（日立、Version 13 の日本語のマニュアル）: 頭は `https://itpfdoc.hitachi.co.jp/manuals/3021/`
  - 表 1:
    - 定義と即時実行: `30213L4200/AJSF0039.HTM`・`30213L4920/AJSO0096.HTM`・`30213L4200/AJSF0060.HTM`
    - スケジュールと休業日の振替: `30213L4200/AJSF0045.HTM`
    - 逃した回（`OVERSCHEDULE`・`STARTMODE`）: `30213L4510/AJSX0350.HTM`・`30213L4610/AJSJ0073.HTM`
    - 打ち切り時間（1〜1,440 分）と強制終了: `30213L4810/AJSV0302.HTM`・`30213L4200/AJSF0081.HTM`
    - リトライ: `30213L4810/AJSV0302.HTM`・`30213L4400/AJSG0039.HTM`
    - 再実行（`ajsrerun` の `-a`・`-n`・`-s`・`-t`・`-w`・`-f`・`-p`・`-e`）: `30213L4900/AJSO0124.HTM`
    - 多重起動とスケジュールスキップ: `30213L4200/AJSF0046.HTM`
    - リカバリーと判定ジョブ: `30213L4400/AJSG0032.HTM`・`30213L4400/AJSG0029.HTM`
    - 保存世代数と出力: `30213L4200/AJSF0065.HTM`・`30213L4400/AJSG0099.HTM`
    - メール送信ジョブ: `30213L4400/AJSG0093.HTM`・`30213L4400/AJSG0018.HTM`
    - イベントジョブ: `30213L4400/AJSG0030.HTM`
    - 遅延の監視: `30213L4400/AJSG0062.HTM`
    - デイリー・マンスリースケジュール: `30213L4200/AJSF0068.HTM`・`30213L4810/AJSV0053.HTM`
    - 保存の前の予定の差分: 見当たらない。近いのは `ajsprint` で書き出して外の道具で比べること（定義の比較で、予定の比較ではない）と、
      登録したあとに予定を見ること（`30213L4610/AJSJ0205.HTM`・`30213L4610/AJSJ0091.HTM`・`30213L4200/AJSF0069.HTM`）。
  - 表 2・表 3（Version 13、13-50 まで）:
    - C1: `30213L4920/AJSO0029.HTM`
    - C4: `30213L4420/AJSG0108.HTM`
    - C6・H4: `30213L4420/AJSG0014.HTM`
    - C7・C10・C13: `30213L4220/AJSF0071.HTM`
    - C8: `30213L4420/AJSG0035.HTM`
    - C9: `30213L4620/AJSJ0128.HTM`
    - C11: `30213L4420/AJSG0057.HTM`
    - C12・C18: `30213L4220/AJSF0039.HTM`
    - C14: `30213L4700/AJSK0088.HTM`
    - C15: `30213L4820/AJSV0065.HTM`
    - C16: `30213L5310/ES160008.HTM`
    - H1: `30213L4700/AJSK0008.HTM`
    - H2: `30213L4420/AJSG0062.HTM`
    - H5: `30213L4420/AJSG0103.HTM`
    - H7: `3021311220/AJSL0067.HTM`
    - H8: `30213L4220/AJSF0130.HTM`
    - H9: `30213L4220/AJSF0090.HTM`
    - H10・H11: `30213L4220/AJSF0135.HTM`
    - H12: `30213L4420/AJSG0072.HTM`
    - H13: `30213L5010/AJSL0018.HTM`
    - H14: `30213L4220/AJSF0046.HTM`
    - H15: `30213D2200/AJSH0086.HTM`
    - H16: `3021310720/AJSJ0138.HTM`
    - H17: `30213L4420/AJSG0038.HTM`
    - H19: `30213L4220/AJSF0158.HTM`
    - H20: `30213L4800/AJSV0102.HTM`
    - H21: `30213L4420/AJSG0022.HTM`
    - H24: `https://www.hitachi.co.jp/Prod/comp/soft1/jp1/feature/ai_agent/index.html`
    - H25: `https://www.hitachi.co.jp/Prod/comp/soft1/jp1/product/jp1/price/download/jp1v1310_systemconfig_04_job_j.pdf`
    - H26: `30213L4300/AJSH0014.HTM`
    - H27: `30213L4220/AJSF0085.HTM`
    - H28: `30213L4920/AJSO0131.HTM`
    - 見当たらない（URL なし）: C2・C3・C5・C17・H3・H6・H18・H22・H23
  - 表 4:
    - N1: `30213L4220/AJSF0071.HTM`・`30213L4220/AJSF0076.HTM`
    - N2: `30213L5010/AJSL0008.HTM`・`30213L5010/AJSL0044.HTM`・`30213L5010/AJSL0047.HTM`・`30213L5010/AJSL0018.HTM`・`30213L5010/AJSL0013.HTM`・`30213L5010/AJSL0048.HTM`
    - N3: `30213L4220/AJSF0061.HTM`
    - N4: `30213L4420/AJSG0024.HTM`
    - N5: `30213L4220/AJSF0085.HTM`
    - N6: `30213L4900/AJSO0109.HTM`（ajsexport）・`30213L4900/AJSO0112.HTM`
    - N7・N8: `30213L4420/AJSG0014.HTM`（ジョブの種類・イベントジョブの一覧）
    - N8・N9（別製品 JP1/AJS3 for Cloud Service Applications）: `30213L6120/AJSC0005.HTM`・`30213L6120/AJSC0008.HTM`、
      `https://www.hitachi.co.jp/Prod/comp/soft1/jp1/product/jp1/ajs/pdf/jp1cs_autom_samplelist.pdf`・`https://www.hitachi.co.jp/Prod/comp/soft1/jp1/hack/automation/0007.html`
- **Control-M**（BMC。SaaS の文書と、オンプレの 9.0.21・9.0.22 の文書）: 頭は `https://documents.bmc.com/supportu/controlm-saas/en-US/Documentation/`。頭の違うものは全部書く。
  - 表 1〜表 3:
    - P1・P8・C7・C10・C18: `Job_management.htm`
    - P2・P3・P6・P11: `Job_scheduling.htm`
    - P4: `Utilities/ctmkilljob.htm`
    - P4: `API_CodeRef_JobTypes_sla.htm`
    - P5: `https://documents.bmc.com/supportu/API/Monthly/en-US/Documentation/API_CodeRef_JobProperties.htm`
    - P7・P10・P13・C6: `Job_actions.htm`
    - P9: `https://documents.bmc.com/supportu/9.0.22/en-US/Documentation/Archive_Service.htm`
    - P12・H4: `File_Watcher_Job.htm`
    - P14・P15・P16: `Forecast.htm`
    - C1・C8: `Variables.htm`
    - C2: `API_CodeRef_SecretsInCode.htm`
    - C3: `https://documents.bmc.com/supportu/9.0.21/en-US/Documentation/East_Asian__CJK__language_support_configuration.htm`
    - C4: `Site_Standard_Management.htm`
    - C9: `Job_prerequisites.htm`
    - C11: `Utilities/ctmorder.htm`
    - C12: `Events.htm`
    - C13: `Alerts.htm`
    - C14: `System_Settings.htm`
    - C15: `Workspaces.htm`
    - C16: `Calendars.htm`
    - H1: `https://documents.bmc.com/supportu/9.0.21/en-US/Documentation/Job_actions.htm`
    - H2: `SLA_Management_Job_parameters.htm`
    - H3: `Agent_Restrictions.htm`
    - H5・H8・H14・H22: `API_CodeRef_JobProperties.htm`
    - H6・H7: `File_Transfer_Job.htm`
    - H7: `Add-ons.htm`
    - H9: `Host_Groups.htm`
    - H10: `https://documents.bmc.com/supportu/9.0.22/en-US/Documentation/Getting_Started.htm`
    - H11・H14・H21: `API_Services_RunService.htm`
    - H12: `Users_and_Roles.htm`
    - H15: `Agent_Post-installation_Procedures.htm`
    - H16: `https://documents.bmc.com/supportu/9.0.22/en-US/Documentation/High_Availability.htm`
    - H17: `Jobs_for_Web_Services.htm`
    - H19・H28: `Control-M_Reports.htm`
    - H20: `Finding_and_updating_jobs.htm`
    - H23: `Conversion/Converting_to_Control-M.htm`
    - H24: `https://documents.bmc.com/supportu/9.0.22/en-US/Documentation/MCP_Server.htm`
    - H25: `https://www.bmc.com/it-solutions/control-m-pricing.html`
    - H26: `https://documents.bmc.com/supportu/9.0.22/en-US/Documentation/Control-M_full_installation_system_requirements.htm`
    - H27: `https://documents.bmc.com/supportu/9.0.21/en-US/Documentation/Changes_History.htm`
    - 見当たらない（URL なし）: C5・C17・H13・H18
  - 表 4（9.0.22 と Monthly の API の文書は 403 で読めず、SaaS の文書で見た）:
    - N1: `Forecast.htm`・`Workload_Policies.htm`
    - N2・N4・N5: `API_CodeRef_JobProperties.htm`（Mail の AttachOutput・RunOnAllAgentsInGroup・When の StartDate・EndDate・ActivePeriod）
    - N2・N8: `Job_types.htm`
    - N3: `Variables.htm`
    - N4: `Host_Groups.htm`・`https://documents.bmc.com/supportu/9.0.20/help/Main_help/en-US/JefMultiAgent.htm`
    - N6: `Workspaces.htm`・`API_DeployDescriptor.htm`
    - N8: `DB_Configuration.htm`
    - N9: `File_Transfer_Job.htm`・`Add-ons.htm`
    - 見当たらない（URL なし）: N7
- **Rundeck**: 頭は `https://docs.rundeck.com/docs/`。「有償版」は PagerDuty Runbook Automation。
  - 表 1:
    - 定義・今すぐ実行・スケジュール・打ち切り・リトライ: `manual/jobs/creating-jobs.html`
    - 逃した回: `manual/schedules/missedjobfires.html`
    - 二重起動と待ち行列: `manual/jobs/job-queue.html`
    - 流れと条件: `manual/jobs/job-workflows.html`・`manual/jobs/conditional-logic.html`
    - 再実行: `rd-cli/commands.html`（`rd retry`。既定で失敗したノードだけ）・`manual/jobs/job-resume.html`（失敗した手順から続ける。有償版）
    - 履歴: `manual/08-activity.html`・`manual/projects/execution-history-cleaner.html`（どの版の機能かは書かれていない）
    - 通知と遅延: `manual/jobs/job-notifications.html`
    - カレンダー: `manual/calendars.html`
    - webhook での起動: `manual/webhooks/run-job.html`
    - 予定: `api/`（`GET /api/31/job/[ID]/forecast`）・`rd-cli/commands.html`。画面に次の回が出るかは文書に見当たらない（推測）。
  - 表 2・表 3（6.2 の文書）:
    - C1・C4・H21: `manual/jobs/job-options.html`
    - C2: `manual/log-filters/mask-passwords.html`
    - C3: `manual/projects/node-execution/powershell.html`
    - C5・H3・H26: `administration/install/system-requirements.html`
    - C6: `manual/jobs/conditional-logic.html`
    - C7: `manual/calendars.html`
    - C8・C12: `manual/jobs/job-plugins/workflow-steps/builtin.html`
    - C8: `manual/log-filters/`
    - C9: `manual/jobs/job-queue.html`
    - C10: `https://github.com/rundeck/rundeck/issues/4288`
    - C11: `rd-cli/commands.html`
    - C13: `https://github.com/rundeck/rundeck/issues/9312`
    - C14: `administration/security/audit-trail.html`
    - C16・H11: `api/`
    - C17・H15: `administration/install/windows.html`
    - C18: `https://github.com/rundeck-plugins/ui-job-graph`
    - H1・H18: `manual/notifications/`
    - H2: `manual/jobs/job-notifications.html`
    - H4・H11・H13: `manual/webhooks.html`
    - H5: `https://github.com/rundeck/rundeck/issues/1848`
    - H6・H7・H22: `manual/jobs/job-plugins/`
    - H8・H9: `manual/projects/node-execution/ssh.html`
    - H10: `manual/`
    - H12: `administration/security/authorization.html`
    - H14: `learning/getting-started/jobs/pieces-of-a-job.html`
    - H16: `administration/cluster/autotakeover/`
    - H17: `manual/jobs/job-plugins/workflow-steps/http-request.html`
    - H19: `administration/monitoring/execution-metrics.html`
    - H20: `manual/document-format-reference/job-yaml-v12.html`
    - H23: `manual/jobs/job-plugins/node-steps/ps1-scheduled-jobs-list.html`
    - H24: `mcp/`
    - H25: `https://www.pagerduty.com/pricing/process-automation/`
    - H25: `https://github.com/rundeck/rundeck`
    - H27: `manual/projects/scm/git.html`
    - H28: `manual/projects/project-archive.html`
    - 見当たらない（URL なし）: C15
  - 表 4:
    - N1: `manual/calendars.html`・`manual/calendars/system-calendars.html`
    - N2: `manual/notifications/email.html`
    - N2・N8: `manual/jobs/job-plugins/`（SQL Runner・MongoDB Command Runner）
    - N3: `manual/jobs/job-variables.html`
    - N4: `manual/jobs/creating-jobs.html`（Dispatch to Nodes・Thread Count・If a node fails）
    - N6: `manual/projects/project-archive.html`・`manual/projects/scm/git.html`
    - N9: `manual/jobs/job-plugins/node-steps/aws-s3.html`・`manual/jobs/job-plugins/workflow-steps/azure.html`・`manual/jobs/job-plugins/workflow-steps/file-transfer.html`
    - 見当たらない（URL なし）: N5
- **Apache Airflow 3.3.2**: 頭は `https://airflow.apache.org/docs/apache-airflow/stable/`
  - 表 1:
    - 手動の起動・catchup・再実行（Re-run Tasks の Clear）: `core-concepts/dag-run.html`
    - スケジュール: `core-concepts/dags.html`・`authoring-and-scheduling/timetable.html`
    - 打ち切り・リトライ: `core-concepts/tasks.html`
    - 同時の数: `https://airflow.apache.org/docs/task-sdk/stable/api.html`（`max_active_runs`）
    - 履歴とログ: `ui.html`・`howto/usage-cli.html`（`airflow db clean`）
    - 通知: `administration-and-deployment/logging-monitoring/callbacks.html`
    - 休日: `howto/timetable.html`（休日を飛ばす timetable の例）
    - 出来事での起動: `authoring-and-scheduling/event-scheduling.html`
    - 遅延: `howto/deadline-alerts.html`
  - 表 2・表 3（provider は standard 1.21.0・edge3 5.1.0 の stable）:
    - C1: `templates-ref.html`
    - C2: `security/secrets/mask-sensitive-values.html`
    - C3: `https://airflow.apache.org/docs/apache-airflow-providers-standard/stable/_api/airflow/providers/standard/operators/bash/index.html`
    - C4: `best-practices.html`
    - C5・H3・H23・H26: `installation/prerequisites.html`
    - C6・H6: `https://airflow.apache.org/docs/apache-airflow-providers-standard/stable/operators/bash.html`
    - C8: `core-concepts/xcoms.html`
    - C9・H5: `administration-and-deployment/pools.html`
    - C10: `https://airflow.apache.org/docs/apache-airflow-providers-standard/stable/operators/hitl.html`
    - C11: `core-concepts/backfill.html`
    - C12: `authoring-and-scheduling/asset-scheduling.html`
    - C12: `https://airflow.apache.org/docs/apache-airflow-providers-standard/stable/sensors/external_task_sensor.html`
    - C13・C17・C18・H10・H19: `ui.html`
    - C14: `security/audit_logs.html`
    - C15: `https://airflow.apache.org/docs/apache-airflow/3.1.0/release_notes.html`
    - C15・C16: `https://airflow.apache.org/blog/airflow-3.1.0/`
    - C16: `release_notes.html`
    - H1・H18: `administration-and-deployment/logging-monitoring/metrics.html`
    - H2: `howto/deadline-alerts.html`
    - H4: `authoring-and-scheduling/event-scheduling.html`
    - H7: `https://airflow.apache.org/docs/apache-airflow-providers-sftp/stable/index.html`
    - H8: `security/workload.html`
    - H9: `core-concepts/executor/index.html`
    - H9: `https://airflow.apache.org/docs/apache-airflow-providers-edge3/stable/install_on_windows.html`
    - H11: `stable-rest-api-ref.html`
    - H12: `core-concepts/auth-manager/simple/index.html`
    - H12: `core-concepts/auth-manager/index.html`
    - H13: `https://airflow.apache.org/docs/apache-airflow-providers-imap/stable/_modules/airflow/providers/imap/sensors/imap_attachment.html`
    - H14: `configurations-ref.html`
    - H15: `howto/run-with-systemd.html`
    - H16: `administration-and-deployment/scheduler.html`
    - H17: `https://airflow.apache.org/docs/apache-airflow-providers-http/stable/operators.html`
    - H21: `core-concepts/params.html`
    - H22: `authoring-and-scheduling/dynamic-task-mapping.html`
    - H24: `https://airflow.apache.org/docs/apache-airflow-providers-common-ai/stable/hooks/mcp.html`
    - H25: `https://github.com/apache/airflow/blob/main/LICENSE`
    - H25: `https://airflow.apache.org/ecosystem/`
    - H27: `administration-and-deployment/dag-bundles.html`
    - H28: `howto/usage-cli.html`
    - 見当たらない（URL なし）: C7・H20
  - 表 4:
    - H7・N2・N8・N9（既定で入る provider）: `extra-packages-ref.html`。Preinstalled の印は Web では消えるので、
      GitHub のタグ 3.3.2 の `airflow-core/docs/extra-packages-ref.rst` で確かめた（印は common.sql・ftp・http・imap・sqlite だけ）
    - N2: `https://airflow.apache.org/docs/apache-airflow-providers-smtp/stable/_api/airflow/providers/smtp/operators/smtp/index.html`
    - N3: `core-concepts/variables.html`
    - N4: `authoring-and-scheduling/dynamic-task-mapping.html`・`core-concepts/dags.html`（trigger rule）
    - N5: `https://airflow.apache.org/docs/task-sdk/stable/api.html`（DAG の start_date・end_date）
    - N6: `administration-and-deployment/dag-bundles.html`
    - N8: `https://airflow.apache.org/docs/apache-airflow-providers-common-sql/stable/operators.html`・`https://airflow.apache.org/docs/apache-airflow-providers-common-sql/stable/supported-database-types.html`
    - N9: `https://airflow.apache.org/docs/apache-airflow-providers-amazon/stable/transfer/local_to_s3.html`・`https://airflow.apache.org/docs/apache-airflow-providers-amazon/stable/operators/s3/s3.html`・
      `https://airflow.apache.org/docs/apache-airflow-providers-microsoft-azure/stable/transfer/local_to_wasb.html`
    - 見当たらない（URL なし）: N1
- **Windows のタスク スケジューラ**: 頭は `https://learn.microsoft.com/en-us/windows/win32/taskschd/`
  - 表 1:
    - 今すぐ実行・次の予定: `registeredtask`・`registeredtask-getruntimes`
    - トリガー: `task-triggers`・`taskschedulerschema-eventtrigger-triggergroup-element`
    - 逃した回・リトライ・打ち切り・PC を起こす（`WakeToRun`）: `taskschedulerschema-settings-tasktype-element`・`tasksettings`
    - 二重起動: `tasksettings-multipleinstances`
    - 動作（順に流す・メールの廃止）: `task-actions`・`emailaction`
  - 表 2・表 3（Windows 11・Server 2022〜2025）:
    - C1・C8・H7・H17・H22: `task-actions`
    - C2・H8・H12・H15: `security-contexts-for-running-tasks`
    - C3・C6・H1: `https://learn.microsoft.com/en-us/troubleshoot/windows-server/system-management-components/troubleshoot-scheduled-tasks-not-running`
    - C4: `https://learn.microsoft.com/en-us/windows/win32/api/taskschd/ne-taskschd-task_creation`
    - C5: `https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nf-taskschd-itasksettings-get_waketorun`
    - C7・H4・H13: `trigger-types`
    - C9・H14: `taskschedulerschema-multipleinstancespolicytype-simpletype`
    - C10・C13・C15・C16・C17・C18・H11・H18・H19・H24・H25・H26: `about-the-task-scheduler`
    - C11・H2・H3: `taskschedulerschema-settings-tasktype-element`
    - C12: `taskschedulerschema-subscription-eventtriggertype-element`
    - C14: `https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/auditing/audit-other-object-access-events`
    - H5: `taskschedulerschema-priority-settingstype-element`
    - H6: `https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nn-taskschd-iemailaction`
    - H9・H28: `https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-query`
    - H10: `https://learn.microsoft.com/en-us/windows-server/manage/windows-admin-center/use/manage-servers`
    - H16: `https://learn.microsoft.com/en-us/powershell/module/scheduledtasks/register-clusteredscheduledtask`
    - H20: `https://learn.microsoft.com/en-us/powershell/module/scheduledtasks/get-scheduledtask`
    - H21: `https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nf-taskschd-iregisteredtask-run`
    - H23: `https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-create`
    - H27: `taskschedulerschema-version-registrationinfotype-element`
  - 表 4:
    - N2: `emailaction`（"This object is no longer supported"、End of client support は Windows 7）・`task-actions`
    - N4・N6: `https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2008-R2-and-2008/cc725745(v=ws.11)`（schtasks の `/s`・`/create /xml`）
    - N5: `trigger-endboundary`
    - N6: `https://learn.microsoft.com/en-us/troubleshoot/windows-server/group-policy/user-gpp-scheduled-task-item-fails-apply`
    - N7: `trigger-types`・`task-idle-conditions`・`sessionstatechangetrigger-statechange`
    - N8・N9: `task-actions`（動作はプログラムの起動。メール・メッセージは廃止）
    - N1・N3 の × は、API（トリガー・動作・設定）にその項目が無いことからの判断（推測）
- **cron**: cronie の `https://man7.org/linux/man-pages/man5/crontab.5.html`・`https://man7.org/linux/man-pages/man8/cron.8.html`、
  Debian の `https://manpages.debian.org/bookworm/cron/cron.8.en.html`、anacron の `https://man7.org/linux/man-pages/man8/anacron.8.html`
