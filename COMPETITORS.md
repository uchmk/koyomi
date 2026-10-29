# 競合との比較

公式の文書で確かめたものは事実、そうでなければ「推測」と書く。企画役が更新する。

最後に確かめた日: 2026-10-29（企画役 2 周目）。印は ○ ある・△ 一部・× 無い。表の印は、下の「出どころ」に書いた公式の文書で確かめた事実。
「見当たらない」は、文書に書かれていなかっただけで、無いと確かめたわけではない（推測）。
koyomi の列は、`TODO.md` の項目の番号か、`DECISIONS.md` で決めた版。

| 機能 | koyomi | JP1/AJS3 | Rundeck | Airflow | タスク スケジューラ | cron |
|---|---|---|---|---|---|---|
| ジョブの定義と手動実行 | TODO 1・2 | ○ | ○ | ○ | ○ | △ 今すぐ流すコマンドが無い |
| 時刻のスケジュール | TODO 5（種類を選ぶ欄はエディターで。cron 式は持たない） | ○ ルールは 144 個まで | ○ Quartz の cron 式 | ○ cron 式・timetable | ○ cron 式は無い | ○ |
| 逃した回（止まっていた間の分） | TODO 5（実行しない）・v0.1（1 回だけ） | ○ exec・skip・plan | × 有償版で「逃した」と記録するだけ | ○ catchup（3.x の既定は切） | ○ StartWhenAvailable | × anacron は日単位で ○ |
| 打ち切り時間と強制終了 | TODO 3（孫のプロセスまで止める） | ○ Windows では子プロセスまでは止めない | ○ | △ 画面の「失敗にする」で止まるかは文書に見当たらない | ○ 既定 72 時間 | × |
| リトライ | v0.1 | ○ 1〜12 回・1〜10 分 | ○ | ○ | ○ | × |
| 前の回が動いているとき | TODO 5（飛ばす）・v0.1（あとで） | ○ 待つ・飛ばす | ○ 待たせるのは有償版 | △ 同時の数の上限だけ | ○ 並行・待つ・無視・前を止める | × |
| 依存（ジョブネット）・分岐 | TODO 2（線）・v0.1（分岐・待ち合わせ） | ○ 判定ジョブ・リカバリー | ○ 条件の分岐は有償版 | ○ | △ 順に流すだけ | × |
| 失敗したところからの再実行 | v0.1（`rerun`・`run --from`） | ○ 異常終了ジョブから・その次から・指定したジョブから・指定したものだけ | △ 失敗したノードだけ流し直す。失敗した手順から続けるのは有償版 | ○ Clear（Downstream・Failed） | 見当たらない | × |
| 実行履歴とログ | TODO 2 | ○ 1〜99 世代、標準出力はファイル名を書いたときだけ | ○ 既定 60 日・50 回 | ○ | △ 最後の 1 回の結果だけ | △ syslog の行だけ |
| 失敗の通知 | TODO 4（システムの通知・ベル・webhook）・v0.2（メール） | ○ メール・JP1 イベント | ○ メール・webhook | ○ コールバック | × メールの動作は廃止 | △ 出力があればメール |
| 休日・営業日のカレンダー | v0.1（日本の祝日・振替・第 n 営業日） | ○ 振替 4 種と猶予日数 | △ 有償版で、飛ばすだけ | △ timetable を自分で書く | × | × |
| 出来事での起動 | v0.1（ファイル到着・起動時・別のジョブネット） | ○ ファイル・イベントログ・メールなど | △ webhook だけ | ○ AssetWatcher（ファイル待ちは勧めない） | ○ イベントログ・起動・ログオン（ファイルは無い） | × @reboot だけ |
| 遅延の監視 | v0.2 | ○ 開始遅延・終了遅延 | ○ 平均より長いとき | △ Deadline Alerts は試験中 | × | × |
| これからの予定の表示 | TODO 5（次の 10 回） | ○ 日・月の予定の窓 | △ API と CLI の forecast だけ | △ 次の 1 回 | △ 登録したタスクの GetRunTimes | × |
| 保存の前に、予定の変わり方を見せる | TODO 6（`DECISIONS.md` の「koyomi だけの強み」） | 見当たらない | 見当たらない | 見当たらない | 見当たらない | × 予定を出さない |
| 眠っている PC の予定を前もって見せる | 提案（2 周目。持ち主が決める） | 見当たらない | 見当たらない | 見当たらない | △ タスクごとに起こす（`WakeToRun`）。起こせないときの知らせは見当たらない | × |

## 確かめていない候補（推測。3 周目に公式の文書で確かめて本表へ移す）

企画役 3 周目の洗い出し（2026-10-29）。競合の印は記憶による推測で、公式の文書ではまだ確かめていない。
koyomi の列は `DECISIONS.md` の「3 周目の候補」の番号。

| 機能 | koyomi | JP1/AJS3 | Rundeck | Airflow | タスク スケジューラ | cron |
|---|---|---|---|---|---|---|
| 引数・環境変数への業務日の埋め込み | 候補 A1 | ○ マクロ変数 | △ オプションと日付の書式 | ○ Jinja の `{{ ds }}` | × | × |
| 秘密をジョブに渡す | 候補 A2 | 見当たらない | ○ Key Storage | ○ Connections・Variables | △ 実行ユーザーのパスワードだけ | × |
| 出力の文字コード（Shift_JIS） | 候補 A3 | 見当たらない | 見当たらない | 見当たらない | × | × |
| 実行の前の点検 | 候補 A4 | △ 定義の検査 | 見当たらない | △ DAG の読み込みの誤り | × | × |
| 実行中は PC を眠らせない | 候補 A6 | 見当たらない | 見当たらない | 見当たらない | △ 「アイドルなら止める」の設定 | × |
| 出力の文字列・ファイルで終了を判定 | 候補 A7 | △ 判定ジョブ | ○ ログのフィルター | △ センサー | × | × |
| 次の 1 回だけ飛ばす・時刻をずらす | 候補 A8 | ○ 計画一時変更 | △ スケジュールを切るだけ | △ 一時停止だけ | △ 無効にするだけ | × |
| ジョブの間で値を渡す | 候補 A9 | ○ 引き継ぎ情報 | △ | ○ XCom | × | × |
| 排他（同じ資源を同時に使わせない） | 候補 A10 | ○ 排他ジョブ | △ | ○ pool | × | × |
| 人の確認待ち | 候補 A11 | △ 保留で代わりにする | △ 有償版の承認 | ○ Human-in-the-loop | × | × |
| 期間を指定してまとめて流す | 候補 A12 | △ | × | ○ backfill | × | × |
| 時刻に加えて、ほかのジョブネットの終わりを待つ | 候補 A13 | ○ ジョブネットコネクタ | △ ジョブを呼ぶだけ | ○ ExternalTaskSensor・Asset | × | × |
| 失敗に「対応済み」の印 | 候補 B1 | 見当たらない | 見当たらない | △ メモを付けるだけ | × | × |
| 操作の記録（誰が流した・止めた） | 候補 B3 | ○ | ○ | ○ Audit Log | △ | × |
| 今日のガントチャート | 候補 C5 | ○ | △ | ○ Gantt | × | × |
| 月の予定表と印刷 | 候補 C6 | ○ マンスリースケジュール | △ | × | × | × |
| 常駐のアイコン（タスクトレイ） | 候補 C1 | 見当たらない | × ウェブの画面 | × ウェブの画面 | × | × |
| ジョブネットの間のつながりの図 | 候補 C11 | △ | × | △ Asset の図 | × | × |

## 出どころ

- **JP1/AJS3**（日立、Version 13 の日本語のマニュアル）: 頭は `https://itpfdoc.hitachi.co.jp/manuals/3021/`
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
- **Rundeck**: 頭は `https://docs.rundeck.com/docs/`。「有償版」は PagerDuty Runbook Automation。
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
- **Apache Airflow 3.3.2**: 頭は `https://airflow.apache.org/docs/apache-airflow/stable/`
  - 手動の起動・catchup・再実行（Re-run Tasks の Clear）: `core-concepts/dag-run.html`
  - スケジュール: `core-concepts/dags.html`・`authoring-and-scheduling/timetable.html`
  - 打ち切り・リトライ: `core-concepts/tasks.html`
  - 同時の数: `https://airflow.apache.org/docs/task-sdk/stable/api.html`（`max_active_runs`）
  - 履歴とログ: `ui.html`・`howto/usage-cli.html`（`airflow db clean`）
  - 通知: `administration-and-deployment/logging-monitoring/callbacks.html`
  - 休日: `howto/timetable.html`（休日を飛ばす timetable の例）
  - 出来事での起動: `authoring-and-scheduling/event-scheduling.html`
  - 遅延: `howto/deadline-alerts.html`
- **Windows のタスク スケジューラ**: 頭は `https://learn.microsoft.com/en-us/windows/win32/taskschd/`
  - 今すぐ実行・次の予定: `registeredtask`・`registeredtask-getruntimes`
  - トリガー: `task-triggers`・`taskschedulerschema-eventtrigger-triggergroup-element`
  - 逃した回・リトライ・打ち切り・PC を起こす（`WakeToRun`）: `taskschedulerschema-settings-tasktype-element`・`tasksettings`
  - 二重起動: `tasksettings-multipleinstances`
  - 動作（順に流す・メールの廃止）: `task-actions`・`emailaction`
- **cron**: cronie の `https://man7.org/linux/man-pages/man5/crontab.5.html`・`https://man7.org/linux/man-pages/man8/cron.8.html`、
  Debian の `https://manpages.debian.org/bookworm/cron/cron.8.en.html`、anacron の `https://man7.org/linux/man-pages/man8/anacron.8.html`
