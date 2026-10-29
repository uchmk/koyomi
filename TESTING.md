# Testing

The checks a person (or an agent with a screen) makes. Each section is `## N. Title` over a table of `# | Do | Expect`.
A test takes a row off the list by naming its number in a doc comment on the `#[test]` (`/// 1.1: …`), and
`cargo run --example make-testcheck` writes what is left to `TESTING-CHECKS.md`.

## 1. Start

| # | Do | Expect |
|---|---|---|
| 1.1 | Run `koyomi` | A window of 1280x800 opens with the title "koyomi" |
| 1.2 | Move and resize the window, close it and start it again | The window comes back at the same place and size |
| 1.3 | Press `F12` | The browser opens the repository's new issue form with the version and OS filled in |

## 2. Definitions

| # | Do | Expect |
|---|---|---|
| 2.1 | Put `経理/月次締め.toml` and `共通/バックアップ.toml` under `jobnets/` and open the window | The left tree shows 共通 and 経理 with `▲ バックアップ  未確認` and `▲ 月次締め  未確認`; the Japanese names are letters, not boxes |
| 2.2 | Run `koyomi list` and `koyomi list --toml` with the engine stopped | The engine starts; the same list as the tree is printed |

## 3. Engine

| # | Do | Expect |
|---|---|---|
| 3.1 | Open the window with no engine running, then close the window | The engine starts by itself and is still running after the window is closed (`koyomi engine status` answers) |
| 3.2 | Run `koyomi engine` while one is up, then `koyomi engine stop` | The first prints "もう動いている" and exits 1; stop stops the running one |

## 4. Approval

| # | Do | Expect |
|---|---|---|
| 4.1 | Select a net, press 確かめる… | A box lists the program, each argument on its own line, and the folder; Enter = 確かめた, Esc = やめる |
| 4.2 | `koyomi approve 経理/月次締め`, answer `y`, restart the engine | The same content is printed; the net shows `✔ 確認済み` and still does after the restart |
| 4.3 | `koyomi approve 経理/月次締め` and answer anything but `y` | It stays `▲ 未確認` and the exit code is 1 |
| 4.4 | In the box press やめる | The net stays `▲ 未確認` |
| 4.5 | With the box up, press Tab, then Enter | Tab moves between 確かめた and やめる only; Enter answers the box and nothing behind it is pressed or selected |
| 4.6 | Open the box on a net with a long diff, close it, open it on a one-job net; then make the window small | The title, the rows, the program and the buttons are inside the window every time, and the box does not grow while it is up |

## 5. Edits

| # | Do | Expect |
|---|---|---|
| 5.1 | Approve a net, then add a comment line to its file by hand | Within seconds it is `▲ 未確認` again |
| 5.2 | Approve it again | The box (and `approve`) shows the change as `+` / `-` lines against the approved text |

## 6. Check

| # | Do | Expect |
|---|---|---|
| 6.1 | `koyomi check <file>` on a good file and on files with a typo'd field, bad syntax, no/empty `run`, an unknown `after`, a cycle, a duplicate id | 0 for the good one; for the others the file, the line and the cause, and exit 1 |
| 6.2 | Look at a broken file in the tree and try `approve` | `✖ 定義の誤り`; it cannot be approved |
| 6.3 | A file with `timeout = 60` (no unit) and one with `retry = { count = 2 }` | `timeout は "30m" のように文字列で書く…` for the first, "この版ではまだ使えない: retry" for the second; neither can be approved |

## 7. Safety of the places

| # | Do | Expect |
|---|---|---|
| 7.1 | Let Everyone write a definition, then `check` and `approve` | Both exit 1 and name the file |
| 7.2 | Same, with the window open | A warning band names the place and who; 確かめる… is disabled for every net |
| 7.3 | Break `approved.toml` by hand, then `list` and `approve` | Every net shows `▲ 未確認`, a warning names `approved.toml`, `approve` exits 1 and the file is left as it was; once it is mended the approvals come back without a restart |

## 8. Runs

| # | Do | Expect |
|---|---|---|
| 8.1 | Approve a net, select it, press 今すぐ流す | 今日 lists the run as `▶ 実行中` with its start time, and the detail switches to the run's picture |
| 8.2 | Watch while it runs | Each circle goes `·` → `▶` → `✓`; the line out of a finished job is painted in its colour; the running job's log flows under the picture, newest line at the bottom |
| 8.3 | A net whose one job exits 1 | That job is `✕ 異常終了` with its exit code, the jobs after it `· 未実行`, and the run `✕ 異常終了` in 今日 and in `koyomi status --today` |
| 8.4 | Press 強制終了, then Enter; press 強制終了 again, then 強制終了する | The first Enter only closes the box (the keyboard starts on やめる); the second ends the job as `■ 強制終了` and the run as `✕ 異常終了` |
| 8.5 | Press 縦に流す, close the window and open it again | The picture flows downwards, and still does after the restart (`state.toml`); 横に流す brings it back |
| 8.6 | Click a circle, and click a job's row in words | The log under the picture is that job's; the circle is outlined |
| 8.7 | `koyomi run 経理/月次締め --wait` | The run id is printed, then its line and one line per job (symbol, word, name, times, duration); exit 0 only when every job ended ok |
| 8.8 | `koyomi run` the same net while it runs | `動いている間はもう一度は流せない` and exit 1 |
| 8.9 | `koyomi run` on an unconfirmed net | `未確認のため流さない` with `koyomi approve …` and exit 1 |
| 8.10 | `koyomi status`, `koyomi status --today`, `koyomi log <run> <job>`, `koyomi kill <run>` | status lists the runs still going (`--today` every run of today); log prints the tail; kill ends the run |
| 8.11 | Stop the engine (`koyomi engine stop`) while a run is going, start it again | The run is `✕ 異常終了` and its running job `エンジンが止まった` |
| 8.12 | Set `concurrency = 1` in `koyomi.toml` and run a net with two jobs that have no `after` | The second job is `○ 待ち` until the first ends |
| 8.13 | A job that prints more than `log_max_mb` | Its log ends with `（ログが 10 MB を超えたので、この先は書かない）` and the job still ends with its own exit code |

## 9. Timeout and kill

| # | Do | Expect |
|---|---|---|
| 9.1 | Approve a net with `timeout = "30m"` on a job | The confirm box and `koyomi approve` show `打ち切り: 30m` under that job |
| 9.2 | A job with `timeout = "5s"` that runs `cmd.exe /c ping -n 60 127.0.0.1` | After about 5 s the job is `⏱ 打ち切り` with `打ち切り（5s を超えた）`, the run `✕ 異常終了`, the jobs after it `· 未実行`; the log ends with `! 打ち切り（5s を超えた）`; `ping.exe` is gone from Task Manager |
| 9.3 | 強制終了 a job that runs `cmd.exe /c start /wait notepad` | Both `cmd.exe` and `notepad.exe` are gone from Task Manager; the job is `■ 強制終了` and its log ends with `! 強制終了` |
| 9.4 | 強制終了 a job whose `timeout` is about to pass | The job is `■ 強制終了`, never `⏱ 打ち切り`; the words tell the two apart, the colour is the same |

## 10. Notifications

| # | Do | Expect |
|---|---|---|
| 10.1 | With no `[notify]` in `koyomi.toml`, run a net whose job exits 1, from the window | A system notification `✕ 異常終了  <net>` with the failed job's line and the run id; no sound |
| 10.2 | The same net started with `koyomi run` and with `koyomi run --wait` | The same notification, worded the same |
| 10.3 | Run a net that ends `✓ 正常終了` | No system notification, no sound |
| 10.4 | `[notify]` with `system = ["done", "failed"]`, run a net that ends well | A system notification `✓ 正常終了  <net>` |
| 10.5 | `[notify]` with `sound = ["failed"]`, a failing net | The alert sound with the notification; without that line, none |
| 10.6 | A failing net, then the bell | The bell counts 1 unread; the row is `✕ 異常終了  <net>` with the failed job under it; clicking it shows that run in the detail |
| 10.7 | A net that ends well, then the bell | Nothing new under the bell |
| 10.8 | `[notify]` with `webhook = "https://ntfy.sh/<topic>"` (plain text), start the engine | `koyomi list` prints `注意: koyomi.toml: [notify] の webhook のアドレスが平文で書かれている…`; the bell has the same line once; a failing net sends nothing to the topic |
| 10.9 | Add `hide_webhooks = false` to 10.8 | No note; a failing net posts to the topic with the same title and body as the notification |
| 10.10 | Put the address in Credential Manager as `uchmk/koyomi/ntfy` (generic credential, the URL as the password) and write `webhook = "secret:ntfy"` | No note; a failing net posts to the topic; the address appears nowhere in `koyomi.toml`, the history or the logs |
| 10.11 | `webhook = "secret:nothing"` (no such credential) | `koyomi list` prints a note that it does not read; nothing is sent; runs still start |
| 10.12 | `koyomi check koyomi.toml` with `system = ["failed", "exploded"]` | A line naming `exploded`, exit code 1; with a clean file, `koyomi.toml: 問題なし` and exit code 0 |
| 10.13 | Edit `[notify]` while the engine runs | Within about 5 s `koyomi list` reflects the change; no restart |

## 11. Schedule

| # | Do | Expect |
|---|---|---|
| 11.1 | A confirmed net with `[[when]] every = "day"` and `at` two minutes from now; wait | At that minute the net starts by itself: the 今日 list shows it `▶ 実行中` with the planned time, then its result; the bell stays quiet when it ends well |
| 11.2 | Before the time of 11.1 comes, the 今日 list | A dim row `○ 待ち  <net>  HH:MM:SS` above the runs; it is not a row to open; it is gone once the run starts |
| 11.3 | The detail of a net with `[[when]]` | Under the jobs, `次の 10 回` with rows like `2026-11-02（月）02:00`; `koyomi next <group/name>` prints the same rows; `--count 3` prints three |
| 11.4 | `koyomi next` on a net with no `[[when]]` | `時刻の予定はありません（手で流します）`; the detail shows the same line |
| 11.5 | A net that is `▲ 未確認` with `[[when]]`; let its time come | Nothing runs; the 今日 list shows `🔒 止めた  <net>` with `未確認のため流さなかった（koyomi approve <group/name>）`; the bell has it like an error; the detail's `次の 10 回` carries `未確認のため、時刻が来ても流しません` |
| 11.6 | `koyomi engine stop`, let two times of a net pass, start the engine again | Each passed time is a `↷ 時刻を過ぎた` row with `H:MM に PC が止まっていました`; nothing starts; one system notification `↷ 時刻を過ぎた  <net>` naming both times (`2:00、6:30 の回は時刻を過ぎました（PC が止まっていました）`); the bell has one warning row |
| 11.7 | A net with `at` every minute whose job takes three minutes | The second and third times are `⊘ スキップ` with `前の回がまだ動いていました`; no notification for them |
| 11.8 | Start the engine for the first time on a PC (delete `schedule.toml` in the state folder with the engine stopped) | No `時刻を過ぎた` rows for times before this start |
| 11.9 | Set the PC clock back an hour while the engine runs | Nothing runs twice; the times between are not reported as passed |
| 11.10 | A net with `[[when]] every = "month" day = "last"` at 02:00 | `koyomi next` lists the last day of each coming month |
| 11.11 | A net with `[[when]] every = "week" on = ["mon"]` and `calendar = "jp"` | The rows skip Japanese holidays that fall on a Monday |
| 11.12 | A confirmed net with `[[when]]` exists and `koyomi` is not set to start at sign-in | A band under the title: `時刻で流す定義がありますが、サインイン時に koyomi は起動しません…`; it goes away once the setting is on, or when no confirmed net has `[[when]]` |
| 11.13 | While 11.6's engine is stopped, write a `[[when]]` whose `at` is in the past of today into an unconfirmed net, then start the engine | The row is `🔒 止めた`, never `↷ 時刻を過ぎた` for a time the engine never knew of |

## 12. The week ahead at approval

| # | Do | Expect |
|---|---|---|
| 12.1 | A new net with `[[when]] every = "day" at = ["02:00"]`; open the confirm box | Under the jobs: `この先 7 日の予定: 7 回` and seven rows like `2026-10-31（土）02:00`; `koyomi approve` prints the same before asking for `y` |
| 12.2 | Approve 12.1, then change it to `every = "minutes" n = 15 from = "00:00" to = "23:59"`; open the box | `7 日で 7 回 → 672 回（+665）`, then 20 rows starting `+ ` with the next quarter hours, then `… 他 645 件`; the 2:00 rows are not listed (they stay) |
| 12.3 | Approve 12.2, then change it back to daily 2:00 | `7 日で 672 回 → 7 回（-665）` and 20 rows starting `- ` |
| 12.4 | Approve a daily net, then add `except = ["<a day within the week>"]` | `7 日で 7 回 → 6 回（-1）` and one `- ` row for that day |
| 12.5 | A net with `calendar = "jp"` on weekdays, approved; change it to `calendar = "none"` across a holiday | The holiday's time appears as a `+ ` row |
| 12.6 | A new net with no `[[when]]`; open the box | No week lines at all; after approving, remove a `[[when]]` from a net that had one | `7 日で N 回 → 0 回（-N）` with the `- ` rows |
| 12.7 | Open the box, wait, open it again a minute later | The counts are from the moment the box opens: a time that passed meanwhile is gone from the list |

## 13. `--keys`

| # | Do | Expect |
|---|---|---|
| 13.1 | `set KOYOMI_KEYS_DONE=<dir>\done.txt` then `koyomi --keys "<Wait:1500><Shot:start>"` | `start.png` is written beside the done file; the done file ends with `keys: done` |
| 13.2 | `koyomi --keys "<Wait:1500><State:s><Quit>"` | `s.txt` holds `engine: up`, `nets: N`, `runs: N`, `selected: -`, `asking: -`, `flow: horizontal`; the window closes by itself; the done file has `quit: yes` then `keys: done` |
| 13.3 | `koyomi --keys "<Wait:30000>"` and close the window with × while it waits | The done file says `keys: closed` and `left: 1 not pressed` |
| 13.4 | `koyomi --keys "<Nope>"` | Exit code 2, the message starts `koyomi: --keys:`, and the done file says `keys: refused` |
| 13.5 | `koyomi --keys` (no script) or `koyomi --keys j --more` | Exit code 2 and the usage; nothing opens |
| 13.6 | Move the window under `--keys "<Wait:30000>"`, close it, then start `koyomi` without `--keys` | The window comes back where it was before the scripted run: a script keeps no place and writes no `state.toml` |
| 13.7 | `koyomi --keys "<Theme:light><Lang:en><Wait:500><Shot:en>"` | The picture is light and in English; `common.toml` is unchanged afterwards |
| 13.8 | `koyomi --keys "<Wait:1500><Click:500,500><State:c>"` with a net in the tree at that point | `c.txt` says `selected: <group/name>` |

## 14. Keys (`keymap.toml`)

| # | Do | Expect |
|---|---|---|
| 14.1 | Press Ctrl+Q with the tree focused | The window closes (the engine keeps running) |
| 14.2 | Press Ctrl+, and then F1 | The status bar says the settings, then the help, are not wired yet; nothing else changes |
| 14.3 | Open the palette (Ctrl+Shift+P) | "Quit" shows `Ctrl+Q` as its key |
| 14.4 | Write `<uchmk>\koyomi\keymap.toml` with `[[app.keymap]]` `on = "<C-q>"` `run = "none"` and another binding `on = "<C-w>"` `run = "quit"`, restart | Ctrl+Q does nothing; Ctrl+W quits; the palette shows `Ctrl+W` |
| 14.5 | Add a line with `run = "explode"` and one with `on = "<F12>"`, restart | The bell lists two warnings naming the file: `explode` is no action, F12 is fixed; the other keys still work |
| 14.6 | Type Ctrl+Q while the search box of the palette has the focus | The palette takes the key (nothing quits); Esc, then Ctrl+Q quits |
