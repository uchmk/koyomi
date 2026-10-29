# TODO

## v0.1 の項目（企画で採用したもの）

仕様と受け入れ条件は `DECISIONS.md`。番号は `DECISIONS.md` の「`TODO.md` の 1〜6」と同じ。上から順に作る。

- [ ] 1. 定義を読んで確かめる（L）: `crates/koyomi-core` の TOML のモデルと検査、中身の hash で見る確認済み、
  他のユーザーが書ける定義の止め、`ito-confirm` での確かめ、`koyomi check`・`koyomi approve`。
  `DECISIONS.md` の「1 周目」、「実行するコマンドの安全」、「ファイル」。
- [ ] 2. 手で流すことと、結果とログ（L）: `koyomi engine`（`ito_ipc::door`）、`run`・`status`・`log`・`kill`、状態の一覧の文言と記号。
  `DECISIONS.md` の「1 周目」、「構成」、「CLI」、「3 周目の候補」の「D. 決めた細部」。
- [ ] 3. 打ち切り時間と強制終了（S）: `ito-proc` でプロセスの木ごと止める。`DECISIONS.md` の「1 周目」。
- [ ] 4. 失敗の通知（S）: システムの通知・ベル・webhook（アドレスは既定で `ito-secret`）。`DECISIONS.md` の「2 周目」。
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
- [ ] 通知を入れる: 雛形のベルは `notices` が空のまま。`ito_notices::Notice` を push し（`go` に飛び先の型）、`ito-notify` の `Toast::open` で押すと開くトーストにする。
- [ ] コマンドパレットの中身: 箱と `<C-S-p>` は繋いである。`Command` と `commands` に、アプリのコマンドを足す（キーは `ito-keymap` から引く）。手本は minato の `main.rs`。
- [ ] 字体（日本語が出る字体を `egui` に足す）。手本は minato の `fonts.rs`。
- [ ] `TESTING.md` に確認の行を足し、`cargo run --example make-testcheck` で `TESTING-CHECKS.md` を作る。
- [ ] `icon.rs` に本当のアイコンを描く。
- [ ] リリースの workflow は kura か yagura のものを写す。
