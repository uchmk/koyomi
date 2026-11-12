# README の画像の置き場

README の「What it looks like」と、紹介画像（GitHub の Social preview）の右に入れる実画面の置き場。
どれも 0.37.0 のビルドを既定のテーマ・英語の画面（箱の `common.toml` に `language = "en"`）で撮った。
持ち主の設定と状態は使わず、撮るためだけに作った箱（設定・状態・名前付きパイプ・通知の出し先が、すべてその箱だけのもの）のエンジンに、
デモのジョブネット 7 本（Nightly backup・Sales report・Stock sync・Health check・Log cleanup・Daily report・Month-end closing）と、
カレンダー Company（日本の祝日と土日が休み）、3 日分の履歴とログを読ませてつないだ。
今日の Sales report は、集計のジョブ（aggregate）が終了コード 2 で失敗している。

| ファイル | 何を写すか | 撮り方 |
| --- | --- | --- |
| `today.png` | Today。失敗した Sales report の行を選んだところ。今日の実行と予定の表、ガントチャート、右に選んだジョブネットの次の予定と構成 | `scripts/keys.ps1` で窓を `<Size:1600x1000>` にして `<Shot:…>` で撮り（150 % なので 2400x1500）、1600x1000 に縮める |
| `history.png` | History。失敗した Sales report の実行を選んだところ。ジョブの流れ・状態・終了コードと、失敗したジョブのログ | today.png と同じ |
| `edit.png` | Month-end closing の編集の画面。Monthly report の後ろにジョブを 1 つ足し、id・名前・コマンドを入れたところ（保存しない） | today.png と同じ。足すのは Parts の Job、入れるのは `<Paste:…>`（コマンドは 1 行に 1 つ） |
| `koyomi.gif` | Today → 失敗した実行を選ぶ → Month-end closing の流れ → 編集の画面 → ジョブを足す → Company のカレンダー → History で失敗したジョブのログ → 対応済みにする（Mark as dealt with）。操作を右下に英語の字幕で出す | 各画面を today.png と同じく撮り、1200x750 に縮めて字幕（Segoe UI Bold、`C:/Windows/Fonts/segoeuib.ttf`）を重ね、ffmpeg で 1 枚 2.5〜3 秒・10 fps・128 色の GIF にする（8 枚で約 21 秒、くり返す） |
| `banner-shot.png` | `docs/banner.html` の右の実画面。Today（行を選ばない） | today.png と同じ |

撮るときの注意:

- **持ち主の設定と状態を使わない。**`KOYOMI_CONFIG_HOME`・`KOYOMI_STATE_HOME`・`UCHMK_CONFIG_DIR` を箱の中に向け、
  `KOYOMI_ADDRESS` を箱だけの名前付きパイプにし、通知は `ITO_NOTIFY_SINK` でファイルに出す。持ち主のエンジンとサービスには触らない。
- デモのジョブネットのコマンド（`D:\ops\…`）は架空のもの。今日の予定の残り（23:50 の Daily report）が来る前に、箱のエンジンを止める。
- **全部のコマを見て、利用者の名前・ホストの名前・IP アドレス・token が写っていないことを確かめる。**接続先は「This PC」だけにする。
- クリックのあとは、タイトルバーの空いたところを指す（`<Hover:0.3,0.015>`）。指したままだとツールチップや行の色が写る。
- 編集の画面では上のタブが隠れる。続けてほかのタブを撮るときは、右上の Close で戻ってから押す（そのまま押すと Parts の部品が足される）。
  編集は保存しない（定義を変えない）。
- History の行の位置は、実行中の実行（いちばん上に出る）があるかで 1 行ずれる。実行中の行が出てから撮る。
- 対応済みにすると箱の状態が変わる（「failures waiting」が消える）ので、最後に撮る。
- 紹介画像は `docs/social-preview.png`（元は `docs/banner.html`。描き直し方は HTML の冒頭）。
  GitHub に API が無いので、Settings > General > Social preview から手で上げる。
