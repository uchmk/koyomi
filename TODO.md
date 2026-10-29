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
- [x] 5. 時刻のスケジュールと次の 10 回（M）: `[[when]]` を読んで起動する。時刻を過ぎた回と前の回が動いているときは既定だけ。
  `DECISIONS.md` の「2 周目」、「実行のタイミング」。
  - [x] core: `[[when]]` を読む（once・day・minutes/hours・week・month・year・dates、except/start/end）。`when.rs` に日の時刻・次の回・2 つの時刻の間に来た回の計算。
  - [x] エンジン: `schedule.toml` に最後に見た時刻を持ち、起動と 1 秒ごとに来た回を流す。60 秒より遅れた回は `↷ 時刻を過ぎた`、前の回が動いていれば `⊘ スキップ`、未確認なら `🔒 止めた`（流さずに履歴に残す）。
  - [x] 通知: 時刻を過ぎた回はジョブネットごとに 1 回にまとめる。ロックは異常と同じ。
  - [x] `koyomi next <group/name> [--count N]` と窓の「次の 10 回」。今日の一覧にこれから来る回の行。
  - [x] 自動起動の帯（時刻で動く定義があり、サインインで起動しない設定のとき）。
  - [x] 単体テスト（月末・第 n 曜日・夏時間）、TESTING.md の 11 章、CHANGELOG。
- [x] 6. 確かめるときに 7 日の予定の変わり方を見せる（S）: 1 の確かめと 5 の計算を組み合わせる。
  `DECISIONS.md` の「2 周目」、「koyomi だけの強み（企画役の提案、2026-10-29 に持ち主が採用）」。
  - [x] `review::week`: 今の `[[when]]` と前に確かめた版の `[[when]]` で 7 日の回を数え、数と増減（`+`・`-`）を 20 件まで、残りは数。確かめの箱と `koyomi approve` に出す。単体テスト、TESTING.md の 12 章、CHANGELOG。

## 雛形の配線

雛形（ito の `templates/app`）から作った。雛形で動いているのは、窓（場所を覚える）、common.toml のテーマ・言語・倍率、
`--version` / `--help`、F12 の不具合報告、`koyomi.com`、アイコンと版の情報、TESTING の表の生成まで。
下は**依存は入っているが、まだ配線していないもの**。先に作った別のアプリが手本になる。自前で書かずに ito のクレートを使う
（足りない所は ito に頼む。`scripts/ito-first.py` が見ている）。

- [x] `--keys` の台本: `ito-keyscript`（`Runner`、`KOYOMI_KEYS_DONE`）。`<State:名前>` は `ui::Browser::report`（エンジン・件数・選んだもの）。
- [x] `keymap.toml` と設定の「キー」ページ: `ito-keys`・`ito-keymap`・`ito-prefs` の `keys_page`。既定のキーは `defaults/keymap.toml`
  （`<C-,>` 設定・`<F1>` ヘルプ・`<C-q>` 終了）。手本は minato の `keymap.rs`・`prefs.rs`。
  - [x] `keymap.rs`（`Keys::load`・`resolve`・`label`）と `main.rs` の読み（`read_keys`・`press`・`run_key`）。利用者の `keymap.toml` は起動時に読み、間違いはベルに。
  - [x] 「キー」ページ（`keys_page`、`ito_keymap::edit` で書く、2 秒の見張り）は設定の画面と一緒に（`settings.rs`・`prefs.rs`）。
- [x] `?` と F1 のヘルプ: `ito-help`（`ito-keymap` の一覧を描く）。手本は michi の `app.rs`・`ui.rs`。`help.rs`（組は keymap の層・いつでも・木の上の箱・ヘルプ自身）。
- [x] 通知と設定の「通知」ページ: `ito-notify`（システムの通知・音・webhook）と `ito-prefs` の `notify_page`。手本は minato の `app.rs`・`settings.rs`。
  通知そのものは 4 で（エンジンが出す）。ページは `settings.rs` の `NOTIFY`、`koyomi.toml` の読み書きと見張りは `prefs.rs`（`notify_edit`・`write_config`）。
- [x] 設定の画面: `ito_prefs::show`（Language・CLOCK・Theme の共通のページ + キー）。手本は minato の `settings.rs`。
  `settings.rs`（一般・テーマ・通知・キー・詳細・情報）と `prefs.rs`（common.toml・テーマ・keymap.toml・koyomi.toml の `[notify]` の読み書きと 2 秒の見張り）。
- [x] 窓の場所を戻す: 雛形は `window::Saved` で戻し `Tracker::follow` で書く。終わるときの `Tracker::close` は `on_exit` で。
- [x] 通知を入れる: ベルに `ito_notices::Notice<String>`（`go` は回の id。押すとその回を開く）。システムの通知はエンジンが出すので、押しても窓は開かない（4 の「持ち主に聞くこと」）。
- [x] コマンドパレットの中身: 箱と `<C-S-p>` は繋いである。`Command` と `commands` に、アプリのコマンドを足す（キーは `ito-keymap` から引く）。手本は minato の `main.rs`。
  不具合の報告・倍率を戻す・設定・キー・終了。回ごとの操作（今すぐ動かす・止める）は木の中の箱にあり、パレットには足していない。
  - [x] 「設定」（キーは `keymap` の `settings`）。
  - [x] 「キー」（ヘルプ。キーは `keymap` の `help`）。
- [x] 字体（日本語が出る字体を `egui` に足す）。手本は minato の `fonts.rs`。
- [x] `TESTING.md` に確認の行を足し、`cargo run --example make-testcheck` で `TESTING-CHECKS.md` を作る。項目ごとに節を足した（1〜17）。
- [x] `icon.rs` に本当のアイコンを描く。暗いタイルの上の暦のページ（cyan の帯・6 つの日・金色の今日）。`build.rs` が同じファイルで `koyomi.exe` の絵も作る。
- [x] リリースの workflow は kura か yagura のものを写す。`scripts/lanes.sh sync` が ito の雛形の `release.yml` を書いていた（`lanes.conf` の `workflows=`）。写す必要はない。

## 持ち主に聞くこと

実行役が迷って、いちばん小さく戻しやすい形に決めたもの。違えば直す。

- 秘密の値の `***` への置き換え（2）: v0.1 では `ito-secret` に登録する画面も欄もないので、置き換える対象がなく、ログはそのまま書く。登録の口を作るとき（4 の webhook）に一緒に入れる。
- ジョブの作業フォルダー（2）: `dir` がないジョブはエンジンの作業フォルダーで動く（エンジンを起こした所。窓から起きたときはその exe の場所）。定義のフォルダーにする方がよければ直す。
- `state.toml` の `flow`（2）: `--keys` の窓は `Browser::start` の `state` が `None` で、読みも書きもしない（配線で済み）。

- 時間の文字列の読み（3）: `timeout = "30m"` を読む部品は ito にない（`ito_common::text` は書く向きだけ）ので、uchmk/ito#76 を立て、届くまで `koyomi-core/src/duration.rs` に置く（h・m・s をこの順、単位なしと 0 は誤り）。届いたら置き換える。
- 打ち切りの出どころ（3）: 止めの合図は 1 つの旗で、先に立てた側（人の強制終了か、timeout のタイマーか）を記号に使う。同時なら先に来た方。`timeout` の時間はジョブが始まってから（待ちの時間は数えない）。

- 通知の出どころ（4）: 通知はエンジンが出す（時刻・窓・CLI のどれで流しても同じ言葉、窓が閉じていても出る）。その代わり、システムの通知を押しても窓は開かない（別プロセスで、窓は開いていないかもしれない）。押して開きたければ、窓が開いているときだけ窓から出す形に変える。
- `hide_webhooks` の既定（4）: `true`。平文の `https://…` は `koyomi list`・ベル・`koyomi check koyomi.toml` に注意を出し、`hide_webhooks = false` と書くまで送らない（黙って送らない・既定は控えめ）。
- ベルに並ぶもの（4）: 異常終了の回と `[notify]` の注意だけ。正常終了は `[notify]` の `system` に `done` を書けば（設定の「通知」ページの切り替え）システムの通知には出るが、ベルには出ない。
- `***` の置き換え（4）: webhook のアドレスは `secret:<名前>` で OS の資格情報ストアから読み、エンジンのメモリーにだけ置く。koyomi が自分のファイルに書くことはないので、ログの置き換えは今も要らない（ジョブの環境変数に秘密を渡す口がまだない）。口を作るときに一緒に入れる。
- `keys_do_not_reach_the_tree_behind_the_box`（4）: 全体のテストと並べて回すと、ごくまれに Tab の後の `focus` が 0 のまま落ちる（単独では通る）。項目 4 で触っていない所なので、そのままにしてある。

- `[[when]]` の形（5）: `every = "day" | "week" | "month" | "year" | "once" | "minutes" | "hours"`、`at = ["02:00"]`（once と minutes/hours 以外は必須）、`on`（week の曜日）、`day`（month の日・`"last"`・`"first-business"`・`"last-business"`・`"2nd mon"`）、`dates`（once・year）、`from`/`to`（minutes/hours の時間帯）、`except`・`start`・`end`。`day = 31` は 31 日のない月は飛ばす（前の日に寄せない）。違えば直す。
- 時刻を過ぎたの線（5）: 来るべき時刻から 60 秒より後に気づいた回を `↷ 時刻を過ぎた` にする（`schedule::LATE`）。エンジンは 1 秒ごとに見るので、動いていれば数秒で気づく。
- 最後に見た時刻（5）: `schedule.toml` の `checked`（UTC）。何かを流した・残したときはすぐ、何もなければ 10 秒ごとに書く。初めて起動した PC（ファイルがない）は今から始め、それより前の時刻は「過ぎた」と言わない。時計が戻ったときも今から始める（二度流さない）。
- `🔒 止めた` の言葉（5）: 他のユーザーが書ける場所・`koyomi.toml` の誤り・定義の誤り・未確認のどれでも `🔒 止めた`（`Locked`）にし、理由の文で分ける。ja の言葉を「ロック」から「止めた」にした（人が止めたのでなく koyomi が止めた、と読めるように）。
- 判定の順（5）: PC が止まっていた回は、未確認でも警告があっても `↷ 時刻を過ぎた`（起きたことが先）。次に警告・設定の誤り・定義の状態、最後に前の回が動いているか。
- 過ぎた回の数（5）: 止まっていた間に来た時刻は全部を 1 回ずつ残す（まとめて 1 行にしない）。通知だけジョブネットごとに 1 つにまとめる。1 日に 1440 回を超える `minutes` は読まない。
- 未確認の `[[when]]`（5）: 未確認の定義でも、今のファイルの `[[when]]` で時刻を数え、来た回を `🔒 止めた` として残す（確かめ忘れに気づけるように）。「次の 10 回」も今のファイルで数え、`未確認のため、時刻が来ても流しません` を添える。
- 今日の一覧のこれから（5）: 確認済みの定義の今日これから来る回を `○ 待ち` の薄い行で、流した回の上に出す。押しても開かない（回ではないので）。
- 自動起動の帯（5）: 確認済みで `[[when]]` のある定義があり、`ito_common::startup::enabled("koyomi")` が偽のときに出す。帯に設定を入れるボタンは付けていない（設定の画面で入れる。「勝手に入れない」）。
- 時刻の回の通知（5）: 時刻を過ぎた回の通知は `failed` の出来事として出す（`[notify]` を書かなくても出る）。別の出来事に分けたければ直す。

- 設定の「既定に戻す」（配線）: `common.toml` と `keymap.toml` だけを `*.bak-<時刻>` に退ける。`koyomi.toml`・定義・`approved.toml`・記録は設定ではなく仕事なので触らない。minato にある「設定の持ち出し（bundle）」の札は付けていない（持ち出すものが common.toml と keymap.toml だけで、コピーで足りる）。
- `--keys` の `<Theme:>`・`<Lang:>`（配線）: 窓が持つ common.toml の写しだけを変え、ファイルは書かない（前と同じ）。設定の画面で変えたときはファイルに書き、窓は見張りで追う（同じ机の他のアプリと同じ道）。
- 設定の画面の間のキー（配線）: 設定が出ている間は `keymap.toml` のキーを読まない（Esc で閉じる・「キー」ページが押されたキーを取る）。F12・倍率・パレットは「キー」ページがキーを待っている間だけ止める。
- 設定の画面が書く `koyomi.toml`（配線）: 「通知」ページは `[notify]` だけを書く（他の行とコメントは残す）。`DECISIONS.md` の「画面」の設定の行にある同時に動かす数・日数・ログの大きさは、持ち主が決めた印のない節なので画面には出さず、ファイルのままにした（要れば一般のページに行を足す）。
- 平文のアドレスと「通知」ページ（配線）: `hide_webhooks` が既定のまま平文の `https://…` が書いてあると、ページはそのアドレスを見せたうえで注意を出し、ページで何かを変えた最初の書き込みで資格情報ストアに移す（エンジンは移るまで送らない。前と同じ）。送る側の読み（`notify::read`）と見せる側の読み（`notify::parse`）を分けた。
- 「通知」ページの読み直し（配線）: `koyomi.toml` を 2 秒ごとに見張り、`secret:` の読み戻しはスレッドで（起動の直後はページに既定が出て、読めしだい差し替わる）。エンジンへの合図はなく、エンジンが 5 秒ごとに読み直すのを待つ。

- 7 日の予定の並べ方（6）: 増える回と消える回を日時の順に混ぜて 20 件（`+`・`-` の印で分ける）、残りは `… 他 N 件`。変わらない回は出さない。初めての確かめは全部の回を印なしで出す。
- 前の版が今の読み方で読めないとき（6）: 前に確かめた版の `[[when]]` を今の読み方で読めなければ、前は 0 回として数える（今の回が全部 `+` で出る）。koyomi の版を上げて `[[when]]` の読み方が変わったときにだけ起きる。
- 7 日の起点（6）: 確かめの箱を開いた時点（`koyomi approve` は打った時点）。箱を開いたままでは更新しない。

- `<Theme:>`・`<Lang:>` の効き方（配線 `--keys`）: koyomi は common.toml の見張りをまだ持たないので、台本の切り替えはその窓のメモリーにだけ効かせ、common.toml は書かない（持ち主の設定を変えない）。設定の画面と見張りを配線したら michi と同じ「書いて見張りが効かせる」形に寄せてよい。
- `<State:名前>` の中身（配線 `--keys`）: `engine:`（up / starting / waiting / down: 理由）、`nets:`、`warnings:`、`runs:`、`selected:`、`run:`、`job:`、`asking:`（approve <id> / kill <id> / -）、`flow:` の行。足りなければ足す。koyomi 固有の読み（`Dialect::reads`）はまだない。
- 設定・ヘルプのキー（配線 keymap）: 設定の画面とヘルプを配線したので、「まだ使えません」の文言は外した。
- ヘルプの中身（配線ヘルプ）: `keymap.toml` の層（今は `app` の 1 つ）と、koyomi が自分で読むキー・木の上の箱のキー・ヘルプ自身のキーだけを並べる。木や回の操作のキーはまだ keymap にないので載らない（足せば自然に載る）。`?` は `defaults/keymap.toml` の 2 つ目の `help` の結びとして足した（一覧に出て、利用者が外せる）。パレットの検索の箱に入れた `?` は箱の字になり、ヘルプは開かない。
- パレットの `report_bug`・`reset_scale` の文言（配線 keymap）: `lang/*.toml` のキーが `report_reset_scale` 1 つに潰れていて、パレットにキー名がそのまま出ていた。`report_bug`・`reset_scale` の 2 つに直した。
