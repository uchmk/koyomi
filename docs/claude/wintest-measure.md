# 実機の測り方（koyomi の規則）

実機のセッション（持ち主の PC の main モデル）と、使い捨ての subagent `wintest`（[.claude/agents/wintest.md](../../.claude/agents/wintest.md)）が守る規則。
チェックを付ける規則は [TODO.md の「`[x]` の規則」](../../TODO.md)。kura の同じ文書（`docs/claude/wintest-measure.md`）と方針をそろえてある。

## 1. 測ってから「見た目」と呼ぶ

- 見た目に聞こえる行にも、読めるものが代わりにあることが多い。先に探す。
  位置や色はピクセル、数は文字、状態は `<State:name>`、プロセスは `Get-CimInstance Win32_Process`、ジョブの結果は `jobs/` の記録。
- 人の目でしか決まらない行（色が「見える」か、「目立つ」か、「滑らか」か）は測れない。その行は **どの代わりを試し、なぜ主張を支えられなかったか** を書く。
- 代わりの測定（プロセスの数だけ、など）は行全体の PASS にならない。測れなかった部分を **未確認** に挙げる。

### 1a. 矩形と画素（Tier 1）の現状

見た目の行で、期待値のすべてを機械が測った行だけが `[x]` になりうる（TODO.md の規則）。ただし測るための道具がまだ揃っていない。

- **ito には入っている**（a55c886）: state JSON（`Runner::state_json(true)`）、`<Snap:>`、部品の test id（`ito_a11y::test_id`）、`ito_keyscript::set_wgpu_backend`。
- **koyomi がまだ配線していない**: `state_json(true)` を選ぶこと、アプリの状態（画面・オーバーレイ・アプリ固有の値）を渡すこと、main で `set_wgpu_backend` を呼ぶこと、主要なウィジェットに Test ID を付けること。
  この配線が済むまで、koyomi の見た目の行は `[~]` までにする。
- **今ある**: `scripts\keys.ps1` の `<State:名前>`（文字）、`<Shot:名前>`（窓全体の PNG）、done ファイル（`KOYOMI_KEYS_DONE`）、`scripts\visual-check.py`、`scripts\evidence.py`（`-Evidence` の証拠）。
- 閾値を書いた check spec（`tests/visual/<行>.toml`）は、実行の**前に**書く。見たあとで決めた閾値は数えない。
- AI Vision の答え・見た印象は、測定に数えない。

画素の判定は Windows の実機だけ。Linux のレーンでは画素を判定しない。

## 2. GUI の走らせ方

- `pwsh -NoProfile` で `scripts\keys.ps1 -Bin <exe> -Dir <dir> -Out <走らせるごとに新しい dir> -Isolate`。
- 走らせる前に **`KOYOMI_ADDRESS` を、その走りだけの名前で** 環境変数に入れる。`--keys` の窓はこれがないと起動を拒否する（本物のエンジンに繋がるため）。同じ名前を二つの走りで使わない。
- `-Isolate` は `KOYOMI_CONFIG_HOME`・`KOYOMI_STATE_HOME`・`UCHMK_CONFIG_DIR` を走りのフォルダの空の場所に向ける。持ち主の設定・状態に触れない。
- 終わりは `<Quit>`、プログラムを起こす走りは `-Launches`。
- 終了コードは走らせた直後に保存する。`keys: done` は「測定が走った」だけで、行が通った証拠ではない。`keys: stalled`・`keys: refused`・`keys: failed`・`keys: unsettled` は結果ではない（そのまま報告する）。
- 古い状態ファイルを使い回さない。期待値の 1 つずつに観測を対応させる（前提 → 操作 → 期待値ごとの観測）。

## 3. 机・場所・後片付け

- デスクトップを使う走りは **`Local\wintest-desktop`** のロックを、最初の走りから後片付けまで持つ。取れなければ少し待ち、だめなら `BLOCKED`。
- 書いてよいのは scratch の下だけ（新しい GUID のフォルダ）。リポジトリ（`C:\dev\koyomi`）と未コミットのものは scratch に置かない。
- 渡されていない fixtures、`qa-reports`、証拠、ごみ箱全体を消さない。`scripts\make-fixtures.ps1` は既存のフォルダを消すので、渡された fixtures には回さない。
- 止めてよいのは **自分で起こしたプロセスだけ**（PID・開始時刻・exe のパスで確かめる）。持ち主の koyomi のエンジンやジョブ、ブラウザのタブ、エディタ、設定は触らない。
- OS の設定を変えない（レジストリ、ファイアウォール、SMB、表示倍率、スクリーンセーバー、プロファイル）。**ファイルの ACL を変えない**。
- 後片付けは try/finally。失敗したら別に書く。

## 4. 証拠

R: にだけある証拠は証拠ではない（再起動で消える）。取ったらすぐ C: に写す。

- 行ごとに: 行 ID、結果（`PASS` / `FAIL` / `CANNOT-MEASURE(理由)` / `BLOCKED`）、期待値それぞれに対する観測、実行したコマンド、終了コード、生の出力の抜粋、証拠の場所、後片付けの結果、未確認。
- 走らせたこと（プログラムが起動した）と確かめたこと（期待値が観測と合った）を混ぜない。

## 5. 誰が何をするか

- `wintest` は測って証拠を返すだけ。ビルド・コード編集・チェックを付ける・Issue・git はしない。
- チェックを付けるかは main が決める。測れなかった行は付けない。
- ACL の検査で出た警告は、オーナーに報告するだけ（ACL は変えない）。
