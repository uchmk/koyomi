# 変更ログ

形式は [Keep a Changelog](https://keepachangelog.com/ja/1.1.0/)、日付は JST。

## [Unreleased]

### 追加

- 定義を読んで確かめる: `jobnets/<グループ>/<名前>.toml` を読み、誤りは行番号つきで示す。確かめた定義は中身全体の hash で
  `approved.toml` に覚え、1 字でも変わればまた未確認になる。窓の確かめの箱と `koyomi check` / `list` / `approve` / `engine`。
  他のユーザーが書ける場所があれば帯で知らせ、直るまで確かめない。
- 手で流すことと、結果とログ: `koyomi run <グループ/名前> [--date] [--wait]` で確認済みの定義を流す（`after` の順、先行が全部そろってから、
  同時に動く数は `koyomi.toml` の `concurrency`。異常が 1 つでもジョブネットは異常終了、その先は未実行）。ジョブの出力は
  `logs/<日>/<実行>/<ジョブ>.log`（既定 10 MB まで）、出来事は `history/<年>/<月>/<日>.toml`、流した定義の写しは実行のフォルダーに。
  `koyomi status` / `log` / `kill`。窓は「今日」の一覧、選んだ回の図（駅のような丸と名前、通った結果で塗る線、横・縦は `state.toml`）、
  図の下に流れるログ、今すぐ流す・強制終了（箱は「やめる」から始まる）。状態は記号と言葉の組（✓ 正常終了・✕ 異常終了・■ 強制終了 …）。
- 打ち切り時間と強制終了: ジョブの `timeout = "30m"`（h・m・s をこの順で。単位なし・0 は誤り）。時間を過ぎるとプロセスの木ごと止め、
  `⏱ 打ち切り（30m を超えた）`。人が止めたときは `■ 強制終了`。どちらもログの終わりに書き、確かめの箱と `koyomi approve` に打ち切りの行が出る。
- 失敗の通知: 回が終わるとエンジンが知らせる（時刻・窓・CLI のどれで流しても同じ）。`koyomi.toml` の `[notify]` は ito-prefs の形
  （`system`・`sound`・`webhook`/`[[webhooks]]`・`hide_webhooks`）。書かなければ異常終了だけをシステムの通知で、音なし。
  窓のベルに異常終了の回が並び、押すとその回を開く。webhook のアドレスは既定で OS の資格情報ストア（`secret:<名前>`）。
  平文で書いてあれば注意を出し（`koyomi list`・ベル・`koyomi check koyomi.toml`）、`hide_webhooks = false` と書くまで送らない。

## [0.1.0] - 2026-10-29

### 追加

- ito の `templates/app` から作った（窓・テーマ・言語・倍率・F12 の不具合報告・`koyomi.com`）。
