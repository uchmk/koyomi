# テスト用 subagent と「[x] の規則」を別のアプリへ移す手順（koyomi での実例）

fude・yagura・michi に同じ方針を取ってもらうための記録。kura の `wintest` と `[x]` の規則を、koyomi にそのまま移した順番。

## 何を移したか

| 移したもの | 出どころ | koyomi での置き場 |
|---|---|---|
| 実機の測定 subagent `wintest` | kura `.claude/agents/wintest.md` | `.claude/agents/wintest.md` |
| 測り方の規則 | kura `docs/claude/wintest-measure.md` | `docs/claude/wintest-measure.md` |
| 「`[x]` の規則」（実機確認と見た目の判断） | kura `docs/claude/lanes.md` の該当節 | `TODO.md` 先頭 |
| 走らせる道具（`--keys` の実行、証拠、検査器） | ito `scripts/lanes/` | `scripts/lanes.sh sync` が `scripts/` にコピー |
| 配線の設定 | — | `scripts/lanes.conf`（`isolate=`・`evidence=`・`todo_marks=`・`keys_env=`） |

見た目の subagent `vision` は、Tier 2（AI の見た目判断）が持ち主に承認されていないので移していない。`qa-tester` は kura の `qa-role.md` と `qa-audit` 手順が要るので、まだ移していない。

## 順番（この順にやると詰まりにくい）

1. **ito の rev をそろえる。** アプリの `Cargo.toml` の ito の依存を全部同じ rev にする。`scripts/lanes.sh sync` が `FILES` を要求するので、rev が古いと sync が止まる（koyomi では `evidence.py` がなくて止まった）。
2. **API の移行。** ito-keyscript の `Do::Unsettled` の腕を足す。`Runner::finish` は `Runner::end(Ending::Closed)` に替える（戻り値の `Ended` を使う）。
3. **`--keys` の隔離。** 窓が本物のエンジンに繋がらないように、`KOYOMI_ADDRESS` が空なら起動を拒否する。`lanes.conf` の `isolate=` にアプリの設定・状態の環境変数を並べる。
4. **テストの分け方。** 実機の ACL に依存するテストは、通常の `cargo test` から外して `KOYOMI_TEST_ACL` の付いたときだけ動かす（`scripts/verify-acl.sh`）。`verify.sh` は「ALL OK（ACL は未確認）」と出す。ACL が通った、とは書かない。
5. **不安定なテストの直し方。** 壁時計の時間を見るテストは、プロセス生成の遅れを含む。動いているものを止める前提が要るテストは、先にログの `started` などを待つ。閾値を緩めない。
6. **subagent を置く。** `.claude/agents/wintest.md` と `docs/claude/wintest-measure.md` を移し、アプリの名前・環境変数・scratch・証拠の場所に書き換える。
7. **`[x]` の規則を置く。** TODO.md の先頭に、チェックを付けるのは main だけ、測れない行は `[~]`、見た目の行は check spec（`tests/visual/<行>.toml`）を実行の前に書く、の規則を書く。
8. **配線の残り。** state JSON・`<Snap:>`・部品の test id を入れて、やっと Tier 1 の見た目の行が測れる。それまでは `[~]`。

## 注意（koyomi で踏んだもの）

- この PC の `AppData` には、Codex の sandbox ユーザーの変更権限の ACE がある。ACL 検査はそれを警告にし、警告があると起動が拒否される（実機で確認はまだ）。移植先でも、先に `icacls` で同じ状態かを見る。
- `wintest` に ACL を変えさせない。ACL の判断はオーナーがする。
- `keys.ps1` の `-Isolate` は隔離の変数を設定するが、`KOYOMI_ADDRESS` は呼び出し側が毎回別の名前で入れる。
- Windows のプロセス生成は並列テスト中に遅くなる。プロセスを使うテストは、待つ条件を書く。
