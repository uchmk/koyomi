# TODO

## v0.1 の項目（企画で採用したもの）

仕様と受け入れ条件は `DECISIONS.md`。番号は `DECISIONS.md` の「`TODO.md` の 1〜6」と同じ。上から順に作る。

- [x] 1. 定義を読んで確かめる（L）: `crates/koyomi-core` の TOML のモデルと検査、中身の hash で見る確認済み、
  他のユーザーが書ける定義の止め、`ito-confirm` での確かめ、`koyomi check`・`koyomi approve`。
  `DECISIONS.md` の「1 周目」、「実行するコマンドの安全」、「ファイル」。
  - [x] `koyomi-core`: `jobnet`（TOML のモデルと検査、行番号つきの誤り、まだ使えない欄は「この版ではまだ使えない」）、
    `approved`（中身全体の sha256 で見る確認済み、`approved.toml`）、`scan`（`jobnets/<group>/<name>.toml` の読み）。
  - [x] `koyomi engine`（`ito_ipc::door`、`ito-watch` で定義の変化を追う）、`koyomi check`・`list`・`approve`・`engine status|stop`。
  - [x] 窓: 左の木（確認済み ✔・未確認 ▲・定義の誤り ✖）、右の内容、`ito-confirm` の確かめの箱（前に確かめた版との差分は `ito-diff`）。
  - [x] 他のユーザーが書ける場所（`ito_common::access`）は帯で知らせ、直るまで確かめない。
  - [x] 日本語の字体（`ito_fonts::register` + `ito_common::fonts::JAPANESE`）。
- [x] 2. 手で流すことと、結果とログ（L）: `koyomi engine`（`ito_ipc::door`）、`run`・`status`・`log`・`kill`、状態の一覧の文言と記号。
  `DECISIONS.md` の「1 周目」、「構成」、「CLI」、「3 周目の候補」の「D. 決めた細部」。
  - [x] `koyomi-core`: `run`（1 回の実行のモデル: `after` の順、全部の先行がそろって動く、異常の先の「未実行」、
    ジョブネットは異常が 1 つでも異常、状態の記号と文言の表）、`layout`（層に分けて並べる）、`history`（`[[event]]` の読み書きと戻し）、`config`（`koyomi.toml`）。
  - [x] エンジン: `ito_proc::run_lines` をワーカースレッドで回し、`logs/<日>/<実行>/<ジョブ>.log`（既定 10 MB まで）と
    `history/<年>/<月>/<日>.toml` を `kept::Writer` で書く。起動したときの定義の写しで最後まで流す。同じジョブネットが動いている間は断る。
    未確認・誤り・他のユーザーが書ける場所があるときは流さない。`KOYOMI_JOBNET`・`JOB`・`RUN`・`DATE`。
  - [x] CLI: `run <ジョブネット> [--date <業務日>] [--wait]`、`status [--today] [--toml]`、`log <実行> <ジョブ>`、`kill <実行> [<ジョブ>]`。
  - [x] 窓: 「今日」の一覧（予定・状態の記号と色・開始・終了・所要）、選んだ回の図（駅のような丸と名前、通った結果で塗る線、横・縦は `state.toml`）、
    図の下にログ（動いている間は流れる）、今すぐ流す・強制終了（`ito-confirm`）。
  - [x] 単体テスト（順・遷移・履歴の戻し）、`TESTING.md` の行、`CHANGELOG.md`。
- [x] 3. 打ち切り時間と強制終了（S）: `ito-proc` でプロセスの木ごと止める。`DECISIONS.md` の「1 周目」。
  - [x] core: `timeout = "30m"`（h・m・s の組み合わせ）を読む。0 と単位なしは誤り。`Job`・`JobRun` に秒で持つ。
  - [x] エンジン: タイマーが止めの合図を立て、`ito_proc` がプロセスの木ごと止める。打ち切り（⏱）と強制終了（■）は合図の出どころで分ける。
    ログの終わりに「打ち切り（30m を超えた）」「強制終了」と書く。
  - [x] 確かめの表示（窓・`approve`）に打ち切りの行。
  - [x] 単体テスト（読み・打ち切り・強制終了の速さ）、`TESTING.md` の行、`CHANGELOG.md`。
- [x] 4. 失敗の通知（S）: システムの通知・ベル・webhook（アドレスは既定で `ito-secret`）。`DECISIONS.md` の「2 周目」。
  - [x] core: `koyomi.toml` の `[notify]` を表のまま持つ（`deny_unknown_fields` を通す）。
  - [x] `notify.rs`: ito-prefs の `NotifyCfg` で読み、書かれていなければ `failed` だけ・音なし・`hide_webhooks = true`。平文のアドレスは注意を出して送らない。
  - [x] エンジン: 終わった回を `Runner::take_over` で受け取り `tell`。`secret:` の読み戻しは別スレッド（世代で古い答えを捨てる）。`Snapshot.notes`。
  - [x] 窓: ベルに異常終了の回（押すとその回を開く）と `[notify]` の注意。CLI: `list` の `注意:`、`koyomi check koyomi.toml`。
  - [x] 単体テスト（既定・平文・`secret:`・未知の出来事・言葉）、TESTING.md の 10 章。
- [ ] 5. 時刻のスケジュールと次の 10 回（M）: `[[when]]` を読んで起動する。時刻を過ぎた回と前の回が動いているときは既定だけ。
  `DECISIONS.md` の「2 周目」、「実行のタイミング」。
- [ ] 6. 確かめるときに 7 日の予定の変わり方を見せる（S）: 1 の確かめと 5 の計算を組み合わせる。
  `DECISIONS.md` の「2 周目」、「koyomi だけの強み（企画役の提案、2026-10-29 に持ち主が採用）」。

## 雛形の配線

雛形（ito の `templates/app`）から作った。雛形で動いているのは、窓（場所を覚える）、common.toml のテーマ・言語・倍率、
`--version` / `--help`、F12 の不具合報告、`koyomi.com`、アイコンと版の情報、TESTING の表の生成まで。
下は**依存は入っているが、まだ配線していないもの**。先に作った別のアプリが手本になる。自前で書かずに ito のクレートを使う
（足りない所は ito に頼む。`scripts/ito-first.py` が見ている）。

- [ ] `--keys` の台本: `ito-keyscript`（`Dialect`・`Runner`・`Press`）。手本は michi の `main.rs`。`lanes.conf` の `keys_env` と合わせる。
- [ ] `keymap.toml` と設定の「キー」ページ: `ito-keys`・`ito-keymap`・`ito-prefs` の `keys_page`。既定のキーは `defaults/keymap.toml`
  （`<C-,>` 設定・`<F1>` ヘルプ・`<C-q>` 終了）。手本は minato の `keymap.rs`・`prefs.rs`。
- [ ] `?` と F1 のヘルプ: `ito-help`（`ito-keymap` の一覧を描く）。手本は michi の `app.rs`・`ui.rs`。
- [ ] 通知と設定の「通知」ページ: `ito-notify`（システムの通知・音・webhook）と `ito-prefs` の `notify_page`。手本は minato の `app.rs`・`settings.rs`。
- [ ] 設定の画面: `ito_prefs::show`（Language・CLOCK・Theme の共通のページ + キー）。手本は minato の `settings.rs`。
- [ ] 窓の場所を戻す: 雛形は `window::Saved` で戻し `Tracker::follow` で書く。終わるときの `Tracker::close` は未配線。
- [x] 通知を入れる: ベルに `ito_notices::Notice<String>`（`go` は回の id。押すとその回を開く）。システムの通知はエンジンが出すので、押しても窓は開かない（4 の「持ち主に聞くこと」）。
- [ ] コマンドパレットの中身: 箱と `<C-S-p>` は繋いである。`Command` と `commands` に、アプリのコマンドを足す（キーは `ito-keymap` から引く）。手本は minato の `main.rs`。
- [x] 字体（日本語が出る字体を `egui` に足す）。手本は minato の `fonts.rs`。
- [ ] `TESTING.md` に確認の行を足し、`cargo run --example make-testcheck` で `TESTING-CHECKS.md` を作る。
- [ ] `icon.rs` に本当のアイコンを描く。
- [ ] リリースの workflow は kura か yagura のものを写す。

## 持ち主に聞くこと

実行役が迷って、いちばん小さく戻しやすい形に決めたもの。違えば直す。

- 秘密の値の `***` への置き換え（2）: v0.1 では `ito-secret` に登録する画面も欄もないので、置き換える対象がなく、ログはそのまま書く。登録の口を作るとき（4 の webhook）に一緒に入れる。
- ジョブの作業フォルダー（2）: `dir` がないジョブはエンジンの作業フォルダーで動く（エンジンを起こした所。窓から起きたときはその exe の場所）。定義のフォルダーにする方がよければ直す。
- `state.toml` の `flow`（2）: `--keys` がまだ配線されていないので、窓は今は必ず書く。`--keys` を配線するときに `Browser::start` の `state` を `None` にする。

- 時間の文字列の読み（3）: `timeout = "30m"` を読む部品は ito にない（`ito_common::text` は書く向きだけ）ので、uchmk/ito#76 を立て、届くまで `koyomi-core/src/duration.rs` に置く（h・m・s をこの順、単位なしと 0 は誤り）。届いたら置き換える。
- 打ち切りの出どころ（3）: 止めの合図は 1 つの旗で、先に立てた側（人の強制終了か、timeout のタイマーか）を記号に使う。同時なら先に来た方。`timeout` の時間はジョブが始まってから（待ちの時間は数えない）。

- 通知の出どころ（4）: 通知はエンジンが出す（時刻・窓・CLI のどれで流しても同じ言葉、窓が閉じていても出る）。その代わり、システムの通知を押しても窓は開かない（別プロセスで、窓は開いていないかもしれない）。押して開きたければ、窓が開いているときだけ窓から出す形に変える。
- `hide_webhooks` の既定（4）: `true`。平文の `https://…` は `koyomi list`・ベル・`koyomi check koyomi.toml` に注意を出し、`hide_webhooks = false` と書くまで送らない（黙って送らない・既定は控えめ）。
- ベルに並ぶもの（4）: 異常終了の回と `[notify]` の注意だけ。正常終了は `[notify]` の `system` に `done` を書けばシステムの通知には出るが、ベルには出ない（設定の口は作っていない）。
- `***` の置き換え（4）: webhook のアドレスは `secret:<名前>` で OS の資格情報ストアから読み、エンジンのメモリーにだけ置く。koyomi が自分のファイルに書くことはないので、ログの置き換えは今も要らない（ジョブの環境変数に秘密を渡す口がまだない）。口を作るときに一緒に入れる。
- `keys_do_not_reach_the_tree_behind_the_box`（4）: 全体のテストと並べて回すと、ごくまれに Tab の後の `focus` が 0 のまま落ちる（単独では通る）。項目 4 で触っていない所なので、そのままにしてある。
