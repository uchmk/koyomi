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
| 2.2 | [x] Run `koyomi list` and `koyomi list --toml` with the engine stopped | The engine starts; the same list as the tree is printed |

## 3. Engine

| # | Do | Expect |
|---|---|---|
| 3.1 | Open the window with no engine running, then close the window | The engine starts by itself and is still running after the window is closed (`koyomi engine status` answers) |
| 3.2 | [x] Run `koyomi engine` while one is up, then `koyomi engine stop` | The first prints "もう動いている" and exits 1; stop stops the running one |
| 3.3 | Run `koyomi startup on`, then `koyomi startup` and `koyomi engine status` | Both say サインイン時の起動: あり (the status on its last line); the Run key `uchmk.koyomi` under HKCU holds `koyomi.exe engine`; the settings' General page shows the switch on; after signing in again the engine is up with no window |
| 3.4 | Run `koyomi startup off`, then `koyomi startup oops` | The first says サインイン時には起動しない and the key is gone; the second prints the usage and exits 2 |

## 4. Approval

| # | Do | Expect |
|---|---|---|
| 4.1 | Select a net, press 確認する… | A box lists the program, each argument on its own line, and the folder; Enter = 確認した, Esc = キャンセル |
| 4.2 | [x] `koyomi approve 経理/月次締め`, answer `y`, restart the engine | The same content is printed; the net shows no mark or word of its own (just its name) and still does after the restart |
| 4.3 | [x] `koyomi approve 経理/月次締め` and answer anything but `y` | It stays `▲ 未確認` and the exit code is 1 |
| 4.4 | In the box press キャンセル | The net stays `▲ 未確認` |
| 4.5 | With the box up, press Tab, then Enter | Tab moves between 確認した and キャンセル only; Enter answers the box and nothing behind it is pressed or selected |
| 4.6 | Open the box on a net with a long diff, close it, open it on a one-job net; then make the window small | The title, the rows, the program and the buttons are inside the window every time, and the box does not grow while it is up |

## 5. Edits

| # | Do | Expect |
|---|---|---|
| 5.1 | [x] Approve a net, then add a comment line to its file by hand | Within seconds it is `▲ 未確認` again |
| 5.2 | [x] Approve it again | The box (and `approve`) shows the change as `+` / `-` lines against the approved text |

## 6. Check

| # | Do | Expect |
|---|---|---|
| 6.1 | [x] `koyomi check <file>` on a good file and on files with a typo'd field, bad syntax, no/empty `run`, an unknown `after`, a cycle, a duplicate id | 0 for the good one; for the others the file, the line and the cause, and exit 1 |
| 6.2 | Look at a broken file in the tree and try `approve` | `✖ 定義の誤り`; it cannot be approved |
| 6.3 | [x] A file with `timeout = 60` (no unit) and one with `retry = { count = 2 }` | `KOYOMI-E3308 timeout が文字列ではありません。` with the action `timeout は "30m" のように、文字列で指定してください。` for the first, `KOYOMI-E3328 retry に wait が指定されていません。` with the action `retry は { count = 2, wait = "5m" } の形式で指定してください。` for the second; neither can be approved |

## 7. Safety of the places

| # | Do | Expect |
|---|---|---|
| 7.1 | [x] Let Everyone write a definition, then `check` and `approve` | Both exit 1 and name the file |
| 7.2 | Same, with the window open | A warning band names the place and who; 確認する… is disabled for every net |
| 7.3 | [x] Break `approved.toml` by hand, then `list` and `approve` | Every net shows `▲ 未確認`, a warning names `approved.toml`, `approve` exits 1 and the file is left as it was; a break made while the engine runs is noticed only at the next engine start; once mended the approvals come back without a restart |

## 8. Runs

| # | Do | Expect |
|---|---|---|
| 8.1 | Approve a net, select it, press 今すぐ実行 | 今日 lists the run as `▶ 実行中` with its start time, and the detail switches to the run's picture |
| 8.2 | Watch while it runs | Each circle goes `·` → `▶` → `✓`; the line out of a finished job is painted in its colour; the running job's log flows under the picture, newest line at the bottom |
| 8.3 | [x] A net whose one job exits 1 | That job is `✕ 異常終了` with its exit code, the jobs after it `· 未実行`, and the run `✕ 異常終了` in 今日 and in `koyomi status --today` |
| 8.4 | Press 強制終了, then Enter; press 強制終了 again, then 強制終了する | The first Enter only closes the box (the keyboard starts on キャンセル); the second ends the job as `■ 強制終了` and the run as `✕ 異常終了` |
| 8.5 | Press 縦に流す, close the window and open it again | The picture flows downwards, and still does after the restart (`state.toml`); 横に流す brings it back |
| 8.6 | Click a circle, and click a job's row in words | The log under the picture is that job's; the circle gets a blue halo |
| 8.9 | Watch a run with one job running, one failed, and one not reached, flowing sideways and downwards | The running job's lines carry pulses moving left to right (top to bottom), and its circle sends out a fading ring; lines a result came through are thick in that result's soft colour, curving between rows; lines not reached yet are dotted; finished and running circles are filled with their symbol inside; under each name is `9:00–9:04` (`9:00–` while it runs, ` exit 3` after a bad end, in red) |
| 8.7 | [x] `koyomi run 経理/月次締め --wait` | The run id is printed, then its line and one line per job (symbol, word, name, times, duration); exit 0 only when every job ended ok |
| 8.8 | [x] `koyomi run` the same net while it runs | `koyomi: KOYOMI-E3278 … は実行中のため、もう一度実行できません。` and exit 1 |
| 8.9 | [x] `koyomi run` on an unconfirmed net | `koyomi: KOYOMI-E3741 ジョブネット … は確認されていないため、実行できません。` with the action `koyomi approve … で確認してから、再度実行してください。` and exit 1 |
| 8.10 | [x] `koyomi status`, `koyomi status --today`, `koyomi log <run> <job>`, `koyomi kill <run>` | status lists the runs still going (`--today` every run of today); log prints the tail; kill ends the run |
| 8.11 | [x] Stop the engine (`koyomi engine stop`) while a run is going, start it again | The run is `✕ 異常終了` and its running job `強制終了` |
| 8.12 | [x] Set `concurrency = 1` in `koyomi.toml` and run a net with two jobs that have no `after` | The second job is `○ 待ち` until the first ends |
| 8.13 | [x] A job that prints more than `log_max_mb` | Its log ends with `（ログが 10 MB を超えたので、この先は書き込みません）` and the job still ends with its own exit code |

## 9. Timeout and kill

| # | Do | Expect |
|---|---|---|
| 9.1 | Approve a net with `timeout = "30m"` on a job | The confirm box and `koyomi approve` show `打ち切り: 30m` under that job |
| 9.2 | [x] A job with `timeout = "5s"` that runs `cmd.exe /c ping -n 60 127.0.0.1` | After about 5 s the job is `⏱ 打ち切り` with `打ち切り（5s を超えました）`, the run `✕ 異常終了`, the jobs after it `· 未実行`; the log ends with `! 打ち切り（5s を超えました）`; `ping.exe` is gone from Task Manager |
| 9.3 | 強制終了 a job that runs `cmd.exe /c start /wait notepad` | Both `cmd.exe` and `notepad.exe` are gone from Task Manager; the job is `■ 強制終了` and its log ends with `! 強制終了` |
| 9.4 | [x] 強制終了 a job whose `timeout` is about to pass | The job is `■ 強制終了`, never `⏱ 打ち切り`; the words tell the two apart, the colour is the same |

## 10. Notifications

| # | Do | Expect |
|---|---|---|
| 10.1 | With no `[notify]` in `koyomi.toml`, run a net whose job exits 1, from the window | A system notification `✕ 異常終了  <net>` with the failed job's line and the run id; no sound |
| 10.2 | The same net started with `koyomi run` and with `koyomi run --wait` | The same notification, worded the same |
| 10.3 | [x] Run a net that ends `✓ 正常終了` | No system notification, no sound |
| 10.4 | [x] `[notify]` with `system = ["done", "failed"]`, run a net that ends well | A system notification `✓ 正常終了  <net>` |
| 10.5 | [x] `[notify]` with `sound = ["failed"]`, a failing net | The alert sound with the notification; without that line, none |
| 10.6 | A failing net, then the bell | The bell counts 1 unread; the row is `✕ 異常終了  <net>` with the failed job under it; clicking it shows that run in the detail |
| 10.7 | A net that ends well, then the bell | Nothing new under the bell |
| 10.8 | [x] `[notify]` with `webhook = "https://ntfy.sh/<topic>"` (plain text), start the engine | `koyomi list` prints `KOYOMI-W3267 koyomi.toml の [notify] の webhook のアドレスが、平文で指定されています。そのアドレスには送信しません。` with its action; the bell has the same line once; a failing net sends nothing to the topic |
| 10.9 | [x] Add `hide_webhooks = false` to 10.8 | No note; a failing net posts to the topic with the same title and body as the notification |
| 10.10 | [x] Put the address in Credential Manager as `uchmk/koyomi/ntfy` (generic credential, the URL as the password) and write `webhook = "secret:ntfy"` | No note; a failing net posts to the topic; the address appears nowhere in `koyomi.toml`, the history or the logs |
| 10.11 | [x] `webhook = "secret:nothing"` (no such credential) | `koyomi list` prints a note that it does not read; nothing is sent; runs still start |
| 10.12 | [x] `koyomi check koyomi.toml` with `system = ["failed", "exploded"]` | A line naming `exploded`, exit code 1; with a clean file, `koyomi.toml: 問題はありません。` and exit code 0 |
| 10.13 | [x] Edit `[notify]` while the engine runs | Within about 5 s `koyomi list` reflects the change; no restart |
| 10.14 | [x] With the default `[notify]`, `koyomi run <net> --wait` a failing net four times in a row on the same engine | Four `✕ 異常終了` notifications; `koyomi engine status` shows the same pid afterwards (the engine did not crash) |

## 11. Schedule

| # | Do | Expect |
|---|---|---|
| 11.1 | [x] A confirmed net with `[[when]] every = "day"` and `at` two minutes from now; wait | At that minute the net starts by itself: the 今日 list shows it `▶ 実行中` with the planned time, then its result; the bell stays quiet when it ends well |
| 11.2 | Before the time of 11.1 comes, the 今日 list | A dim row `○ 待ち  <net>  HH:MM:SS` above the runs; it is not a row to open; it is gone once the run starts |
| 11.3 | [x] The detail of a net with `[[when]]` | Under the jobs, `次の 10 回` with rows like `2026-11-02（月）02:00`; `koyomi next <group/name>` prints the same rows; `--count 3` prints three |
| 11.4 | [x] `koyomi next` on a net with no `[[when]]` | `時刻の予定はありません（手動で実行します）。`; the detail shows the same line |
| 11.5 | [x] A net that is `▲ 未確認` with `[[when]]`; let its time come | Nothing runs; the 今日 list shows `🔒 停止  <net>` with `未確認のため、実行しませんでした（koyomi approve <group/name>）。`; the bell has it like an error; the detail's `次の 10 回` carries `未確認のため、時刻が来ても実行しません。` |
| 11.6 | `koyomi engine stop`, let two times of a net pass, start the engine again | Each passed time is a `↷ 時刻を過ぎた` row with `H:MM に PC が停止していました。`; nothing starts; one system notification `↷ 時刻を過ぎた  <net>` naming both times (`2:00、6:30 の実行は時刻を過ぎました（PC が停止していました）`); the bell has one warning row |
| 11.7 | [x] A net with `at` every minute whose job takes three minutes | The second and third times are `⊘ スキップ` with `前の回がまだ実行中でした。`; no notification for them |
| 11.8 | [x] Start the engine for the first time on a PC (delete `schedule.toml` in the state folder with the engine stopped) | No `時刻を過ぎた` rows for times before this start |
| 11.9 | Set the PC clock back an hour while the engine runs | Nothing runs twice; the times between are not reported as passed |
| 11.10 | [x] A net with `[[when]] every = "month" day = "last"` at 02:00 | `koyomi next` lists the last day of each coming month |
| 11.11 | [x] `calendars/jp.toml` with `holidays = "japan"`; a net with `calendar = "jp"` and `[[when]] every = "week" days = ["mon"] at = ["02:00"]`; `koyomi next <net> --count 12` | The rows skip Japanese holidays that fall on a Monday (no 2026-11-23, no 2027-01-11) |
| 11.12 | A confirmed net with `[[when]]` exists and `koyomi` is not set to start at sign-in | A band under the title: `時刻で実行する定義がありますが、サインイン時に koyomi は起動しません。…`; it goes away once the setting is on, or when no confirmed net has `[[when]]` |
| 11.13 | [x] While 11.6's engine is stopped, write a `[[when]]` whose `at` is in the past of today into an unconfirmed net, then start the engine | The row is `🔒 停止`, never `↷ 時刻を過ぎた` for a time the engine never knew of |

## 12. The week ahead at approval

| # | Do | Expect |
|---|---|---|
| 12.1 | [x] A new net with `[[when]] every = "day" at = ["02:00"]`; open the confirm box | Under the jobs: `この先 7 日の予定: 7 回` and seven rows like `2026-10-31（土）02:00`; `koyomi approve` prints the same before asking for `y` |
| 12.2 | [x] Approve 12.1, then change it to `every = "minutes" n = 15 from = "00:00" to = "23:59"`; open the box | `7 日で 7 回 → 672 回（+665）`, then 20 rows starting `+ ` with the next quarter hours, then `… 他 645 件`; the 2:00 rows are not listed (they stay) |
| 12.3 | [x] Approve 12.2, then change it back to daily 2:00 | `7 日で 672 回 → 7 回（-665）` and 20 rows starting `- ` |
| 12.4 | [x] Approve a daily net, then add `except = ["<a day within the week>"]` | `7 日で 7 回 → 6 回（-1）` and one `- ` row for that day |
| 12.5 | [x] A net with `calendar = "jp"` on weekdays, approved; change it to a calendar file with no holidays (`holidays = "none"`) across a holiday | The holiday's time appears as a `+ ` row |
| 12.6 | [x] A new net with no `[[when]]`; open the box | No week lines at all; after approving, remove a `[[when]]` from a net that had one | `7 日で N 回 → 0 回（-N）` with the `- ` rows |
| 12.7 | [x] Open the box, wait, open it again a minute later | The counts are from the moment the box opens: a time that passed meanwhile is gone from the list |

## 13. `--keys`

| # | Do | Expect |
|---|---|---|
| 13.1 | `set KOYOMI_KEYS_DONE=<dir>\done.txt` then `koyomi --keys "<Wait:1500><Shot:start>"` | `start.png` is written beside the done file; the done file ends with `keys: done` |
| 13.2 | `koyomi --keys "<Wait:1500><State:s><Quit>"` | `s.txt` holds `engine: up`, `nets: N`, `runs: N`, `selected: -`, `asking: -`, `flow: horizontal`; the window closes by itself; the done file has `quit: yes` then `keys: done` |
| 13.3 | `koyomi --keys "<Wait:30000>"` and close the window with × while it waits | The done file says `keys: closed` and `left: 1 not pressed` |
| 13.4 | [x] `koyomi --keys "<Nope>"` | Exit code 2, the message starts `koyomi: --keys:`, and the done file says `keys: refused` |
| 13.5 | [x] `koyomi --keys` (no script) or `koyomi --keys j --more` | Exit code 2 and the usage; nothing opens |
| 13.6 | Move the window under `--keys "<Wait:30000>"`, close it, then start `koyomi` without `--keys` | The window comes back where it was before the scripted run: a script keeps no place and writes no `state.toml` |
| 13.7 | `koyomi --keys "<Theme:light><Lang:en><Wait:500><Shot:en>"` | The picture is light and in English; `common.toml` is unchanged afterwards |
| 13.8 | `koyomi --keys "<Wait:1500><Click:500,500><State:c>"` with a net in the tree at that point | `c.txt` says `selected: <group/name>` |

## 14. Keys (`keymap.toml`)

| # | Do | Expect |
|---|---|---|
| 14.1 | Press Ctrl+Q with the tree focused | The window closes (the engine keeps running) |
| 14.2 | Press Ctrl+, and then F1 | Ctrl+, opens the settings; Esc closes them; F1 opens the key help over the tree |
| 14.3 | Open the palette (Ctrl+Shift+P) | "Quit" shows `Ctrl+Q` as its key |
| 14.4 | Write `<uchmk>\koyomi\keymap.toml` with `[[app.keymap]]` `on = "<C-q>"` `run = "none"` and another binding `on = "<C-w>"` `run = "quit"`, restart | Ctrl+Q does nothing; Ctrl+W quits; the palette shows `Ctrl+W` |
| 14.5 | Add a line with `run = "explode"` and one with `on = "<F12>"`, restart | The bell lists two warnings naming the file: `explode` is no action, F12 is fixed; the other keys still work |
| 14.6 | Type Ctrl+Q while the search box of the palette has the focus | The palette takes the key (nothing quits); Esc, then Ctrl+Q quits |

## 15. Settings (Ctrl+,)

| # | Do | Expect |
|---|---|---|
| 15.1 | Press Ctrl+, | The settings cover the tree: General, Theme, Keys, Advanced, About down the left; the foot of the nav opens `common.toml`, with `koyomi.toml` above it |
| 15.1a | At the foot of the settings' list, press 「開く: koyomi.toml」, then 「開く: common.toml」; then Tab from the page to the foot | Each opens its own file in the OS's editor (a missing `koyomi.toml` is made empty first and the engine reads it as the defaults); Tab comes to koyomi.toml, then common.toml; the list above is shorter by the second button, not hidden under it |
| 15.2 | Press Esc, or ← at the top | The tree is back as it was (the selected net still selected) |
| 15.3 | General: pick 日本語 | The window turns Japanese at once; `common.toml` has `language = "ja"` within 2 seconds; minato or kura at the same desk follow |
| 15.4 | General: pick 125 % | The window scales; `scale = 1.25` is in `common.toml`; Ctrl+0 puts it back and the file follows |
| 15.5 | General: switch the clock's date on and change its format | The status bar's clock shows the date in that format at once |
| 15.6 | General: "Start at sign-in" | The switch shows what the OS has; toggled, the registry (Windows) or the autostart folder gets koyomi, and the band about starting at sign-in (TESTING 11) goes away at the next start |
| 15.7 | General: "Forget the window's place" | `state_dir("koyomi")\window.toml` is gone; the next start opens at the default size in the middle |
| 15.8 | Theme: pick Dark, then Light, then System | The colours change at once, the sample card with it; `theme = "…"` is in `common.toml`; System follows the OS's light/dark |
| 15.9 | Theme: edit `common.toml` by hand to `theme = "tsumugi Light"` while the window is open | The window turns light within 2 seconds without a restart |
| 15.10 | Keys: click Ctrl+Q's key, press Ctrl+W | The row shows Ctrl+W; `<uchmk>\koyomi\keymap.toml` holds only that change; Ctrl+W quits from then on and the palette shows it |
| 15.11 | Keys: while the page waits for a key, press F12 and Ctrl+Shift+P | They are taken as the key (no bug form, no palette); Esc gives the page up |
| 15.12 | Keys: bind F12 to an action | The row says koyomi reads F12 itself and the binding never runs |
| 15.13 | Keys: ↺ on a changed row | The built-in key is back and the line is gone from `keymap.toml` (the file is removed when nothing is left in it) |
| 15.14 | Keys: write `run = "explode"` into `keymap.toml` by hand while the window is open | The bell lists the warning within 2 seconds; the other keys still work |
| 15.15 | Advanced: "Open the config folder" and "Open the state folder" | The file manager opens `<uchmk>` and `state_dir("koyomi")` |
| 15.16 | Advanced: "Put the settings back" | `common.toml` and `keymap.toml` are moved aside as `*.bak-<stamp>`; the definitions, `koyomi.toml`, `approved.toml` and the runs are untouched; the window goes back to the defaults at once |
| 15.17 | About | The version, the commit, the OS line and the licence; "Report a bug" opens the issue form as F12 does |
| 15.18 | Type `keymap` in the search box | The Keys page is offered; Enter opens it with its rows highlighted |
| 15.19 | Open the settings while the engine runs a job | The run still ends (the bell still rings for a failed one) and the tree shows it once the settings close |

## 16. Help (F1, ?)

| # | Do | Expect |
|---|---|---|
| 16.1 | Press F1, or `?` | A card over the tree, the tree dimmed behind it: Everywhere (Ctrl+Q, Ctrl+,, F1 / ?), Always (F12, Ctrl+Shift+P, Ctrl++ / Ctrl+-, Ctrl+0), A box over the tree, Keys |
| 16.2 | Press Esc; open again and press F1; open again and press `?`; open again and click the dimmed tree | Each closes the card; the tree is as it was |
| 16.3 | Make the window short (about 300 px tall) and open the help | The card is cut to the window; ↓ ↑ PgDn PgUp End Home and the wheel scroll the list; a wide window lays it out in two columns, a narrow one in one |
| 16.4 | Open the palette (Ctrl+Shift+P) and type `?` in its search box | The search box gets the `?`; the help does not open |
| 16.5 | In `keymap.toml` bind `<F2>` to `help` (and `<F1>` to `none`), then open the help with F2 | The Everywhere group shows F2 / ? for the help and no F1 |
| 16.6 | With the help open press Ctrl+Q | koyomi quits (the keymap still works under the help) |
| 16.7 | Select a net, press Approve… so its box shows, then press F1 and Enter | The help closes nothing behind it: Enter does not press the box's button; Esc closes the help first, then the box |
| 16.8 | Open the palette and look at the Keys entry | It shows F1 (the first key bound to `help`); picking it opens the help |
| 16.9 | Press Ctrl+, while the help is open | The settings cover the tree and the help is closed; Esc brings back the tree without the help |

## 17. Settings: Notifications

| # | Do | Expect |
|---|---|---|
| 17.1 | With no `koyomi.toml`, open the settings and the Notifications page | A table with the columns Ended ok / Ended abnormally: the system notification is on for Ended abnormally only, no sound, no webhook; "Keep the addresses out of the file" is on; nothing is written yet |
| 17.2 | Switch the system notification on for Ended ok | `koyomi.toml` gets `[notify]` with `system = ["done", "failed"]` and nothing else; within 5 s `koyomi list` shows no note, and a net that ends well is told of (10.4) |
| 17.3 | Switch the sound on for Ended abnormally and pick another sound | `sound = ["failed"]` and `sound_failed = "<name>"` are written; a failing net plays it (10.5) |
| 17.4 | Press ↺ at the top | The switches go back to 17.1; `[notify]` keeps only what still differs from the defaults (the webhooks and the switch stay) |
| 17.5 | Add a webhook with an `https://ntfy.sh/<topic>` address, ntfy, for Ended abnormally | `koyomi.toml` gets `[[notify.webhooks]]` with `url = "secret:webhook-1"`; the address is in Credential Manager under `uchmk/koyomi/webhook-1`; the page shows the address; a failing net posts to the topic |
| 17.6 | Press "Send a test" at the top, then the test button on the webhook's row | A system notification `koyomi: 試しの通知` (the first with the sound when one is on); the topic gets the same text; the second sends to that topic only |
| 17.7 | Switch "Keep the addresses out of the file" off | `hide_webhooks = false` and the address itself are written; the credential is deleted; `koyomi check koyomi.toml` says nothing about it |
| 17.8 | Write `webhook = "https://ntfy.sh/<topic>"` by hand with `hide_webhooks` unset, then open the page | The page shows the red note about the plain-text address (10.8) with the address in the webhook's row; changing anything on the page moves the address to the credential store and `secret:webhook-1` into the file; the note goes |
| 17.9 | Edit `[notify]` with a text editor while the page is open | Within 2 s the page shows it; a line it cannot read is shown in red on the page and the switches stay |
| 17.10 | Open the page with `concurrency = 2` and a comment in `koyomi.toml`, change a switch | Only `[notify]` changed; `concurrency`, the comment and the line order stay |
| 17.11 | Search the settings for `webhook` or `[notify]` | The results show the Notifications page |

## 18. Settings: Records

| # | Do | Expect |
|---|---|---|
| 18.1 | With no `koyomi.toml`, open the settings' General page | Under "Records": Days of logs and history to keep = 90 days, Jobs at a time = 4, Log size = 10 MB; nothing is written |
| 18.2 | Pick 180 days | `koyomi.toml` gets `keep_days = 180` at the top (before any table); no box asks (it is longer); `koyomi engine status` shows the sweep line again within 5 s |
| 18.3 | Put `logs/2026-01-05/` and `history/2026/01/06.toml` in the state folder, then pick 30 days | A box says 30 days are kept, 180 → 30, and that 2 days of logs and history go at the engine's next sweep; Enter writes `keep_days = 30`; within 5 s the two are gone and `engine status` says `ログ 1 日分と履歴 1 日分を削除しました。` |
| 18.4 | Pick a shorter number again with nothing old there, and press Esc | The box says no days go right away; Esc keeps the file as it was; the row shows the days as before |
| 18.5 | Type 2 in Jobs at a time and press Enter | `concurrency = 2` is written at the top; a net with 3 parallel jobs runs 2 at a time within 5 s |
| 18.6 | Type 0, `abc` or 1.5 in Log size | A red note under "Records" says to enter a whole number of 1 or more; nothing is written; the field goes back to the number in the file |
| 18.7 | Write `keep_days = 45` by hand with a comment above `[notify]`, open the page, pick 60 | The row showed "45 days" among the choices; only `keep_days` changed; the comment, `[notify]` and the line order stay |
| 18.8 | [x] Put `logs/notes/` and `logs/2026-01-05-old/` in the state folder and shorten the days | Neither is counted nor removed: only names that are a day (`2026-01-05`, `2026/01/05.toml`) are swept |
| 18.9 | Search the settings for `keep_days`, `concurrency`, `log_max_mb` or `koyomi.toml` | The results show the General page |
| 18.10 | [x] On a test profile with runs from 3 or more days ago, write `archive = true` and `keep_days = 1` in `koyomi.toml` | Within 5 s those days leave `logs/` and `history/`; `archive/YYYY-MM.zip` in the state folder holds them under the same names (`history/2026/10/28.toml`, `logs/2026-10-28/<run>/<job>.log`); `engine status` says `…を削除しました。` and `n 日分を archive/ に移動しました。`; today and yesterday stay |
| 18.11 | [x] `koyomi archive`; `koyomi archive --toml` | A line per month, `2026-10  履歴 n 日・ログ n 日  <size>`, then the folder; under `--toml`: `dir`, `errors` and a `[[zip]]` per month with `month`, `path`, `bytes`, `history`, `logs` |
| 18.12 | [x] `koyomi archive show <one of those days>`; `koyomi archive log <its run> <job>`; `koyomi archive show` of a day not there | The day's runs as `koyomi history` lists them; the job's whole log; `<day> は archive/ にありません。` and exit 1 |
| 18.13 | [x] `koyomi history --from <the first of those days>` | Those days' runs are listed again (the engine reads them from the zip), oldest first, before today's, each once |
| 18.14 | [x] Make the month's zip read-only, then change `keep_days` so that another day of that month goes | That day stays in `logs/` and `history/`; `engine status` says `削除できなかった項目が 1 件あります（…YYYY-MM.zip: …）。`; once the zip is writable again, the next sweep puts the day in and removes it |
| 18.15 | [x] Put a copy of a zip as `archive/2025-01.zip` and write `archive_days = 30` | At the sweep it goes and `engine status` says `古い zip を 1 個削除しました。`; the zips of the months of the last 30 days stay |
| 18.16 | Under "Records", turn on 削除する前に archive/ に保管する, then off | `archive = true`, then `archive = false`, is written at the top of `koyomi.toml`; the comments and the line order stay; searching the settings for `archive` shows the General page |
| 18.17 | With it on, shorten the days so that some go (as 18.3) | The box says the n days go into archive/ at the next sweep and can be read with koyomi archive, not that they cannot be brought back; with it off the box says as in 18.3 |
| 18.18 | `koyomi log <run> <job>` for a run whose day is only in `archive/` (18.10), with the engine running and then stopped | Read from the zip: with the engine, the last 64 KB from a line start (all of a shorter log), as for a log in `logs/`; with it stopped, the whole log (note: every CLI call autostarts the engine, so the stopped case cannot be observed from the CLI); for a run in neither `logs/` nor `archive/`, the same error as before |
| 18.19 | In the window, open 履歴 with a range that holds one of those days; pick one of its runs, then a job | The day's runs are in the list with the others (each once); the job's log shows in the log pane (its end, as for a run in `logs/`); a day in a month with no zip shows nothing and no error |

## 19. Retry

| # | Do | Expect |
|---|---|---|
| 19.1 | [x] Write a job with `retry = { count = 2, wait = "5s" }` and run `koyomi check`, then `koyomi approve` | check says OK; the approve text shows `やり直し: 2 回まで、5s 間隔で` under the job |
| 19.2 | [x] Make that job `cmd.exe /c exit 3` and run it | The log shows `! やり直し (1/2): 5s 待機します`, then `! やり直し (2/2): 5s 待機します`; the job turns 異常 after about 10 s with the reason `終了コード 3、やり直し 2 回`; the run screen and the history show the same reason |
| 19.3 | [x] Make the job fail once and succeed the second time (a flag file) | The run ends 正常; the job's reason is `やり直し 1 回`; the log has `(1/2)` and no `(2/2)` |
| 19.4 | [x] Give the job `timeout = "2s"` and a command that sleeps 10 s | 打ち切り after 2 s, no やり直し line: a timeout is not run again; the same for `koyomi kill` during the job (強制終了) |
| 19.5 | [x] With `wait = "60s"`, run `koyomi kill` while the log says やり直し | The job ends 強制終了 at once (not after the wait); the log ends with `! 強制終了` |
| 19.6 | [x] Write `retry = 2`, `retry = { count = 0, wait = "5m" }`, `retry = { count = 2 }` or `retry = { count = 2, wait = 300 }` | `koyomi check` refuses each with the line number and what to write (the shape, 1 to 100, wait missing, a string like "5m") |

## 20. A passed time run once, a time that waits (`missed`, `overlap`)

| # | Do | Expect |
|---|---|---|
| 20.1 | [x] Write `missed = "once"` at the top of a net with `at = ["02:00"]`, approve it, stop the engine, set the clock past 02:00 (or wait), and start the engine | The run starts at once with 遅れて実行（2:00 の予定を 8:12 に実行しました） as its reason (only when the start is more than 60 s late) on the run screen, in `koyomi status` and in the history; the notification title reads `正常（遅れて実行）  <title>`; the approve text says 時刻を過ぎた実行: 起動したとき最後の 1 回を実行する（遅れて実行） |
| 20.2 | [x] The same with two passed times (`at = ["02:00", "03:00"]`) | Only the 03:00 run starts late; 02:00 is 時刻を過ぎた with `2:00 に PC が停止していました。` |
| 20.3 | [x] The same net, but unconfirmed, or with a definition folder writable by others | Nothing starts late: the passed time is 停止 with the same reason an on-time one gets |
| 20.4 | [x] Write `overlap = "queue"` and `at = ["10:00", "10:01"]` with a job that sleeps 3 minutes | At 10:01 nothing starts; `koyomi engine status` shows `待ち: <title>  10:01 の回は、前の回が終了するまで待っています（10:01 から待っています）。`; when the 10:00 run ends, the 10:01 run starts (how 予定, planned 10:01, no 遅れて実行) |
| 20.5 | [x] `at = ["10:00", "10:01", "10:02"]` with the same job | 10:01 waits; 10:02 is スキップ with `前の実行がまだ続いていました（その次の実行が待機しています）` |
| 20.6 | [x] Run `koyomi engine stop` while a time waits | The waiting time is recorded スキップ with `待機中にエンジンが停止しました`; nothing starts when the engine comes back |
| 20.7 | [x] Write `missed = "later"` or `overlap = 1` | `koyomi check` refuses with the line number: `KOYOMI-E3306 missed の値が正しくありません。` with `missed には、"skip", "once" のいずれかを指定してください。` / `KOYOMI-E3306 overlap の値が正しくありません。` with `overlap には、"skip", "queue" のいずれかを指定してください。`; `missed = "skip"` and `overlap = "skip"` are accepted, as is leaving both out |

## 21. Exit-code judge and PowerShell script jobs (`judge`, `shell`, `script`)

| # | Do | Expect |
|---|---|---|
| 21.1 | [x] Write `judge = { warn = "1", error = "2.." }` on a job and run `koyomi check`, then `koyomi approve` | check says OK; the approve text shows `判定: 警告 1、異常 2..、それ以外は 0 のみ正常` under the job |
| 21.2 | Make that job `cmd.exe /c exit 1` with a job after it, and run it | The job is 警告 with the reason `終了コード 1（警告）`, the next job runs, and the run ends 警告 (yellow) on the run screen, in `koyomi status`, in the history and in the notification title |
| 21.3 | [x] The same job with `exit 2`, and with `exit 5` and no `judge` at all | Both are 異常 with `終了コード 2` / `終了コード 5`; the next job reads `· 未実行  <job>  先行の a が異常終了しました` |
| 21.4 | [x] Give the warn job `retry = { count = 2, wait = "5s" }` and `exit 1` | No `! やり直し` line in the log: a 警告 end is not run again; `exit 2` is retried as before |
| 21.5 | [x] Write `shell = "powershell"` and `script = '''
Write-Output "hi"
exit 4'''` with `judge = { warn = "4" }`, approve and run | The approve text shows `シェル: powershell` and the script lines under `スクリプト:`; the log has `hi`; the job is 警告 with `終了コード 4（警告）`; no profile is loaded and nothing is asked (the job ends by itself) |
| 21.6 | [x] Put a `"`, a `$`, a `'` and a Japanese word in the script (`Write-Output "a `"b`" $env:USERNAME 'c' 日本語"`) | The log shows the line as PowerShell prints it: quotes and `$` reach the shell as written (the script is passed as `-EncodedCommand`) |
| 21.6a | [x] A `shell = "powershell"` job with `Write-Warning 'careful'`, `Write-Verbose 'said' -Verbose`, `Write-Host 'hi'`, `Get-Item 'C:\nope\nope'` and `exit 0`, with `judge = { error_output = "^Get-Item" }` | No `#< CLIXML`, `<Objs` or `_x000D_` in the log and no 「モジュールを初めて使用するための準備」 line; `! 警告: careful`, `! 詳細: said`, `hi` (once) and the error's lines (`! Get-Item : …`, `! 発生場所 行:4 文字:1` …) as PowerShell prints them; the job is 異常 with the reason `出力に異常の行がありました: Get-Item : …` |
| 21.7 | [x] Write `judge = { warn = "5..2" }`, `judge = { ok = "0" }`, `judge = "1"`, `shell = "cmd"`, `shell = "powershell"` with no `script`, `script` with no `shell`, or both `run` and `shell` | `koyomi check` refuses each with the line number and what to write (start not above end, unknown field, the shape, `shell には、"powershell" のいずれかを指定してください。`, `KOYOMI-E3418 shell に対応する script が指定されていません。`, `KOYOMI-E3419 script に対応する shell が指定されていません。`, `KOYOMI-E3417 run と shell が同時に指定されています。`) |

## 22. Rerun and start at a job (`rerun`, `run --from`)

| # | Do | Expect |
|---|---|---|
| 22.1 | [x] Run a net whose second job fails (`exit 3`), then `koyomi rerun <run>` | A new run with the reason `再実行（<run>）`; the first job is ✓ with the reason `前の回の結果` and keeps its old times, and no new log for it appears under `logs/<day>/<new run>/`; the failed job runs again and has a log; `koyomi status --today` lists both runs |
| 22.2 | [x] `koyomi rerun <run> --from <first job>`, then `--from x` | The first job runs again too (a new log, the reason `再実行（<run> を <first job> から）`); `--from x` is refused with `KOYOMI-E3201 ジョブ x が存在しません。` and nothing starts |
| 22.3 | [x] `koyomi rerun` while the run is still going; then a run where everything passed, without `--from` | `KOYOMI-E3202 実行 <run> はまだ終了していません。`; `KOYOMI-E3203 実行 <run> は、すべてのジョブが完了しています。` with the action `--from <ジョブ> でジョブを指定すると、そのジョブから再実行できます。`; nothing starts either time |
| 22.4 | [x] Change the net's file (add a job) and `koyomi approve` it, then `koyomi rerun` the old run | The rerun uses the old copy (`logs/<day>/<run>/definition.toml`): the added job is not in it, and `definition.toml` under the new run is the old text |
| 22.5 | [x] Change the net's file without approving, then `koyomi rerun` | Refused with `KOYOMI-E3741 ジョブネット … は確認されていないため、実行できません。` with the action `koyomi approve <group/name> で確認してから、再度実行してください。` |
| 22.6 | [x] `koyomi run <group/name> --from <second job>` | The jobs before it are ⊘ スキップ with the reason `<second job> から実行したため、スキップしました`, the run line reads `<second job> から実行`, and only the later logs exist |
| 22.7 | In the window, open a run that is over and open 「再実行 ▾」; press 「停止したところから」; then pick a job in the flow and open the menu again | One 「再実行 ▾」 button; 「停止したところから」 starts a new run that opens (selected in 今日); with a job picked the menu also has 「<job> から再実行」, 「<job> の次から（<job> はスキップ）」 and 「（<job> は警告終了）」, and without one 「選んだジョブから再実行」 is grey; the button is grey while the top warning is shown |
| 22.8 | [x] `koyomi rerun` with yesterday's run id (from `koyomi history`), and with an id that never was | The rerun starts for yesterday's business day (`--date` is the old run's date); the unknown id is refused with `KOYOMI-E3277 実行 <id> が存在しません。` |
| 22.9 | [x] `koyomi rerun <run> --next <first job>`, then `--next <first job> --warn`, then `--next <last job>` and `--warn` alone | The first job does not run (no log) and is ⊘ スキップ (then ⚠ 警告) with the reason `次のジョブから再実行したため、スキップにしました` (`…警告終了にしました`); the run line reads `再実行（<run> を <first job> の次から）` and the jobs after it run; `--next <last job>` with nothing else to run is refused with `KOYOMI-E3206 実行 … には、ジョブ <job> の後に実行するジョブがありません。`; `--warn` alone is refused with `--warn goes with --next` |

## 23. Hold, skip, pause (`hold`, `release`, `skip`, `pause`, `resume`)

| # | Do | Expect |
|---|---|---|
| 23.1 | [x] `koyomi hold <group/name>` on a confirmed net with a time a minute away, then `koyomi list`, the window's tree, and wait for the time | `ジョブネット <name> を保留しました（koyomi release <group/name> で解除できます）。`; the list and the tree show `⏸ <name>  保留`; when the time comes, 今日 and `koyomi status --today` have a ⏸ 保留 run with the reason `保留中です（koyomi release <group/name>）。`, no job ran, no log folder appeared, and no notification was sent; `koyomi engine status` has the line `保留中のジョブネット: <group/name>（koyomi release <group/name> で解除できます）。` |
| 23.2 | [x] While held, `koyomi run <group/name>` and the window's 今すぐ実行; then `koyomi hold` again | `KOYOMI-E3742 ジョブネット <group/name> は保留されているため、実行できません。`, the button is grey and the detail shows the ⏸ 保留中 line; the second hold is refused with `KOYOMI-E3723 ジョブネット <group/name> はすでに保留されています。` |
| 23.3 | [x] `koyomi release <group/name>` (and once more), then wait for the next time | `ジョブネット <group/name> の保留を解除しました。`, then `KOYOMI-E3725 ジョブネット <group/name> は保留されていません。`; the mark goes back to ✔ and the next time runs as usual |
| 23.4 | Run a net a → b → c where a sleeps 10 seconds; while a runs, `koyomi skip <run> b` (also the window: pick b in the flow and press 「b をスキップ」) | `<run> の b をスキップしました`; b is ⊘ スキップ with the reason `人の操作でスキップしました` and no log; c runs after a ends; the run ends ✔; `koyomi history` lists only the run line, b's skip is seen in `koyomi show`/`koyomi log` |
| 23.5 | [x] `koyomi skip <run> a` while a runs; `koyomi skip <run> c` after the run is over; `koyomi skip <run> x` | `KOYOMI-E3207 ジョブ a はすでに実行中のため、スキップできません。`, `KOYOMI-E3279 実行 <run> はすでに終了しています。`, `KOYOMI-E3201 ジョブ x が存在しません。`; the skip button is grey when the picked job runs or has ended |
| 23.6 | [x] `koyomi pause`, then `koyomi list`, `koyomi engine status`, the window, and wait for a time; then `koyomi run <group/name>` | `スケジュールを停止しました（koyomi resume で再開できます）。`; the list starts with `⏸ スケジュールを停止しています。koyomi resume で再開できます。`, engine status has the line `スケジュールを停止しています（koyomi resume で再開できます）。`, and the window shows the band 「⏸ スケジュールを停止しています。時刻が来ても実行しません…」 with the スケジュールを再開する button while the left panel's スケジュールを停止する button is gone; the time becomes a ⏸ 保留 run with the reason `スケジュールを停止しています（koyomi resume）。` and nothing runs; `koyomi run` still starts the net |
| 23.7 | [x] `koyomi pause` again, then `koyomi resume` (CLI or the band's button), then `koyomi resume` again | `KOYOMI-E3737 スケジュールはすでに停止しています。`; `スケジュールを再開しました。` and the band goes away; `KOYOMI-E3738 スケジュールは停止していません。` |
| 23.8 | [x] Hold a net and pause, then `koyomi engine stop` and `koyomi engine start`; look at `state_dir("koyomi")/held.toml` | The file has `paused = true` and `held = ["<group/name>"]`; after the restart the hold and the pause are still on (list shows both) |
| 23.9 | [x] Write `held = 1` into `held.toml` while the engine runs (or before `engine start`), then wait for a time | The warning band and `koyomi list` say `KOYOMI-E3705 held.toml を読み込めません。（…）読み込めるようになるまで、ジョブは実行されません。`, nothing runs, `koyomi hold`/`pause` are refused with the same line; correcting the file clears it at the next scan |
| 23.10 | [x] Queue a time behind a running run (`overlap = "queue"`), then `koyomi hold <group/name>` (or `koyomi pause`) before the run ends | The waiting time turns into a ⏸ 保留 run with the reason `待っている間に保留されました（koyomi release …）。` (`待っている間にスケジュールが停止されました（koyomi resume）。`), and `engine status` no longer lists it under 待ち |
| 23.11 | [x] Net a → b → c and a branch `side` beside them (all quick echoes). `koyomi hold <group/name> b`, then `koyomi hold <group/name> b` again and `koyomi hold <group/name> x`; `koyomi run <group/name>` | `ジョブネット <name> のジョブ b を保留しました。この位置に到達した回は待ちます（koyomi release <group/name> b で解除できます）。`; then `KOYOMI-E3724 ジョブネット <group/name> のジョブ b はすでに保留されています。` and `KOYOMI-E3722 ジョブネット <group/name> にジョブ x が存在しません。`; the run does a and side, then b is ⏸ 保留 with the reason `保留中（koyomi release <group/name> b）`, b has no log, c stays 待ち and the run is not over; `koyomi engine status` lists `保留中のジョブ: <group/name> の b（koyomi release <group/name> b で解除できます）。` |
| 23.12 | [x] While the run waits at b, `koyomi release <group/name> b` (then once more) | `ジョブネット <group/name> のジョブ b の保留を解除しました。`, then `KOYOMI-E3726 ジョブネット <group/name> のジョブ b は保留されていません。`; the same run goes on: b and c run and the run ends ✔ (no new run appears) |
| 23.13 | In the window: open the net, click b in the flow and press 「b を保留」; look at the tree; start the net, open the run and pick b; press 「ジョブ b の保留を解除する」 | The station b shows ⏸ with the line 「⏸ のジョブは保留中です…」 under the flow; the tree row has `⏸ ジョブを保留`; in the run detail b waits as ⏸ and the button reads 「ジョブ b の保留を解除する」; pressing it lets the run go on and the marks go away |
| 23.14 | [x] Hold a job `sub` that calls a child jobnet (`use`), then run the parent | The child's jobs do not start; `sub` stays 待ち (the 【子ジョブネット】sub row reads 待ち); inside the held `sub`, only `sub/x` shows ⏸ 保留 and `sub/y` reads ○ 待ち `sub/x の終了を待っています`; releasing `sub` runs the child and the parent goes on |
| 23.15 | [x] Look at `held.toml` after 23.11; `koyomi engine stop` and `start`; an old `held.toml` with only `held = ["<group/name>"]` | It has `[[jobs]]` with `net = "<group/name>"` and `job = "b"`; the hold is still on after the restart; the old file still reads (the net is held, no jobs) |
| 23.16 | Open an over run that ended ✔ with no job picked | The button row shows the small dim line 「ジョブを選ぶと、そのジョブから再実行します。」 after 比べる |

## 24. Calendar (`calendars/<name>.toml`, `calendar`, `business_day`, `on_closed`, `koyomi calendar`)

Set `KOYOMI_DATE` where a step needs a day that is not today. `app_dir("koyomi")` is the settings folder.

| # | Do | Expect |
|---|---|---|
| 24.1 | [x] Write `calendars/会社.toml` with `format = 1`, `holidays = "japan"`, `weekly_off = ["sat", "sun"]`, `closed = [{ date = 2026-11-20, name = "創立記念日" }]`, `open = [2026-11-28]`; then `koyomi calendar 会社 --month 2026-11` | The heading `会社 2026-11（営業日 19 日）`, a weekday header 月〜日, the weeks with `( 3)`, `( 7)( 8)`, `(20)`, `(23)` and `[28]`, then the lines `3 文化の日`, `20 創立記念日`, `23 勤労感謝の日`, `28 休日出勤`, `祝日の表: 2027-12-31 まで` |
| 24.2 | [x] `koyomi calendar 工場` (no such file), `koyomi calendar 会社 --month 2026-13` | `koyomi: KOYOMI-E4104 カレンダー 工場 は存在しません。` followed by its action (exit 1); `--month takes YYYY-MM` |
| 24.3 | [x] A net with `calendar = "会社"` and `[[when]] every = "month", business_day = 5, at = ["09:00"]`; `koyomi next 経理/<name>`, the window's detail | The next times are the 5th business day of each month (2026-11: 11-09, since 11-03 is 文化の日 and the weekend is off); the window shows the same rows under 次の 10 回 |
| 24.4 | [x] `business_day = -1`, then `day = 20, shift = -3` (same net), `koyomi next` | The last business day of each month (2026-11-30, 2026-12-31 since 12-31 is not in `closed`); 3 business days before the 20th (2026-11-17 since 11-20 is closed: 19→18→17) |
| 24.5 | [x] `calendar = "会社"`, `[[when]] every = "week" days = ["sat"] at = ["02:00"]` and `[[when]] every = "day" at = ["03:00"]`, `koyomi next`; then `on_closed = "prev"`, `"next"`, `"run"` on the first | `skip` is the default: no 02:00 at all, and Saturday, Sunday and 文化の日 have no 03:00; `prev` adds 02:00 on the Friday before (2026-11-06), `next` on the Monday after (2026-11-02, 11-09), `run` on the Saturday itself; a moved time that meets one already on that day runs once |
| 24.6 | `business_day = 5` in a net without `calendar` and no `_group.toml` naming one; `koyomi check <file>` and `koyomi list` | `KOYOMI-E3375 ジョブネットが business_day、shift、または on_closed を使用していますが、calendar が指定されていません。` with the action `ジョブネットまたは _group.toml に、calendar を指定してください。`; `koyomi list` shows `✖ <name>  定義の誤り` (not ✕ 無効) and the net does not run |
| 24.7 | Write `jobnets/経理/_group.toml` with `format = 1` and `calendar = "会社"`; a net of 経理 without `calendar`; `koyomi check` the net and the window's detail | The net counts by 会社 (its 次の 10 回 skip the weekend); `koyomi check jobnets/経理/_group.toml` says `_group.toml: 問題はありません。`; write `x = 1` into `_group.toml`: every net of 経理 shows `✖ … 定義の誤り` with `_group.toml: KOYOMI-E3370 _group.toml に、未知の項目 x があります。` |
| 24.8 | `calendar = "工場"` (no file), then write `calendars/工場.toml` with `holidays = 1` | `KOYOMI-E4104 カレンダー 工場 は存在しません。…（fk.toml）` (the ID first, the net's file name last), then for the bad file `注意: KOYOMI-E4110 カレンダー 工場 の holidays に、使用できない値が書かれています。…` in `koyomi list` and `koyomi check` (no `calendars/工場.toml:` in front); the window's band 「カレンダーの注意」 shows it too; nothing else stops. The engine rescans calendars every 10-15 s, so wait before looking at the window |
| 24.9 | [x] The system date at 2027-10-15 (fewer than 90 days left; `KOYOMI_DATE` is only passed to jobs and does not move the clock), `koyomi list`, the window, `koyomi check <a net on 会社>` (`koyomi check calendars/会社.toml` prints it only when the calendar has a table: 問題はありません。 otherwise) | The band and the `注意:` line read `KOYOMI-W4152 祝日の表は 2027-12-31 までの分です（あと 77 日）。…`; `koyomi check` prints the same line and exits 0 |
| 24.10 | [x] The system date at 2028-01-05 (`KOYOMI_DATE` does not move the clock), a net on 会社 with `every = "day"`, `at` a minute away; `koyomi check calendars/会社.toml`, `koyomi next`, wait for the time | The band reads `KOYOMI-W4153 祝日の表が古くなっています（2027-12-31 まで）。それより後の日は実行されません。…`; `koyomi check` prints it and exits 1; `koyomi next` marks each row with `KOYOMI-W4153 祝日の表が古くなっています（2027-12-31 まで）。それより後の日は実行されません。` (the short form, no action); when the time comes, 今日 has a 🔒 run with the reason `祝日の表の有効期間が過ぎています（2027-12-31 まで）。`, no job ran, and a notification was sent (verified with the fake clock, `UCHMK_FAKE_CLOCK`) |
| 24.11 | In the window, pick 会社 under カレンダー; use ◀ ▶; double-click a Saturday, a holiday, a weekday, and each again | The head reads `会社 ◀ 2026年11月 ▶ 営業日 19 日`; the weekly-off boxes run 日 to 土; the month table starts on Sunday, one framed cell per day: days off sit on a darker fill, holidays in the gold accent, closed days in magenta, `open` days in the done colour, each named day's name under its date in the cell (cut to 3 characters and … when longer; the full name on hover), today outlined in blue; a legend of those colours under it, then the names list; the double-clicks write `calendars/会社.toml` (`open = [...]` / `closed = [...]` appear and go), the engine reads it again and the table follows within a few seconds; `カレンダー calendars/会社.toml を書き込みました。` under the month |
| 24.11b | In 会社's month, look before clicking; click 11/7 (a Saturday), then a weekday, then a day of another month after ▶; right-click a weekday and choose この日を休業にする; on a picked day press the box's button; Tab to a day and press Enter | Beside the month a box shows today first: `この日に実行されるもの（<月>/<日> <曜>・営業日 <n>）` (n counts the business days of the month up to it), each approved net's start that day as `HH:MM  <name>` (nets on any calendar), or 「この日に始まるものはありません。」; a click moves the box and the outline (white; today stays blue, thicker when picked) to that day and writes nothing; a day off shows its name or 休み in the head; the menu item and the box's button (この日を休日出勤にする / この日を休業にする / この日を元に戻す) toggle the day as a double-click does; Enter on a day only picks it |
| 24.11a | Pick 会社 under カレンダー, go to a month with approved nets on it, press 「この月の予定を印刷…」 | `書き出し中…` at once (the window does not stop), then `印刷に送りました（PDF は <state>\exports\plan-<年>-<月>.pdf）` or, where the OS cannot send it, the PDF opens and `PDF を開きました（印刷はそこから行います）: …`; the PDF matches `koyomi plan <年>-<月> --pdf` for that month |
| 24.12 | Make `calendars/会社.toml` read-only, click a day | The engine's write error shows in the error colour; the table does not change |
| 24.13 | Pick a net that uses 会社 in the tree, then pick 会社 under カレンダー | The calendar view lists the net under 「このカレンダーで実行する定義」 and its next 10 times under it, counted by 会社 |
| 24.14 | [x] `koyomi approve` a net on 会社 with `every = "day"` | The week's count in the approval box and in the CLI skips the weekend (5 times, not 7) |
| 24.15 | With `calendars/` empty: look under the tree, open the カレンダー tab; then 「＋ 新しいカレンダー」 (also from the heading's right-click and the palette's 「新しいカレンダー…」); Enter with no name, then the name 会社 when it exists, then `a/b`; then the name 工場, untick 日本の祝日, tick only 日 and 作成する | The heading カレンダー and 「カレンダーはまだありません。」 show, and the tab opens the box; `KOYOMI-E4102 カレンダーの名前が指定されていません。`, `KOYOMI-E4101 カレンダー 会社 はすでに存在します。`, `KOYOMI-E4103 カレンダーの名前 a/b に、使用できない文字が含まれています。`; `calendars/工場.toml` holds `format = 1`, `holidays = "none"`, `weekly_off = ["sun"]`, the month view of 工場 opens when the engine answers, with `カレンダー calendars/工場.toml を書き込みました。`, and `koyomi calendar 工場` shows only Sundays off |
| 24.16 | Open 工場 in the カレンダー view; untick 日 and tick 土; then 名前の変更… (also from the row's right-click) with the name 工場, then 倉庫; with a net `calendar = "倉庫"`, 削除…, press Enter once, then 削除… again and 削除する | `calendars/工場.toml` says `weekly_off = ["sat"]` and the month follows; `KOYOMI-E4113 カレンダー 工場 の変更前と変更後の名前が同じです。`, then `カレンダーの名前を 工場 から 倉庫 に変更しました。` and the file is `倉庫.toml` with the same content; the delete box lists the net's file under `KOYOMI-W4114 calendar = "倉庫" と指定しているジョブネットは自動では修正されず、修正するまで無効になります。（…）` and starts on キャンセル, so Enter deletes nothing; 削除する reads `カレンダー 倉庫 を削除しました（以前の内容は versions/_calendars/倉庫 に残しました）。`, removes the file, keeps it as `versions/_calendars/倉庫/<日時>.toml`, and the net turns invalid until fixed |
| 24.17 | Open a net in the editor, its ジョブネット pane; open the カレンダー list and pick 「新しいカレンダー…」; name it 夜間 and 作成する; then 保存 | The list shows （なし）, the calendars, and 「新しいカレンダー…」 at the end; the box opens over the editor, which stays, and waits with its button off until the engine answers; `calendars/夜間.toml` is written, the editor's result line reads `カレンダー calendars/夜間.toml を書き込みました。`, the pane's カレンダー says 夜間 and the net is unsaved (Ctrl+Z takes it back); after 保存 the file says `calendar = "夜間"` |
| 24.18 | Pick 会社 under カレンダー; right-click a weekday, then a Saturday, then a closed day, and choose 名前を付ける… on each; type a name and press Enter (or 決定する); once with the field emptied. Then 祝日の表を取り込む…, paste the path of the Cabinet Office's `syukujitsu.csv` (with its quotes, as コピー as path gives it) and press Enter; once more with a CSV that has a bad date | The field under the month shows `<日> の名前` with the name the day has; a weekday and the Saturday go into `closed` under that name (still off), a closed day keeps its place with the new name, an emptied name leaves the day closed with none; the names list follows. The table is written to `calendars/syukujitsu.csv` byte for byte, `祝日の表を取り込みました（calendars/syukujitsu.csv、<first> 年から <last> 年まで）。` shows, the one there before is in `versions/_calendars/syukujitsu/<日時>.csv`, and the holiday line follows; the bad CSV says `KOYOMI-E4150 祝日の表の <n> 行目にある日付 <x> を読み込めません。` (no path in front) and nothing is written. Esc or キャンセル closes the field |
| 24.19 | Pick 会社 under カレンダー and click a weekday; at the right of the head press 日, then ◀ and ▶, then 週, then 月; close the window on 日 and open it again | The switch reads 日 \| 週 \| 月 in that order. 日 shows that one day as a timetable (its weekday, date and name on top, a row for each time something starts, or 「この日に始まるものはありません。」); the tag says what the day is (営業日 3, 文化の日, 休み), the head says `11/4(水)`, and ◀ ▶ move one day. 週 shows its seven days, Sunday first, and 月 the month. The window opens again on 日 |

## 25. Parts of the flow (lines `on` / `code`, branch, join, time, file, notify, child jobnet)

`koyomi check <file>` reads the lines and parts; the window's 流れ draws a job as a circle and a part as a square (`【分岐】` and the like before its name in `koyomi show` and the notifications).

| # | Do | Expect |
|---|---|---|
| 25.1 | A net: `a` (fails, exit 3), `b` with `after = [{ job = "a", on = "error" }]`, `c` with `after = ["a"]`; run it | `b` runs, `c` ends `· 未実行`（`c  先行の a が異常終了しました`）; the run is ✕ 異常（`a` failed）even though the error line led on; the thick line a→c stays in the border colour, the thin line a→b takes `a`'s colour |
| 25.2 | [x] `d` with `after = [{ job = "a", on = "any" }]` after a failing and after a passing `a` | `d` runs both times; `koyomi show` of the net lists `線: a (any)` under `d` |
| 25.3 | [x] `a` ends by `timeout = "1s"`, then by `koyomi kill`; `b` has `on = "error"`, `c` has `on = "ok"` | Both times `b` runs and `c` is 未実行 (`先行の a が異常終了`): 打ち切り and 強制終了 count as abnormal on a line |
| 25.4 | [x] `p` with `kind = "branch"`, `after = [{ job = "a", on = "any" }]` (a plain `after = ["a"]` follows only a 0 from `a`); `x` with `after = [{ job = "p", code = "0" }]`, `y` with `code = "1..3"`, `z` with `code = "else"`; `a` exits 0, 2, 9 in turn | Exactly one of x / y / z runs each time (0 → x, 2 → y, 9 → z); the others are 未実行 (`p の線の条件に一致しませんでした`); `p` reads `【分岐】p` and ends ○ with `a` 's code as its reason |
| 25.5 | [x] `j` with `kind = "join"`, `wait = "all"`, `after = ["a", "b"]` (a 1 s, b 5 s), `c` after `j`; then `wait = "any"` | `all`: `c` starts after `b` ends; `any`: `c` starts right after `a`, `b` keeps running and the run ends when `b` does |
| 25.6 | [x] `k` (a plain job) with `after = ["a", "b"]` | `k` waits for both, like `wait = "all"`: a job with 2 or more lines waits for all of them |
| 25.7 | [x] `t` with `kind = "time"`, `at` 2 minutes ahead, `after = ["a"]`, `c` after `t`; then `at = "00:00"` | `t` reads 実行中 `【時刻待ち】t` with the reason `HH:MM を待っています` and a dashed line; at the time it ends ✓ 正常終了 `HH:MM になりました` and `c` runs; `00:00`: it ends at once (✓ 正常終了) with `00:00 を過ぎていました` |
| 25.8 | [x] `t` with `days = 1`, `at = "00:00"`; `koyomi kill <run> t` | The reason reads `YYYY-MM-DD 00:00 を待っています` (tomorrow); kill ends it ✕ 強制終了, the run ends ✕ 異常, `c` is 実行なし |
| 25.9 | [x] `f` with `kind = "file"`, `file = 'C:\in\*.csv'`, `settle = "3s"`, `timeout = "10m"`, `c` after `f` running `cmd /c echo %KOYOMI_FILE%`; drop `note.txt`, then `Sales.csv` and keep writing it for 5 s | `f` waits (`C:\in\*.csv を待っています`), `note.txt` changes nothing; `Sales.csv` ends `f` 3 s after it stops changing: ○ `C:\in\Sales.csv が到着しました`; `c`'s log shows `C:\in\Sales.csv`; without `settle` it ends as soon as the file is there |
| 25.10 | [x] `f` with `timeout = "5s"` and no file | `f` ends ✕ 打ち切り（`打ち切り（5s を超えました）`）after 5 s, the run ends ✕ 異常 |
| 25.11 | `n` with `kind = "notify"`, `after = [{ job = "a", on = "error" }]`, and `to = "x"` on it; then without `to`; `a` fails | `koyomi check`: `KOYOMI-E3311 to が正しい配列になっていません。` with the action `to は to = ["accounting"] のように指定してください。`; without it the notification arrives as `✕ 異常  <title>: n` with the line of `a` and the run's id and day; `n` ends ○ `通知しました` at once; the run is ✕ 異常 (`a` failed) |
| 25.12 | [x] `n` after a passing `a` | The notification is a `done` one (title `✓ 正常終了  <title>: n`, state `done`) with the run's line |
| 25.13 | [x] `jobnets/共通/バックアップ.toml` (confirmed, jobs `x`, `y`); a net with `c` `kind = "jobnet"`, `use = "共通/バックアップ"`, `after = ["a"]`, `b` after `c`; run it | The run lists `a`, `c/x`, `c/y`, `【子ジョブネット】c`, `b` in that order; `c/x` and `c/y` run after `a`, `b` after them; `c` ends with the child's result; the child's own `[[when]]` is not used |
| 25.14 | [x] `use = "共通/無い"`; then make `バックアップ.toml` invalid (`x = 1`); then leave it unconfirmed (edit it) | `koyomi check` (and `koyomi list --toml`): `KOYOMI-E3376 子ジョブネット 共通/無い が存在しません。`, `KOYOMI-E3377 子ジョブネット 共通/バックアップ の定義に誤りがあります。`; `koyomi list` shows only `✖ 親  定義の誤り`; the parent is ✕ 無効 and does not run; unconfirmed: `koyomi run` says `KOYOMI-E3356 子ジョブネット 共通/バックアップ が確認されていません。` and nothing starts |
| 25.15 | [x] `バックアップ.toml` gets a `kind = "jobnet"` part with `use` = the parent | Both are ✕ 無効 with `KOYOMI-E3353 子ジョブネットの呼び出しが輪になっています。（共通/バックアップ → <parent> → 共通/バックアップ）` (the ids in the order they call, from the net it uses first: the parent's line starts at `共通/バックアップ`, `バックアップ`'s at `<parent>`) |
| 25.16 | [x] A part with no `after` (`kind = "join"`) and a job nothing follows that has `kind = "time"` | `KOYOMI-E3426 ジョブ j に after が指定されていません。` and `KOYOMI-E3427 ジョブ t の後に続くジョブがありません。`; `after = ["a", "b"]` with `b` after `a` after `b`: `KOYOMI-E3428 after が輪になっています。（a → b → a）` |
| 25.17 | [x] `koyomi show` / the approval box of a net with every kind of part | Each part is headed by its kind (`分岐 p`, `待ち合わせ j` …) with `線:` per line, `待ち方: 全部が終わってから`, `時刻: 06:00（1 日後）`, `待つファイル:`, `落ち着くまで:`, `使う定義: 共通/バックアップ`; a job's lines read `線: a (error)` |
| 25.18 | [x] `koyomi run` a net with 3 running jobs and `concurrency = 2`, one of them a `kind = "time"` part | The part waits without taking a slot: the two jobs run and the part is 実行中 at the same time |

## 26. Starts by events (`[[when]] file = …` + `settle`, `on_start = true`)

A `[[when]]` with `file = 'D:\受信\*.csv'` starts the net when such a file comes (`ito-watch`, and a look every 5 s besides); `settle = "10s"` waits until the size and change time have kept for that long. `on_start = true` starts the net when the engine starts (the PC's start or sign-in). Each is an event, not a time: `koyomi next` and the window list it as a row with no time.

| # | Do | Expect |
|---|---|---|
| 26.1 | [x] A net with `[[when]] file = 'D:\受信\*.csv'` and a job echoing `%KOYOMI_FILE%`; `koyomi approve` it; copy `sales.csv` into `D:\受信` | A run starts within a few seconds; `koyomi show` says `ファイルが来たとき: D:\受信\sales.csv`（the history reason is `ファイル D:\受信\sales.csv が到着しました。`）; the job's log is the path |
| 26.2 | [x] The same with `settle = "10s"`; write the file slowly (append every 3 s for 20 s, then stop) | No run while it is being written; a run starts about 10 s after the last write |
| 26.3 | [x] Copy `note.txt` into `D:\受信` | Nothing starts: only `*.csv` counts |
| 26.4 | [x] Two files are already in `D:\受信` when the net is approved (or the engine starts) | No run; the history has a ⊘ スキップ row `監視を開始した時点で、すでに 2 個のファイルがありました（D:\受信\*.csv）。実行しませんでした。`; a file copied in afterwards starts a run |
| 26.5 | [x] Copy `b.csv` in while the run for `a.csv` is still going | `b.csv` waits; its run starts right after the first run ends (one run per file, in name order) |
| 26.6 | [x] Overwrite `a.csv` (new size or time) after its run ended | A run starts again; a file left as it is does not start another, and the engine's restart does not run it again |
| 26.7 | [x] Delete `D:\受信` before the engine starts, then make it and copy a file in | `koyomi check` (or the window) says `待つファイルのフォルダーを監視できません（5 秒ごとに確認します）。`; the file still starts a run within about 5 s |
| 26.8 | [x] The net has `calendar = "会社"` and the file comes on a Sunday | No run; a ⊘ スキップ row `ファイル … が到着しました。 ただし、休業日のため、実行しませんでした（on_closed = "run" を指定すると実行できます）。`; with `on_closed = "run"` in the `[[when]]` the run starts |
| 26.9 | [x] The `[[when]]` has `except = [<today>]` or `start = <tomorrow>` | No run; a ⊘ スキップ row `ファイル … が到着しました。 ただし、except、または start / end の範囲外のため、実行しませんでした。` |
| 26.10 | [x] `koyomi hold` the net (or `koyomi pause`), then copy a file in | No run; a ⏸ 保留 row with the reason, as for a time; `koyomi release` / `resume` does not run it afterwards (the file counts as recorded) |
| 26.11 | [x] A net with `[[when]] on_start = true`; `koyomi approve`; `koyomi engine stop` and start the engine again (or sign in again) | A run starts once; `koyomi show` says `PC の起動・サインインのとき（エンジンの起動）`（the history reason is `起動したとき`）; the engine's running on does not start it again |
| 26.12 | [x] The same net, unapproved, when the engine starts; and with a calendar on a closed day | 🔒 `未確認のため、実行しませんでした（koyomi approve …）`; ⊘ スキップ `エンジンが起動しました。 ただし、休業日のため、実行しませんでした（on_closed = "run" を指定すると実行できます）。` |
| 26.13 | `koyomi next` and the net's 予定 in the window for a net with both events and `every = "day"` | The rows `ファイルが来たとき: D:\受信\*.csv（10s 変わらなくなってから）` and `PC の起動・サインインのとき（エンジンの起動）` come first, then the times; there is no `koyomi review` CLI: `koyomi next` is the CLI half, and the same two rows under 今週 are checked in the window |
| 26.14 | [x] `koyomi check` a `[[when]]` with `file = 'in\*.csv'`, with `file = 'D:\*\a.csv'`, with `file = …` and `on_closed = "prev"`, and with `on_start = false` | `KOYOMI-E3342 file が絶対パスではありません。`; `KOYOMI-E3345 file のフォルダーの部分に、* または ? が含まれています。`; `KOYOMI-E3333 file / on_start の [[when]] では、on_closed に "prev" と "next" を指定できません。`; `KOYOMI-E3338 on_start に true 以外の値が指定されています。` |

## 27. Links between jobnets (`[[when]] after_jobnet = …` + `on`)

A `[[when]]` with `after_jobnet = "経理/売上取込"` starts the net when a run of that net ends as `on` says: `on = "ok"` (the default: 正常終了 or 警告終了), `"error"` (異常終了, 打ち切り, 強制終了) or `"any"`. However the run before was started (a time, the window, `koyomi run`, `koyomi rerun`, a file, or another net before it), its business day is handed on as `KOYOMI_DATE`. A net starts this way at most once a business day; a second end on the same day is recorded as ⊘ スキップ and told of, so a person runs it by hand if it is wanted.

| # | Do | Expect |
|---|---|---|
| 27.1 | [x] Net `g/a` (one job echoing hi) and net `g/b` with `[[when]] after_jobnet = "g/a"` and a job echoing `%KOYOMI_DATE%`; `koyomi approve` both; `koyomi run g/a` | When `g/a` ends 正常終了, a run of `g/b` starts within a few seconds; its history row's reason is `g/a が終了しました（正常終了）。` (and `how = "after"` in the history file); its job's log is today's date |
| 27.2 | [x] `koyomi rerun` a run of `g/a` whose business day was 2 days ago | `g/b` starts again, with that run's business day (`KOYOMI_DATE` is 2 days ago, and the history row shows that date) |
| 27.3 | [x] `g/a`'s job made to fail (`exit 1`); `koyomi run g/a` with `g/b` on `on = "ok"` (default), then with `on = "error"`, then `on = "any"` | With the default, nothing starts (no row at all for `g/b`); with `"error"` and `"any"`, `g/b` starts with `g/a が終了しました（異常終了）。` |
| 27.4 | [x] `g/a`'s job ends 警告終了 (`warn_codes`), `g/b` on `on = "error"` and then on `on = "ok"` | `"error"`: nothing; `"ok"`: `g/b` starts (`g/a が終了しました（警告終了）。`) |
| 27.5 | [x] `koyomi run g/a` twice on the same business day (after 27.1) | The second time `g/b` does not start: a ⊘ スキップ row `g/a が終了しました（正常終了）。 ただし、同じ業務日（<date>）に、すでに実行されています（<run id>）。後続のジョブネットは実行しませんでした。必要な場合は、手動で実行してください（koyomi run g/b）。`, and a notification says so |
| 27.6 | [x] After 27.5, `koyomi run g/b` by hand (and from the window) | It runs: only the automatic start is once a day |
| 27.7 | [x] `g/b` given a job that sleeps 60 s; `koyomi run g/a` twice quickly (two different runs of `g/a` ending while `g/b` is still running, with the second `g/a` run `rerun` from another day) | The second end: ⊘ スキップ `… ただし、前の回がまだ実行中のため、実行しませんでした。`; nothing queues (the skip row's date is the day it was planned, i.e. today, not the second run's business day) |
| 27.8 | [x] `g/b` unapproved; `koyomi hold g/b`; `koyomi pause`; each time `koyomi run g/a` | 🔒 `未確認のため、実行しませんでした（koyomi approve …）`; ⏸ 保留 with the reason; ⏸ with the pause's reason — as for a time |
| 27.9 | [x] `g/b` with `calendar = "会社"`, and `koyomi rerun` a run of `g/a` whose business day is a Sunday | ⊘ スキップ `g/a が終了しました（正常終了）。 ただし、休業日のため、実行しませんでした（on_closed = "run" を指定すると実行できます）。`; with `on_closed = "run"` in the `[[when]]`, `g/b` runs with that Sunday as `KOYOMI_DATE` |
| 27.10 | `koyomi check` with `after_jobnet = "g/none"`; and with `g/a` after `g/b` and `g/b` after `g/a` | `KOYOMI-E3378 先行のジョブネット g/none が存在しません。`; `KOYOMI-E3379 after_jobnet の指定が輪になっています。（g/a → g/b → g/a）` on both (the window shows them as ✗ 誤り and does not run them); a net after `g/a` that is not itself in the loop is fine |
| 27.11 | [x] `koyomi check` a `[[when]]` with `after_jobnet = "売上取込"`; with `on = "sometimes"`; with `every = "day"` and `on = "ok"`; with `after_jobnet = …` and `on_closed = "prev"` | `KOYOMI-E3334 after_jobnet が グループ/名前 の形式ではありません。`; `KOYOMI-E3306 on の値が正しくありません。` with `on には、"ok", "error", "any" のいずれかを指定してください。`; `KOYOMI-E3332 day の [[when]] では、on を指定できません。`; `KOYOMI-E3333 after_jobnet の [[when]] では、on_closed に "prev" と "next" を指定できません。` |
| 27.12 | [x] `koyomi next`, the net's 予定 in the window, and `koyomi show` for `g/b` | A row with no time: `g/a が終わったとき（正常終了・警告終了のとき）` (`異常終了のとき` / `どのように終了しても` for the other `on`s) |
| 27.13 | [x] Three nets `g/a` → `g/b` → `g/c` (each after the one before); `koyomi rerun` a run of `g/a` from 3 days ago | `g/b` and then `g/c` run, each with `g/a`'s business day; the history has the three rows with that date |

## 28. History search and the rest of the CLI (`history`, `show`, `list --group`, `--help`, `--toml`)

The window's 履歴 (a row under 今日) and `koyomi history` show the runs that started on a range of days (the last 7 when none is given, at most a year; days the sweep put into `archive/` are read from there, 18.19), narrowed by jobnet and state on the asking side. Every command takes `--help` (its own lines of the usage, exit 0) and `--toml` (the answer as TOML for another program: the commands that answer with data print it as such, the rest print `ok = true|false` and the `lines` they would have said).

| # | Do | Expect |
|---|---|---|
| 28.1 | [x] Some runs over the last days (ok, error, skipped); `koyomi history` | The runs of the last 7 days up to today, oldest first, each as `status --today` prints it (the mark, the id, the title, the date, the times, the jobs under it) |
| 28.2 | [x] `koyomi history --from 2026-10-01 --to 2026-10-15`; `--from` alone; `--to` alone | Those days only; `--from` alone goes up to today; `--to` alone covers the 7 days up to it |
| 28.3 | [x] `koyomi history --status error`; `--status ok g/a`; `koyomi history g\a` | Only the runs of that state; only that net's; `\` reads as `/` |
| 28.4 | [x] `koyomi history --status nope`; `--from 2026/10/01`; `--from 2026-10-20 --to 2026-10-01`; `--from 2024-01-01` | `--status takes one of ok, warn, …` and the usage (exit 2); `--from takes YYYY-MM-DD` (exit 2); `koyomi: KOYOMI-E3272 範囲 2026-10-20 → 2026-10-01 は、終了日が開始日より前のため指定できません。` (exit 1); `koyomi: KOYOMI-E3273 範囲が 1 年を超えているため、指定できません。` (exit 1) |
| 28.5 | [x] `koyomi history --status held` with no held run in the days | `該当する実行はありません。` (exit 0) |
| 28.6 | [x] `koyomi history --toml`, `koyomi status --toml`, `koyomi list --toml`, `koyomi show g/a --toml` | The runs as `[[run]]` tables, each with its `[[run.jobs]]`; the snapshot (`[[defs]]`, `warnings`, `held`, `paused`, …); the definition as the engine read it (`id`, `file`, `standing`, `[net]`, `[[problems]]`, where `line = 0` is a problem of the whole file such as a missing child) |
| 28.7 | [x] `koyomi next g/a --toml`; `koyomi calendar 会社 --toml`; `koyomi log <run> <job> --toml` | `unconfirmed = …` and `[[next]]` with `at = "2026-11-02 09:00:00"` or `event = "…"`; `calendar`, `month`, `business_days` and a `[[day]]` per date with `kind = "business" / "open" / "closed" / "holiday" / "weekly_off"` and `name` when it has one; `run`, `job`, `text` |
| 28.8 | [x] `koyomi hold g/a --toml`; `koyomi engine status --toml` with the engine down; `koyomi check bad.toml --toml` | `ok = true` + `lines = ["…"]`; `ok = false` + `lines = ["koyomi: KOYOMI-E1067 エンジンが起動していません。"]` (exit 1); `ok = false` and the problems as `lines` (exit 1) |
| 28.9 | [x] `koyomi approve g/a --toml` on an unconfirmed net | The preview and `確認した場合は y を入力してください（y 以外を入力すると、未確認のままです）:` are still said as text (a person answers); after `y`, `ok = true` + `lines` |
| 28.10 | [x] `koyomi list --help`, `koyomi history -h`, `koyomi --toml list --help`, `koyomi pause --help` | Only that command's lines of the usage under `usage:`, then `every command takes --help …` (exit 0); `pause --help` prints the `pause \| resume` line |
| 28.11 | [x] `koyomi list --group 経理`; `koyomi list --group nothing`; `koyomi list --toml --group 経理` | Only that group's nets in the tree (the warnings and notes still above); `koyomi: KOYOMI-E3607 グループ nothing が存在しません。` (exit 1); the snapshot with only those defs |
| 28.12 | [x] `koyomi show g/a` (confirmed, unconfirmed, held, and ✗ 誤り) | The file and its standing (`⏸ ジョブネットを保留` when held, `未確認`, `定義の誤り` with the problem line under it), then what `approve` shows (the jobs, the times, the next week) without the question or the diff; a ✗ net prints its problems and nothing under them; exit 0 |
| 28.13 | The window: click the 履歴 tab | The right side shows 履歴: 開始日 (7 days ago), 終了日 (today), ジョブネット = すべて, 状態 = すべて, 検索; the rows of those days newest first (columns 状態 / ジョブネット / 日付 / 開始 / 終了; the duration is cut off at the default window width; coloured by state), the quick ranges 今日 / 過去7日 / 過去30日, 成績 and CSV 出力; the count above; the tree's and 今日's picks are off |
| 28.14 | [x] Change 開始日 to a date and press 検索; type `2026/10/1` and press 検索 | The rows of the new days; `日付は YYYY-MM-DD の形式で指定してください。` in red and the rows stay |
| 28.15 | Pick a net in ジョブネット, then a state in 状態 | The rows narrow at once (no 検索 needed); the count follows; `該当する実行はありません。` when nothing matches |
| 28.16 | [x] Click a row in 履歴 | Under the rows: the run's picture, its jobs' logs on click, 再実行 and the rest as under 今日; the row stays marked |
| 28.17 | [x] While 履歴 is open: click a net in the tree, a run in the 今日 table, or a calendar; then the 履歴 tab again | Each closes 履歴 and shows its own detail; 履歴 opens again on the last 7 days with nothing narrowed |
| 28.18 | Narrator / NVDA over 履歴 | The 履歴 row, the 検索 button and each run row are read (row: the date, the state, the title, the times) |
| 28.19 | [x] `koyomi --keys` report with 履歴 open | A `history: 2026-10-23..2026-10-30 * * 12` line (the days, the net or `*`, the state key or `*`, the rows shown) |
| 28.20 | A job whose reason has line breaks (a `[[when]] eventlog` net started by an event whose message has several lines, e.g. from PowerShell `eventcreate /L APPLICATION /SO MyApp /T ERROR /ID 100 /D "line 1$([char]10)line 2"`); `koyomi history`, `koyomi show <net>` and `koyomi run <net> --date <a week ago>..<today> --only missing --dry` | Each run and job is one row: the reason's line breaks read as spaces. `koyomi show <run> <job>`'s 理由 keeps them (BLOCKED: `eventcreate` needs administrator) |

## 29. Editor (`ui/canvas.rs`, `koyomi new` / `versions` / `restore`)

The 編集 button on a jobnet's detail opens the editor over the picture: a canvas with the parts, a shelf of the seven parts on the left, and a pane on the right for the part or the net that was double-clicked. The editor writes the whole file back from what it shows (comments in the file are lost; the previous text is kept under `versions/`), the engine checks the text before it is written, and a save from the editor counts as confirmed. Parts the person did not move get their place from 整列 and are not written.

| # | Do | Expect |
|---|---|---|
| 29.1 | Select a jobnet in the tree, click 編集 | The canvas replaces the picture: the parts where the file (or 整列) puts them, the shelf on the left, 保存 / 整列 / 元に戻す / やり直す / ジョブネット / ← at the top; the tree stays usable |
| 29.2 | A jobnet whose file does not read (a `問題` row) | 編集 is greyed out; nothing opens |
| 29.3 | Drag a part from the shelf onto the canvas; click a part in the shelf | The part appears where it was dropped with a new id (`job2`, `notify1` …) and is selected; a click puts it after the cursor's part and links it with 成功したら |
| 29.4 | Drag from the right port of a part to another part | A line (成功したら) from the first to the second; dropping on nothing or on the same part draws no line |
| 29.5 | Right-click a line | A menu: 成功したら / 失敗したら / どちらでも / 線を削除する; the choice changes or removes the line |
| 29.6 | Double-click a job | The right pane shows its fields (id, 名前, 実行, フォルダー, 制限時間, 再試行, シェル, 警告にする/エラーにする); a bad value (an empty id, `制限時間 = abc`, an id already used) turns red and is not applied; a good one applies at once |
| 29.7 | Double-click a branch, join, time, file or notify part | The pane shows that part's fields (the condition, the time, the path, the message); the same red rule |
| 29.8 | Click ジョブネット at the top | The pane shows the net's fields (名前, カレンダー, 逃した回, 重なったら) and its `[[when]]` list: each with a kind chooser and its own fields, 追加 / 削除, and 次の 10 回 below; a bad field keeps the last good list and shows the error in red |
| 29.9 | Drag on empty canvas; wheel; Ctrl+wheel; Shift+drag; Ctrl+click | Drag pans, the wheel scrolls, Ctrl+wheel zooms around the pointer, Shift+drag selects the parts in the box, Ctrl+click adds one to the selection |
| 29.10 | Keys: arrows, `a`, `c` + arrow + Enter, Enter, Delete | Arrows move the cursor to the nearest part that way; `a` adds a job after the cursor; `c` starts a line from the cursor, arrows pick the target, Enter draws it (Escape cancels); Enter opens the cursor's pane; Delete removes the selected parts and their lines |
| 29.11 | Ctrl+Z / Ctrl+Y / Ctrl+C / Ctrl+V / Ctrl+A | Undo and redo every change (a rename keeps the pane on the same part); copy and paste the selection (new ids, offset a little); select all |
| 29.12 | 整列, with two separate flows and two parts that have no line before or after them | Every part moves to where `layout::align` puts it (layers along the flow, rows across); each flow keeps its own rows, the second wholly below the first (no rows shared); the two loose parts stand in one row below the rest, side by side along the flow in the order they stood (not stacked in the first column), in the vertical flow too; undo brings the old places back |
| 29.13 | The flow button (横 / 縦) in the toolbar | The canvas and the pictures elsewhere switch between horizontal and vertical; the choice is kept in `state.toml` (`flow`) |
| 29.14 | Make a job with no `run` (or two jobs with one id), Ctrl+S | The problems show in red under the toolbar with the part named; 保存 is greyed out and Ctrl+S does nothing until they are fixed |
| 29.15 | Fix the problems, 保存 (Ctrl+S) | 保存した appears; the file is rewritten from the editor (comments gone), the old text is under `versions/<group>/<name>/<time>.toml`, and the net is confirmed (no 未確認 badge, no `approve` needed) |
| 29.16 | Edit the file by hand (or `koyomi restore`) while the editor is open, then 保存 | A box: ほかで変更されています, whose text reads `KOYOMI-E3103 開いたあとに、このファイルがほかの場所で変更されています。…`, with the differences between what the editor started from and the file now; 読み直す drops the editor's changes and loads the file, キャンセル keeps editing, 上書きする saves over it (the hand edit is kept as a version); Escape = キャンセル |
| 29.17 | With unsaved changes: ← , the tree, a run in the 今日 table, 履歴, or the palette's 閉じる | A box: 変更を破棄しますか; 破棄して閉じる leaves, キャンセル (the default, Escape) stays |
| 29.18 | Without changes: ← | The editor closes at once and the picture is back |
| 29.19 | Double-click a child jobnet part (or select it and Enter) | Its net opens in the editor on top; ← comes back to the parent (asking first when the child has changes) |
| 29.20 | Ctrl+Shift+P while editing | The palette lists 編集: 保存 / 整列 / 全体を見る / 並べる向きを変更する / 元に戻す / やり直す / 閉じる above the usual commands; they are gone when the editor is closed |
| 29.21 | [x] `koyomi new 経理/新規` | `jobnets/経理/新規.toml` from the template (one job, no `[[when]]`), the net appears unconfirmed; `new` on an existing file refuses |
| 29.22 | [x] `koyomi versions 経理/新規`; `koyomi restore 経理/新規 <time>` | The versions newest first; restore writes that version back (today's text becomes a version too) and the net is unconfirmed until `approve`. After a delete, `koyomi versions` prints only the version id; an edit made outside koyomi creates no version |
| 29.23 | Narrator / NVDA over the editor | The toolbar buttons, the shelf's parts, the pane's fields and the box buttons are read |
| 29.24 | `koyomi --keys` report with the editor open | An `editor: 経理/新規 dirty=false parts=1 selected=[] cursor=- pane=net problems=0 connect=false` line |
| 29.25 | Open a net that is not confirmed (edited by hand, or after `koyomi restore`) | A 未確認 badge beside the title, its hover saying the net does not run when its time comes and that 保存 confirms it; no badge on a confirmed or a new net, and none after 保存 |
| 29.26 | 検査 with nothing wrong; then name a calendar that is not there (`calendar = "ない"`) or a child jobnet that is not there, 検査 again | `検査: 問題なし` in green under the toolbar; then `検査: 修正が必要な箇所があります` in red with the same lines a save would refuse with (`KOYOMI-E4104 カレンダー … は存在しません。…（<group>/<name>.toml）`, the ID first and the file last); nothing is written and the net stays as it was |
| 29.27 | Look under the canvas | A legend: 成功したら ✓ (solid green), 失敗したら (dashed red), どちらでも *, 終了コード, それ以外 else, then each part's mark (↻ ▣ ◇ ⊕ 🕑 📄 ✉ 👤 and the chores); the same marks are inside the stations on the canvas (not 【) |
| 29.28 | Under the legend, with `[[when]]` every day at 09:00; then add a `quiet` span over one of them; then remove all `[[when]]` | この先 7 日の開始 with `7 回` and a strip of 7 days (dates at the dividers, a tick at each 09:00; hover a tick for its date and time); the quiet one moves or goes as `on_quiet` says; with no `[[when]]` it says the net runs only by hand |
| 29.29 | A net with many parts, zoomed in; look at the bottom-right corner of the canvas; click and drag there | A small picture of every part as a dot (red for a part with a problem, blue for the selected) and a blue frame for what is in view; a click or a drag brings that place to the middle; on a small canvas or with one part it is not drawn |
| 29.30 | Open the editor; look at the right; double-click a job; empty its 実行; press Escape; press › at the pane's top | The pane is there from the start with the net's fields (基本設定 / 実行方法 / いつ / 次の 10 回); each label has a red `*` when it must be filled or `(任意)` when it may be empty; the job's fields come in sections (基本設定 / 実行コマンド・スクリプト / タイムアウトと再試行 / 結果の判定 / 別のアカウントで実行する) with 保存 at the bottom right and このジョブを削除する at the bottom left; the empty 実行 turns red; Escape goes back to the net's fields; › folds the pane to a strip at the right edge and a click on the strip opens it again |
| 29.31 | Open a net that calls a child net; double-click the child to go into it; press 戻る at the right of the header | The child's canvas opens; 戻る (‹) is at the right of the header beside ジョブネット and takes you back to the parent, the same as 戻る at the left; at the top net there is no 戻る at the right |

## 30. Import, export and the daily backup (`koyomi export` / `import`, `backups/`)

| # | Do | Expect |
|---|---|---|
| 30.1 | [x] `koyomi export -o out.toml --all` with nets, a group file, a calendar and `koyomi.toml` that has `webhook = "secret:x"` | One TOML with `[[calendar]]`, `[[group]]`, `[[jobnet]]` (each file's text as it is, comments kept) and `[settings]` (`koyomi.toml` as a table: its comments are not kept) whose webhook addresses are `***` (a `secret:<name>` stays, since it is only a name); the count is printed |
| 30.2 | [x] `koyomi export -o out.toml` again; `-n 経理/月次締め` alone; `--group 経理`; `--calendars`; `--settings` | The second write is refused (`koyomi: KOYOMI-E3801 ファイル out.toml はすでに存在します。上書きしません。`); a net comes with its group file, calendar and child nets; a group brings its nets; the other flags pick only those |
| 30.3 | [x] `koyomi import out.toml --dry-run` on a PC where one net differs | The plan: `追加`/`上書き`/`同じ` per thing with a `+`/`-` diff under what differs; exit 1 and nothing written |
| 30.4 | [x] `koyomi import out.toml` with a difference and no `--on-conflict` | Refused with the three choices named; nothing written |
| 30.5 | [x] `koyomi import out.toml --on-conflict overwrite` | The net is replaced, its old file is under `versions/<group>/<name>/`, and it is `未確認` in the tree until approved; new nets come in unconfirmed too |
| 30.6 | [x] `--on-conflict rename` and `--on-conflict skip` | rename adds the different one under a new name beside the old; skip leaves it and brings in the rest; a bundle with a missing child net or a broken file is refused whatever the choice |
| 30.7 | [x] `koyomi import out.toml --toml` | The plan as TOML (`lines`, `conflicts`, `problems`, `undecided`, `writes`, `applied`) |
| 30.8 | Palette → 書き出す | Under the detail: `書き出しました: <state>/exports/<YYYYMMDD-HHMMSS>.toml（N 件…）` and the file is there |
| 30.9 | Palette → 取り込む…, type a wrong path, Enter | The box keeps the field with `KOYOMI-E3158 <path> を読み込めません。（…）` in red; Esc closes it |
| 30.10 | Type the path of a bundle whose nets differ, Enter | The plan box (the lines of 30.3, cut after 30 with `あと N 行`) with 上書きする / キャンセル / 名前を変更して追加 / 異なるものは飛ばす; Enter on the first frame = キャンセル, Tab moves, Esc = キャンセル |
| 30.11 | Pick 上書きする | `取り込みました。ジョブネットは未確認です。…` under the detail; the tree shows the nets `未確認`; old files are under `versions/` |
| 30.12 | Type a bundle with nothing different, Enter | 取り込む / キャンセル; a bundle that is all the same says `書き込むものはありません。（すべて同じ内容か、スキップされます）` with one 閉じる button |
| 30.13 | [x] Start the engine, wait, `koyomi engine status` | A `バックアップ: <time> に実行し、backups/<YYYY-MM-DD>.toml に N 件を書き込みました（7 日分を保持します）。` line; the file is a bundle `import` reads; an empty settings folder says `バックアップ: <time> に実行しましたが、バックアップする定義がないため、ファイルを書き込みませんでした。` and leaves the old day files |
| 30.14 | Narrator / NVDA over the 取り込む box | The field is read as ファイル, the buttons by their labels, the plan's lines as text |

## 31. A new jobnet from the window

| # | Do | Expect |
|---|---|---|
| 31.1 | Start `koyomi` with no definitions | The tree shows `＋ 新しいジョブネット` above `定義がありません…`; the message names the button |
| 31.2 | Press `＋ 新しいジョブネット` | The 新しいジョブネット box opens in the middle with グループ and 名前; the keyboard is in 名前; the window behind takes no clicks |
| 31.3 | [x] Press 作成する with a field empty; then type `a/b`, `..`, `a:b` or `締め.` as the name | `KOYOMI-E3151 グループまたは名前が指定されていません。` and `KOYOMI-E3152 グループまたは名前に、使用できない文字が含まれています。` in red; the box stays open |
| 31.4 | [x] Type a group and a name a net already has (try other upper/lower case) | `KOYOMI-E3150 ジョブネット <group/name> はすでに存在します。`; the box stays open |
| 31.5 | [x] Type `テスト` and `新規1`, Enter | The box closes; the editor opens titled `編集 テスト/新規1（新規・未保存） *` with one job `最初のジョブ`; no file is written yet |
| 31.6 | [x] Edit the job, Ctrl+S | `jobnets/テスト/新規1.toml` is written; the title loses `（新規・未保存）` and `*`; the tree shows the net under テスト as confirmed, not `未確認` |
| 31.7 | Right-click a group heading in the tree → このグループに新しいジョブネット | The box opens with グループ filled in; the グループ list (今あるグループ) picks another one |
| 31.8 | Ctrl+N, and palette → 新しいジョブネット… (the hint says Ctrl+N) | Both open the box; Ctrl+N does nothing while the editor is open; Tab moves between the fields, Esc closes the box |
| 31.9 | [x] Open the box while the editor has unsaved changes, fill it, Enter | `KOYOMI-E3154 編集中の変更が保存されていないため、ジョブネットを作成できません。…`; nothing is lost |
| 31.10 | [x] Make `テスト/新規2`, then before saving write `jobnets/テスト/新規2.toml` by hand, then Ctrl+S | `KOYOMI-E3105 ジョブネット テスト/新規2 は、ほかの場所で先に作成されています。…`; the hand-written file is unchanged; no overwrite button |
| 31.11 | [x] Make a net and press 閉じる without saving | The discard question comes first, with the keyboard on キャンセル (Enter keeps the editor); Tab, then Enter on 破棄して閉じる leaves no file |
| 31.12 | [x] `koyomi --keys "<Wait:2500><C-n><Wait:400>shime<Enter><Wait:300><State:b><S-Tab>keiri<Enter><Wait:500><State:c><Quit>"` | `b.txt` has `make: /shime (KOYOMI-E3151 グループまたは名前が指定されていません。…)`; `c.txt` has `editor: keiri/shime dirty=true parts=1 … new=true` (with no nets yet; once a group exists, its list sits between the two fields, so press `<S-Tab>` twice) |
| 31.13 | Narrator / NVDA over the box | The fields are read as グループ and 名前, the buttons by their labels; the error is read as text |


## 32. 日付の埋め込み

| # | Do | Expect |
|---|---|---|
| 32.1 | [x] A job `run = ["cmd.exe", "/c", "echo", "{date}", "{month-1}"]`; `koyomi run <net> --date 2026-01-15 --wait`, then `koyomi log` | The log has `2026-01-15 202512` |
| 32.2 | [x] A net with `calendar = "会社"` (土日 off) and `{date-1bd:%m%d}`; run it with `--date 2026-11-02` | The log has `1030` (the Friday) |
| 32.3 | [x] The same `{date-1bd}` in a net with no calendar; `koyomi run` | Refused: `KOYOMI-E3740 定義に誤りがあるため、ジョブネットを実行できません。（…）` (`koyomi check` says `KOYOMI-E3472 {date-1bd} は営業日で日付を移動しますが、calendar が指定されていません。`); no run starts |
| 32.4 | [x] Write `{date-1x}` in `run`; `koyomi check` | A problem with the line number: `KOYOMI-E3317 run の置き換えの指定を読み込めません。（{date-1x} の単位 x を読み込めません。）` |
| 32.5 | [x] A PowerShell `script` with `@{ a = 1 }` and `{ … }` | Runs as written; the braces are not touched |
| 32.6 | [x] `koyomi rerun` a run of an earlier day | The tokens are written for that run's business day, not today |
| 32.7 | [x] `koyomi run <net> --date 2026-11-02 --dry` | Each job prints as `id: command @ dir` with the dates written out; no run starts (`koyomi status` shows nothing new) |

## 33. 環境変数と秘密

| # | Do | Expect |
|---|---|---|
| 33.1 | [x] `echo hunter2 \| koyomi secret set db`, then `koyomi secret check db` | `シークレット db は存在します。`; the value is not shown anywhere |
| 33.2 | [x] A job `run = ["cmd.exe", "/c", "echo", "%DB_PASS%"]` with `env = { DB_PASS = "secret:db" }`; run it, then `koyomi log` | The log has `***`, not `hunter2` |
| 33.3 | [x] The same job after `koyomi secret delete db` | The job fails with `KOYOMI-E3223 シークレット db が存在しません。` with the action `koyomi secret set db で保存してください。` |
| 33.4 | [x] `env = { OUT = "{month-1}" }` and a job that echoes `%OUT%` | The log has last month as `YYYYMM` |
| 33.5 | [x] `env = { KOYOMI_X = "1" }` or `env = { 1A = "x" }`; `koyomi check` | A problem with the line number |
| 33.6 | [x] Open the review of a job with `env` | `環境変数 DB_PASS: secret:db` is listed; no secret value |

## 34. エンジンが落ちていたことの通知

| # | Do | Expect |
|---|---|---|
| 34.1 | Start the engine, kill its process (`taskkill /f`), wait 2 minutes, start it again | A notification `エンジンが異常終了していました`, and a band at the top of the window with the span |
| 34.2 | [x] `koyomi engine stop`, wait 11 minutes, start it again | `エンジンが停止していました` is told the same way |
| 34.3 | [x] `koyomi engine stop` and start it again within a minute | Nothing is told |
| 34.4 | The band an hour after the start | It is gone |

## 35. 実行前の点検

| # | Do | Expect |
|---|---|---|
| 35.1 | [x] A job `run = ["C:/nope/x.exe"]`; `koyomi check` the file | `KOYOMI-E3181 ジョブ <job> の実行ファイル C:/nope/x.exe が見つかりません。` and exit code 1 |
| 35.2 | [x] A job with `dir = "C:/nope"` | `KOYOMI-E3182 ジョブ … の作業フォルダー C:/nope が存在しません。` |
| 35.3 | [x] A `kind = "file"` part watching `C:/nope/*.csv` | `KOYOMI-E3183 ジョブ … が監視するフォルダー C:\nope が存在しません。`（Windows ではパスを `\` 区切りで表示する） |
| 35.4 | [x] `run = ["cmd.exe", ...]` with `dir = "C:/work/{month}"` | No complaint about the dir (it has no name until the day) |
| 35.5 | A net whose calendar uses the holiday table, within 60 days of its end | A line about the table; exit code stays 0 |

## 36. 失敗の対応済み

| # | Do | Expect |
|---|---|---|
| 36.1 | Let two jobs fail; look at the status bar | Red `未対応の失敗 2` appears left of the version |
| 36.2 | Open one failure from the bell | The count drops to 1 |
| 36.3 | Press "すべて既読にする" in the bell | The count disappears |
| 36.4 | Restart the app | The acknowledged failures stay out of the count (`acked.toml` in the state folder) |

## 37. ジョブの詳細

| # | Do | Expect |
|---|---|---|
| 37.1 | Open a finished run; click a job that failed | A table above the log shows state, start, end, took, 終了コード, リトライ, コマンド |
| 37.2 | Click a job that retried | リトライ shows `回数 / 上限` |
| 37.3 | Click a branch or join part | No コマンド row |

## 38. ログの検索

| # | Do | Expect |
|---|---|---|
| 38.1 | Open a run with a log; type `error` in the box above the log | Only the lines with error/ERROR stay; `N / M 行が一致` shows |
| 38.2 | Type `ERROR` | Only upper-case ERROR lines stay (smart case) |
| 38.3 | Type `(` | A red regex error; the whole log stays |
| 38.4 | Clear the box | The whole log is back |

## 39. 先行を待つ時刻

| # | Do | Expect |
|---|---|---|
| 39.1 | [x] Net B: `[[when]] every="day" at=["HH:MM"] wait_for="g/A"`; A runs daily and is still running at HH:MM | B does not start at HH:MM; it starts right after A ends, with the same business day |
| 39.2 | [x] Add `wait_until` a few minutes after HH:MM; keep A running past it | B is recorded as 異常 with the reason (wait_until), and a notification comes |
| 39.3 | [x] A is not scheduled that day (e.g. `days` excludes today) | B starts at HH:MM without waiting |
| 39.4 | In the editor open B's `[[when]]` | Fields 先行の終わりを待つ / 待つのは何時まで show and save to the file |

## 40. ステータスバー

| # | Do | Expect |
|---|---|---|
| 40.1 | Open the window with the engine running | The bar shows `次の実行 MM-DD HH:MM` (the soonest time of any net not on hold) |
| 40.2 | Start a run | `実行中 N` appears in green while it runs and goes when it ends |
| 40.3 | `koyomi pause` | `スケジュール停止中` shows in red; `koyomi resume` clears it |
| 40.4 | Stop the engine while the window is open | The bar says the engine is not reachable / starting |
| 40.5 | §102's two engines (102.1); open a window from each console (`koyomi`) | Within 30 s, after the engine word: B `待機中: a を監視しています` in the colour of 未承認; A `b が待機しています` in grey. Your usual profile (neither section) shows no such words |
| 40.6 | Close A's window (it would start A's engine again); do 102.4 and watch B's window; open A's window again after 102.7 starts A | B: `a から引き継いで実行しています（YYYY-MM-DD HH:MM から）` in green. A, until B gives them back: `b が引き継いでいます` (or `b に確認しています`) in the colour of 未承認. Then, within 30 s: B `待機中: a を監視しています（YYYY-MM-DD HH:MM に戻しました）`, A `b が待機しています`. While an engine is down or starting, its window shows none of these |

## 41. 出力とファイルでの判定

| # | Do | Expect |
|---|---|---|
| 41.1 | [x] A job that prints `ERROR x` and exits 0, with `judge = { error_output = "^error" }` | The job ends 異常, reason `出力に異常の行がありました: ERROR x`; it is retried if `retry` is set |
| 41.2 | [x] A job that exits 3 and leaves `C:/out/{date}.csv`, with `judge = { ok_file = "C:/out/{date}.csv" }` | The job ends 正常; the log says the file made it normal |
| 41.3 | [x] Same job with the file missing | The job ends 異常 (exit 3) |
| 41.4 | In the editor open the job | Fields 出力にあれば失敗 / あれば成功にするファイル show and save; a bad regex is refused on save |

## 42. 時刻を 1 回だけずらす

| # | Do | Expect |
|---|---|---|
| 42.1 | [x] `koyomi shift g/a 09:00 09:30` (a net with a time at 09:00 later today) | It answers that the time was moved; at 09:00 nothing starts; at 09:30 the net runs once with planned time 09:00 |
| 42.2 | The next day | The net runs at 09:00 as the definition says (the file was not changed) |
| 42.3 | [x] `koyomi shift g/a 09:00 09:30` then `koyomi shift g/a 09:00 --cancel` | The move is taken back; 09:00 runs as usual |
| 42.4 | [x] A time the net does not have, or a new time before the old one or already past | It is refused with the reason |
| 42.5 | [x] `koyomi skip-next g/a` (an approved net with a time later today) | It answers which time it skips; `held.toml` has a `[[skip]]` with it; at that time nothing starts and the history shows the time as スキップ「次の 1 回だけスキップ」; the time after runs as usual |
| 42.6 | [x] `koyomi skip-next g/a` twice, then `koyomi skip-next g/a --cancel` | The second skips the time after the first; `--cancel` takes both back and the times run; `--cancel` with nothing skipped is refused |
| 42.7 | `koyomi rename g/a g/b` with a time skipped | The skip moves to g/b and still holds |

## 43. 操作の記録

| # | Do | Expect |
|---|---|---|
| 43.1 | [x] `koyomi hold g/a`, `koyomi release g/a`, then `koyomi ops` | Two lines with the time, `hold` / `release` and `g/a`, oldest first |
| 43.2 | Run a net by hand, save one in the editor, then `koyomi ops` | `run` and `save` lines appear |
| 43.3 | [x] A refused operation (hold a net that is already held) | No line is written |
| 43.4 | [x] `koyomi ops --from <yesterday>` after restarting the engine | The earlier lines are still there; `koyomi history` is unchanged |

## 44. 使っている所を探す

| # | Do | Expect |
|---|---|---|
| 44.1 | [x] `koyomi uses <calendar name>` | Lines `calendar   g/x` for each net that counts by it |
| 44.2 | [x] `koyomi uses g/a` where g/b has a child-net job `use = "g/a"` and `after_jobnet = "g/a"` | `child net  g/b (job …)` and `follows    g/b (on Ok)` |
| 44.3 | [x] `koyomi uses robocopy` (a job runs `C:/tools/Robocopy.exe`) | `program    g/x (job …)`; the folder and `.exe` do not matter |
| 44.4 | [x] `koyomi uses nothing-here` | `(not used)` |

## 45. 初めての起動の案内

| # | Do | Expect |
|---|---|---|
| 45.1 | Start the window with an empty jobnets folder | A panel "First steps" with three commands appears under the title bar |
| 45.2 | Click a command | It is on the clipboard; the panel stays |
| 45.3 | Press Esc, then Ctrl+Shift+P > "First steps" | Esc closes it; the palette opens it again |
| 45.4 | Start the window with at least one job net | No panel; the palette entry opens nothing |

## 46. 用語のやさしい言い方

| # | Do | Expect |
|---|---|---|
| 46.1 | Set `easy = true` in common.toml | The tree's new button reads "+ New job flow (jobnet)" with a dotted underline; hovering explains it |
| 46.2 | Select an unapproved net | The approve button reads "check and allow (approve)…" with the same hover |
| 46.3 | Set `easy = false` | The usual words come back, no underline |

## 47. 版を比べる

| # | Do | Expect |
|---|---|---|
| 47.1 | Save a net twice, open it in the editor, pick "Compare versions of a job flow" in the palette | A box shows the newest kept version against the file as it is, as a diff |
| 47.2 | Press "Older" (no older one: nothing happens), then "Newer" | The pair moves by one version; the title line shows `from → to (n/m)` |
| 47.3 | Press "Restore this version" | The box closes; the file is that version, the net is unapproved, and the file it replaced is kept under `versions/` |
| 47.4 | Press Esc | The box closes without changing the file |
| 47.5 | Open it on a net that was never saved twice | The box says "no versions kept" |

## 48. エディターの補い

| # | Do | Expect |
|---|---|---|
| 48.1 | Spread parts wide, zoom in, pick "Edit: show everything" in the palette (or the toolbar button) | Every part is in view at once |
| 48.2 | Break a part (e.g. a missing `run`), pan far away, click its row in the problem list | The part is selected, its fields open and it is brought to the middle |
| 48.3 | Type a two-line sticky note in a part's fields | The note shows in a box under the part's name; saving and reopening keeps it |
| 48.4 | At 100% zoom, give parts notes of 3 or 4 lines, press 整列 in the horizontal flow, then switch to the vertical flow and press 整列 again | No note box or name overlaps another part, its name or its note, in both flows; 元に戻す takes back the last 整列 |
| 48.5 | In a net with no notes and short names, press 整列 | The parts stand 150 apart along the flow and 52 across it, as before |
| 48.6 | Open a net whose leftmost part has a long note (and a long name) | The whole note and name are inside the canvas's left edge on the first view; the `pos` values in the file are unchanged and the editor shows no unsaved change |
| 48.7 | On that net, use 全体を見る | Every note is fully in view |

## 49. 回復の通知

| # | Do | Expect |
|---|---|---|
| 49.1 | [x] Run a net whose job exits 3, then change it to exit 0 and run it again | The second end shows "Back to normal" naming the abnormal run, after the usual ok end |
| 49.2 | [x] Run it ok once more | No recovery notice (the run before it was ok) |

## 50. ジョブの間で値を渡す

| # | Do | Expect |
|---|---|---|
| 50.1 | [x] Job `a` writes `rows = 5` to the file named by `KOYOMI_OUT`; job `b` (after `a`) prints the file named by `KOYOMI_IN` | b's log shows `[a]` and `rows = 5` |
| 50.2 | [x] Make `a` write text that is not TOML | b's `KOYOMI_IN` has no `[a]` table; the run still goes on |

## 51. 排他（lock）

| # | Do | Expect |
|---|---|---|
| 51.1 | [x] Give two nets `lock = "db"`; start the first (a slow job), then start the second by hand | The second is refused with a message naming the lock and the net that holds it |
| 51.2 | [x] Start a net without `lock` meanwhile | It starts normally |
| 51.3 | [x] Wait for the first to end, start the second again | It starts |
| 51.4 | [x] Give two nets `lock = { name = "db", shared = true }` and one `lock = "db"`; start both shared nets (slow jobs), then the plain one by hand | Both shared nets run together; the plain one is refused with 「排他ロック「db」は、… が共有で使用しています。」 |
| 51.5 | [x] While the plain `lock = "db"` net runs, start a shared one by hand | Refused with 「共有ロック「db」は、… が排他で使用しています。」 |
| 51.6 | [x] Put `[locks]` `"api" = 2` in koyomi.toml; give three nets `lock = "api"` (slow jobs) and start them one after another | The first two run together; the third is refused with 「排他ロック「api」は、同時に 2 本まで使用できます。… が使用しています。」 |
| 51.7 | [x] Give the third of 51.6 a schedule a minute ahead while the other two still run | At its time the day shows it 🔒 停止, with the same reason |
| 51.8 | [x] Write `"api" = 0` (or `"a/b" = 2`) under `[locks]` | The engine shows `KOYOMI-E1040 [locks] の "api" の数が正しくありません。…` (or, for the name, `KOYOMI-E1039 [locks] の a/b が、ロック名として使用できません。…`) |
| 51.9 | Save a net with `lock = { name = "db", shared = true }` from the editor | The file still has the same `lock` line |

## 52. 人の確認待ち

| # | Do | Expect |
|---|---|---|
| 52.1 | [x] Add an "approval" part (shelf) between two jobs and start the net | The run waits at the approval part and a notification arrives |
| 52.2 | [x] Press "承認する" on the run's toolbar | The part ends 正常 and the next job runs |
| 52.3 | [x] Start again and press "却下する" (or `koyomi approve-step <run> <job> --no`) | The part ends 異常, the run fails and the next job does not run |

## 53. 期間をまとめて流す

| # | Do | Expect |
|---|---|---|
| 53.1 | [x] `koyomi run <net> --date 2026-05-01..2026-05-03 --dry` | The commands for each of the three days are printed |
| 53.2 | [x] Run it without `--dry` | The days run one after another, each started after the last ended |
| 53.3 | [x] Make one day fail, then run the range | It stops at that day with a non-zero exit |
| 53.4 | [x] `--date 2026-05-03..2026-05-01` | Refused: the range ends before it starts |
| 53.5 | [x] For a net that runs daily, with runs on some days of the last week and one of them ended 異常: (a run cut off by stopping the engine counts as 異常終了, so its day shows 流さない 異常終了) `koyomi run <net> --date <a week ago>..<today> --only missing --dry` | Each day is listed: 流す for the days with no run (実行の記録がない, or only a 時刻を過ぎた/保留/未確認で止めた record, or a スキップ whose reason is 待っている間にエンジンが止まった: stop the engine while a time waits for the run before it), 流さない with the record's state and id for a day whose last record is any other スキップ and for the others; nothing starts |
| 53.6 | [x] The same with `--only failed --dry` | The day that ended 異常 is now 流す too; a day off the net's calendar is 流さない 予定のない日 |
| 53.7 | [x] Run `--only failed` without `--dry` | One line says how many days run and which, then only those days run in turn; with none to run it says 流す日はない and exits 0 |
| 53.8 | [x] `koyomi run <net> --only missing` (no range) | Refused: `--only` goes with `--date A..B` |
| 53.9 | [x] With `keep_days = 7`, `koyomi run <net> --date <30 days ago>..<today> --only missing --dry` | The days older than 7 days with no record are 流さない 記録が残っていない（keep_days より前）, and one line says 記録が残っていない日は流さない。範囲の日を全部流すなら --only を外す; with `archive = true` and those days in `archive/`, their runs are read from the zip and the days are listed as in 53.5 |
| 53.10 | [x] On a standby PC (`[standby]`, not taken over): `koyomi run <net> --date A..B --only missing` | Refused: 待機中の PC では --only を使えない（流しているのは <primary> で、実行の記録もそちらにある）。<primary> で流す; nothing starts. After it has taken over, it picks the days as in 53.5 |

## 54. 平均の所要時間と終了の予想

| # | Do | Expect |
|---|---|---|
| 54.1 | [x] Run a net that takes a minute a few times, then start it once more and run `koyomi status` | Under the run: "いつもは …、HH:MM:SS ごろに終わる見込み" |
| 54.2 | [x] Run `koyomi status` after the run has gone past the average | Only "いつもは …" shows, with no expected time |
| 54.3 | [x] Run `koyomi status` for a net that never passed | No average line |

## 55. MCP（Claude Code から）

| # | Do | Expect |
|---|---|---|
| 55.1 | [x] `claude mcp add koyomi -- koyomi mcp`, start the engine, then ask Claude Code for today's runs | `koyomi_status` answers without asking |
| 55.2 | [x] Ask it to run a net | Claude Code asks first; the first call only lists the commands, and nothing starts until it is called again with `confirm = true` |
| 55.3 | [x] Ask it to stop a running run | Same: nothing stops without `confirm = true` |
| 55.4 | Stop the engine and ask for the runs | `koyomi mcp` starts an engine by itself (client::call), so the tool answers with the runs and `isError:false`; with the engine unable to start (its address unreachable) it answers `isError:true` saying the engine is not running |

## 56. ガント（今日のチャート）

| # | Do | Expect |
|---|---|---|
| 56.1 | Run two nets today, then click the 今日 tab | A lane per run on an hour axis; the bar spans start to end, in the state's colour |
| 56.2 | While a net is running | Its bar keeps growing to now |
| 56.3 | Click a lane | That run opens under the chart |
| 56.4 | With no runs today | 「今日の実行はありません」 |
| 56.5 | Confirm a net that starts later today; on the 今日 tab double-click its 待ち row | The net page opens on that net |
| 56.6 | Then look at the right of the tab row and press 戻る (hover it first) | 戻る sits left of the date; its hover says 今日に戻る; the 今日 tab comes back with the 待ち row selected and in view; 戻る is gone. Going on through links (a run, a net, a calendar from the right pane) and pressing 戻る walks back one link at a time; clicking a tab or the tree forgets the way back |

## 57. 月の予定表（plan）

| # | Do | Expect |
|---|---|---|
| 57.1 | [x] `koyomi plan` with the engine running | Each day of this month with the times approved nets start; days with none show `-` |
| 57.2 | [x] `koyomi plan 2026-02 --pdf out.pdf`, open the PDF | A4 pages titled with the month, text selectable, page numbers at the foot |
| 57.3 | `koyomi plan --print` | The PDF path is printed, and the PDF goes to the printer (or opens in the viewer when it has no print verb) |
| 57.4 | [x] A net on a calendar that closes a day | That day shows no start for the net |

## 58. ネット間のつながり（map）

| # | Do | Expect |
|---|---|---|
| 58.1 | [x] `koyomi map` with a net that follows another (`after_jobnet`) and one that uses a child net | One line each: `g/a -> g/b (after ok)`, `g/a -> g/b (child)` |
| 58.2 | `koyomi map --dot \| dot -Tsvg > map.svg`, open it | The nets as boxes, arrows from the first to the dependent, labelled |
| 58.3 | `koyomi map --mermaid`, paste into a Mermaid viewer | A left-to-right flowchart of the same links |
| 58.4 | [x] With no links | `(no links)` |


## 59. 出力の文字コード（encoding）

| # | Do | Expect |
|---|---|---|
| 59.1 | [x] A job that prints a Shift_JIS file (`run = ["cmd.exe", "/c", "type", 'sjis.txt']`) with `encoding = "sjis"`, run it | The log shows the Japanese text, not mojibake |
| 59.2 | [x] The same job with `encoding = "utf8"` | The log does not show the Japanese text (the bytes are not UTF-8) |
| 59.3 | [x] No `encoding` (auto), a program that prints UTF-8, then one that prints in the console code page | Both read right |
| 59.4 | [x] `encoding = "cp932"` | `KOYOMI-E3306 encoding の値が正しくありません。encoding には、"auto", "sjis", "utf8" のいずれかを指定してください。` on that line |
| 59.5 | Open the job in the editor, pick Shift_JIS under 出力の文字コード, save | The file gets `encoding = "sjis"`; the review shows 出力の文字コード: Shift_JIS; picking 自動 removes the line |

## 60. エディターの部品の棚

| # | Do | Expect |
|---|---|---|
| 60.1 | Open a jobnet in the editor | Every shelf button has a name, the first one 【ジョブ】 |
| 60.2 | Drag the shelf's right border | The shelf gets wider or narrower (between about 90 and 320 px); the canvas follows |
| 60.3 | Click a job on the canvas | The pane's heading reads 【ジョブ】 |

## 61. 実行中は PC を眠らせない

| # | Do | Expect |
|---|---|---|
| 61.1 | Run a net with a job that takes a few minutes, then `powercfg /requests` in an administrator prompt | Under SYSTEM: `koyomi: ジョブを実行中です` |
| 61.2 | After the run ends, `powercfg /requests` again | The koyomi line is gone |
| 61.3 | Set sleep to 1 minute on a laptop, run a job of 5 minutes without touching the PC | The PC stays awake until the job ends, then sleeps after a minute |
| 61.4 | A net that only waits (a time part, an approval) | No hold: the PC may sleep |

## 62. 眠っている PC を前もって見せる・PC を起こして実行する

| # | Do | Expect |
|---|---|---|
| 62.1 | Set sleep to 30 minutes, open a net whose next times are tonight | Times more than 30 minutes away read `この PC は 30 分で眠る設定です。起こさなければ実行されません。`; nearer ones have no mark |
| 62.2 | Same, under 今日 on the left | The coming times today carry the same mark |
| 62.3 | Set sleep to never | No mark anywhere |
| 62.4 | In the editor's net pane, check 「PC がスリープ中でも復帰させて実行する」 and save | The file has `wake = true`; the next times read `PC を起こして実行する` |
| 62.5 | [x] `koyomi engine status` | `PC を起こします: <時刻の 2 分前>（<ネット>）` (`koyomi status` lists only the running runs) |
| 62.6 | `powercfg /waketimers` in an administrator prompt | A timer by koyomi's engine for that time |
| 62.7 | Put the PC to sleep before the time | It wakes 2 minutes before, the net runs, and the PC sleeps again afterwards |
| 62.8 | Turn wake timers off in the power plan (Advanced settings → Sleep → Allow wake timers: Disable), wait a minute | The marks read `PC を起こして実行する設定ですが、起こせません: 電源プランで復帰タイマーが無効です`; `koyomi engine status` shows `PC を起こせません: 電源プランで復帰タイマーが無効です` |
| 62.9 | On a laptop on battery with wake timers allowed only when plugged in | The reason reads バッテリー… |

## 63. タスクトレイの常駐アイコン

| # | Do | Expect |
|---|---|---|
| 63.1 | Start koyomi, close the window | The koyomi calendar icon stays in the task tray with a green ✔ badge; hovering reads `✔ koyomi: 次 HH:MM` (or `異常なし`); the right-click menu starts with the same words greyed |
| 63.2 | Run a net whose job takes a minute | The badge turns blue … and the tip reads `実行中 1` |
| 63.3 | Run a net that fails | The badge turns red ✖ and the tip reads `異常 1` (every failed run of today not marked 対応済み counts, the same number as the window's 未対応 chip and 未対応を見る (N)); a later 正常 run does not clear it; marking the run 対応済み in the window (within a few seconds) turns it green again |
| 63.4 | Left-click the icon while the window is closed | The window opens |
| 63.5 | Minimise the window, then left-click the icon | The window comes back to the front |
| 63.6 | Right-click → 今日の一覧 | The window comes forward with ガント in the detail (unless a net is open in the editor) |
| 63.6a | Right-click with nothing failed today; then run two nets that fail, mark one run 対応済み in the window, close the window, right-click → 未対応を見る (2); do it again after marking the other | 未対応を見る (0) is greyed at first; after the failures it reads 未対応を見る (2) and the window comes forward on the newest failed run not marked 対応済み (its picture and log in the detail); with every one marked it opens ガント instead. With a net open in the editor the window only comes forward. `koyomi tray` (with `engine --no-tray`) shows the same item and count |
| 63.6b | Right-click → 次の 1 回だけスキップ… | The window comes forward with the 次の 1 回だけスキップ box: each approved net on a schedule with its next time. 飛ばす fills 飛ばす実行 with that time and moves 次の実行 to the one after; 戻す clears it; Esc or 閉じる closes the box. A held or unapproved net is not listed. With a net open in the editor the window only comes forward |
| 63.6c | Run six nets that fail, then right-click | Under 未対応を見る (6) the menu lists the five newest as `✕ <net title> HH:MM`, then `ほか 1 件…`. Choosing a `✕` line brings the window forward on that run (its picture and log in the detail); `ほか 1 件…` does what 未対応を見る does. Mark a run 対応済み and it leaves the list within a few seconds. `koyomi tray` (with `engine --no-tray`) lists the same |
| 63.7 | Right-click → スケジュールを停止する | The badge turns grey –, the tip reads `スケジュールを停止しています`, the window's band says the schedule is paused; the menu now reads スケジュールを再開する |
| 63.8 | Right-click while a job runs | 終了（ジョブ 1 件が実行中） is greyed |
| 63.9 | Right-click → 終了（エンジンを停止する） with nothing running | The icon goes away; `koyomi engine status` says the engine is not running |

## 64. 手で流すときに値を渡す（`[params]`、`run --set`）

| # | Do | Expect |
|---|---|---|
| 64.1 | [x] A net with `[params]` `month = "{month-1:%Y-%m}"` and `dept = "all"`, and a job `run = ["cmd.exe", "/c", "echo", "{param:dept}", "{param:month}"]`; `koyomi run <net> --dry` | The job's line shows `all` and last month as `YYYY-MM` |
| 64.2 | [x] `koyomi run <net> --set dept=経理 --set month=2026-09 --wait`, then `koyomi log` | The log has `経理 2026-09` |
| 64.3 | [x] `koyomi run <net> --set nope=1` | Refused: `koyomi: KOYOMI-E3479 値 nope は、[params] にありません。（定義されているのは dept・month）`; nothing runs |
| 64.4 | [x] `koyomi rerun <that run> --from <job>` | The log has `経理 2026-09` again (the values of the first run) |
| 64.5 | [x] The net's scheduled time comes | The job runs with the defaults |
| 64.6 | In the window, select the net and press 今すぐ実行 | A box 値を決めて実行 lists dept and month with the defaults filled in; change dept, press Enter, and the run's log has the new dept |
| 64.7 | Press 今すぐ実行 on a net without `[params]` | It starts at once, with no box |
| 64.8 | [x] Write `{param:x}` in a job of a net whose `[params]` has no `x`; open the window | The net shows the problem `KOYOMI-E3476 {param:x} の値が、[params] にありません。` and cannot be approved |

## 65. 通知に失敗したジョブのログの末尾

| # | Do | Expect |
|---|---|---|
| 65.1 | [x] A job `run = ["cmd.exe", "/c", "(for /l %i in (1,1,15) do @echo line %i) & exit /b 3"]`; `koyomi run <net> --wait` | The system's notification has a last line `<job>: line 15` under the job's line |
| 65.2 | [x] With a webhook on for `failed` (e.g. ntfy), the same run | The message ends with `<job> のログの末尾` and `line 6` … `line 15` |
| 65.3 | A job with `env = { PW = "secret:db" }` that echoes `%PW%` and exits 1 | Both the notification and the webhook show `***`, not the secret |
| 65.4 | [x] A run that ends 正常, or a job that fails before writing anything | The notice is as before, with no log lines |

## 66. タスク スケジューラの XML の取り込み

| # | Do | Expect |
|---|---|---|
| 66.1 | [x] `schtasks /create /tn "\koyomi-test\夜間" /tr "cmd.exe /c echo hi" /sc daily /st 02:30 /f`, then `schtasks /query /xml /tn "\koyomi-test\夜間" > t.xml` and `koyomi import t.xml --dry-run` | `追加: ジョブネット koyomi-test/夜間`; nothing is written (the file is UTF-16 and reads) |
| 66.2 | [x] `koyomi import t.xml`, then `koyomi show koyomi-test/夜間` and `koyomi next koyomi-test/夜間` | The net has `every = "day"` `at = ["02:30"]` and one job `run = ["cmd.exe", "/c", "echo", "hi"]`; it is 未確認, and after approving `next` shows 02:30 each day |
| 66.3 | [x] A task with `/sc weekly /d MON,FRI /st 08:00 /ri 15 /du 10:00` | `every = "minutes"`, `n = 15`, `from = "08:00"`, `to = "17:59"`, `days = ["mon", "fri"]` |
| 66.4 | [x] A task with `/sc monthly /mo LAST /d TUE` or `/sc daily /mo 2`, or an event trigger | The plan ends with `KOYOMI-W…` lines (for example `KOYOMI-W3516 …のトリガーは取り込めません。`) saying which trigger did not come over; the rest comes in |
| 66.5 | [x] A task that runs as SYSTEM, or with `%windir%` in its command | `KOYOMI-W3505 タスクの実行ユーザーは取り込まれません。` and `KOYOMI-W3508 job… に含まれる %…% は、koyomi では展開されません。` |
| 66.6 | The window's 取り込む box with the same `t.xml` | The same plan and notes as the CLI |
| 66.7 | [x] An XML that is not a task (`<a/>`), and a task with only a ComHandler action | `koyomi: KOYOMI-E3502 タスク スケジューラの XML ではありません。一番外側の要素は <a> です。` / `koyomi: KOYOMI-E3503 取り込めるプログラム（<Exec> の <Command>）がありません。` |
| 66.8 | [x] From cmd, `schtasks /query /xml /tn "\koyomi-test\夜間" > t2.xml` (the console code page, CP932, with Japanese in it), then `koyomi import t2.xml --dry-run` | It reads, and the plan is the same as 66.1 |

## 67. 遅延の監視

| # | Do | Expect |
|---|---|---|
| 67.1 | [x] A net with `late_end = "20s"` and a job `run = ["ping", "-n", "40", "127.0.0.1"]`; approve it and `koyomi run <net>`; wait 25 s | One notification `⏰ 遅れ  <name>` / `<name>: <time> を過ぎても終了していません`; `koyomi status --today` shows `⏰終了の遅れ` on the run, and only one notification comes however long it runs |
| 67.2 | [x] `koyomi show <net>` for that net | A line `終わらなければ知らせる: 20s` |
| 67.3 | [x] A net with `[[when]] every = "minutes"` `n = 1`, `overlap = "queue"`, `late_start = "30s"` and a job that takes 3 minutes | When the next minute's time has waited 30 s, one notification `<name>: <time> になっても開始されていません`; when that time starts later, the run is marked `⏰開始の遅れ` and not told again |
| 67.4 | [x] A net that ended 正常 at least 3 times taking about 1 minute; make its job take 3 minutes and run it | After 2 minutes (the usual time ×2, at least the usual time + 1 minute), one notification `<name>: いつもは 1 分ですが、すでに 2 分 実行中です`, and the mark `⏰いつもより長い` |
| 67.5 | [x] The same net with `late_slow = false` | No notification for taking long |
| 67.5b | [x] `concurrency = 1`; three nets with `priority = 1`, `3` and `5` (a 1-minute job each) started by hand in that order while a fourth runs | When the fourth ends, the `priority = 5` net starts, then 3, then 1; waiting jobs say `同時に実行する数（1）の空きを待っています`. Approving a net with `priority = 5` shows `空きの枠を先に取る（priority = 5）` |
| 67.6 | [x] `late_end = "6"` or `late_start = "25:00"` in a net; `koyomi check <file>` | `KOYOMI-E3385 期限 6 を読み込めません。`（action: `"06:00"（時刻）または "30m"（長さ）の形式で指定してください。`）/ the same ID on `25:00`; the net cannot be approved |
| 67.7 | [x] Stop and start the engine after 67.1 | The history keeps the mark (`kind = "late"`); the run is closed as before, and no late notice is sent again |

## 68. 2 つの実行を比べる

| # | Do | Expect |
|---|---|---|
| 68.1 | [x] A net a → b where b prints `start` / `loaded 10` / `end` when a file is there, and `start` / `no such file` and exits 3 when not; run it once with the file and once without; `koyomi compare <run-ok> <run-error>` | `A` and `B` lines as `history` prints the runs, `所要 … → …（±…）`; under `ジョブ（A → B、≠ は状態か終了コードが異なります）`, `= a …` and `≠ b ✓ 正常終了 … 終了コード 0  →  ✕ 異常終了 … 終了コード 3`; then `ログの違い: b` with ` start`, `-loaded 10`, `-end`, `+no such file` (a's log is the same and is not shown) |
| 68.2 | [x] The same with `--toml` | `a`, `b`, `a_seconds`, `b_seconds` and a `[[job]]` each with `differs`, `[job.a]` / `[job.b]` (`state`, `seconds`, `exit`) and `log_diff` only on b |
| 68.3 | [x] Compare two runs of different nets, one with a job the other lacks | `別のジョブネットです: … と …`; the job only one run has shows `（なし）` on the other side and `≠` |
| 68.4 | [x] `koyomi compare <run> 20261030-999999-00` | `koyomi: 実行がありません: 20261030-999999-00`, exit 1 |

## 69. 流さない時間帯

| # | Do | Expect |
|---|---|---|
| 69.1 | [x] A net with `[[when]] every = "minutes"` `n = 2` and `quiet = ["<now>-<now + 5 min>"]` (a span that starts now); approve it | `koyomi show <net>` has a line `実行しない時間帯: …（時刻が来た実行は終了後に実行する）`; no run starts in the span; at its end one run starts (`koyomi status --today`), its planned time the first one in the span; the later times in the span are recorded `⊘ スキップ` with `実行しない時間帯のため、スキップしました。H:MM の回が、時間帯の終了を待っています。` |
| 69.2 | [x] The same with `on_quiet = "skip"` | Each time in the span is recorded `⊘ スキップ` with `実行しない時間帯のため、スキップしました（HH:MM-HH:MM）。`; the times after the span start as usual |
| 69.3 | [x] 69.1 with `koyomi hold <net>` while the time waits, then let the span end | The time is recorded `⏸` (保留中), not started |
| 69.4 | [x] `koyomi run <net>` in the span | The run starts at once (by hand is not held back) |
| 69.5 | [x] `quiet = ["12:00"]`, `quiet = ["12:00-12:00"]` or `on_quiet = "later"`; `koyomi check <file>` | `KOYOMI-E3383 時間帯 12:00 を読み込めません。` / `KOYOMI-E3384 時間帯 12:00-12:00 の開始と終了が同じです。` / `KOYOMI-E3306 on_quiet の値が正しくありません。`; the net cannot be approved |
| 69.6 | [x] `quiet = ["23:00-01:00"]` on a net at 00:30 | The 00:30 time starts at 01:00 |
| 69.7 | [x] 69.1 with a job that takes 2 minutes, the span ending on one of the net's own times | At the end the waited time starts first; the time due at the end finds the run still going and, by `overlap = "queue"`, waits and runs when it ends (not recorded `⊘ スキップ`; with `overlap = "skip"` it is recorded `⊘ スキップ` `前の回がまだ実行中でした。`) |

## 70. 通知の宛先

| # | Do | Expect |
|---|---|---|
| 70.1 | [x] `koyomi.toml`: `[[notify.webhooks]]` three times — one with no `name`, one `name = "経理"`, one `name = "ops"` (each a test URL that shows what it got); a failing net with no `notify_to` | Its `failed` notice reaches the place with no name only |
| 70.2 | [x] The same net with `notify_to = ["経理"]` | `koyomi show <net>` has `通知の宛先: 経理`; the notice reaches 経理 only; the system's notification still shows |
| 70.3 | [x] A net with `notify_to = ["経理"]` and a `kind = "notify"` part with `to = ["ops"]` | The part's notice reaches ops only; the run's end reaches 経理 only |
| 70.4 | [x] `notify_to = ["夜間"]` (no place has it); `koyomi check <file>` | `KOYOMI-W1068 ジョブネットが指定した通知先 夜間 が、koyomi.toml の [notify] に存在しません。`; the file still reads (a warning) |
| 70.5 | [x] `notify_to = "経理"`, `notify_to = ["経理", "経理"]`, or `to = ["ops"]` on a plain job; `koyomi check <file>` | `KOYOMI-E3311 notify_to が正しい配列になっていません。` / `KOYOMI-E3324 notify_to に同じ名前 経理 が 2 回指定されています。` / `KOYOMI-E3401 to は kind = "notify", "mail" のジョブにだけ指定できます。`; the net cannot be approved |
| 70.6 | [x] Stop the engine for a while with webhooks named and not, then start it | The outage notice reaches every place |

## 71. メールの通知

| # | Do | Expect |
|---|---|---|
| 71.1 | `koyomi secret set mail-ops` (the SMTP password); `koyomi.toml`: `[[notify.mails]]` with `host`, `user`, `password = "secret:mail-ops"`, `from`, `to = ["<your address>"]` (port left out: 587, STARTTLS); a net that fails | A mail of the net's failure, with the run's last lines; no mail when the net ends well (only `failed` unless `states` says more) |
| 71.2 | The same with `port = 465` and a server that takes TLS from the start | The mail arrives |
| 71.3 | `states = ["done", "failed"]` on the mail; a net that ends well | A mail arrives for `done` too |
| 71.4 | `name = "ops"` on the mail; a net with `notify_to = ["ops"]`, another with no `notify_to` | Only the first net's notice is mailed; `koyomi check` on a net naming `ops` gives no warning |
| 71.5 | `password = "hunter2"` (plain text) | The engine's log and the settings page say `[[notify.mails]] 1: …password…` (write `secret:`) |
| 71.6 | A wrong `host` or password | The failure is in the engine's log (stderr), after one retry 30 s later; the run's state is not changed |
| 71.7 | Settings → Notifications, the Mail card: "Add a mail place", fill in the server, sender and recipients | `koyomi.toml` gets a `[[notify.mails]]` with them and no `states`; the note under Password names `koyomi secret set` |
| 71.8 | With `password = "secret:mail-ops"` stored, "Send a test here" | A test mail arrives; with the secret not stored, the window's failure band says it is not in the credential store |
| 71.9 | A mail with `password = "hunter2"` in the file; open the Mail card | The password is not shown, a warning says it is written plainly; changing the port writes the file without the plain password |
| 71.10 | "Remove" on a mail place | Its `[[notify.mails]]` table goes from `koyomi.toml`; the other tables keep their comments |

## 72. 祝日の表を足す

| # | Do | Expect |
|---|---|---|
| 72.1 | [x] Download `syukujitsu.csv` from the Cabinet Office's "国民の祝日について" page and put it, as it is (Shift_JIS), in `calendars/`; `koyomi check calendars/syukujitsu.csv` | `<file>: 問題はありません。…〜… 年の祝日 N 日分があり、祝日は <last year>-12-31 まで確認できます。` |
| 72.2 | [x] A calendar with `holidays = "japan"`; the window's calendar page for a year in the file | The holidays and their names are the file's |
| 72.3 | Edit the file so its last line is `2028/1/1,元日` (save as UTF-8); wait a few seconds | The band's "祝日の表は … まで" goes away or moves to 2028-12-31; `koyomi check calendars/<name>.toml` says no caution |
| 72.4 | Write a line `x,y` in the middle of the file | The window's band says `KOYOMI-E4150 祝日の表の N 行目にある日付 x を読み込めません。…` (no file name in front, in the band and in `koyomi check` on a calendar file); nets run on the built-in rules |
| 72.5 | [x] Remove the file (with today near the end of 2027) | The band reads `KOYOMI-W4152 祝日の表は 2027-12-31 までの分です（あと N 日）。` and its action names `syukujitsu.csv` as the way to carry it on |
| 72.6 | [x] `koyomi export -o out.zip --calendars` with the file in `calendars/` | The bundle has the calendars' `.toml` but not `syukujitsu.csv` |

## 73. CSV の書き出し

| # | Do | Expect |
|---|---|---|
| 73.1 | [x] `koyomi list --csv jobs.csv`; open it in Excel | One row per job of every definition, with the heads グループ・ジョブネット・名前・確認・カレンダー・ジョブ…; Japanese is not garbled |
| 73.2 | [x] `koyomi history --from <a week ago> --csv runs.csv` | One row per run, oldest first; 秒 is the run's length; 異常のジョブ names the jobs that ended 異常 |
| 73.3 | [x] `koyomi plan --csv plan.csv` | Every day of this month; a day with no start is one row with only its date and weekday |
| 73.4 | [x] `koyomi plan --csv -` | The table on the console with no BOM, and no file name said |
| 73.5 | A net whose name has a comma; `koyomi list --csv -` | The name is in double quotes; Excel shows it in one cell |
| 73.6 | [x] Set the language to English in Settings; `koyomi plan --csv -` | The heads are Day,Weekday,Time,Jobnet |
| 73.7 | `koyomi history --from <a week ago> --csv runs.xlsx`; open it in Excel | A workbook: bold frozen heads; 日付・開始・終了 are dates (sortable), 秒 is a number |
| 73.8 | A job whose id starts with `=`; `koyomi list --csv jobs.csv` and open it in Excel | The id shows as text with a leading `'`, not as a formula |

## 74. ジョブネットの名前を変更する・複製・削除する

| # | Do | Expect |
|---|---|---|
| 74.1 | Right-click a net in the tree → "Rename or move…"; pick another group, Enter | The net is under the new group, still confirmed and (if held) still held; Compare versions on it shows its old file |
| 74.2 | [x] A net another net uses as a child net; rename it without the checkbox | The result line lists the nets that use it and says to use `--fix`; nothing moved |
| 74.3 | The same with "Point them at the new name too" ticked | Both moved along; the user's file has the new name in `use`, its comments kept, and a version of its old file |
| 74.4 | [x] `koyomi rename 経理/月次 総務/月次 --fix` | `ジョブネット 経理/月次 の名前を 総務/月次 に変更しました。参照も修正しました（…）。` |
| 74.5 | [x] "Copy…" on a net, Enter | A `<name>-2` net appears, unconfirmed; the original is not touched |
| 74.6 | "Delete…" on a net; Enter at once | Nothing is deleted (the box starts on Cancel) |
| 74.7 | "Delete…", Tab, Enter | The net goes from the tree; `koyomi versions <id>` lists its file, and `koyomi restore <id> <version>` brings it back unconfirmed |
| 74.8 | [x] `koyomi delete <id>` on a net another net uses | The nets that use it are printed; after `y` the engine refuses and says to change them first |
| 74.9 | [x] Rename a net while it runs | `KOYOMI-E3756 ジョブネット … は実行中のため、変更できません。` |
| 74.10 | With a net open in the editor, right-click a net | The three items are greyed out ("Close the editor first") |

## 75. 繰り返しの部品（`kind = "loop"`）

| # | Do | Expect |
|---|---|---|
| 75.1 | [x] A job with `kind = "loop"`, `file = 'C:\in\*.csv'`, `run = ["cmd.exe", "/c", "echo %KOYOMI_INDEX% %KOYOMI_FILE%"]`; put `b.csv` and `a.csv` in `C:\in`; start the net | The job ends 正常; its log has `▶ 1/2: …a.csv`, `1 …a.csv`, `▶ 2/2: …b.csv`, `2 …b.csv` |
| 75.2 | [x] The same with no file in the folder | 正常 at once; the log says no file matched and nothing ran |
| 75.3 | [x] `count = 3` instead of `file` | Three passes, `KOYOMI_INDEX` 1, 2, 3 |
| 75.4 | [x] `count = 3` with a command that exits 3 on the second pass | 異常, the reason `2/3 回目で異常: 終了コード 3`; the third pass does not run |
| 75.5 | [x] Add `retry = { count = 1, wait = "1s" }` to 75.4 | The second pass is tried again once before the job ends 異常; the reason ends with `やり直し 1 回` |
| 75.6 | [x] Stop the job (強制終了) during a pass of a long loop | The pass is killed and no later pass starts |
| 75.7 | [x] Write both `file` and `count`, or neither | The definition does not load; the problem names the line and says to write one of them |
| 75.8 | In the editor, drag 繰り返し from the shelf onto the canvas | A circle (it runs a process) with the file and count fields over the command fields; Save writes `kind = "loop"` |
| 75.9 | [x] `koyomi show <net>` with a loop | The loop's line shows `このファイルごとに 1 回` or `繰り返す回数: 3 回` above its command |

## 76. 外への生存の合図（`[ping]`）

| # | Do | Expect |
|---|---|---|
| 76.1 | Make a check on healthchecks.io (or Uptime Kuma's push monitor); `echo <its URL> \| koyomi secret set ping`; write `[ping]` with `url = "secret:ping"` and `every = "1m"` in koyomi.toml; start the engine | Within a minute the watcher shows a ping; then one each minute |
| 76.2 | Stop the engine (`koyomi engine stop`), or pull the PC's network | The pings stop; the watcher tells of it after its grace time |
| 76.3 | `url = "https://…"` in plain text, `hide_webhooks` not written | No ping goes; the window's notes say the address is in plain text and how to move it to the credential store |
| 76.4 | [x] Add `hide_webhooks = false` to `[notify]` | The plain address is pinged |
| 76.5 | `url = "secret:nope"` (not in the store) | No ping goes; the notes say `nope` is not in the credential store |
| 76.6 | [x] `every = "30s"`, or `url = "hc-ping.com/x"` | koyomi.toml does not read; the engine says why and runs nothing until it is fixed |
| 76.7 | Remove `[ping]` while the engine runs | The pings stop within a minute, with no note |

## 77. 月の成績（report）

| # | Do | Expect |
|---|---|---|
| 77.1 | [x] `koyomi report` after a few runs | The month, then a line per net: runs, pass rate, usual time, and the jobs that failed most |
| 77.2 | [x] `koyomi report 2026-09 --csv r.csv` | A CSV with one row per net, rates in percent and averages in seconds |
| 77.3 | History view, open 成績 | The same lines for the rows the filter shows |

## 78. システムのイベントログ（Application ログ、ソース `koyomi`）

| # | Do | Expect |
|---|---|---|
| 78.1 | [x] Let a net end ✓, then a net that fails (`exit 1`) | Event Viewer → Windows ログ → Application, source `koyomi`: id 1000 (情報) and id 2000 (エラー) with the title and the failed jobs |
| 78.2 | [x] A net stopped at its `timeout`; one stopped with 強制終了 | id 2001 and id 2002 (エラー) |
| 78.3 | [x] The failed net runs well on the next start | id 1100 (情報) after the 1000 |
| 78.4 | [x] A net held with ⏸ | Nothing is written (a person did it) |

## 79. 動かす条件（`require`）

| # | Do | Expect |
|---|---|---|
| 79.1 | Write `require = ["ac"]` in a net; unplug the laptop; run it | The job waits (待機) with the reason 「AC 電源に接続されるのを待っています（require = "ac"）」; no slot is taken |
| 79.2 | Plug the AC in again | The job starts within a few seconds |
| 79.3 | `require = ["network"]`, turn the network off, then on | Waits with 「ネットワークに接続されるのを待っています（require = "network"）」, then starts |
| 79.4 | `require = ["idle:1m"]`, keep using the mouse | Waits with 「PC が 1 分間使用されなくなるのを待っています（require = "idle"）」; starts after a minute untouched |
| 79.5 | [x] `require = ["wifi"]` | The net is invalid, with the message 「require は …」 |

## 80. よくある作業の部品（`kind = "delete"` / `"zip"` / `"copy"` / `"empty"`）

| # | Do | Expect |
|---|---|---|
| 80.1 | [x] A folder with an old `a.log` (made 40 days ago), a new `b.log` and an old `c.txt`; a net with `kind = "delete"`, `path` = that folder, `match = "*.log"`, `keep_days = 30`; run it | Only `a.log` goes; the log says `deleted: …a.log` and `1 file(s) deleted` |
| 80.2 | [x] `kind = "zip"`, `from = 'D:\out\*.csv'`, `into = 'D:\arc\{date}.zip'` | `D:\arc\<today>.zip` holds the csv files; run again: replaced, ✓ |
| 80.3 | [x] `kind = "copy"` from a folder with subfolders into a new folder, then run again | All copied ✓; the second run copies nothing and is still ✓; a file only in `into` stays |
| 80.4 | `kind = "copy"` with `into` on a share that does not exist | ✕ (robocopy 8 or more); `retry` tries again |
| 80.5 | `kind = "empty"`, `path` = a folder with files and subfolders | The folder stays, empty; the log says how many items went |
| 80.6 | [x] `path = 'C:\'` on `empty`, `keep_days = 0` on `delete`, `into = 'a.7z'` on `zip`, `run` on a chore | Each is refused when read, with the line and why |
| 80.7 | The editor: drag 古いファイルを削除する from the parts; fill フォルダー and 名前 | The pane shows フォルダー・名前・何日より古いものを削除するか, timeout and retry; saving writes `kind = "delete"`; `D:\` there shows `KOYOMI-E3566 path D:\ は、ドライブや共有の一番上のため、使用できません。` and is not applied |
| 80.8 | [x] Approve a net with a chore | The confirm box shows each chore's paths and days instead of a command |

## 81. ログの行・イベントログで起動する（`[[when]] log` / `eventlog`）

| # | Do | Expect |
|---|---|---|
| 81.1 | [x] A net with `[[when]] log = 'D:\app\app.log'` and `contains = "ERROR"`; `koyomi approve`; append `ok` and then `ERROR disk full` to the file | A run starts within a few seconds (only for the second line); `koyomi show` says `ログ`（`ログ D:\app\app.log に行が書き込まれました。 内容: ERROR disk full`） |
| 81.2 | [x] The file already has `ERROR` lines when the net is approved (or the engine starts) | Nothing starts: only lines written after the watch starts count |
| 81.3 | [x] Append three `ERROR` lines at once | One run; the reason ends with `（ほか 2 件）` |
| 81.4 | [x] Append an `ERROR` line while the run is still going | No second run; a ⊘ スキップ row `… に行が書き込まれました。 ただし、前の回がまだ実行中のため、実行しませんでした。 …` |
| 81.5 | [x] Rotate the log (rename `app.log` to `app.1.log` and make a new `app.log`), then append an `ERROR` line to the new file | A run starts: the new file is read from its start |
| 81.6 | [x] `[[when]] log = 'D:\nothere\app.log'`; then make the folder and the file with an `ERROR` line | Nothing is said while the file is not there; the new file is read from its start and the run starts |
| 81.7 | `[[when]] eventlog = "Security"`, `ids = [4625]` (run the engine as administrator); sign in with a wrong password once | A run starts; `koyomi show` says `イベントログ Security に記録されました。 内容: ID 4625 …` |
| 81.8 | `[[when]] eventlog = "Application"`, `source = "MyApp"`; `eventcreate /L APPLICATION /SO MyApp /T ERROR /ID 100 /D "boom"` | A run starts; an event from another source does not start one |
| 81.8a | [x] `[[when]] eventlog = "Application"`, `ids = [1000, 2000]` and no `source`; run any net by hand so it ends ✓ (koyomi writes ID 1000, source `koyomi`) | The net does not start: koyomi's own records are not counted when no `source` is written. Then write `source = "koyomi"` and `ids = [2000]`, and run another net that fails: this net starts (named on purpose, koyomi's records count); it ends ✓ (1000), so it does not start itself again |
| 81.9 | [x] `eventlog = "NoSuchLog"`, or `Security` with the engine not as administrator | The window's 注意 says `KOYOMI-E3715 ジョブネット <net> が監視する イベントログ … を読み込めません。（…）`; the net is still valid |
| 81.10 | [x] The net has `calendar = "会社"` and the line comes on a closed day; or the net is held | ⊘ スキップ `… ただし、休業日のため、実行しませんでした（on_closed = "run" を指定すると実行できます）。`; ⏸ 保留 with the reason, not run afterwards |
| 81.11 | The editor: add a `[[when]]` of ログに行が出たら and one of イベントログに出たら | The fields are ログのファイル・含む文字, and イベントログ・イベント ID・ソース; except / start / end / on_closed are not shown; saving writes `log` / `contains` and `eventlog` / `ids` / `source` |
| 81.12 | [x] `koyomi next` for the net | The rows `ログに「ERROR」の行が書かれたとき: D:\app\app.log` and `イベントログ Security に記録されたとき（ID 4625）` come first |
| 81.13 | [x] `koyomi check` a `[[when]]` with `log = 'app.log'`, with `log = 'D:\*.log'`, with `log = …` and no `contains`, with `eventlog = "Application"` and neither `ids` nor `source`, with `ids = [70000]` | `KOYOMI-E3342 log が絶対パスではありません。`; `KOYOMI-E3343 log が 1 つのファイルのパスではありません。`; `KOYOMI-E3335 [[when]] に contains が指定されていません。`; `KOYOMI-E3336 eventlog には ids または source が必要です。`; `KOYOMI-E3346 ids が 0 から 65535 までの整数の配列ではありません。` |

## 82. ファイルを送る・受け取る部品（`kind = "send"` / `"fetch"`）

| # | Do | Expect |
|---|---|---|
| 82.1 | [x] `kind = "send"`, `from = 'D:\out\*.csv'`, `into = '\\nas\share\in\{date}'`; run it | The csv files are in the share's folder for today (made when missing); the log says `sent: …` for each; ✓ |
| 82.2 | [x] `kind = "fetch"`, `from = '\\nas\share\out\*.csv'`, `into = 'D:\in'`; run it; then once more with no csv in the share | ✓ and the files are in `D:\in`; the second run is ⚠ 警告 with `no file to fetch: …` |
| 82.3 | An SFTP server you can reach with a key; `koyomi sftp hostkey host` prints `host_key = "SHA256:…"`; `kind = "send"`, `into = "sftp://user@host/in"`, `key = 'C:\keys\id_ed25519'` and that `host_key` | ✓; the files are in `/in` on the server; the log says `sent: <name> (<n> bytes)` for each; `known_hosts` is not used |
| 82.4 | `kind = "fetch"`, `from = "sftp://user@host/~/out/*.csv"`, `into = 'D:\in'` | ✓; each csv under the home's `out` is in `D:\in`; a folder that matches is not fetched; no match is ⚠ 警告 |
| 82.5 | The same send with a wrong `host_key`, to a wrong port, or with a key the server does not take; then `password = "secret:sftp"` instead of `key` after `koyomi secret set sftp` | ✕ with exit code 2 and `! <why>` in the log; nothing asks anything; `retry` tries again; with the password it is ✓ and the log never shows it |
| 82.6 | [x] `koyomi check` a send with `from = "sftp://…"`, a fetch with `into = "sftp://…"`, `into = "sftp://-oProxyCommand=x/in"`, `from = "sftp://h/out/"` on fetch, `key` on `copy`, an sftp:// send without `host_key`, `password = "plain"`, both `key` and `password` | Each is refused with the line and why; the missing `host_key` names `koyomi sftp hostkey` |
| 82.7 | The editor: drag ファイルを送る and ファイルを受け取る from the parts | The pane shows 送るもの・送り先・SSH の秘密鍵 (受け取るもの・置くフォルダー・SSH の秘密鍵), timeout and retry; and ホスト鍵・パスワード; saving writes `kind = "send"` / `"fetch"` with `from`・`into`・`key`・`host_key`・`password` |
| 82.8 | [x] Approve a net with a send | The confirm box shows 元・先・SSH の鍵・ホスト鍵・パスワード instead of a command |

## 83. HTTP で呼ぶ部品（`kind = "http"`）

| # | Do | Expect |
|---|---|---|
| 83.1 | [x] `kind = "http"`, `url = "https://example.com/"`; run it | ✓; the log starts with `HTTP 200` and then the start of the page (cut at 4000 characters with ` ...`) |
| 83.2 | [x] `url = "https://example.com/nope"`; run it; then add `expect = [404]` and run again | The first is ✕ with exit code 1 and `HTTP 404`; the second is ✓ |
| 83.3 | [x] `url = "https://no-such-host.invalid/"` with `retry = { count = 1, wait = "1m" }` | ✕ with exit code 2 and `no answer: …` (why) in the log; it is tried once more after a minute |
| 83.4 | [x] A test API you can see (e.g. a local echo server): `method = "POST"`, `body = '{"day": "{date}", "名前": "テスト"}'`, `headers = { Authorization = "secret:api", X-Run = "{date}" }` with the secret `api` set | The server gets the body as UTF-8 JSON with today's date filled in, `Content-Type: application/json; charset=utf-8`, the Authorization value of the secret, and `X-Run` as written (`{date}` is not filled in a header); the job's log shows neither the secret nor the script |
| 83.5 | [x] The same with `content_type = "text/plain"` | The server gets `Content-Type: text/plain` |
| 83.6 | [x] `koyomi check` with `url = "ftp://x"`, `method = "get"`, `body` with `GET`, `expect = [99]`, `expect = []`, `headers = { Content-Type = "x" }`, `content_type` without `body`, `url` on `copy` | Each is refused with the line and why |
| 83.7 | The editor: drag HTTP で呼ぶ from the parts | The pane shows URL・メソッド・送る中身・ヘッダー (one `名前: 値` a line)・中身の種類・正常とする状態コード, timeout and retry; saving writes `kind = "http"` with those fields (`method` only when not GET); a header line without `:` or a code that is not a number is refused |
| 83.8 | [x] Approve a net with an http | The confirm box shows `POST https://…`, the body, the header names (not their values) and the codes instead of a command |

## 84. 別のアカウントで実行する（`run_as`）

Make a test account first: `net user koyomi-test <password> /add` (as administrator), and `koyomi secret set koyomi-test` with the same password.

| # | Do | Expect |
|---|---|---|
| 84.1 | A job `run = ["C:/Windows/System32/whoami.exe"]` with `run_as = { user = "koyomi-test", password = "secret:koyomi-test" }`; approve and run it | ✓; the log has `▶ koyomi-test のアカウントで実行します` and then `<computer>\koyomi-test` |
| 84.2 | The same with `run = ["C:/Windows/System32/cmd.exe", "/c", "echo %KOYOMI_JOB% %USERPROFILE%"]` | The job's id and the test account's profile folder |
| 84.3 | [x] `koyomi secret delete koyomi-test` (or a wrong secret name), then run | ✕ with `KOYOMI-E3225 実行ユーザー koyomi-test のパスワードのシークレット … が存在しません。`; nothing is started |
| 84.4 | Set the secret to a wrong password; run | ✕ with Windows' reason (the user name or password is incorrect) in the log; the password is not in the log |
| 84.5 | A job that prints the password (`cmd /c echo <password>`) as the account | The log shows `***` |
| 84.6 | A long job (`ping -n 600 127.0.0.1`) as the account; stop it from the window; and one with `timeout = "5s"` | 強制終了 / 打ち切り; no `ping.exe` is left running (Task Manager, user koyomi-test) |
| 84.7 | An administrator account as `run_as`, running `net session` | It fails with access denied: there is no elevation |
| 84.8 | A `kind = "copy"` to a share only koyomi-test can write, with `run_as` | ✓; the files are in the share |
| 84.9 | `koyomi check` with `password = "hunter2"`, `password = "secret:"`, no `user`, `user = "a b"`, an unknown key, `run_as` on a `"time"` part | Each is refused with the line and why; the plain password one says to use `koyomi secret set` |
| 84.10 | The editor: a job's pane, fill 実行のアカウント and the secret name; save | The file has `run_as = { user = …, password = "secret:…" }`; only one of the two filled is refused |
| 84.11 | [x] Approve a net with `run_as` | The confirm box shows `実行のアカウント: CORP\batch（パスワードはシークレット batch）`, and no password |

## 85. 繰り返しの回を並べて動かす（`max`）

| # | Do | Expect |
|---|---|---|
| 85.1 | [x] `concurrency = 4`; a loop `count = 6`, `max = 3`, `run = ["cmd.exe", "/c", "ping -n 6 127.0.0.1 > nul & echo %KOYOMI_INDEX%"]`; run it | It ends in about 10 s (not 30); Task Manager shows 3 `ping.exe` at a time; the log starts with `▶ 6 件の実行を 3 件ずつ並行して実行します` and has each `▶ i/6 番目の実行` once, each followed by its index |
| 85.2 | [x] `concurrency = 2`; the same loop with `max = 3`, and another net's job started while it runs | The loop runs 2 at a time (`▶ 6 件の実行を 2 件ずつ…`); the other job waits with `同時に実行する数（2）の空きを待っています` until the loop ends |
| 85.3 | [x] `file = 'C:\in\*.csv'` with 5 files, `max = 2`, a script that echoes `%KOYOMI_FILE%` | ✓; each file once, under its own heading |
| 85.4 | [x] `count = 6`, `max = 2`, where pass 1 exits 3 at once and the others sleep 5 s | ✕ with `1/6 番目の実行で異常です: 終了コード 3`; pass 2 ran to its end; passes 3–6 did not start |
| 85.5 | `count = 4`, `max = 2`, `ping -n 600 127.0.0.1`; stop it from the window | 強制終了 at once; no `ping.exe` is left; the log has `! 強制終了` under both running passes |
| 85.6 | [x] `count = 4`, `max = 2`, `timeout = "5s"`, `ping -n 600 127.0.0.1` | ✕ 打ち切り after about 5 s; no more passes start |
| 85.7 | [x] `koyomi check` with `max = 0`, `max = 17`, `max = "2"`, `max` on a `"job"` | Each is refused with the line and why |
| 85.8 | The editor: a loop's pane, fill max with 3; save; then empty it | The file has `max = 3`, then no `max`; `0` or `x` is refused |
| 85.9 | [x] Approve a net with `max = 3` | The confirm box shows `並行して実行する数: 3` under the loop |

## 86. 長さの書き方（D4）

| # | Do | Expect |
|---|---|---|
| 86.1 | [x] Run a net whose job takes about 1 minute 5 seconds; `koyomi show <run>` | The run and the job lines read `1分5秒`, not `0:01:05` |
| 86.2 | A job of a few seconds; open the run in the window and pick the job | 所要 reads `3秒`; a 41-minute one reads `41分`; a 65-minute one `1時間5分` |
| 86.3 | [x] Set `lang = "en"` in common.toml; repeat 86.1 | `1 min 5 s` |
| 86.4 | [x] `koyomi compare <a> <b>` for two runs that took 1 min and 1 min 10 s; `koyomi report` | `所要 1分 → 1分10秒（+10秒）`; いつもは in words |

## 87. Start over HTTP (`[http]`, TODO 82)

| # | Do | Expect |
|---|---|---|
| 87.1 | [x] `koyomi secret set koyomi-http` (a long random value); koyomi.toml `[http]` `port = 8790`, `token = "secret:koyomi-http"`; wait a few seconds; `koyomi status` | The status has `HTTP: 127.0.0.1:8790 で待ち受けている`; `netstat -an` shows 127.0.0.1:8790 only, not 0.0.0.0 |
| 87.2 | [x] `curl -i -X POST http://127.0.0.1:8790/runs -d '{"net":"g/a"}'` with no token, then with `-H "Authorization: Bearer wrong"` | 401 both times; no run starts; the engine's log has `http: POST /runs 401 …` and never the token |
| 87.3 | [x] The same with the right token and `-d '{"net":"g/a","params":{"day":"2026-10-31"}}'` | 202 and `{"run":"<id>"}`; the run goes with that param; 操作の記録 has a by-hand run |
| 87.4 | `curl -H "Authorization: Bearer …" http://127.0.0.1:8790/runs/<id>` while it runs and after it ends; then a run id of yesterday | JSON with `state`, `jobs`, `"over": false`, then `true`; yesterday's run is found too |
| 87.5 | [x] POST a net that is held, unconfirmed or does not exist; POST `{}`, `{"net":"g/a","dry":true}`, `not json`; GET `/runs/nope`; `PUT /runs` | 422 with the same reason `koyomi run` gives; 400 for the bodies; 404; 405 |
| 87.6 | [x] Write `token = "plain"`, then `port = 0`, then `host = "0.0.0.0"` under `[http]` | Each is refused when koyomi.toml is read and the band says why; the listener from the last good koyomi.toml stays (an engine that starts with a bad one listens on nothing), and it goes on once the file is good again |
| 87.7 | `koyomi secret delete koyomi-http` and restart the engine; or take 8790 with another program first | A note says it cannot listen and why; it is tried again every 60 s; once fixed it listens without a restart |
| 87.8 | [x] Remove `[http]` while the engine runs | It stops listening within a few seconds; `koyomi status` has no HTTP line |
| 87.9 | [x] `ipv6 = true`; `curl -g "http://[::1]:8790/runs/<id>"` with the token | Answers the same as on 127.0.0.1 |

## 88. The tray apart from the engine (`koyomi engine --no-tray`, `koyomi tray`, TODO 86)

| # | Do | Expect |
|---|---|---|
| 88.1 | `koyomi engine --no-tray` | No koyomi icon in the task tray; `koyomi engine status` answers |
| 88.2 | Then `koyomi tray` | The icon appears with the engine's color and tooltip; a second `koyomi tray` says `koyomi: KOYOMI-E3708 トレイはすでに起動しています。（…）` and exits 1 |
| 88.3 | Run a net that ends ✕ with `[notify] system` on for failures | The tray shows the OS notification within about a second; the engine shows none itself |
| 88.4 | Right-click the icon | The last item reads `トレイを閉じる（エンジンは起動したままです）` and is enabled even while a job runs; スケジュールを停止する pauses the engine (`koyomi engine status` says so) |
| 88.5 | `koyomi engine stop` while the tray is up | The icon turns grey with `koyomi: エンジンは起動していません`; start the engine again with `--no-tray` and it comes back without old notifications |
| 88.6 | Pick トレイを閉じる | The tray exits; `koyomi engine status` still answers |
| 88.7 | `koyomi engine` (with its own tray), then `koyomi tray` | `koyomi tray` says the engine has its own tray and exits 1 |
| 88.8 | With 88.2's tray up, suspend only the process of `koyomi tray` for about 10 s, make three nets fail meanwhile, then resume it. Suspend and resume by process id (Sysinternals `pssuspend <pid>`, `pssuspend -r <pid>`), never by name: the window, the tray and the engine are all `koyomi.exe`. The command line tells them apart: `Get-CimInstance Win32_Process -Filter "Name='koyomi.exe'"` | One notification `🔔 通知が 3 件あります` with `最後の通知: <the last one's title>` and `開く`; the failed sound once. Its `開く` brings the window forward on 今日 |

## 89. The engine as a Windows service (`koyomi service`, TODO 86)

| # | Do | Expect |
|---|---|---|
| 89.1 | [x] `koyomi service status`; `koyomi service start`; `koyomi service run` (not elevated, nothing installed) | `サービスはインストールされていません。koyomi service install でインストールできます。`; `koyomi: KOYOMI-E1045 サービス … はインストールされていません。` and exit 1; `koyomi: KOYOMI-E1051 koyomi service run は、Windows のサービスとして起動された場合にだけ実行できます。` and exit 2 |
| 89.2 | [x] `koyomi service install` in a terminal that is not elevated | It asks for the password, then says `koyomi: KOYOMI-E1053 管理者の権限が必要です。「管理者として実行」で開いたターミナルで、再度実行してください。`; nothing is installed |
| 89.3 | `koyomi engine stop`; `koyomi startup on`; then `koyomi service install` in an elevated terminal, giving your Windows password | It says it installed `koyomi-<user>` and started it; `services.msc` shows `koyomi (<user>)`, 自動（遅延開始）, logged on as you; `koyomi startup` says `トレイだけ` |
| 89.4 | In a terminal that is not elevated: `koyomi service stop`, `koyomi service status`, `koyomi service start` | Each works without an administrator; status shows `停止中（koyomi service start で開始できます）` then `実行中（pid …）` |
| 89.5 | With the service running: `koyomi engine status`; open the window; `koyomi run g/a` | The window and the CLI talk to the service's engine; no second engine starts; the run goes |
| 89.6 | `koyomi service stop` while a long job runs | The job's process tree ends (as with `koyomi engine stop`); the run ends 強制終了; `engine.log` in the settings folder has the engine's lines with times |
| 89.7 | With the service stopped, open the window or `koyomi status` | `KOYOMI-E1062 サービスのエンジンが停止しています。koyomi service start を実行して、サービスを開始してください。`; no engine is started by hand |
| 89.8 | Sign out; wait past a net's scheduled time; sign in | The net ran while you were signed out; the tray (started at sign-in) shows the result |
| 89.9 | Give a wrong password at install, then `koyomi service start` | Start says the service could not sign in, and that the password is wrong |
| 89.10 | `koyomi service uninstall` (elevated) | The service is gone; `koyomi startup` points at the engine again; opening the window starts an engine as before |
| 89.11 | Do 89.3 and 89.10 with the koyomi that `cargo install --path crates/koyomi` put in `~\.cargo\bin` (its `koyomi-com.exe`), not a copy in Temp or Downloads; then `Get-MpThreatDetection \| Sort-Object InitialDetectionTime \| Select-Object -Last 3` (TODO 89) | No new detection (a copy in Temp named `koyomi.com` was taken as `Behavior:Win32/Persistence.A!ml`); `koyomi.exe` and `koyomi-com.exe` are still in `~\.cargo\bin` |
| 89.12 | Copy `koyomi.com` to a folder under `%TEMP%`; from there, in an elevated terminal: `koyomi service install`; then `koyomi startup on` (not elevated) | Both say the exe is in Temp, to put it in `~\.cargo\bin` or Program Files, and that `KOYOMI_ALLOW_TEMP=1` lets it through; install asks no password; nothing is installed; `koyomi startup` is unchanged |
| 89.13 | With `koyomi startup on` set from `~\.cargo\bin`, the same Temp copy with `$env:KOYOMI_ALLOW_TEMP = "1"`: `koyomi service install` (elevated); then `Remove-Item Env:KOYOMI_ALLOW_TEMP` and `koyomi service uninstall` from the Temp copy | Install asks for the password and installs as in 89.3 (this is how tests install from Temp). Uninstall works without the variable and says sign-in still starts only the tray because the exe is in Temp; `koyomi startup on` from `~\.cargo\bin` points it at the engine again |
| 89.14 | Run the window from the Temp copy; in settings turn sign-in start on | The switch goes back off, and under it, in the error colour, it says the exe is in Temp (with `KOYOMI_ALLOW_TEMP=1`); `koyomi startup` still says it is off |
| 89.15 | After 89.14, click the switch again | The Temp line goes away while it asks, then comes back with the switch off |

## 90. Reading the engine over HTTP (`GET /health`, `/nets`, `/runs`, `/runs/<id>/log`; for TODO 80, 81, 87)

With `[http]` set as in §87 and `T` the token.

| # | Do | Expect |
|---|---|---|
| 90.1 | [x] `curl -H "Authorization: Bearer T" http://127.0.0.1:8790/health`; then the same with no token | `{"ok": true, "version": …, "proto": …, "status": "<koyomi status's line>"}`; 401 without the token |
| 90.2 | [x] `GET /nets` with the token, with one net held and one unconfirmed | Each net with `id`, `group`, `name`, `standing` (`confirmed`/`unconfirmed`/`invalid`) and `held`; no definition text; `paused` follows `koyomi pause` |
| 90.3 | [x] `GET /runs`; then `GET "/runs?from=2026-10-30&to=2026-10-31"`; then `?from=bad` and `?to=2026-10-31` | Today's and running runs, each with `"over"`; the two days' runs, oldest first; 400 for both bad ones |
| 90.4 | [x] `GET "/runs/<id>/log?job=<job>"` for a job whose command has a registered secret in its arguments | The log's tail as `koyomi log` shows it, the secret as `***`; without `job` 400; an unknown job 404 |
| 90.5 | [x] `POST /nets`, `DELETE /health` | 405; nothing changes |

## 91. メールを受けて起動する（`[[when]] mail`）

| # | Do | Expect |
|---|---|---|
| 91.1 | `koyomi secret set m` (the mailbox's password); a net with `[[when]] mail = "imaps://me@example.com@imap.example.com/INBOX"`, `password = "secret:m"`, `subject = "請求"`; `koyomi approve`; send a mail with the subject `11月の請求書` to the mailbox | A run starts within about a minute; `koyomi show` says `メール`（`imaps://… にメールが到着しました。 内容: 差出人 <…>「11月の請求書」`） |
| 91.2 | The mailbox already has mail with `請求` when the net is approved (or the engine starts) | Nothing starts: only mail that comes after the watch starts counts |
| 91.3 | Send three matching mails within the same minute | One run; the reason ends with `（ほか 2 件）` |
| 91.4 | `sender = "billing@"`, no `subject`; send one mail from `Billing <BILLING@corp.example>` and one from another address | Only the first starts a run (any case) |
| 91.5 | Send a matching mail while the run is still going | No second run; a ⊘ スキップ row `… にメールが到着しました。 ただし、前の回がまだ実行中のため、実行しませんでした。 …` |
| 91.6 | Open the mailbox in a mail client after the run | The mail is still unread and in its folder: koyomi does not mark, move or delete mail |
| 91.7 | `password = "secret:nothere"`; or a wrong password with `koyomi secret set`; or `/NoSuchFolder` | The window's 注意 says `KOYOMI-E2174` (secret not found), `KOYOMI-E2172` (wrong password) or `KOYOMI-E2173` (no such folder), each as `ジョブネット <net> のメールボックス … …` (for example `KOYOMI-E2173 ジョブネット <net> のメールボックス … のフォルダーが存在しません。`); the net is still valid; after fixing it the warning goes within about a minute; the password never appears in the log or the warning |
| 91.8 | The net has `calendar = "会社"` and the mail comes on a closed day; or the net is held | ⊘ スキップ `… ただし、休業日のため、実行しませんでした（on_closed = "run" を指定すると実行できます）。`; ⏸ 保留 with the reason, not run afterwards |
| 91.9 | The editor: add a `[[when]]` of メールが来たら | The fields are メールボックス・パスワード・差出人・件名; except / start / end / on_closed are not shown; saving writes `mail` / `password` / `sender` / `subject`; a password not starting with `secret:` is refused |
| 91.10 | [x] `koyomi next` for the net; approve it | The row `メールが来たとき: imaps://…（件名「請求」）` comes first; the confirm box shows the same |
| 91.11 | [x] `koyomi check` a `[[when]]` with `mail = "imap://me@host"`, with `mail = "me@host"`, with `password = "hunter2"`, with no `password`, with neither `sender` nor `subject` | `KOYOMI-E3393 mail に、暗号化されない imap:// が指定されています。`; `KOYOMI-E3392 mail に指定された me@host が正しくありません。`; `KOYOMI-E3316 password が secret:<名前> の形式で指定されていません。`; `KOYOMI-E3335 [[when]] に password が指定されていません。`; `KOYOMI-E3336 mail には sender または subject が必要です。` |

## 92. Another PC over the LAN (`[http] lan = true`, `[[remote]]`, `koyomi remote`; TODO 80)

PC A runs the engine with `[http]` as in §87 plus `lan = true`; PC B calls it. `T` is A's token.

| # | Do | Expect |
|---|---|---|
| 92.1 | On A: `koyomi http fingerprint`; start the engine; `koyomi status` | 64 hex digits in pairs with `:`; the status says `HTTP: LAN のポート 8790 で待ち受けています。TLS の指紋は … です。` with the same fingerprint; the certificate is in the settings folder's `tls`, and the same after a restart |
| 92.2 | From B: `curl http://A:8790/health`; `curl -k -H "Authorization: Bearer T" https://A:8790/health` | Plain HTTP gets no answer; over TLS the same JSON as §90.1 |
| 92.3 | On B: `koyomi remote peek https://A:8790` | `fingerprint = "…"`, the same as 92.1 shows on A; an address with nothing listening says it cannot reach it |
| 92.4 | On B: `koyomi secret set a-token` (T); koyomi.toml gets `[[remote]]` with `name = "a"`, `url = "https://A:8790"`, the fingerprint and `token = "secret:a-token"`; `koyomi remote` | `a  https://A:8790  <A's status line>` |
| 92.5 | On B: `koyomi remote status a`, `remote nets a`, `remote runs a` | A's status line and version; A's nets with their standing and 保留; today's runs, the running ones marked 実行中 |
| 92.6 | On B: `koyomi remote run a <group/name>` | `a: ジョブネット <group/name> を実行しました。（<run id>）`; the run shows on A as started over HTTP |
| 92.7 | On B: change one digit of the fingerprint; `koyomi remote status a` | `KOYOMI-E2103 接続先 a の証明書が登録された fingerprint と一致しません。（接続先が示した値: …）…`; nothing is sent |
| 92.8 | On B: `koyomi secret set a-token` with a wrong value; or delete it | `KOYOMI-E2107 接続先 a がトークンを受け付けません。…`; `KOYOMI-E2105 接続先 a のトークンを保存したシークレット a-token が存在しません。…` |
| 92.9 | `koyomi check` koyomi.toml with `url = "http://A:8790"`, a short fingerprint, `token = "T"`, two `[[remote]]` with the same name | Each refused: `KOYOMI-E1021 [[remote]] a の url が正しくありません。…`, `KOYOMI-E1022 … fingerprint が SHA-256 の形式ではありません。…`, `KOYOMI-E1023 … token が、secret:<名前> の形式で指定されていません。…`, `KOYOMI-E1028 … が重複しています。…` |
| 92.10 | [x] `koyomi remote status nope`; `koyomi remote frob` | `KOYOMI-E2116 接続先 nope は koyomi.toml の [[remote]] に存在しません。（登録済み: a）…`; the usage line, exit 2 |

## 93. Standby for another PC (`[standby]`, `koyomi standby [giveback]`; TODO 87)

PC A runs nets as in §92 (`lan = true`). PC B has the same nets confirmed, `[[remote]]` `a` as in §92.4, and `[standby]` with `watch = "a"`, `every = "5s"`, `misses = 2`.

| # | Do | Expect |
|---|---|---|
| 93.1 | [x] Start B's engine; `koyomi status` and `koyomi standby` on B | `待機中: 接続先 a を監視しています。この PC では、スケジュールとイベントによる実行を行いません。`; the window's cautions show the same line |
| 93.2 | [x] Let a net's time pass on both | It runs on A only; B's history has no row for it |
| 93.3 | [x] On B: `koyomi run <net>` | It runs on B (by hand still goes) |
| 93.4 | [x] Stop A's engine; wait about 15 s | B tells `koyomi: 待機系から実行を引き継ぎました` with `接続先 a が応答しなくなったため、… からこの PC で実行します。（KOYOMI-E2101 接続先 a に接続できません。…）` and `引き継いでいない回: …（koyomi rerun <id> で、続きから実行できます）。`; `koyomi status` says `接続先 a から引き継いで実行しています（… から）。` followed by how to give it back with `koyomi standby giveback`; the next time runs on B |
| 93.5 | [x] Restart B's engine | Still taken over at once (`standby.toml` in the state folder has `active = true`) |
| 93.6 | [x] Start A's engine again | B tells `koyomi: 両方の PC で実行しています` once, and stays taken over |
| 93.7 | [x] On B: `koyomi standby giveback`; again | `a に戻しました。再び監視します。`, back to 93.1; the second says `koyomi: KOYOMI-E3784 a から引き継いでいないため、実行を戻せません。…`; `koyomi ops` shows `giveback` |
| 93.8 | [x] On B: change one digit of `a`'s fingerprint; stop A | A note `KOYOMI-E2103 接続先 a の証明書が登録された fingerprint と一致しません。…` (no `[standby] a:` in front); B does not take over while the note shows (only no connection counts) |
| 93.9 | [x] `koyomi check` koyomi.toml with `watch = "nope"`, `every = "1s"`, `misses = 0` | Each refused: `KOYOMI-E1015 [standby] の watch に指定された nope は、[[remote]] の name にありません。…`, `KOYOMI-E1017 [standby] の every が範囲外です。…`, `KOYOMI-E1018 [standby] の misses が範囲外です。…` |
| 93.10 | [x] Remove `[standby]` from B's koyomi.toml | B runs its schedules again; `koyomi standby` says `koyomi.toml に [standby] も [primary] も指定されていません。` |

## 94. People and roles over HTTP (`[[http.user]]`; TODO 83)

`[http]` as in §87 plus `[[http.user]]` `sato` (`role = "view"`, `token = "secret:sato"`) and `ito` (`role = "run"`, `token = "secret:ito"`); `koyomi secret set sato` / `ito` with `S` / `I`. `T` is `[http]`'s token.

| # | Do | Expect |
|---|---|---|
| 94.1 | [x] `curl -H "Authorization: Bearer S" http://127.0.0.1:8790/runs` | 200 and the runs |
| 94.2 | [x] `curl -X POST -H "Authorization: Bearer S" -d '{"net":"<group/name>"}' http://127.0.0.1:8790/runs` | 403 `KOYOMI-E2201 利用者 sato のロールは view です。この操作には run ロールが必要です。…`; nothing starts |
| 94.3 | [x] The same POST with `I`; then with `T` | 202 both times; `koyomi ops` shows the two `run` lines ending `（HTTP: ito）` and `（HTTP: default）` |
| 94.4 | [x] Run a net from this PC's window and with `koyomi run` | Their `koyomi ops` lines have no `（HTTP: …）` |
| 94.5 | [x] Remove `token` from `[http]` (keep the users); restart the engine | It listens; `T` gets 401, `S` and `I` as before |
| 94.6 | [x] `koyomi check` with `role = "admin"`, `name = "default"`, two users named `sato`, `token = "S"`, and `[http]` with neither `token` nor users | Each refused with its reason |
| 94.7 | [x] `koyomi secret delete sato` (or never set it); restart | The status says `KOYOMI-E2177 [http] のトークンを保存したシークレット sato を読み込めません。（…）…` and tries again every 60 s |

## 95. The Web UI (`http://127.0.0.1:<port>/`; TODO 81)

`[http]` and `[[http.user]]` as in §94 (`S` view, `I` run, `T` default). Open the engine's address in a browser.

| # | Do | Expect |
|---|---|---|
| 95.1 | [x] Open `http://127.0.0.1:8790/` | The サインイン form; no other request than `/`, `/theme.css`, `/app.css`, `/app.js`, `/me` (the browser's network tab) |
| 95.2 | [x] Sign in with a wrong token | トークンが一致しません。; still signed out |
| 95.3 | [x] Sign in with `S` | The header shows `sato（view）` and 稼働中; at the left ジョブネット一覧 by group, each net with the mark of its last run today; in the middle 今日 with today's date; no 今すぐ実行 or 強制終了 anywhere; the token field is empty |
| 95.4 | [x] Look at 今日 with nets that start later today and runs that ended, failed and run now | As the window's 今日: a ○ 待ち row for each start still to come (予定 and 次の実行), then the runs newest first (開始・終了・時間, `…` while running); rows tinted by state; a chip per state that has rows (`✕ 異常終了 1` …) |
| 95.5 | [x] Press a chip; press it again | Only that state's rows; then all again |
| 95.6 | [x] Click a run row | The pane at the right: its state, `id · date · times · time taken`, the reason, and its jobs with state, times and exit code |
| 95.7 | [x] ログ on a job of it | The log tail under the jobs, secrets as `***`; while the run goes it follows |
| 95.8 | [x] Click a 待ち row, or a net in the tree | The pane shows the net: its id, 次の実行, 今日の予定 and today's runs of it (a click opens the run) |
| 95.9 | [x] Reload the page | Still signed in (the cookie); refreshes every 5 s; the picked row and the focus stay through a refresh |
| 95.10 | [x] 履歴, then pick a day | That day's runs; no 次の実行 column; 今日 comes back to today's |
| 95.11 | In the window's 設定, set the theme to a dark one, then a light one, then system, reloading the page each time; then put `high_contrast = true` in common.toml and reload | The page takes the window's colours each time, also with a page-darkening add-on such as Dark Reader on; with system it follows the browser's light or dark; with high_contrast the stronger colours |
| 95.12 | [x] `curl -i http://127.0.0.1:8790/theme.css` | 200 `text/css` with `:root {` and `--bg`; no token needed |
| 95.13 | サインアウト, then sign in with `I` | `ito（run）`; on a picked confirmed net 今すぐ実行 (greyed on 未承認 and held ones) |
| 95.14 | [x] 今すぐ実行 a net, then 今すぐ実行 in the question | A new running run in 今日, picked in the pane; `koyomi ops` shows `run … （HTTP: ito）` |
| 95.15 | [x] Pick a running run, 強制終了 | The question starts on キャンセル; with 強制終了する its jobs stop (■ 強制終了); `koyomi ops` shows the kill with `（HTTP: ito）` |
| 95.16 | [x] `curl -i http://127.0.0.1:8790/` | 200 with `Content-Security-Policy` (`script-src 'self'`, `style-src 'self'`, `frame-ancestors 'none'`), `X-Content-Type-Options: nosniff`, `Cache-Control: no-store` |
| 95.17 | [x] With a signed-in cookie, `curl -X POST --cookie "ito_session=…" -d '{"net":"<group/name>"}' http://127.0.0.1:8790/runs` (no `X-Ito-Csrf`) | Refused; nothing starts |
| 95.18 | [x] Put a job's title or log line `<img src=x onerror=alert(1)>` and view it | Shown as text; no dialog |
| 95.19 | With `[http] lan = true`, open `https://<PC>:<port>/` from another PC (accept the self-signed certificate after checking its fingerprint) | The same page; the cookie is `Secure` |
| 95.20 | [x] Pause the schedule (`koyomi engine pause`); stop the engine | スケジュール停止中 in the header and a band over 今日; then エンジンに接続できません in red, the page stays |
| 95.21 | With `[standby]` (or `[primary]`) on the engine; then §102's take-over and give-back | The header says which PC runs the schedules in the window's words and colours (40.5, 40.6): `待機中: … を監視しています` amber, `… から引き継いで実行しています（YYYY-MM-DD HH:MM から）` green, `… が待機しています` grey; after a give-back the standby's chip ends `（YYYY-MM-DD HH:MM に戻しました）` |
| 95.22 | Make the window narrower than 900 px | The tree, 今日 and the pane stack in one column; nothing is cut off |
| 95.23 | [x] As `I`, open a net and press 「ジョブネットを保留」, then 「ジョブネットを保留」 in the question; look at the tree and the pane; `koyomi run <group/name>`; `koyomi ops` | The line `<net> を保留にしました。` (not `[object Object]`); the tree row has ⏸; the pane says `ジョブネットを保留中です。時刻が来ても実行しません。…` and 今すぐ実行 is grey, the button now reads 「ジョブネットの保留を解除する」; `koyomi run` says `KOYOMI-E3742 ジョブネット <group/name> は保留されているため、実行できません。koyomi release <group/name> で保留を解除してから、再度実行してください。`; `koyomi ops` shows `hold … （HTTP: <user>）` |
| 95.24 | [x] Press 「ジョブネットの保留を解除する」, then 「ジョブネットの保留を解除する」 in the question; press 今すぐ実行 | `<net> の保留を解除しました。`; ⏸ is gone and 今すぐ実行 is lit; the net starts; `koyomi ops` shows `release … （HTTP: <user>）` |
| 95.25 | [x] Run a net a → b → c where a sleeps 90 seconds; open the run and look at the jobs' 「スキップ」 buttons; press the one on b, then 「b をスキップ」 in the question | Only the jobs that have not started (b, c) have the button, a has none; `b をスキップしました。`; b is ⊘ スキップ with `人の操作でスキップしました` and no 「ログ」; `koyomi ops` shows `skip <run> b （HTTP: <user>）` |
| 95.26 | [x] After 95.25 (b skipped while a still runs), watch c | c waits for a to end, then runs; the run ends ✓ (as 23.4). (checked 2026-11-02 with the CLI on a 4 s `a`: c ran right after a ended; a failing `a` is covered by a unit test) |
| 95.27 | [x] On a net with `[[when]] at`, press 「次の実行を飛ばす」 and 「飛ばす」 in the question | `ジョブネット <net> の <date> <time> をスキップします（定義は変更しません。取り消す場合は koyomi skip-next <net> --cancel を実行してください）。`; 飛ばす実行 lists `⊘ スキップ <time>`; `koyomi ops` shows `skip-next … （HTTP: <user>）` |
| 95.28 | [x] Press 「飛ばしを戻す」 and 「戻す」 in the question | `ジョブネット <net> のスキップを取り消しました。`; 飛ばす実行 is gone, the 次の実行 comes back; `koyomi ops` shows `unskip-next … （HTTP: <user>）` |
| 95.29 | [x] As `I`, open a net a → b → c; look at the pane's 「ジョブ」 list; press 「保留」 beside b, then 「b を保留」 in the question; look at the tree | The list has a, b, c, each with 「保留」; `b を保留にしました。`; b shows ⏸ and its button reads 「保留を解除」; the tree row of the net has **no** ⏸ (only b is held) and 今すぐ実行 is still lit; `koyomi ops` shows `hold <group/name> b （HTTP: <user>）` |
| 95.30 | [x] After 95.29, press 今すぐ実行 and open the run | a runs and ends ✔; b waits as ⏸ 保留 and c is 待ち; the run does not end; each job row has 「保留」 or 「保留を解除」 (b reads 「保留を解除」), also on a (already over) |
| 95.31 | [x] In that run, press 「保留を解除」 on b, then 「b の保留を解除する」 in the question | `b の保留を解除しました。`; b and c run and the run ends ✔ (no new run appears); the ⏸ is gone from the net's 「ジョブ」 list; `koyomi ops` shows `release <group/name> b （HTTP: <user>）` |
| 95.32 | (`held.toml`, `koyomi list` and the Web UI after a restart checked 2026-11-02; the window after it not yet) Hold b from the Web UI (95.29); in a shell `koyomi list`; open the window; `koyomi engine stop`, then `koyomi list` | `koyomi list` and the window (as 23.13) show b held; the hold is still on after the restart (`held.toml` has `[[jobs]]` with `net` and `job = "b"`); the Web UI shows b ⏸ again |
| 95.33 | [x] Hold b with `koyomi hold <group/name> b`; wait for the Web UI's refresh (5 s); then release b in the Web UI; `koyomi release <group/name> b` again | The pane shows b ⏸ with 「保留を解除」 without a reload; releasing works; the second release prints `KOYOMI-E3726 ジョブネット <group/name> のジョブ b は保留されていません。…` |
| 95.34 | [x] As a `view` user (no `run` role on the net), open the net and a run | No 「保留」 / 「保留を解除」 button on the jobs, in the pane or in the run (⏸ still shows in the pane's 「ジョブ」 list, not in the run's table); with a `roles` entry giving `run` on another net only, this net has no buttons while the other has them; with the token of a `run` user the buttons are there |
| 95.35 | [x] Press 「保留」 on b, then 「キャンセル」 in the question | Nothing is held; no toast; `koyomi ops` has no new line |
| 95.36 | [x] A net with a job that calls a child jobnet (`use`), and one with a branch/part job | Only the net's own jobs are listed (not the child's inside jobs, no job id with `/`); holding the calling job holds the child as 23.14 and the child's jobs show no ⏸ of their own in the tree or the pane |
| 95.37 | [x] `curl -X POST /nets/hold -d '{"net":"<group/name>","job":"b"}'` as a `view` user, and as an `edit` user whose `nets` leaves the net out; then as a `run` user for a job id that is not in the net and for a job already held | `view`/outside `nets`: 403 and nothing is held; unknown job: 422 `KOYOMI-E3722 ジョブネット <net> にジョブ x が存在しません。…`; already held: 422 `KOYOMI-E3724 ジョブネット <net> のジョブ b はすでに保留されています。…`; `release` of a job not held: 422 `KOYOMI-E3726 ジョブネット <net> のジョブ b は保留されていません。…`; in the page the same text shows as a red toast (e.g. a stale page pressing 「保留」 on a job held from the shell) |
| 95.38 | [x] `GET /nets` with a held job, an unnamed job and a job with `name` set, as `run` and as `view` | Each net has `jobs: [{id, name, held}]` (`name` is null when unset; `held` true only for the held one); the net's own `held` stays false; the `view` user gets the same list. A job with `name` set shows that name in the list and the question (`<name> を保留にしますか？<net> のどの実行も、ここに到達したら待機します。`; buttons `<name> を保留` / `<name> の保留を解除する`), while `koyomi ops` shows the job id |
| 95.39 | [x] Hold the net (ジョブネットを保留) and job b together; release the net only; release b | Releasing the net leaves b held (⏸ on b, a run waits there); releasing b lets runs go; each toast names what was released |
| 95.40 | [x] Hold a job, then 「再実行 ▾」 a run of that net from a job before it; hold a job of a run that is already over and start a new run | The rerun waits at the held job as 23.11 does; the over run's row shows 「保留を解除」 for it and the new run waits there |

## 96. Another PC in the window (removed)

The old `PC ▾` band and the page it opened are gone. The status-bar link (§124) opens the whole window on another PC.

## 97. Rerun, skip and hold from the Web UI (TODO 81)

As §95, signed in with `I` (run) unless said.

| # | Do | Expect |
|---|---|---|
| 97.1 | [x] Pick a net, ジョブネットを保留, then ジョブネットを保留 in the question | The tree shows ⏸ and the button turns to ジョブネットの保留を解除する, with the note that it does not run; its next scheduled time does not start; `koyomi ops` shows `hold … （HTTP: ito）` |
| 97.2 | [x] ジョブネットの保留を解除する and confirm | The ⏸ mark goes; `koyomi ops` shows `release … （HTTP: ito）` |
| 97.3 | [x] Let a run fail, pick it, 再実行 ▾ → 停止したところから（完了したジョブは引き継ぐ）, confirm | A new run that starts from the failed job (passed jobs carried over), picked in the pane |
| 97.4 | [x] 再実行 ▾ → `<middle job> から再実行`, confirm | A new run that starts from that job |
| 97.5 | [x] 再実行 ▾ on a failed run → `<failed job> の次から（… はスキップ）`; again with `（… は警告終了）` | New runs that start after that job, which ends ⊘ (then △) |
| 97.6 | [x] While a run waits on a long job, スキップ on a later job that has not started, confirm | The job ends as ⊘ and the next ones go when their turn comes |
| 97.7 | [x] Any of these, then キャンセル (or Esc) in the question | Nothing is sent |
| 97.8 | [x] Sign in with `S` (view) | No 今すぐ実行, 保留, 再実行 ▾, スキップ, 次の実行を飛ばす or 強制終了 buttons; ログ is there |
| 97.9 | [x] `curl -X POST -H "Authorization: Bearer <S>" -d '{}' http://127.0.0.1:8790/runs/<id>/rerun` | 403; nothing starts |
| 97.10 | [x] `curl -X POST -H "Authorization: Bearer <I>" -d '{"from":"a","next":"b"}' http://127.0.0.1:8790/runs/<id>/rerun` | 400 (`KOYOMI-E3780 from と next は、同時に指定できません。どちらか一方を指定してください。`) |
| 97.11 | [x] `curl -X POST -H "Authorization: Bearer <I>" -d '{"job":"nope"}' http://127.0.0.1:8790/runs/<id>/skip` | 422 with the engine's reason |
| 97.12 | [x] 今すぐ実行 on a net with `[params]` | One question with a field per value, its default filled in; キャンセル sends nothing |
| 97.13 | [x] Change one value, keep the others, 今すぐ実行 | The run gets the changed value, the others take their defaults |
| 97.14 | [x] `curl -H "Authorization: Bearer <S>" http://127.0.0.1:8790/nets` | Each net has `params` with its names and defaults (`{}` when none), and `today` (the starts still to come today), `next`, `then` (the one after today's last) and `skips`; a held or unapproved net has no `today` and `next` null |
| 97.15 | [x] `curl -X POST -H "Authorization: Bearer <I>" -d '{"net":"g/a"}' http://127.0.0.1:8790/nets/skip-next` (an approved net with a time later today) | 200 with `"skip": true` and the time it skips, as `koyomi skip-next g/a` says; at that time nothing starts; `koyomi ops` shows `skip-next g/a （HTTP: ito）` |
| 97.16 | [x] The same with `{"net":"g/a","cancel":true}`; then once more | 200 with `"skip": false` and the time runs; the second is 422 (nothing skipped) |
| 97.17 | [x] The same POST with `<S>`; `GET /nets/skip-next`; `{"net":"g/a","job":"b"}` | 403; 405; 400 |
| 97.18 | [x] In the page, pick the net, 次の実行を飛ばす, 飛ばす | The line names the time it skips; the pane lists it under 飛ばす実行 and shows 飛ばしを戻す; 飛ばしを戻す, 戻す takes it back |

## 98. Definitions from the `edit` role (`GET /nets/text`, `POST /nets/propose`; TODO 83)

As §94, plus `[[http.user]]` `kato` (`role = "edit"`, `token = "secret:kato"`; `koyomi secret set kato` with `K`). The window is open on this PC.

| # | Do | Expect |
|---|---|---|
| 98.1 | [x] `curl -H "Authorization: Bearer I" "http://127.0.0.1:8790/nets/text?net=g/a"` | 403 `KOYOMI-E2201 利用者 ito のロールは run です。この操作には edit ロールが必要です。…` |
| 98.2 | [x] The same with `K` | 200 with `text` (the file as it is), `sha256` and `standing` |
| 98.3 | `curl -X POST -H "Authorization: Bearer K" -d '{"net":"g/a","text":"<the text with a job changed>","base":"<sha256 from 98.2>"}' http://127.0.0.1:8790/nets/propose` | 200, `"confirmed": false`; the file on disk is the new text; the window shows `g/a` as 未確認 and its confirm box shows the change |
| 98.4 | [x] Wait for `g/a`'s next scheduled time, or `koyomi run g/a` | It does not run the new text (refused as unconfirmed) |
| 98.5 | Confirm `g/a` in the window | It runs again as scheduled, with the new text |
| 98.6 | [x] Send 98.3 again with the old `base` | 409 `KOYOMI-E3102 ジョブネット g/a は、ほかの場所で変更されています。…`; the file is not touched |
| 98.7 | [x] Send a text that does not read (`format = 1` and a job without `run`) | 422 `KOYOMI-E3106 ジョブネット g/a の定義に誤りがあるため、保存できません。（…）…`; the file is not touched |
| 98.8 | [x] Send a new net `g/new` without `base` | 200; `jobnets/g/new.toml` is there, 未確認 in the window |
| 98.9 | [x] `koyomi ops` | `propose g/a （HTTP: kato）` and `propose g/new （HTTP: kato）` |
| 98.10 | [x] `koyomi versions g/a` | The text before 98.3 is kept as a version |

## 99. A standby copies the definitions (TODO 87)

As §93, plus on A `[[http.user]]` `b` (`role = "edit"`, `token = "secret:b"`) whose token is the one B keeps as `koyomi secret set a`. A has `g/a` confirmed; B has none of A's nets.

| # | Do | Expect |
|---|---|---|
| 99.1 | [x] Start B's engine; wait a few seconds | B says `[standby] 定義 g/a を写しました（未確認です。この PC の画面で確認するまで実行されません）。` for each confirmed net of A; B's window shows them 未確認 |
| 99.2 | [x] Confirm them in B's window | They show 確認済み; nothing more is copied (the hashes match) |
| 99.3 | [x] Change and confirm `g/a` on A; wait up to 5 minutes | B copies `g/a` again, 未確認, the confirm box shows the change; `koyomi versions g/a` on B keeps the one before |
| 99.4 | [x] On A, change `g/a` but do not confirm it | B does not copy it |
| 99.5 | [x] On A make `b`'s role `view`; restart B | B's cautions and `koyomi standby` show `KOYOMI-E2210 接続先 a が要求を拒否しました。トークンの利用者に edit ロールが必要です。…`; watching goes on |
| 99.6 | [x] A net on A that calls a child net (`kind = "jobnet"`, `use = "g/child"`), both new to B | Both land on B in one copy (the child first) |
| 99.7 | [x] Stop A so that B takes over; leave `g/a` unconfirmed on B | B does not run `g/a` (unconfirmed); it copies no more, and the copy's caution from 99.5 is gone |
| 99.9 | [x] Start A again; `koyomi standby giveback` on B | B watches again and copies at once (no 5-minute wait) |
| 99.8 | [x] A net only on B | Still there after copies |

## 100. Nets per person over HTTP (`nets`; TODO 94)

As §98, plus `nets = ["経理/*"]` on `kato` (`K`). Nets `経理/a` and `人事/b`, both confirmed, each with a run today.

| # | Do | Expect |
|---|---|---|
| 100.1 | [x] `curl -H "Authorization: Bearer K" http://127.0.0.1:8790/me` | `"nets": ["経理/*"]`; with `S`, `"nets": null` |
| 100.2 | [x] `GET /nets` and `GET /runs` with `K` | Only `経理/a` and its runs; with `S`, both nets and all runs as before |
| 100.3 | [x] `GET /runs/<人事/b's run>` and `GET /runs/<人事/b's run>/log?job=…` with `K` | 404 both times, as for a run that is not there |
| 100.4 | [x] `POST /runs` `{"net":"人事/b"}` with `K`; also `POST /nets/hold` and `/nets/skip-next` with it | 422 `KOYOMI-E3720 ジョブネット 人事/b の定義が存在しません。…`; nothing starts or changes; `koyomi ops` has no line for them |
| 100.5 | [x] `POST /runs/<人事/b's run>/rerun` and `/stop` with `K` | 422 `KOYOMI-E3277 実行 … が存在しません。…`; nothing starts or stops |
| 100.6 | [x] `GET "/nets/text?net=人事/b"` with `K`; then `net=経理/a` | 404 `KOYOMI-E3608 ジョブネット 人事/b の定義が存在しません。…`; 200 with the text |
| 100.7 | `POST /nets/propose` with `K` for a new `人事/new`; then for a new `経理/new` | 403 `KOYOMI-E2209 利用者 K は、ジョブネット 人事/new を提案できません。（提案できるのは 経理/*）…`, no file; 200, `経理/new` is 未確認 in the window |
| 100.8 | [x] `GET /health` with `K` | `koyomi engine pid N` only (no net names after it); with `S`, the whole line |
| 100.9 | Open the Web UI and sign in with `K` | Only `経理/a` and its runs |
| 100.10 | [x] `koyomi check` with `nets = []`, `nets = ["経理"]` and `nets = ["*/*/x"]` | Each refused with `KOYOMI-E1029 [[http.user]] kato の設定が正しくありません。（…）…` |
| 100.11 | This PC's window and `koyomi list` and `koyomi history` | Both nets and all runs, as before |

## 101. Run a command on another PC (`kind = "ssh"`; TODO 95)

An SSH server you can reach that is already set up (e.g. Windows' OpenSSH Server on this PC as `ssh://<you>@localhost`; this check does not install or turn one on), and a key it takes in `~\.ssh\id_ed25519`. Commands below are for a Windows server (`cmd`); on Linux use the `sh` in brackets.

| # | Do | Expect |
|---|---|---|
| 101.1 | [x] `koyomi sftp hostkey ssh://localhost`, then `koyomi sftp hostkey localhost` | `host_key = "SHA256:…"`, the same both times |
| 101.2 | [x] `kind = "ssh"`, `on = "ssh://<you>@localhost"`, `command = "echo こんにちは & ping -n 4 127.0.0.1 & echo done"` (`echo こんにちは; sleep 3; echo done`) with that `host_key`; run it and watch its log | The lines come as they are printed, not all at the end; こんにちは is not garbled; ✓ |
| 101.3 | [x] `command = "echo oops 1>&2 & exit 3"` (`echo oops >&2; exit 3`); then add `judge = { warn = "3" }` | ✕ with exit code 3 and `! oops` in the log; with the judge, ⚠ 警告 |
| 101.4 | [x] A wrong `host_key`; then `on = "ssh://<you>@localhost:2"`; then `key` = a key the server does not take; also with `retry = { count = 1, wait = "1m" }` | ✕ each time with exit code 255, the why in Japanese as the reason (`localhost: …`) and on the log's last line after `! `; nothing asks anything; the retry tries once more |
| 101.5 | [x] `password = "secret:ssh"` instead of the key, after `koyomi secret set ssh` (a server that takes passwords) | ✓; neither the log nor the confirm box shows the password |
| 101.6 | [x] `env = { DAY = "{date}" }` and `command = "echo %DAY%"` (`echo $DAY`) | When the server's `AcceptEnv` takes `DAY`: today's date. When it does not (the default): `! KOYOMI-W3213 接続先 localhost が環境変数 DAY を受け付けませんでした。接続先の sshd_config の AcceptEnv を確認してください。` and the job goes on |
| 101.7 | [x] `command = "ping -n 120 127.0.0.1"` (`sleep 120`) with `timeout = "10s"`; then without the timeout, 強制終了 it | 打ち切り, then 強制終了, each within about 10 s of the ask (an ssh job is asked first). Under `run_as`, where the ask cannot reach `koyomi ssh run` (DECISIONS.md), the command on the server may go on: look with Task Manager (`ps`) |
| 101.8 | Linux only: `command = "kill -9 $$"` | ✕ with exit code 255 and `localhost: KOYOMI-E3216 接続先のコマンドがシグナル KILL で終了しました。…` as the reason |
| 101.9 | [x] `koyomi check` with `on = "db1"`, `on = "ssh://db1/tmp"`, no `command`, an 8001-character `command`, no `host_key`, `password = "plain"`, both `key` and `password`, `on` on a `copy`, `encoding` on an ssh | Each is refused with the line and why in Japanese; the missing `host_key` names `koyomi sftp hostkey` |
| 101.10 | The editor: drag 別の PC で実行する（SSH） from 基本 | The pane shows 実行する PC・接続先の PC で実行するコマンド・接続先のホスト鍵の指紋・SSH の秘密鍵のファイル・パスワード, 結果の判定, timeout and retry; saving writes `kind = "ssh"` with `on`・`command`・`host_key` (and `key`・`password`・`judge` when set); an `on` without `ssh://` is refused |
| 101.11 | Approve a net with an ssh | The confirm box shows 実行する PC, コマンド (lines joined with ⏎) and the host key instead of a script |
| 101.12 | A Linux server: `command = "sleep 600"`; run it with the release `koyomi.exe` engine on Windows (a window program, no console), and once the log shows it started, 強制終了 it; then the same with an engine that has a console (`cargo run -p koyomi -- engine`, or the engine on Linux or macOS), and with `run_as`. Then stop one while it connects: on the server `nc -lk 2222` (takes the connection, never answers), `on = "ssh://<you>@<server>:2222"`, run it with the release `koyomi.exe` and 強制終了 it within 10 s | The release `koyomi.exe` and the console alike: 強制終了 within a few seconds, the reason `強制終了、<server>: 停止しました（コマンドが開始されていた場合は、TERM で停止するよう要求しました）`, `pgrep -f "sleep 600"` on the server finds nothing, and the job's log has no `Entering debug mode` or `[DBG]` lines. `run_as`: 強制終了 at once with no 要求しました. While it connects: 強制終了 within about 1 s (not 10 s), with the same reason |

## 102. Hand back and take over runs (`give_back`, `[primary]`, `takeover = "rerun"`; TODO 96)

Two engines on this PC, A and B, each in its own temp folders, so the real profile is not touched. Open two consoles. In A's: `$env:KOYOMI_CONFIG_HOME = "$env:TEMP\koyomi-t96\a\config"`, `$env:KOYOMI_STATE_HOME = "$env:TEMP\koyomi-t96\a\state"`, `$env:KOYOMI_ADDRESS = '\\.\pipe\koyomi-t96-a'`; in B's the same with `b`. Run every command below in the console of the engine it names.

- Keep two tokens once: `koyomi secret set t96-a` and `koyomi secret set t96-b` (any values; the store is shared by both).
- A: `[http] lan = true`, `port = 8791`, `[[http.user]]` `b` (`role = "edit"`, `token = "secret:t96-b"`); `[[remote]]` `b` (`url = "https://localhost:8792"`, B's `koyomi http fingerprint`, `token = "secret:t96-a"`); `[primary]` `standby = "b"`, `every = "5s"`, `misses = 2`.
- B: `[http] lan = true`, `port = 8792`, `[[http.user]]` `a` (`role = "run"`, `token = "secret:t96-a"`); `[[remote]]` `a` (`url = "https://localhost:8791"`, A's fingerprint, `token = "secret:t96-b"`); `[standby]` `watch = "a"`, `every = "5s"`, `misses = 2`, `give_back = "auto"`.
- Nets on A, confirmed, and copied to B and confirmed there (§99): `t/r` with `takeover = "rerun"` and `t/l` without, each with job `one` (`cmd /c exit 0`) then `two` (`cmd /c ping -n 120 127.0.0.1 > nul`); `t/clock` with a time every few minutes.

| # | Do | Expect |
|---|---|---|
| 102.1 | [x] Start B's engine, then A's; `koyomi standby` on both | A: `[primary] 接続先 b は待機中です。この PC で実行します。`; B: `待機中: 接続先 a を監視しています。…`. `t/clock` runs on A only |
| 102.2 | [x] `koyomi check` with `[standby]` and `[primary]` both, `give_back = "sometimes"`, a net with `takeover = "maybe"` | Each is an error (`KOYOMI-E1038 [standby] と [primary] が同時に指定されています。…` for the first) |
| 102.3 | [x] Stop B; restart A; `koyomi standby` on A at once, then after 10 s | First `[primary] 接続先 b に問い合わせています。応答があるまで、スケジュールとイベントによる実行を待機させます。`; then the log (not the console) gets `koyomi: 接続先 b が応答しないため、この PC で実行します。` (the toast title is `koyomi: 待機系の PC`) and A runs. Start B again |
| 102.4 | [x] On A `koyomi run t/r` and `koyomi run t/l`; once both are in `two` and 5 s have passed, stop A's engine process (`Stop-Process` on the pid you started) | After about 15 s B shows a toast titled `koyomi: 待機系から実行を引き継ぎました`, and its log line is `koyomi: …` containing `a で実行していた 2 回のうち、1 回を続きから実行します。` and `引き継いでいない回: <id>（koyomi rerun <id> で、続きから実行できます）。` |
| 102.5 | [x] B's window: the runs of `t/r` and `t/l` | `t/r` runs `two` (`one` shows it ended ok on A, not run again); `t/l` is ↷ with `a で実行していましたが、引き継いでいません（<A's id> / …）`, `two` 未実行 with `a で実行していました（終了したかどうかは不明です）` |
| 102.6 | [x] On B `koyomi rerun <t/l's id>` | `t/l` runs from `two` on B |
| 102.7 | [x] Start A again (same console), when no net has a time within ±30 s of the release (else 409 `KOYOMI-E3717 確認済みのジョブネットの時刻（…）が近いため、実行の引き渡しを受け付けられません。…`, see 102.10) | A says `koyomi: 待機系の PC` `接続先 b が引き継いで実行していました。この PC はスケジュールとイベントによる実行を停止し、実行が戻されるまで待機します。`; `t/clock` does not run on A; B says `koyomi: 引き継ぎを待っています`; after about 10 s B says `koyomi: 実行を戻しました` (`接続先 a が引き取りに来たため、実行を戻しました。再び監視します。`) and A `接続先 b から実行が戻りました。この PC で実行します。` |
| 102.8 | [x] B's `standby.toml` in its state folder; `koyomi ops` on B | One line each for the takeover and the hand-back; ops has `handover-release` |
| 102.9 | [x] Repeat 102.4 and 102.7 with `koyomi run t/l` started on B just before A comes back | The run on B goes on to the end after the hand-back; A runs the schedules |
| 102.10 | [x] Make `t/clock`'s next time 20 s after you start A again in 102.7 | B answers 409 (`KOYOMI-E3717 確認済みのジョブネットの時刻（t/clock HH:MM:SS）が近いため、実行の引き渡しを受け付けられません。少し待ってから、再度実行してください。`); A's `koyomi standby` shows it; it is handed back on a later ask, and `t/clock` runs once (on one of them) |
| 102.11 | [x] `give_back = "manual"` on B (restart B); repeat 102.4, then start A | A waits; B's `koyomi standby` says `実行を戻すには、koyomi standby giveback を実行してください。`; `koyomi standby giveback` on B says `a が引き取りを待っているため、次に問い合わせてきたときに戻します。`, and within 5 s A runs. `koyomi ops` on B has `giveback` |
| 102.12 | [x] Take `[primary]` out of A; with `give_back = "auto"` repeat 102.4, then start A | A runs at once; B says `koyomi: 両方の PC で実行しています` (`接続先 a が再び応答しています。両方の PC で実行しています。a が 2 回続けて応答すると、…`) and after about 10 s `koyomi: 実行を戻しました` |
| 102.13 | [x] Put `[primary]` back; on B give `a` `nets = ["t/*"]`; repeat 102.4, then start A | A stays waiting; its cautions show `KOYOMI-E2210 接続先 b が要求を拒否しました。トークンの利用者に run ロールが必要です。…`. Take `nets` out: A takes them back |
| 102.14 | [x] Clean up | Stop both engines (only the two started here), `koyomi secret delete t96-a` and `t96-b`, remove `%TEMP%\koyomi-t96` |

## 103. A click on a notification opens its run (TODO 99)

Setup: a confirmed net `t/n` with one job `cmd /c exit 1`; `[notify]` `system` has `failed` and `warn` (the 通知 page). Note the pids of the windows already open and leave them alone.

| # | Do | Expect |
|---|---|---|
| 103.1 | With the window open on 今日, `koyomi run t/n` and click the notification when it shows | The window comes forward on that run's detail (`t/n` marked at the left) |
| 103.2 | Run it again; press the notification's 開く button instead | The same |
| 103.3 | Close every window; run it again and click the notification | A window starts and opens on that run |
| 103.4 | Leave the notification in the Action Center until the next day (or change the PC's date); click it | The window opens 履歴 (7 days) with that run chosen and the pane at the right open |
| 103.5 | Open `t/n` in the editor, run it, click the notification | The window comes forward; the editor stays as it was |
| 103.6 | Make `t/n` late (`late` after a few seconds, job `cmd /c ping -n 30 127.0.0.1 > nul`) and let it end with exit 1 | Two notifications (遅れ and 異常); neither replaces the other; each opens the same run |
| 103.7 | Start the engine with `--no-tray` and `koyomi tray`; run `t/n` and click the notification | The window opens on the run (the tray showed the notification) |
| 103.8 | Kill the engine (`taskkill /f` on the pid you started), start it again, click `エンジンが異常終了していました` | Nothing opens |

## 104. Themes and a narrow window (TODO 97)

Setup: a few confirmed nets, one with a long name (`経理/とても長い名前の月次締めのジョブネット`) left unconfirmed and held, one with a long command and work folder. Take the shots with `--keys "<Theme:dark>…<Shot:…>"` as well as by hand.

| # | Do | Expect |
|---|---|---|
| 104.1 | With the window open, write `high_contrast = true` in common.toml, in the dark and then the light theme | The window turns to the stronger colours at once (as the Web UI does); removing the line goes back |
| 104.2 | Narrow the tree at the left until the long name does not fit | The name ends in `…`; `未確認` and `保留` stay whole at the right; hovering the row shows the whole name |
| 104.3 | Narrow the window until the net list, a run's jobs and the net page's jobs no longer fit | The word columns (name, command, folder, reason) end in `…` and the table stays inside the window; ID, kind and times stay whole; the heads end in `…` too, and hovering a cut cell or head shows all of it |
| 104.4 | Narrow the window on a calendar page, in the editor with nothing to save, and in the status-bar list | Each row of buttons wraps to a new line instead of pushing the page off the right side |
| 104.5 | Look at the headings of the net list, the net page, a run's jobs and the right pane | All the same size and weight; a screen reader reads them as headings |

## 105. A role per net over HTTP (`roles`; TODO 101)

As §100 without `nets` on `kato`, plus `roles = { "経理/*" = "run" }` on `sato` (`S`, `role = "view"`) and `roles = { "人事/*" = "view" }` on `ito` (`I`, `role = "run"`). Nets `経理/a` and `人事/b`, both confirmed, each with a run today.

| # | Do | Expect |
|---|---|---|
| 105.1 | [x] `curl -H "Authorization: Bearer S" http://127.0.0.1:8790/me` | `"role": "view"`, `"roles": {"経理/*": "run"}`; with `K`, `"roles": null` |
| 105.2 | [x] `GET /nets` with `S` | Both nets; `経理/a` has `"role": "run"`, `人事/b` `"role": "view"`; with `K`, no `role` on the nets |
| 105.3 | [x] `POST /runs` `{"net":"経理/a"}` with `S`; then `{"net":"人事/b"}` | 202 and it runs; then 403 `KOYOMI-E2202 利用者 sato のジョブネット 人事/b でのロールは view です。この操作には run ロールが必要です。…`, nothing starts |
| 105.4 | [x] `POST /runs/<人事/b's run>/stop` and `POST /nets/hold` `{"net":"人事/b"}` with `S` | 403 both times; nothing stops or is held. With `経理/a` (and a run of it still going) both are done |
| 105.5 | [x] `GET /runs/<人事/b's run>/log?job=…` with `S` | 200 with the log (view is enough) |
| 105.6 | [x] `GET "/nets/text?net=経理/a"` with `S` | 403 `KOYOMI-E2201 利用者 sato のロールは run です。この操作には edit ロールが必要です。…` (the strongest of `S`'s roles is still not edit) |
| 105.7 | [x] `POST /runs` `{"net":"人事/b"}` with `I`; then `{"net":"経理/a"}` | 403 `KOYOMI-E2202 利用者 … のジョブネット 人事/b でのロールは view です。…`; then 202 |
| 105.8 | On the standby of §99, `POST /standby/release` with `S`; then with `I` | 403 `KOYOMI-E2201 利用者 sato のロールは view です。この操作には run ロールが必要です。…`; then 200 |
| 105.9 | [x] Open the Web UI and sign in with `S` | Next to the name, `sato（view、経理/* は run）`. `経理/a` and its run have 今すぐ実行, 保留, 強制終了 / 再実行 and スキップ; `人事/b` and its run have none of them |
| 105.10 | [x] `koyomi check` with `roles = {}`, `roles = { "経理" = "run" }` and `roles = { "経理/*" = "admin" }` | Each refused with `KOYOMI-E1029 [[http.user]] sato の設定が正しくありません。（…）…` |
| 105.11 | This PC's window and `koyomi run 人事/b` | Both work as before |

## 106. The working folder of a job without `dir` (`work_dir`) and a month's `day = 29`–`31`

Net `経理/a` with job `p` (`run = ["cmd", "/c", "cd"]`, no `dir`) and job `q` (the same `run`, `dir = "out"`). Net `経理/b` that uses `共通/c`, whose job `r` is `cmd /c cd` with no `dir`. Make the folder `jobnets/経理/out`. Both nets confirmed.

| # | Do | Expect |
|---|---|---|
| 106.1 | [x] No `work_dir` in koyomi.toml; run `経理/a` | `p`'s log shows the engine's own folder, as before; `q` ends in ✕ (no `out` there) |
| 106.2 | [x] Write `work_dir = "definition"`; within 5 s run `経理/a` again | `p`'s log shows `…\jobnets\経理`, `q`'s `…\jobnets\経理\out`; both ✓ |
| 106.3 | [x] Run `経理/b` | `r`'s log shows `…\jobnets\共通` (the child's own folder, not `経理`) |
| 106.4 | [x] `koyomi run 経理/a --dry` | `p: cmd /c cd @ …\jobnets\経理` and `q: … @ …\jobnets\経理\out` |
| 106.5 | [x] Write `work_dir = 'D:\batch'` (make `D:\batch` but not `D:\batch\out`) and run `経理/a` | `p`'s log shows `D:\batch`; `q` ends in ✕ |
| 106.6 | [x] Remove `D:\batch`; `koyomi check jobnets/経理/a.toml` | `KOYOMI-E3182 ジョブ q の作業フォルダー D:\batch\out が存在しません。…` and `KOYOMI-E3184 dir のないジョブの作業フォルダー D:\batch が存在しません。koyomi.toml の work_dir の指定を確認するか、フォルダーを作成してください。`; exit 1 |
| 106.7 | [x] Write `work_dir = "batch"`; `koyomi check koyomi.toml` | Refused: `KOYOMI-E1041 work_dir の値 batch が正しくありません。work_dir = '…' のように、"definition"（定義のあるフォルダー）か絶対パスを指定してください。` |
| 106.8 | [x] A net with `[[when]]` `every = "month"`, `day = 31`, `at = ["21:00"]`; `koyomi check` it | `KOYOMI-W3185 [[when]] の day = 31 は、31 日のない月（2・4・6・9・11 月）には実行されません。月末に実行する場合は、day = "last" を指定してください。`, then `<file>: 問題はありません。`; exit 0 |
| 106.9 | [x] The same with `day = 30`, `day = 29` and `day = "last"` | `（2 月）`; `（うるう年でない 2 月）`; no word |

## 107. Run a command on another Windows PC (`kind = "winrm"`; TODO 100)

A Windows PC you can reach whose WinRM already has an HTTPS listener on 5986 and Basic turned on (`winrm set winrm/config/service/auth @{Basic="true"}`; this check does not set either up), and a local account on it. `koyomi secret set winrm` with that account's password. Below, `<pc>` is that PC and `<you>` the account.

| # | Do | Expect |
|---|---|---|
| 107.1 | `koyomi winrm cert <pc>`, then `koyomi winrm cert winrm://<you>@<pc>:5986`; then `koyomi winrm cert <pc>:2` | `cert = "AB:CD:…"` (32 pairs), the same both times; then `KOYOMI-E2160 接続先 <pc>:2 に WinRM で接続できません。（…）…` (no `koyomi: <pc>:2:` in front) and exit 1 |
| 107.2 | `kind = "winrm"`, `on = "winrm://<you>@<pc>"`, `command = "echo こんにちは & ping -n 4 127.0.0.1 & echo done"`, that `cert`, `password = "secret:winrm"`; run it and watch its log | The lines come as they are printed, not all at the end; こんにちは is not garbled; ✓; the password is nowhere in the log |
| 107.3 | `command = "echo oops 1>&2 & exit 3"`; then add `judge = { warn = "3" }` | ✕ with exit code 3 and `! oops` in the log; with the judge, ⚠ 警告 |
| 107.4 | A wrong `cert`; then `on = "winrm://<you>@<pc>:2"`; then `password = "secret:nope"` after `koyomi secret set nope` with a wrong password; also with `retry = { count = 1, wait = "1m" }` | ✕ each time with exit code 255 and the why in Japanese as the reason: `KOYOMI-E2163 接続先 <pc> の証明書が cert と一致しません。（接続先が示した値: …）…`, `KOYOMI-E2160 接続先 <pc>:2 に WinRM で接続できません。（…）…`, `KOYOMI-E2161 接続先 <pc> がユーザー名またはパスワードを受け付けません。（Basic 認証）…` (no `<pc>:` in front of any); the retry tries once more |
| 107.5 | `env = { DAY = "{date}" }` and `command = "echo %DAY%"` | Today's date |
| 107.6 | `command = "ping -n 120 127.0.0.1"` with `timeout = "10s"`; then without the timeout, 強制終了 it; then the same with the release `koyomi.exe` engine | 打ち切り, then 強制終了, each within about 10 s of the ask, the reason ending `<pc>: 停止しました（コマンドが開始されていた場合は停止を要求し、接続先のシェルを削除しました）`; on `<pc>`, Task Manager shows no `PING.EXE` left |
| 107.7 | [x] `koyomi check` with `on = "<pc>"`, `on = "winrm://<pc>"`, `on = "winrm://<you>@<pc>/wsman"`, no `cert`, `cert = "AB:CD"`, no `password`, `password = "plain"`, `host_key` or `key` on a winrm, `cert` on an ssh, `encoding` on a winrm | Each is refused with the line and why in Japanese; the missing `cert` names `koyomi winrm cert` |
| 107.8 | The editor: drag 別の Windows で実行する（WinRM） from 基本 | The pane shows 実行する PC・接続先の PC で実行するコマンド・接続先の証明書の指紋・パスワード (all required), 結果の判定, timeout and retry; a malformed 証明書の指紋 is marked; saving writes `kind = "winrm"` with `on`・`command`・`cert`・`password` (and `judge` when set) |
| 107.9 | Approve a net with a winrm | The confirm box shows 実行する PC, コマンド (lines joined with ⏎), 証明書の指紋 and パスワード as `secret:winrm` (the name, not the value) instead of a script |
| 107.10 | With Negotiate on in that PC's WinRM (`winrm get winrm/config/service/auth`; the default), add `auth = "ntlm"` to 107.2's job and run it; then `on = 'winrm://<DOMAIN>\<user>@<pc>'` with a domain account's secret; then a wrong password | ✓ both times with the same lines as 107.2; then ✕ 255 with `KOYOMI-E2162 接続先 <pc> がユーザー名またはパスワードを受け付けません。（NTLM 認証）…` (no word of Basic) |
| 107.11 | [x] `koyomi check` with `auth = "kerberos"`, `auth = "NTLM"`, and `auth = "ntlm"` on an `ssh` | Each is refused: `KOYOMI-E3306 auth の値が正しくありません。…` twice, then `KOYOMI-E3401 auth は kind = "winrm" のジョブにだけ指定できます。…` |
| 107.12 | The editor: a winrm's pane; pick NTLM, save; reopen it and pick Basic, save; then approve | The shelf and the canvas show the Windows PC mark; 認証 shows Basic and NTLM, each with its own note; saving writes `auth = "ntlm"`, then no `auth` line; the confirm box shows 認証: NTLM (or Basic) |

## 108. Let Tailscale peers in without a token (`[http] tailnet`; TODO 105)

Two of your PCs on one tailnet with Tailscale running: this PC (the engine), whose MagicDNS name is `<pc>` (`<pc>.<tailnet>.ts.net`) and tailnet address `<ip>` (`tailscale ip -4`), and another PC. `<login>` is your Tailscale login name (`tailscale status` shows it). Start from `[http]` `port = 8790`, `tailnet = 8791`, and a `[[http.user]]` `name = "sato"`, `role = "view"`, `tailscale = "<login>"`, no `token`.

| # | Do | Expect |
|---|---|---|
| 108.1 | [x] `koyomi check koyomi.toml` with, one at a time: `tailnet = 8790`; no user with `tailscale`; `tailscale = "sato"`; `tailscale = "tag:"`; `tailscale` but no `tailnet`; the same login on two users (one in capitals) | Each is refused with its line in Japanese (`KOYOMI-E1025 [http] の tailnet が正しくありません。…`, `KOYOMI-E1026 [http] に tailnet が指定されていますが、tailscale で接続する利用者がいません。…`, `KOYOMI-E1031 [[http.user]] … の tailscale が正しくありません。（…）…` twice, `KOYOMI-E1032 [[http.user]] … に tailscale が指定されていますが、[http] に tailnet がありません。…`, `KOYOMI-E1028 … が重複しています。…`); the starting koyomi.toml says `<file>: 問題はありません。` |
| 108.2 | Start the engine | Its log says `koyomi: HTTP: 127.0.0.1 のポート 8790 で待ち受けています。` and `tailnet のポート 8791 でも待ち受けています。Tailscale で識別された接続には、トークンは不要です。` |
| 108.3 | From the other PC: `curl http://<pc>:8791/me`; then `curl http://<pc>:8790/me` | The first is 200 with `"name":"sato"` and the view role, and the engine logs `http: GET /me 200 …ms（sato、Tailscale）`; the second cannot connect (8790 is this PC only) |
| 108.4 | On the other PC, open `http://<pc>:8791/` in a browser | The Web UI opens without asking for a token, as sato with view: the lists show, there are no run or stop buttons |
| 108.5 | Set `role = "run"` and restart the engine; on the other PC `curl -X POST http://<pc>:8791/runs -H "Content-Type: application/json" -d "{\"net\":\"<a net>\"}"`; then the same with `-H "X-Ito-Csrf: 1"`; then 今すぐ流す in the Web UI | 403 `send X-Ito-Csrf: 1`; then the run starts; the Web UI's button works too; the manual-run record names sato |
| 108.6 | `curl -H "Host: example.com" http://<ip>:8791/me`; then `curl http://<ip>:8791/me` | 403 `host not allowed`; then 200 (its own address is a good Host) |
| 108.7 | Change `tailscale` to another login, restart, `curl http://<pc>:8791/me`; then add `token = "secret:sato"` and send that token as `Authorization: Bearer …` | 401; then 200 as sato (a token works on the tailnet port too) |
| 108.8 | On a PC tagged `tag:koyomi-ha` (`tailscale up --advertise-tags=tag:koyomi-ha`), with `[[http.user]]` `name = "ha"`, `tailscale = "tag:koyomi-ha"`; `curl http://<pc>:8791/me` from it; then the same from a PC whose tags are other ones but whose login is `<login>` | 200 as ha; then 401 (a tagged PC is judged by its tags, not the login) |
| 108.9 | Quit Tailscale on this PC (exit it from the tray, or stop the Tailscale service), start the engine and run `koyomi engine status`; then start Tailscale again | The engine starts and the window and Web UI on this PC work; the status line ends `tailnet のポート 8791 は、まだ開いていません。` and the window's notes show `KOYOMI-W3769 tailnet のポート 8791 をまだ開けません。（…）Tailscale を起動してください。…`; within about a minute of Tailscale coming back the log says `koyomi: HTTP: tailnet のポート 8791 を開きました。Tailscale で識別された接続には、トークンは不要です。`, the note goes, and 108.3 works |

## 109. Planned stops (`[[blackout]]`; TODO 106)

A confirmed net `基盤/毎時` with `[[when]]` `every = "hour"`, `at = [":00", ":30"]` (or any times you can wait for), and the window open on 今日. Below, `<a>` and `<b>` are two times a few minutes apart from now, today, as `HH:MM`, with a `:00` or `:30` of the net between them.

| # | Do | Expect |
|---|---|---|
| 109.1 | [x] `koyomi check koyomi.toml` with, one at a time, a `[[blackout]]` with: `from = "2026/12/29"`; `to = "あした"`; `to` before `from`; no `to`; `reason = "x"` | Each is refused with its line in Japanese (`KOYOMI-E3381 [[blackout]] の from 2026/12/29 を日付または日時として読み込めません。…`, the same for `to`, `KOYOMI-E3382 [[blackout]] の to（…）が、from（…）より前になっています。…`, the missing field, the unknown field `KOYOMI-E1033 …`) |
| 109.2 | Add `[[blackout]]` `from = "<today> <a>"`, `to = "<today> <b>"`, `why = "電源工事"` and save | Within 5 s the band above 今日 says `計画停止があります（電源工事）。<today> <a> から <today> <b> まで、時刻が来ても実行しません。`; in 今日 the net's time between `<a>` and `<b>` is a スキップ row with `⊘ 計画停止（電源工事）`, not a 待ち row |
| 109.3 | Wait until `<a>` | The band says `計画停止中です（電源工事）。<today> <b> まで、時刻が来ても実行しません（手動で実行することはできます）。` |
| 109.4 | [x] Wait past the net's time in the stop | No run starts; 今日 and 履歴 show a スキップ record with the reason `計画停止（電源工事、<today> <a>〜<today> <b>）`; `koyomi history` shows the same |
| 109.5 | [x] During the stop, 今すぐ流す on the net (or `koyomi run 基盤/毎時`) | It runs and ends ✓ (a run by hand is not stopped) |
| 109.6 | [x] A run that is still going at `<a>` (a job with `ping -n 300 127.0.0.1`, started by hand just before) | It goes on to its end; nothing stops it |
| 109.7 | [x] After `<b>`; restart the engine | The band goes; the time in the stop is not caught up (no 逃した run for it); the next time after `<b>` runs as usual |
| 109.8 | `koyomi next 基盤/毎時` while a stop is ahead | The next starts step over the stop (none of them is in it); open the net in the editor and look at 次の開始 in its pane: the same |
| 109.9 | Change the stop to `from = "<tomorrow>"`, `to = "<the day after>"`, `why = "年末年始"`; open カレンダー on the month, then the week, then the day | The two days carry the tag `⊘ 計画停止（年末年始）` in the wait colour; the net's chips on them are dim with `⊘`; hovering one says the stop; the day view shows `⊘ 計画停止（年末年始）  <tomorrow>〜<the day after>` under the day and each card `HH:MM  ⊘ 計画停止（年末年始）` |
| 109.10 | [x] `koyomi plan <YYYY-MM>`, then with `--csv <file>` and `--pdf <file>`; then カレンダー's この月の予定を印刷… | Each start on the two days ends with `  ⊘ 計画停止（年末年始）`; the others are as before |
| 109.11 | [x] Remove `why` | The marks read `⊘ 計画停止（理由なし）` and the record's reason `計画停止（<from>〜<to>）` |
| 109.12 | Turn on a screen reader (NVDA) and move over the band, a skipped 今日 row and a calendar chip in the stop | Each is read with the stop and its why |

## 110. Send a mail with files (`kind = "mail"`; TODO 107)

A `[[notify.mails]]` in koyomi.toml that can send (for example Gmail: `host = "smtp.gmail.com"`, `port = 465`, `user`, `password = "secret:gmail"` with an app password set by `koyomi secret set gmail`, `from`), and a mailbox `<you>` you can read. Two small files `D:\out\a.csv` and `D:\out\b.csv`, and a folder `D:\out\big` with a file over 18 MB.

| # | Do | Expect |
|---|---|---|
| 110.1 | [x] A net with a job `kind = "mail"`, `to = ["<you>"]`, `subject = "売上の集計 {date}"`, `body = "添付のとおり。\n{date} の分です。"`, `files = ['D:\out\*.csv']`; approve and run it, then open the mail | ✓; the job's log has `attached: D:\out\a.csv (… bytes)`, the same for b.csv, and `mailed: <you> (… bytes, through smtp.gmail.com)`; the mail's subject and body have today's date, the Japanese is not garbled, and a.csv and b.csv are attached with their names; nothing in the log shows the password |
| 110.2 | [x] Add `cc = ["名前 <another address>"]` and a second `[[notify.mails]]` with `name = "ops"`; `via = "ops"` | Both get it; the log's `through` is ops's host |
| 110.3 | [x] `files = ['D:\out\none.csv']`; then `files = ['D:\out\big\*']` | ✕ with exit code 255 and the reason `KOYOMI-E3232 添付するファイル D:\out\none.csv が見つかりません。…`, then `KOYOMI-E3234 添付するファイルが大きすぎます。（合計 … MB。上限は 18 MB です）…`; no mail arrives |
| 110.4 | [x] `via = "経理"` (no such place); then remove every `[[notify.mails]]`; then put the place back and `koyomi secret delete <its password's secret name>` (e.g. `gmail` for `password = "secret:gmail"`) | ✕ 255 each time: `KOYOMI-E3227 [[notify.mails]] に name = "経理" の送り先が指定されていません。（存在するのは ops）…`, `KOYOMI-E3226 koyomi.toml に [[notify.mails]] が指定されていません。…`, `KOYOMI-E3231 [[notify.mails]] の … のパスワードを資格情報ストアから読み込めません。（…）…`; `koyomi check <the net's file>` warns with a `KOYOMI-W…` line for the first two before it runs |
| 110.5 | [x] A wrong app password | ✕ 255, the reason `smtp.gmail.com: 送れなかった: …` in Japanese, no password in it |
| 110.6 | [x] `koyomi check` with, one at a time: no `to`; `to = ["ops"]`; `to = ["a@x, b@x"]`; `subject = ""`; no `subject`; `files = [""]`; 21 files; `cc` on a `kind = "http"`; `run_as` on the mail; `method` on the mail | Each is refused with its line and why in Japanese (`KOYOMI-E3552 to が指定されていません。…`, `KOYOMI-E3553 ops はメールのアドレスとして読み取れません。…` and the same for `a@x, b@x`, `KOYOMI-E3554 subject が指定されていません。…` for both the empty and the missing one, `KOYOMI-E3557 files に空の項目があります。…`, `KOYOMI-E3556 files が 20 個を超えています。…`, `KOYOMI-E3401 cc は kind = "mail" のジョブにだけ指定できます。…`, `KOYOMI-E3401 run_as は kind = … のジョブにだけ指定できます。…`, `KOYOMI-E3401 method は kind = "http" のジョブにだけ指定できます。…`) |
| 110.7 | [x] `subject = "secret:gmail"`; `body = "secret:gmail"` | The mail's subject and body are the words `secret:gmail`, not the password |
| 110.8 | The editor: drag メールを送る from 知らせる・呼ぶ | It shows the mail mark; the pane has 送り先 (required), CC, 件名 (required), 本文, 添付するファイル, 送るのに使う設定, timeout and retry, and no 別のアカウントで実行する and no 結果の判定; an address that is not one is marked red; two addresses on two lines and an empty line between save as `to = ["…", "…"]` |
| 110.9 | Approve a net with a mail | The confirm box shows 送り先, CC, 件名, 本文 (lines joined with ⏎), each 添付, and 送るのに使う設定 (`最初の [[notify.mails]]` without `via`), instead of a script |
| 110.10 | [x] `koyomi mail`; `koyomi mail run` in a console | `koyomi: mail takes run (what a mail job does)` and exit 2; then exit 255 with `KOYOMI-E3210 環境変数 KOYOMI_MAIL_TO が設定されていません。…` |

## 111. Values every net reads (`[vars]`; TODO 108)

A confirmed net `基盤/共有` with `[params]` `into = '{var:share}\in'` and a job `a` with `run = ["cmd", "/c", "echo", "{param:into}", "{var:share}"]`, and the window open.

| # | Do | Expect |
|---|---|---|
| 111.1 | [x] Without `[vars]` in koyomi.toml: `koyomi check <the net's file>`; then 今すぐ流す on the net | `KOYOMI-W1069 変数 share の値が、koyomi.toml の [vars] に存在しません。この変数を使用するジョブは、実行時にエラーになります。…`; the run does not start and says `KOYOMI-E3477 {var:share} の値が、koyomi.toml の [vars] にありません。…` (the `{var:share}` that came in through `{param:into}`) |
| 111.2 | [x] Add `[vars]` `share = '\10.0.0.1\Backup\{date:%Y%m}'` and save; `koyomi check` again; run it | No 注意; ✓, and the log of `a` is `\10.0.0.1\Backup\<this month>\in \10.0.0.1\Backup\<this month>` |
| 111.3 | [x] `koyomi run 基盤/共有 --dry`, then with `--set into=D:\x` | The first shows the same paths; the second `D:\x \10.0.0.1\Backup\<this month>` |
| 111.4 | [x] Change `share` to `'D:\share'` and save; run again (no confirm in between) | The new run reads `D:\share\in D:\share`; the net stays confirmed (the value is not part of the net) |
| 111.5 | [x] `koyomi check koyomi.toml` with, one at a time, in `[vars]`: `"共有" = 'x'`; `n = 3`; `a = '{var:share}'`; `a = '{param:into}'`; `a = '{date-1q}'` | Each is refused in Japanese: `KOYOMI-E1012 [vars] の名前 共有 は使用できません。…`, `KOYOMI-E1042 [vars] の n が、文字列で指定されていません。…`, `KOYOMI-E1043 [vars] の a に、{param:…} または {var:…} が含まれています。…` (twice), `KOYOMI-E1044 [vars] の a の値が正しくありません。（…）` |
| 111.6 | [x] In the net, `run = ["cmd", "/c", "echo", "{var:}"]`; then `into = '{var:共有}'` | The editor and `koyomi check` refuse each with its line: `KOYOMI-E3474 {var:} に名前が指定されていません。…`, `KOYOMI-E3475 {var:…} の名前 共有 は使用できません。…` |
| 111.7 | A `kind = "ssh"` with `on = "{var:pc}"` and `[vars]` `pc = "<a PC you reach over ssh>"`; a `kind = "http"` with `url = "{var:api}/health"` | Both pass `koyomi check` and run against the values in `[vars]` |

## 112. Run on a group of PCs (`on = "group:…"`; TODO 109)

Two PCs you reach over ssh (WSL, a Linux VM, the Proxmox host…), called pc1 and pc2 here, each with `koyomi sftp hostkey <host>` noted. In koyomi.toml: `[groups]` `web = ["ssh://ops@pc1", { on = "ssh://ops@pc2", host_key = "<pc2's>" }]`. A confirmed net with a job `fan`: `kind = "ssh"`, `on = "group:web"`, `command = "hostname; sleep 5"`, `host_key = "<pc1's>"`.

| # | Do | Expect |
|---|---|---|
| 112.1 | [x] `koyomi check <the net's file>`; run the net | `<file>: 問題はありません。` The log of `fan`: `グループ web の 2 台で実行します（同時に 1 台まで。ジョブは最も悪い結果の PC の結果に合わせて終了します）。`, `[pc1] <pc1's name>`, `[pc1] 終了コード 0`, then the same for pc2, about 10 s in all; ✓ |
| 112.2 | [x] Add `max = 2`; run | Both start together (about 5 s in all), each line marked `[pc1]` or `[pc2]`; ✓ |
| 112.3 | [x] `command = "test $(hostname) = <pc1's name>"`; run; then add `pass = "any"` and run | The first ends ✗ with code 1 (pc2's); the second ✓ with 0 (pc1's) |
| 112.4 | [x] Add a PC that does not answer to the group (`"ssh://ops@10.255.255.1"`) with `pass = "all"`; run; then with `pass = "any"` | ✗ with `KOYOMI-E3215 3 台のうち 1 台が終了コードなしで終了しました。（10.255.255.1: …）…`; with `any`, ✓, and `! [10.255.255.1] …` in the log |
| 112.5 | [x] `max = 1`, `command = "sleep 60"`; run and stop it while pc1 runs | pc1's command is asked to stop (TERM); `! [pc2] 停止されたため、開始しませんでした` and `! [10.255.255.1] 停止されたため、開始しませんでした`; the job ends stopped |
| 112.6 | [x] Take `host_key` off the job; `koyomi check`; run | `KOYOMI-W1070 グループ web の PC ssh://ops@pc1 に host_key が指定されておらず、ジョブにも指定されていません。このジョブは、実行時にエラーになります。…` and the same for `ssh://ops@10.255.255.1`; in the run, `! [pc1] KOYOMI-E3212 グループの PC ssh://ops@pc1 には host_key が指定されていません。…` and pc2 still runs |
| 112.7 | [x] `on = "group:db"`; `koyomi check`; run. Then a `kind = "winrm"` job with `on = "group:web"` | `KOYOMI-E1013 グループ db が、koyomi.toml の [groups] にありません。…` and the run stops with it; the winrm one with `KOYOMI-E1014 グループ web の PC は ssh:// で指定されており、ジョブの kind と一致しません。…` |
| 112.8 | [x] `koyomi check koyomi.toml` with, one at a time, in `[groups]`: `web = 'ssh://h'`; `web = ['ssh://h', 'winrm://u@h2']`; `"web-1" = ['ssh://h']`; `web = ['ssh://h', 'SSH://h']` | Each is refused in Japanese: `KOYOMI-E1001 [groups] のグループ web が、PC の配列で指定されていません。…`, `KOYOMI-E1009 [groups] のグループ web に、ssh:// と winrm:// の PC が混在しています。…`, `KOYOMI-E1012 [groups] の名前 web-1 は使用できません。…`, `KOYOMI-E1011 [groups] のグループ web に、同じ PC が重複して指定されています。（SSH://h）…` |
| 112.9 | In the editor, the `fan` job: set 実行する PC to `ssh://ops@pc1`, then back to `group:web` | On one PC the host key is required and there is no max; on the group the host key is optional, and `max（グループの PC をいくつ同時に実行するか。空なら 1 台ずつ）` and 正常とするのは (全部の PC が正常 / どれか 1 台が正常) show, with the note on `[groups]` under them |
| 112.10 | In the editor, max `65`, then save; then max `2` and どれか 1 台が正常, and save | `65` is red and saving says `KOYOMI-E3535 max の値が範囲外です。max には、1 から 64 までの数を指定してください。`; the second saves `max = 2` and `pass = "any"` |
| 112.11 | Open the net's approval | It shows `同時に実行する PC の数: 2` and `正常とするのは: どれか 1 台が正常のとき`; the change to `pass` left the net to be confirmed again |
| 112.12 | Windows PCs with WinRM over HTTPS: `[groups]` `app = [{ on = "winrm://ops@app01", cert = "<its>" }, { on = "winrm://ops@app02", cert = "<its>" }]`, a `kind = "winrm"` job with `on = "group:app"`, `command = "hostname"`, `password = "secret:<name>"`, `max = 2` | Both run, each line marked `[app01]` or `[app02]`; ✓ |

## 113. Use a definition from a day (`koyomi stage`; TODO 110)

A confirmed net, 経理/締め here, with a job `a` that runs `cmd /c echo old`. A copy of its file, `next.toml` on the desktop, with `echo new` in its place.

| # | Do | Expect |
|---|---|---|
| 113.1 | Open the net in the editor; change `a` to `echo new`; press この日から使う | A row 使い始める日 opens with the first of next month; 保存 is greyed, and its tip says 「この日から使う」を閉じると保存できます。 |
| 113.2 | Type today's date; then a later day; then undo the change (Ctrl+Z) and redo it | Today: 明日より後の日を選択してください。 and 配置する… is greyed; a later day: 配置する… can be pressed; undone: 現在の定義と同じです。変更してから配置してください。, greyed again |
| 113.3 | Press 配置する… (or Enter in the day) | A box 「<day> からこの形にする」 with the file, the day and the change `- echo old` / `+ echo new`; Esc closes it and nothing is staged |
| 113.4 | Again, and press 承認して配置する | `ジョブネット 経理/締め の定義を、<day> から使用するように配置しました（それまでは現在の定義で実行します）。`; the editor shows the net as it is (`echo old`), with a button 「<day> から次の定義」 by the title; the tree shows `→M/D` on the net; the detail shows `<day> から次の定義に切り替わります` and 次の定義を見る |
| 113.5 | [x] Run the net now | It runs `echo old` |
| 113.6 | Press 次の定義を見る | A box with the change, 承認: 承認済み（その日に確認済みで適用されます）, and 編集に読み込む / 閉じる / 配置した定義を取り消す; 閉じる and Esc leave it staged |
| 113.7 | 編集に読み込む; change `echo new` to `echo newer`; 配置する… and 承認して配置する | The editor opens on `echo new` with the day row on its day, and Ctrl+Z goes back to `echo old`; once staged again, there is still one next definition, now `echo newer` |
| 113.8 | [x] `koyomi list`; `koyomi show 経理/締め` | `→ <day> から次の定義に切り替わります: 経理/締め`; in show, `→ <day> から次の定義に切り替わります。（取り消すには、koyomi stage 経理/締め --cancel を実行してください）` |
| 113.9 | [x] `koyomi stage 経理/締め --cancel` | `ジョブネット 経理/締め の <day> からの切り替え予約を取り消しました（現在の定義のまま実行します）。`; the tree and the detail no longer show it; a second `--cancel` says `koyomi: KOYOMI-E3746 ジョブネット 経理/締め に、切り替え予約はありません。…` and exits 1 |
| 113.10 | [x] `koyomi stage 経理/締め <desktop>\next.toml --from <tomorrow>` and answer `n`; again with `y` | The change is shown, then `koyomi: KOYOMI-E3804 ジョブネット 経理/締め の次の定義を配置しませんでした。配置する場合は、再度実行して y を入力してください。` (exit 1); with `y`, `ジョブネット 経理/締め の定義を、<tomorrow> から使用するように配置しました（…）。` |
| 113.11 | [x] `koyomi stage 経理/締め <the net's own file> --from <tomorrow>`; `--from <today>`; `--from 12/1`; a file with `format = [` | `koyomi: KOYOMI-E3806 ジョブネット 経理/締め の今の定義と同じ内容のため、次の定義として配置しません。…`; `KOYOMI-E3108 切り替え日 <today> は今日以前の日付です。…`; `KOYOMI-E3109 切り替え日 12/1 の形式が正しくありません。…`; `KOYOMI-E3107 ジョブネット 経理/締め の定義に誤りがあるため、切り替え予約を配置できません。（…）…` with the fault in the file. None is staged |
| 113.12 | [x] Leave the engine up over midnight into the staged day (or stop it, set the PC's date to that day, start it) | At 0:00 `ジョブネット 経理/締め を <day> からの定義に切り替えました。` (a notice 定義を切り替えました, where notices go); the net's file is the next one, the net is ✓ confirmed, the old one is in its versions, and the net runs `echo new`; nothing is staged any more |
| 113.13 | [x] Stage again; in the state folder's `next.toml`, change the text by hand; switch as in 113.12 | The box in 113.6 says 配置後に手動で修正されました（その日は未確認で適用されます）; after the switch the net is ▲ unconfirmed (`配置したあとに手動で変更されていたため、未確認です。確認するまで実行されません。`) and does not run till approved |
| 113.14 | [x] Stage again; break the text in `next.toml` (`format = [`); switch as in 113.12 | A notice 定義を切り替えられませんでした; a caution `KOYOMI-E3115 ジョブネット 経理/締め の切り替え予約の定義に誤りがあるため、切り替えを実行できません。（…）…`; the net runs `echo old`. Fix or drop it (`--cancel`) and the caution goes |
| 113.15 | [x] Break `next.toml` itself (`next = [[`) | A warning names `next.toml`; `koyomi stage` and 置く… are refused and the file is left as it was |
| 113.16 | [x] Stage a net, then rename it (and a net that uses it) in the window; then delete it | The staged definition follows the new name (and the user's `use =` points at it); after the delete nothing is staged for it |

## 114. Send a definition to another PC (`koyomi remote send`; TODO 111)

Two PCs: this one (検証) and `B` (本番) with `[http] lan = true` and a `[[http.user]]` `D` of role `edit` (and `R` of role `run`). This PC has `[[remote]]` `name = "本番"` for B, its token kept with `koyomi secret set` as `D`'s. A net 経理/締め is confirmed on both, with a job `a` that runs `cmd /c echo old`; on this PC change it to `echo new` and confirm it.

| # | Do | Expect |
|---|---|---|
| 114.1 | With no `[[remote]]`, open the window and pick 経理/締め | No 別の PC に送る button |
| 114.2 | With `[[remote]]` 本番, pick 経理/締め; press 別の PC に送る ▾ | A menu with 本番 に送る… |
| 114.3 | Press 本番 に送る… | 本番 から 経理/締め の今の定義を読んでいます… for a moment, then a box 「経理/締め を 本番 に送る」 with 送り先 本番, 接続先の現在の定義 確認済み, この PC では 確認済み, and the change `- echo old` / `+ echo new` |
| 114.4 | Esc; then again and press キャンセル | The box closes; nothing is sent (B's file is as it was) |
| 114.5 | Again, and press 送る | `本番 に送信しました: 経理/締め（接続先では未確認です。本番 の画面で確認するまで実行されません）`; on B the net is ▲ unconfirmed, does not run, and B's window shows the change from `echo old` to confirm; B's log has `propose` with `（HTTP: D）` |
| 114.6 | Send it again before B confirms it | No box: `本番 の 経理/締め はこの PC と同じ内容です（送信していません）`, and nothing is sent |
| 114.7 | On B confirm it; on this PC change it back to `echo old`, confirm, and send | `本番 に送信しました: 経理/締め（接続先では未確認です。…）`; on B, approve and it runs `echo old` |
| 114.8 | Make a net 経理/新規 on this PC only; send it to 本番 | The box says 接続先の現在の定義 なし（新規に配置する） and the whole file as added; once sent, B has 経理/新規 unconfirmed |
| 114.9 | Open the box for 経理/締め; before pressing 送る, change the net on B in B's editor and save; then press 送る | `送信できませんでした: …`, naming that 経理/締め was changed on 本番 after it was read; B's file keeps B's change |
| 114.10 | Break the net on this PC (`format = [`) | 別の PC に送る ▾ is greyed; its tip says 定義の誤りを修正してから送信してください。 |
| 114.11 | [x] Keep `R`'s token (role `run`) as the secret; send | No box: `KOYOMI-E2201 利用者 R のロールは run です。この操作には edit ロールが必要です。…` (the engine's line goes through as it is); nothing is sent |
| 114.12 | Stop B's engine (or point `url` at a PC that is off); send | 本番 から … を読んでいます… then the reason it could not connect, in one line; no box. While it reads, キャンセル stops waiting |
| 114.13 | [x] `koyomi remote send 本番 経理/締め` with a change, answer `n`; again with `y` | The change (`- 接続先の現在の定義 / + この PC の定義`) is shown, then `koyomi: KOYOMI-E3805 ジョブネット 経理/締め を送信しませんでした。送信する場合は、再度実行して y を入力してください。` (exit 1); with `y`, `本番 に送信しました: 経理/締め（接続先では未確認です。本番 の画面で確認するまで実行されません）` |
| 114.14 | [x] `koyomi remote send 本番 経理/締め` with no change; `koyomi remote send 本番 経理/新規2` for a net B lacks; `koyomi remote send nope 経理/締め` | `本番 の 経理/締め はこの PC と同じ内容です（送信していません）`; `本番 には 経理/新規2 がありません。新しく配置します。` and the whole file; `KOYOMI-E2116 接続先 nope は koyomi.toml の [[remote]] に存在しません。（登録済み: 本番）…` |

## 115. Start when the PC is left alone (`[[when]] idle`; TODO 112)

The engine runs as you (`koyomi engine start` or at sign-in, not the service). A net 保守/空き with `[[when]] idle = "1m"` and one job that runs `cmd /c echo idle`; approve it. Leave the keyboard and the mouse alone where the steps say so (a remote desktop counts as use on that PC).

| # | Do | Expect |
|---|---|---|
| 115.1 | Use the PC, then leave it alone for a little over a minute | One run starts; `koyomi history` gives the reason `PC が 1 分 使用されませんでした。`, and the run's detail in the window says the same |
| 115.2 | Keep leaving it alone for five more minutes | No second run |
| 115.3 | Move the mouse, then leave the PC alone for a minute again | One more run |
| 115.4 | `koyomi engine stop`; in cmd, `timeout /t 90 > nul & koyomi engine start`, and leave the PC alone at once for four minutes; then use it and leave it alone for a minute | Nothing starts while the PC stays alone after the engine started; one run once it has been used and left alone again |
| 115.5 | Make the job `cmd /c ping -n 300 127.0.0.1 > nul`; leave the PC alone; use it and leave it alone again while that run is going | A ⊘ スキップ row `PC が 1 分 使用されませんでした。 ただし、前の回がまだ実行中のため、実行しませんでした。`; nothing starts after the first run ends |
| 115.6 | Add `calendar` with today closed (or hold the net) and leave the PC alone | ⊘ スキップ `PC が 1 分 使用されませんでした。 ただし、休業日のため、実行しませんでした（on_closed = "run" を指定すると実行できます）。` (⏸ 保留 for the held net); not run afterwards |
| 115.7 | Run the engine as the service (§89) with the same net; sign in at the PC's own screen (not a remote desktop) | No warning in the window's 注意; the net runs as in 115.1–115.3 (the service reads the time of the console's session). If Windows does not keep that time on this PC, the 注意 says `KOYOMI-W3714 ジョブネット 保守/空き の idle の [[when]] は、PC が使用されていない時間を取得できないため、実行されません。…` and nothing starts; the same while nobody is signed in at the screen (see 119.6) |
| 115.8 | [x] `koyomi check` with `idle = "30s"`, with `idle = "10"`, with `idle = "10m"` and `on_closed = "next"` | `KOYOMI-E3339 idle に 1 分未満の時間が指定されています。idle には 1m 以上の時間を指定してください。`; an error for the bare `10`; `KOYOMI-E3333 … の [[when]] では、on_closed に "prev" と "next" を指定できません。on_closed には "skip" または "run" を指定してください。` |
| 115.9 | The editor: add a `[[when]]` of PC が空いたら | One field 使われない時間（例: 10m） with `10m`; except / start / end / on_closed are not shown; saving writes `idle = "10m"`; `30s` is refused with the same words as 115.8 |
| 115.10 | [x] `koyomi next 保守/空き`; approve it after a change | The row `PC が 1m 使われなかったとき` comes first, with no time; the confirm box shows the same |
| 115.11 | [x] `schtasks /create /tn "\koyomi-test\空き" /tr "cmd.exe /c echo hi" /sc onidle /i 15 /f`, `schtasks /query /xml /tn "\koyomi-test\空き" > i.xml`, `koyomi import i.xml --dry-run` | The plan has `idle = "15m"` and the warning `KOYOMI-W3513 アイドル時のトリガーを、idle = "15m" として取り込みました。PC が 15m の間使用されなかったときに実行されます。`; then `schtasks /delete /tn "\koyomi-test\空き" /f` |
| 115.12 | `koyomi history --csv runs.csv` after 115.1 | The run's きっかけ is `PC が空いた` |

## 116. Run SQL (`kind = "sql"`; TODO 113)

SQLite needs nothing more: make an empty file with `type nul > D:\koyomi-test\a.db` (an empty file is an empty database). For SQL Server, an instance on this PC or the LAN that listens on TCP (SQL Server Configuration Manager → TCP/IP enabled, a fixed port), a database `koyomi_test`, and the account that runs the engine as a user of it. For PostgreSQL, a server you can reach with a user `koyomi` and a database `koyomi_test`. `koyomi secret set pg` (and `koyomi secret set mssql` for a SQL Server login) first.

| # | Do | Expect |
|---|---|---|
| 116.1 | [x] `koyomi check` on a net with a job `kind = "sql"`, `db = "sqlite://D:\koyomi-test\a.db"` and neither `sql` nor `file`; then both | `KOYOMI-E3246 sql も file も指定されていません。…`; then `KOYOMI-E3562 sql と file は、どちらか一方だけを指定する必要があります。…` |
| 116.2 | [x] The job with `sql = "create table t (id integer, msg text); insert into t values (1, 'a'); insert into t values (2, 'b'); insert into t values (3, 'c');"` and `rows = "3"`; approve it and run it now | ✓; the job's log has `sql: D:\koyomi-test\a.db` and `ran: 4 batches, 3 rows changed` |
| 116.3 | [x] Change it to `sql = "delete from t where id = 99"`, `rows = "1.."`; run it | ✕; the reason is `KOYOMI-E3248 変更した行が 0 行で、rows = 1.. の範囲に含まれません。…` |
| 116.4 | Change it to `sql = "select * from t order by id"`, `out = "D:\koyomi-test\out\{date}.csv"`, no `rows`; run it; open the CSV in Excel | ✓; the folder `out` is made; the log has `result: 3 rows -> D:\koyomi-test\out\<today>.csv` and the four lines `id,msg` … `3,c`; Excel shows the Japanese and the numbers as numbers (right-aligned) |
| 116.5 | [x] Save `select msg from t where msg = 'あ';` in Notepad as UTF-8, then as UTF-16 LE (Unicode), then as ANSI (Shift_JIS); point `file` at it (no `sql`) and run each | ✓ each time; `result: 0 rows` (the table only has a header) — no line says the file could not be read and no mojibake in the log |
| 116.6 | [x] `db = "sqlite://D:\koyomi-test\nothere.db"`; run it | ✕ `KOYOMI-E3244 データベースのファイル D:\koyomi-test\nothere.db が存在しません。…`; no file `nothere.db` is made |
| 116.7 | [x] `sql = "select * from nowhere"`; run it | ✕ `KOYOMI-E3241 D:\koyomi-test\a.db のデータベースが SQL を拒否しました。（no such table: nowhere）…` |
| 116.8 | SQL Server, Windows sign-in: `db = "sqlserver://<host>:<port>/koyomi_test"`, `sql = "create table t (id int, msg nvarchar(10))\nGO\ninsert into t values (1, N'あ'), (2, N'い')\nGO\nselect * from t"`, `out` set, `tls = "trust"` (a server with its own certificate) | ✓; `ran: 3 batches, 2 rows changed`, `result: 2 rows -> …`; the CSV has `あ` and `い` |
| 116.9 | The same with `tls = "verify"` against a server with its own certificate | ✕; the reason is `KOYOMI-E3239 <host>:<port>/koyomi_test に接続できません。（…）サーバーが自分で作った証明書を使っている場合は、tls = "trust" を指定してください。…` |
| 116.10 | [x] `db = "sqlserver://<host>\SQLEXPRESS/koyomi_test"` in `koyomi check`; then `tls = "off"` with a good db | `KOYOMI-E3585 名前付きインスタンス（ホスト\名前）は使用できません。…`; then `KOYOMI-E3560 SQL Server への接続では、tls = "off" は使用できません。…` |
| 116.11 | The job of 116.8 with `run_as` set to another Windows account that is not a user of `koyomi_test` | ✕; the reason has the server's refusal of that account's sign-in (the job ran as it, not as you) |
| 116.12 | SQL Server login: `db = "sqlserver://koyomi@<host>:<port>/koyomi_test"`, no `password` in `koyomi check`; then `password = "secret:mssql"` and run 116.8's select | `KOYOMI-E3550 … には password が指定されていません。…`; then ✓; the password is in no log line, reason or `koyomi show` |
| 116.13 | PostgreSQL: `db = "postgres://koyomi@<host>/koyomi_test"`, `password = "secret:pg"`, `sql = "create table if not exists t (id int); insert into t values (1); select count(*) as n from t;"`, `tls` as the server needs (`off` for a server without TLS) | ✓; the log has `result: 1 rows` and the lines `n` and the count |
| 116.14 | A job `sql = "WAITFOR DELAY '00:05:00'"` (SQL Server) or `select pg_sleep(300)` (PostgreSQL); run it, then 強制終了 after ten seconds | It ends within about a second (not ten); ✕, and the reason has `停止しました（それまでのバッチは反映済みです）`; on the server, `sys.dm_exec_requests` / `pg_stat_activity` no longer has the statement |
| 116.15 | The same job with `timeout = "15s"` | ✕ after about 15 seconds, and the statement is gone from the server as in 116.14 |
| 116.16 | The editor: add 「SQL を実行する」 from 基本 on the shelf; type `sqlserver://db01/売上` in データベース; then `postgres://x@db01/売上`; then `sqlite://C:\a.db` | The SQL box's top right says `SQL Server`, then `PostgreSQL`, then `SQLite`; the password field shows only for PostgreSQL (and for a SQL Server address with a user); TLS is hidden for SQLite; a password left from PostgreSQL stays shown and marked wrong for the SQLite address |
| 116.17 | In the editor, write both SQL and a file; then `rows = "many"`; then save a good one and approve it | Both fields are marked wrong; then the 正常とする行の数 field; the approve box lists データベース, SQL (each line break as ⏎), 結果の置き場所, 正常とする行の数 and, when not verify, TLS |
| 116.18 | [x] `koyomi sql run` by hand in cmd with `set KOYOMI_SQL_DB=sqlite://D:\koyomi-test\a.db` and `set KOYOMI_SQL_TEXT=select count(*) from t` | It prints `sql: …`, the result lines and ends with 0; without `KOYOMI_SQL_DB` it says `KOYOMI-E3210 環境変数 KOYOMI_SQL_DB が設定されていません。koyomi sql run は、kind = "sql" のジョブから実行してください。` |

## 117. Send to and fetch from S3 (`s3://`; TODO 114)

A MinIO server you can reach (`minio server D:\minio-data` on this PC or a NAS; its console makes a bucket `koyomi-test` and an access key), and for 117.10 an Amazon S3 bucket and an IAM user's access key that may put, get and list in it. `koyomi secret set minio` (the MinIO secret key) and `koyomi secret set s3` (the AWS one) first. `D:\koyomi-test\out` holds `a.csv`, `b.csv`, `c.txt` and a folder `sub` with `d.csv`; one file of 1 GB, `big.zip` (`fsutil file createnew D:\koyomi-test\out\big.zip 1073741824`), is added for 117.7 only.

| # | Do | Expect |
|---|---|---|
| 117.1 | [x] `koyomi check` on a net with a job `kind = "send"`, `from = 'D:\koyomi-test\out'`, `into = "s3://koyomi-test/daily/"` and nothing else; then add `key_id = "<the MinIO access key>"` and `password = "secret:minio"`; then `endpoint = "http://<host>:9000"` | `KOYOMI-E3573 s3:// には key_id が指定されていません。…`; then `KOYOMI-E3577 s3:// には region が指定されていません。region = "ap-northeast-1" のように指定してください。…`; then no problem |
| 117.2 | Approve the job and run it now; look at the bucket in the MinIO console | ✓; the log has `sent: s3://koyomi-test/daily/a.csv (… bytes)` for `a.csv`, `b.csv` and `c.txt`; the bucket has `daily/a.csv`, `daily/b.csv`, `daily/c.txt` and no `daily/sub/` |
| 117.3 | [x] `from = 'D:\koyomi-test\out\*.zip'` (no zip there); run it | ⚠ warning (end code 3); the log has `no file to send: D:\koyomi-test\out\*.zip` |
| 117.4 | Add `in/x.csv`, `in/y.csv`, `in/sub/z.csv` and `in/n.txt` in the MinIO console. A job `kind = "fetch"`, `from = "s3://koyomi-test/in/*.csv"`, `into = 'D:\koyomi-test\in'`, the same `key_id`, `password` and `endpoint`; run it | ✓; the folder `in` is made with `x.csv` and `y.csv` only (no `z.csv`, no `n.txt`, no `.part` left); the log has a `fetched: s3://koyomi-test/in/x.csv (… bytes)` line for each |
| 117.5 | `from = "s3://koyomi-test/in/*.zip"`; run it | ⚠ warning; the log has `no file to fetch: s3://koyomi-test/in/*.zip` |
| 117.6 | `password = "secret:minio"` with a wrong value kept under `minio` (`koyomi secret set minio` again); run 117.2's send | ✕; the reason is `KOYOMI-E3251 s3://koyomi-test/daily/a.csv へのアクセスを拒否されました。（…）key_id、password、region を確認してください。`; the secret is in no log line or reason |
| 117.7 | Put `big.zip` (1 GB) in `out`; run 117.2's send; then 強制終了 it while the log shows `big.zip: 30%` | Before the stop, the log has `big.zip: 10%`, `20%` …; after it, ✕ within about ten seconds, the reason has `停止しました（転送中のものは残していません）`, and the bucket has no `daily/big.zip` (and `mc ls --incomplete <alias>/koyomi-test/daily/` lists nothing) |
| 117.8 | Run 117.7 again to the end, then fetch it back (`from = "s3://koyomi-test/daily/big.zip"`) and 強制終了 the fetch halfway | The send ends ✓; the fetch ends ✕ with `止めた（…）` and `D:\koyomi-test\in` has neither `big.zip` nor `big.zip.part` |
| 117.9 | The editor: add 「ファイルを送る」 from the shelf; type `sftp://u@h/in` in 送り先, then `s3://koyomi-test/daily/` | For `sftp://`, SSH の鍵・ホスト鍵・パスワード; for `s3://`, アクセス キー ID・シークレット アクセス キー・リージョン・S3 のサーバー instead (the first two required); the approve box lists key_id, password, region and endpoint as written (the password as `secret:minio`) |
| 117.10 | Amazon S3: `into = "s3://<your bucket>/koyomi/{date}/"`, `key_id = "<the IAM key>"`, `password = "secret:s3"`, `region = "<the bucket's region>"`, no `endpoint`; run it; then fetch `s3://<your bucket>/koyomi/<today>/*.csv` | Both ✓; the AWS console shows `koyomi/<today>/a.csv` …; with a wrong `region`, ✕ and the reason has what S3 answered (it names the region it expects) |
| 117.11 | `koyomi cloud run` by hand in cmd with `set KOYOMI_CLOUD_OP=fetch`, `set KOYOMI_CLOUD_FROM=s3://koyomi-test/in/*.csv`, `set KOYOMI_CLOUD_INTO=D:\koyomi-test\byhand`, `set KOYOMI_CLOUD_KEY_ID=<key>`, `set KOYOMI_CLOUD_PASSWORD=secret:minio`, `set KOYOMI_CLOUD_ENDPOINT=http://<host>:9000` | It prints the `fetched:` lines and ends with 0; without `KOYOMI_CLOUD_OP` it says `KOYOMI-E3210 環境変数 KOYOMI_CLOUD_OP が設定されていません。…` |

## 118. Check what a send or fetch moved (`verify`; TODO 116)

`D:\koyomi-test\out` holds `a.csv`, `b.csv` and `big.zip` (1 GB) as in 117, and `\\10.0.0.1\Backup\koyomi-test` is a shared folder you may write. For 118.3–118.6, an SFTP server you reach with a key (as in 82.3) and, on it, a second user made for transfers only (`ForceCommand internal-sftp`, chrooted to a folder with `incoming` that it may write). For 118.7, the MinIO bucket of 117.

| # | Do | Expect |
|---|---|---|
| 118.1 | [x] `koyomi check` on a send with `verify = "md5"`; a copy with `verify = "sha256"`; an ssh with `verify = "readback"` | Each is refused: `KOYOMI-E3306 verify の値が正しくありません。verify には、"sha256", "readback" のいずれかを指定してください。`, then `KOYOMI-E3401 verify は kind = "send", "fetch" のジョブにだけ指定できます。…` for the other two |
| 118.2 | A job `kind = "send"`, `from = 'D:\koyomi-test\out\*.csv'`, `into = '\\10.0.0.1\Backup\koyomi-test'`, `verify = "sha256"`; approve it and run it | ✓; the log has `sent: D:\koyomi-test\out\a.csv (sha256 <64 hex>)` for `a.csv` and `b.csv`; the share has `a.csv` and `b.csv` and no `.part`; `certutil -hashfile <the share's a.csv> SHA256` prints the same hash |
| 118.3 | The SFTP user that may run commands: `into = "sftp://user@host/in"` with its `key` and `host_key`, `verify = "sha256"`; run it | ✓; the log has `sent: a.csv (… bytes) (sha256 <64 hex>)` for each; `sha256sum /in/a.csv` on the server prints the same hash; `/in` has no `.part` |
| 118.4 | The same with the transfers-only user (`into = "sftp://xfer@host/incoming"`) and `verify = "sha256"`; run it | ✕; the reason starts with `相手でハッシュを取れません（読み戻しなら取れます）`; `/incoming` has no `a.csv` and no `a.csv.part`; with `retry = 2` it is tried 3 times |
| 118.5 | The same user with `verify = "readback"`; run it; then a fetch `from = "sftp://xfer@host/incoming/*.csv"`, `into = 'D:\koyomi-test\back'`, `verify = "readback"` | Both ✓ with `(sha256 …)` on each `sent:` and `fetched:` line, the same hash for the same file both ways; `D:\koyomi-test\back` has no `.part` |
| 118.6 | A Windows PC with OpenSSH Server as the far side (`into = "sftp://user@winpc/C:/in"` or the home), `verify = "sha256"`; run it | ✓; the hash in the log is the one `Get-FileHash C:\in\a.csv` prints there (in lower case) |
| 118.7 | 117.2's send to MinIO with `verify = "sha256"`; then 117.4's fetch with `verify = "readback"`; then the fetch again of a key put from the MinIO console (no checksum) | ✓ both times with `(sha256 …)` on each line; the third logs `  <name>: 接続先にチェックサムがないため、照合していません` under that file and is still ✓ |
| 118.8 | 118.2 with `from = 'D:\koyomi-test\out\big.zip'`; when `big.zip.part` on the share stops growing (koyomi is hashing the two files), run `Add-Content \\10.0.0.1\Backup\koyomi-test\big.zip.part x` in another window at once | ✕ with the reason `届いた中身が違います: big.zip（こちら …、相手 …）`; the share has neither `big.zip` nor `big.zip.part`; with `retry = 1` the second try is ✓ |
| 118.9 | `koyomi history --files`; then `--files --toml`; then `--files --csv D:\koyomi-test\files.csv`; then `koyomi history --files <a net that moves nothing>` | One line per file: run, net, job, `<name>  <n> バイト  sha256 <hex>  ✔ 接続先と一致しました` (or `✖ 接続先と一致しませんでした` for 118.8's); the TOML is `[[file]]` tables with `run`, `jobnet`, `job`, `ended`, `name`, `bytes`, `sha256`, `matched`; the CSV's header is `実行,ジョブネット,ジョブ,終了,ファイル,バイト,SHA-256,照合` and Excel opens it; the last says `送受したファイルはありません` |
| 118.10 | Open 118.3's run in the window's 履歴, then the job's detail | A `送受したファイル` row for each file with its bytes, its hash and `✔ 接続先と一致しました`; a run of a job without `verify` shows the files with no hash and no mark |
| 118.11 | The editor: open a 「ファイルを送る」, set 届いた内容を確認する to `sha256`, then 読み戻す, with 送り先 a shared folder, then `sftp://…`, then `s3://…`; save | The note under the choice changes with each (両方のファイルの SHA-256 をこのマシンで比較し…, 接続先で sha256sum…, 送信・受信したあと、接続先のファイルを読み戻して比較します…, S3 が SHA-256 のチェックサムで確認します…); the approve box has a `確認方法: readback` line; the file has `verify = "readback"`; しない removes the line |

## 119. Start on a sign-in, a sign-out, a lock or an unlock (`[[when]] session`; TODO 115)

Two nets with one job each, approved: 保守/ロック with `[[when]] session = "lock"` and the job `cmd /c echo locked`, and 保守/解除 with `session = "unlock"` and `cmd /c echo unlocked`. Run the engine as you first (`koyomi engine start`), then as the service (§89) where the steps say so. For 119.5, a second Windows account on this PC.

| # | Do | Expect |
|---|---|---|
| 119.1 | Win+L; wait ten seconds; sign back in | 保守/ロック ran once and 保守/解除 ran once; `koyomi history` gives the reasons `<you> が画面をロックしました` and `<you> が画面のロックを解除しました` (`<you>` your account as `PC\name`), and each run's detail in the window says the same |
| 119.2 | Make 保守/ロック's job `cmd /c ping -n 120 127.0.0.1 > nul`; lock, sign back in, and lock again within two minutes | One run; a ⊘ スキップ row `<you> が画面をロックしました ただし、前の回がまだ実行中のため、実行しませんでした。`; nothing starts after the first run ends |
| 119.3 | Add 保守/入る with `session = "sign_in"` and 保守/出る with `session = "sign_out"`; approve them (the engine as you) | The window's 注意 says `KOYOMI-W3269 ジョブネット 保守/入る は、session = "sign_in" では実行されません。サインインのたびに実行するなら、on_start = true を指定してください。…` and `KOYOMI-W3270 ジョブネット 保守/出る は、session = "sign_out" では実行されません。サービスのエンジンで実行してください。`; no warning for 保守/ロック or 保守/解除 |
| 119.4 | Run the engine as the service (§89); repeat 119.1 | The same two runs; no 注意 for any of the four nets |
| 119.5 | The service: sign out, then sign back in; then sign in as the second account (切り替え, or Win+L → 他のユーザー) and sign it out | 保守/出る ran for each sign-out and 保守/入る for each sign-in, the second account's too (the reasons `<account> がサインアウトしました` and `<account> がサインインしました`, each naming whose it was); a lock of the second account starts 保守/ロック as well |
| 119.6 | The service with §115's 保守/空き (`idle = "1m"`) and a net 保守/静か with `require = ["idle:5m"]`; run 保守/静か now while you use the PC, then leave the PC alone; then sign out as the next column says | 保守/空き runs after a minute; 保守/静か waits with `PC が 5 分間使用されなくなるのを待っています（require = "idle"）` while you use the PC and starts once it has been left alone five minutes. Then sign out, stay signed out three minutes and sign back in: 保守/空き did not run while nobody was signed in at the screen (no run from then in `koyomi history`); it runs again once you have used the PC and left it alone a minute |
| 119.7 | [x] `koyomi check` with `session = "logon"`; with `session = "lock"` and `at = ["02:00"]`; with `session = "unlock"` and `on_closed = "prev"` | `KOYOMI-E3306 session の値が正しくありません。session には、"sign_in", "sign_out", "lock", "unlock" のいずれかを指定してください。`; `KOYOMI-E3332 session の [[when]] では、at を指定できません。…`; `KOYOMI-E3333 session の [[when]] では、on_closed に "prev" と "next" を指定できません。on_closed には "skip" または "run" を指定してください。` |
| 119.8 | The editor: add a `[[when]]` of サインイン・ロック; choose each of できごと's four | One choice できごと (サインイン・サインアウト・ロック・ロックの解除), set to ロック at first; except / start / end / on_closed are not shown; saving writes `session = "lock"` (or the one chosen) |
| 119.9 | [x] `koyomi next 保守/ロック`; approve it after a change | The row `画面をロックしたとき` comes first, with no time; the confirm box shows the same |
| 119.10 | (the import of hand-written XML of the same form checked 2026-11-03, in an engine of its own; a task made in taskschd.msc not yet) In taskschd.msc, make a task `\koyomi-test\ロック` with the trigger ワークステーション ロック時 (任意のユーザー) and the action `cmd.exe /c echo hi`; `schtasks /query /xml /tn "\koyomi-test\ロック" > l.xml`; `koyomi import l.xml --dry-run`, then `koyomi import l.xml` and `koyomi show koyomi-test/ロック`; then the same with 特定のユーザー and ワークステーション ロック解除時 | The plan says `追加: ジョブネット koyomi-test/ロック` with only the note `KOYOMI-W3505 タスクの実行ユーザーは取り込まれません。ジョブは、koyomi のエンジンを実行しているユーザーで実行されます。`; the file it writes has `session = "lock"` and `koyomi show` says `画面をロックしたとき`; the second is §120.8; a trigger of リモート接続時 gives the note `KOYOMI-W3514 <SessionStateChangeTrigger> の「RemoteConnect」は取り込めません。取り込めるのは、ロックとロック解除だけです。` and the net comes in with no time (`KOYOMI-W3506 時刻を 1 つも取り込めませんでした。取り込んだジョブネットは、手動で実行するだけになります。`); then delete the task and the net |
| 119.11 | `koyomi history --csv runs.csv` after 119.1 | The runs' きっかけ is `サインイン・ロック` |

## 120. Only one person's sign-in or lock (`[[when]] session` with `user`; TODO 117)

§119's 保守/ロック with `user = '<you>'` added (`<you>` your account as `PC\name`), and 保守/他人 with `[[when]] session = "lock"`, `user = '<other>'` (the second Windows account of §119.5, as `PC\name`) and the job `cmd /c echo other`; both approved.

| # | Do | Expect |
|---|---|---|
| 120.1 | [x] The engine as you (`koyomi engine start`); then approve another net, and run one now | The window's 注意 (and `koyomi list`'s `注意:` line) says `KOYOMI-W3271 ジョブネット 保守/他人 は、user = '<other>' のロック・解除では実行されません。サービスのエンジンで実行してください。`; none for 保守/ロック; it is not in the 警告 band, and approving and 今すぐ流す still work for every net (`koyomi approve` too). (checked 2026-11-03 with the CLI, a made-up `<other>`, in an engine of its own) |
| 120.2 | Win+L; wait ten seconds; sign back in | 保守/ロック ran once with the reason `<you> が画面をロックしました`; 保守/他人 did not run |
| 120.3 | The service (§89): lock yours and sign back in; then sign in as the second account, lock it and sign back in | 保守/ロック ran for your lock only and 保守/他人 for the second account's only, its reason `<other> が画面をロックしました`; no 注意 for either |
| 120.4 | The service: make 保守/ロック's `user` your name alone in other case (`"TARO"` for `PC\taro`); lock and sign back in | 保守/ロック runs (the name is read in any case, on any domain) |
| 120.5 | [x] `koyomi check` with `user = "taro@corp.example"`; with `user = ""`; with `idle = "10m"` and `user = "taro"` | `KOYOMI-E3347 user の形式が正しくありません。user は 'PC\taro'、'CORP\taro'、"taro" のように指定してください。@ や空白は指定できません。` for the first two; `KOYOMI-E3332 idle の [[when]] では、user を指定できません。…` |
| 120.6 | The editor: a `[[when]]` of サインイン・ロック; type `PC\taro` in ユーザー and save; then type `taro@x` | The field reads ユーザー（PC\taro。空ならだれでも）; saving writes `user = "PC\\taro"`; empty writes no `user`; `taro@x` is refused with `KOYOMI-E3347 user の形式が正しくありません。…` |
| 120.7 | `koyomi next 保守/他人`; approve it after a change | The row `画面をロックしたとき（<other> だけ）`; the confirm box shows the same |
| 120.8 | (the import of hand-written XML of the same form checked 2026-11-03, in an engine of its own; a task made in taskschd.msc not yet) §119.10's task with 特定のユーザー (ワークステーション ロック解除時); `koyomi import l.xml --dry-run`, then `koyomi import l.xml` and `koyomi show koyomi-test/ロック` | The plan has no note about the trigger; the file has `session = "unlock"` and `user = "<PC>\\<name>"`, and `koyomi show` says `画面のロックを解除したとき（<PC>\<name> だけ）`; a UserId given as a SID (`S-1-5-21-…`) leaves `user` out with the note `KOYOMI-W3515 <PC>\<name> に限ったトリガーですが、この形式は読み込めないため、session = "unlock" はどのユーザーでも実行されます。user = 'PC\名前' を追記してください。` |

## 121. Engine on Debian, operated from Windows

Run on a Debian 12 or 13 machine or VM with systemd, reachable over TCP from this PC, with the package from `scripts/package-linux.sh` unpacked there. The `view`, `run` and `edit` tokens are added by hand as in `packaging/linux/README.md` (人を足す). 保守/夜間 is an approved net with one job, `sleep 600`. `<addr>` is one address of the machine; the port is 8790 unless a row says otherwise. `koyomi list` and `koyomi history` print in the language of common.toml, else the OS's: on a machine set to English, run them with `LC_ALL=ja_JP.UTF-8` to read the words quoted below.

| # | Do | Expect |
|---|---|---|
| 121.1 | On a fresh machine (no user `koyomi`, no `/var/lib/koyomi`), `sudo ./install.sh` from the unpacked folder | Prints `利用者 koyomi を作成しました。`, `admin の token を作成しました。`, `設定を作成しました: /var/lib/koyomi/.config/uchmk/koyomi/koyomi.toml (port 8790)`, then `インストールが完了しました。`; one `https://<addr>:8790` line per address; the fingerprint; `sudo cat /etc/koyomi/credentials/http-admin` as where the token is; exit 0 |
| 121.2 | `systemctl is-active koyomi`; then `sudo -H -u koyomi koyomi status` | `active`; `koyomi status` answers without an error |
| 121.3 | `sudo stat -c '%a %U:%G' /etc/koyomi/credentials/http-admin` | `600 root:root` |
| 121.4 | With a job running, `sudo ./install.sh` again | Prints `admin の token は既にあります。変更しません。`, `koyomi を停止します。実行中のジョブは停止します。` and `設定は既にあります。変更しません:`; the token (`sudo cat`) and `koyomi.toml` (`sha256sum`) are unchanged; `systemctl is-active koyomi` is `active` again |
| 121.5 | On a fresh machine, `sudo ./install.sh --port 9000` | The URLs show `:9000`; `koyomi.toml` has `port = 9000` and `lan = true` |
| 121.6 | Where it is installed with port 8790, `sudo ./install.sh --port 9001` | Prints `--port は初回だけ有効です。現在の koyomi.toml の port を使います。`; `koyomi.toml` still has `port = 8790` |
| 121.7 | `sudo ./install.sh --uninstall` | `/etc/systemd/system/koyomi.service`, `/usr/local/bin/koyomi` and `/usr/local/sbin/koyomi-secret` are gone; `/var/lib/koyomi`, `/etc/koyomi/credentials/http-admin` and the user `koyomi` are still there; the output names them and the `sudo rm -rf …` command, which it does not run |
| 121.8 | Install again; pipe `test-1` with no newline into `sudo koyomi-secret http-test` with the engine running | Prints `書き込みました: /etc/koyomi/credentials/http-test`, `koyomi を再起動します。実行中のジョブは停止します。` and `再起動しました。`; `sudo cat` of the file prints `test-1` with no newline |
| 121.9 | `sudo koyomi-secret http-prompt` in a terminal; type `abc` at `http-prompt の値: ` and press Enter | Nothing shows while typing; `sudo cat` of the file prints `abc` |
| 121.10 | `sudo stat -c '%a %U:%G' /etc/koyomi/credentials/http-test` | `600 root:root` |
| 121.11 | Pipe nothing into `sudo koyomi-secret http-empty`; then pipe `x` into `sudo koyomi-secret 'bad name'` | First: `値が空です。何も書き込みませんでした。値を指定してください。`, exit 1. Second: `名前が使えません:`, exit 2. No file is written in either case |
| 121.12 | `sudo koyomi-secret http-test secret-value` | Prints the usage, exit 2; the file is unchanged |
| 121.13 | With 保守/夜間 running (`sudo -H -u koyomi koyomi run 保守/夜間`), `time sudo systemctl stop koyomi` | Returns within 30 seconds; `systemctl is-active koyomi` is `inactive`; no `sleep 600` is left (`pgrep -a sleep`); the unit does not restart |
| 121.14 | After 121.13, `sudo systemctl start koyomi`, then `sudo -H -u koyomi koyomi history` | The run is over, not listed as running; its job is written as `強制終了`, or as `エンジンが停止しました` when it did not stop within the wait |
| 121.15 | `sudo -H -u koyomi koyomi engine stop`; wait 15 seconds; then `sudo systemctl start koyomi` | After the stop the unit stays `inactive` (no automatic restart); after the start it is `active` |
| 121.16 | `sudo systemctl restart koyomi` | `active` within 10 seconds; `journalctl -u koyomi -n 20 --no-pager` shows no error |
| 121.17 | `curl -sk -X POST https://<addr>:8790/door -H "Authorization: Bearer <view token>" -H "Content-Type: application/json" -d '{"proto": 45, "request": "Status"}' -w ' %{http_code}\n'` (`-k`: the engine's own certificate) | `200`; the body is `{"ok": true, "text": …}`. The rows below change the token and the `request`: a request with no fields is a string (`"Status"`), one with fields an object (`{"Hold": {"id": "保守/夜間", "job": null}}`) |
| 121.18 | `view` token, `{"Hold": {"id": "保守/夜間", "job": null}}` | `403`; the body says `KOYOMI-E2201 利用者 <name> のロールは view です。この操作には run ロールが必要です。…` |
| 121.19 | `run` token, the same `Hold`; then `{"Release": {"id": "保守/夜間", "job": null}}` | `200` with `"ok": true` both times |
| 121.20 | `run` token, `"List"` | `403`; the body says `KOYOMI-E2201 利用者 <name> のロールは run です。この操作には edit ロールが必要です。…` |
| 121.21 | `edit` token, `"List"` | `200` with `"ok": true` |
| 121.22 | `edit` token, `"Pause"` | `403`; the body says `KOYOMI-E2201 利用者 <name> のロールは edit です。この操作には admin ロールが必要です。…`; the engine keeps running |
| 121.23 | `admin` token, `"Pause"`, then `"Resume"` | Both `200` with `"ok": true`; nets start again after `"Resume"` |
| 121.24 | `"Stop"` with each of the `view`, `run`, `edit` and `admin` tokens | `403` every time; the body says `KOYOMI-E2207 利用者 <name> は、この操作を /door から実行できません。…`; `systemctl is-active koyomi` is still `active` |
| 121.25 | `"Status"` with no `Authorization` header, then with `Bearer nope` | `401` both times |
| 121.26 | `admin` token, body `{"proto": 44, "request": "Status"}` | `409`; the body has `"proto": 45` and a `"version"`, and `"error"` says `KOYOMI-E2108 画面とエンジンのバージョンが一致しません。（画面 44、エンジン 45）…` |
| 121.27 | `edit` token, `{"Propose": {"id": "保守/夜間", "text": "<the net with one command changed>", "base_sha256": "<sha256 of the current text>"}}` | `200` with `"ok": true`; `sudo -H -u koyomi koyomi list` shows the net `▲ 未確認` |
| 121.28 | `run` token, `{"Approve": {"id": "保守/夜間", "sha256": "<sha256 of the proposed text>"}}` | `403`; the body says `KOYOMI-E2201 利用者 <name> のロールは run です。この操作には admin ロールが必要です。…`; the net stays `▲ 未確認` |
| 121.29 | `admin` token, the same `Approve` | `200` with `"ok": true`; `koyomi list` no longer shows `▲ 未確認` for the net; `sudo -H -u koyomi koyomi ops` has an approve line that names `admin` |
| 121.30 | Propose another change, then `admin` token, `Approve` with 64 zeros as `sha256` | `200` with `"ok": false` and an error; the net stays `▲ 未確認`; no new approve line in `koyomi ops` |
| 121.31 | `admin` token, `{"CalendarPut": {"name": "Office", "text": "format = 1\n", "base_sha256": null}}` | `200` with `"ok": true`; `calendars/Office.toml` has that text |
| 121.32 | `edit` token, `{"CalendarText": {"name": "Office"}}` | `200` with `"ok": true`; the text holds the calendar and its `sha256` |
| 121.33 | `admin` token, `CalendarPut` with `"name": "office"` and `"text": "format = 1\nweekly_off = [\"sun\"]\n"` | `200` with `"ok": true`; `calendars/Office.toml` and `calendars/office.toml` both exist |
| 121.34 | `admin` token, `{"CalendarRename": {"from": "Office", "to": "office"}}` | `200` with `"ok": false`; the error is `KOYOMI-E4101 カレンダー office はすでに存在します。…`; both files are unchanged (`sha256sum` before and after) |
| 121.35 | `admin` token, `CalendarPut` of `Office` with a wrong `base_sha256`, then with the current `sha256` | First: `"ok": false`, the error says `KOYOMI-E4105 カレンダー Office は、ほかの場所で変更されています。…`, the file is unchanged. Second: `"ok": true`; the old text is kept under `versions/_calendars/Office/` in the engine's state folder |
| 121.36 | `admin` token, `{"CalendarDelete": {"name": "office", "base_sha256": "<wrong hash>"}}`, then with the current hash | Wrong hash: `"ok": false`, the error says `KOYOMI-E4105 カレンダー office は、ほかの場所で変更されています。…`, the file stays. Current hash: `"ok": true`; `calendars/office.toml` is gone; its old text is kept under `versions/_calendars/office/` |

## 122. The whole window on another PC's engine (link, status-bar list)

Needs a second machine with a koyomi engine reachable over HTTPS: §121's Debian machine, or another Windows PC running `koyomi engine` with its HTTP port open. In this PC's koyomi.toml, a `[[remote]]` named `kei` with its `url`, `fingerprint` and `token` (the token set with `koyomi secret set kei-token`). On the other machine, add these tokens by hand (as in `packaging/linux/README.md`): `view`, `run`, an `edit` narrowed with `nets`, an `edit` without narrowing, and `admin`. A row starts with the window closed unless it says otherwise. To pick a link is to open the list at the left of the status bar (the button with a dot, the link's name and ▴) and press 接続 on that link's row (切断 on the current `[[remote]]`, which goes to `この PC`); §124 has the list itself.

| # | Do | Expect |
|---|---|---|
| 122.1 | No `link` in `state.toml` (or the file absent); open the window; open the status-bar list | The list has the rows `この PC` and `kei`; the title is `koyomi` with no role after it; the tree and the runs are this PC's |
| 122.2 | Pick `kei` | The title reads `koyomi - kei (admin)` (the role of the token; `edit` for an `edit` token); the status bar shows `kei (admin)`; the tree lists the nets of the other machine (`koyomi list` there agrees); the runs are that engine's |
| 122.3 | With `kei` picked, close the window and open it again; then pick `この PC`, close and open again | After the first reopening the title and the status-bar button still say `kei` (`state.toml` has `link = "kei"`); after the second, the title is `koyomi` and the tree is this PC's |
| 122.4 | Set `link = "nope"` in `state.toml`; open the window | The tree says `KOYOMI-E2116 接続先 nope は koyomi.toml の [[remote]] に存在しません。（登録済み: kei）…`; the status bar says `エンジンに接続できません`; no `koyomi engine` process starts on this PC (Task Manager); the list still has the rows `この PC` and `kei` |
| 122.5 | Pick `kei` with the `view` token; then with the `run` token; then with the narrowed `edit` token (one attempt each, the secret changed between them) | Each time the tree says `KOYOMI-E2114 接続先 kei のトークンでは、画面を操作できません。…`; no net or run of `kei` is shown; no engine starts on this PC |
| 122.6 | With the unnarrowed `edit` token, pick `kei`; open a net, change one line, press 保存 | The editor's result line reads `ジョブネット <id> を配置しました（未確認です。確認するまで実行されません）。` (the engine's text, not changed by the window); on the other machine `koyomi list` shows that net as ▲ 未確認 |
| 122.7 | With the `admin` token, pick `kei`; open a net, change one line, press 保存 | The result line reads `ジョブネット <id> を保存しました。`; on the other machine `koyomi list` does not show that net as ▲ 未確認 |
| 122.8 | With the `edit` token, pick `kei`; open a net and change it in the window; on the other machine change that net's file by hand; press 保存 in the window | A box `ほかで変更されています` with two buttons, 読み直す and キャンセル; there is no 上書きする button; 読み直す loads the file on the other machine |
| 122.9 | With `kei` picked and a net open in the editor, stop the engine on the other machine (`koyomi engine stop`, or `sudo systemctl stop koyomi`); wait 10 seconds; press 保存 | The status bar reads `エンジンに接続できません`; the tree shows a line that begins with `KOYOMI-E2101 接続先 kei に接続できません。（…）`; the save's result line starts with `KOYOMI-E2101 接続先 kei に接続できません。` (the window already knows the engine is away, so nothing is sent and the line does not say `届いたかどうかを確認できません`); no engine starts on this PC |
| 122.10 | After 122.9, start the engine on the other machine; then set kei's token to a wrong value and wait about 10 seconds; then set it to the right value | With the wrong value the tree says `KOYOMI-E2107 接続先 kei がトークンを受け付けません。…`; with the right value, within about 10 seconds the status bar and the tree come back with no window restart |
| 122.11 | Pick `kei`; open a net, change it without 保存; pick `この PC` | A box `接続先を切り替える` with the text `編集中のジョブネットに、保存していない変更があります。切り替えると破棄されます。` and the buttons 切り替える and キャンセル; キャンセル keeps the window on `kei` with the edits; 切り替える switches and the edits are gone |
| 122.12 | Pick `kei`; change a net, press 保存 and at once pick `この PC` (the write is still on its way, so the status bar shows `送信中 1 件`) | First the box `接続先を切り替える` (the change counts as unsaved until the answer comes); its 切り替える brings the box `切り替えを待っています` with `送信中 1 件。終わるのを待っています` and キャンセル; キャンセル stays on `kei` and the save's result comes in. If the boxes never show (the answer came first), record this row as not reached |
| 122.13 | On this PC, make a run fail (a net whose job is `cmd /c exit 1`), acknowledge it with 対応済みにする; pick `kei`, then pick `この PC` again | While on `kei`, this PC's failure is neither in the bell nor counted in the status bar; back on this PC the bell reads `通知はありません。` (it is emptied at every switch and not filled again), the status bar does not count the failure, and it stays acknowledged (kept in `acked.toml`, with no file for `kei` until a run there is acknowledged) |
| 122.14 | Pick `kei`; from the tray press 未対応を見る, then 次の 1 回だけスキップ…, then press 開く on a run's notification, one at a time; then from the tray press 今日の一覧 | Each of the first three brings the window to the front and opens nothing for `kei`; 今日の一覧 opens 今日 as before |
| 122.15 | Pick a `kei` whose clock is in another time zone (on Debian, `timedatectl set-timezone` to another zone) | The band at the top reads `エンジンと時刻帯が異なるため、今日の表示がずれます。`; with `kei` in the same zone as this PC the band is not there |
| 122.16 | On `kei`, a net with a warning that bears on it (`koyomi list` on the other machine shows its 注意); pick `kei` | The band reads `他のユーザーが書き換えられる場所があります。修正するまで、何も確認できません。`; approving that net is refused, as it is on this PC |
| 122.17 | Pick `kei`; palette → `書き出す（全部を 1 つのファイルに）` | The result line reads `書き出しました: <this PC's state folder>\exports\<YYYYMMDD-HHMMSS>.toml（N 件。webhook の宛先は *** にしました）`; the file is in this PC's state folder and holds the nets of `kei`, not of this PC |
| 122.18 | Link `この PC`; stop the `koyomi` service (§89) so that `koyomi service` shows it stopped, with the engine stopped; palette → `書き出す（全部を 1 つのファイルに）` | The result line reads `KOYOMI-E1062 サービスのエンジンが停止しています。koyomi service start を実行して、サービスを開始してください。`; no file is written. (Without a service, the window starts the engine and writes the file, so this row does not apply there.) |
| 122.19 | With `kei` picked and a net open in the editor, freeze the engine on the other machine (`kill -STOP` its process); press 保存; wait for the request's 30-second limit; then `kill -CONT` it | The save's result line begins with `KOYOMI-E2102 要求が接続先に届いたかどうかを確認できません。（…）` and its action says to check the history first (the save had already left, so the window cannot tell whether it arrived) |
| 122.20 | As 122.8, on a `kei` link | The box's text reads `KOYOMI-E3104 開いたあとに、このファイルがほかの場所で変更されています。読み込み直すと、ここでの変更は破棄されます。` and does not mention 上書き. On this PC the same box keeps its 上書きする button and its text about it |
| 122.21 | This PC: the 今日 page with no row picked; palette → `書き出す（全部を 1 つのファイルに）`; then the same on ジョブネット with no net picked | Each time the result line `✔ 書き出しました: …` is on the page, above the table or the overview, and it is drawn once. With a row picked on 今日 and the inspector open, the line is under the inspector's buttons instead, still once |

## 123. Calendars through the engine, adding a connection, settings on another PC

Rows marked `kei` need §122's setup: another machine's engine (§121's Debian machine, or another Windows PC running `koyomi engine`), with its `admin` and unnarrowed `edit` tokens. Every other row needs only this PC. A row starts with the window closed unless it says otherwise. To toggle a day is to double-click it (or use its right-click menu); a single click only picks it.

| # | Do | Expect |
|---|---|---|
| 123.1 | This PC, engine running: pick 会社 under カレンダー; toggle a weekday in the month, then toggle it again | The day reads 休業 after the first toggle and is back after the second; the line under the month reads `カレンダー calendars/会社.toml を書き込みました。` each time; `calendars/会社.toml` gains and then loses that day in `closed` |
| 123.2 | This PC: right-click a weekday under 会社; 名前を付ける…; type 創立記念日; 決定する | The field under the month shows the day's name; the day is in `closed` under that name in `calendars/会社.toml`; the line reads `カレンダー calendars/会社.toml を書き込みました。`; the day stays closed |
| 123.3 | This PC: change one of the rules above the month (the holiday or weekly-off boxes) | The line reads `カレンダー calendars/会社.toml を書き込みました。`; the file's `holidays` or `weekly_off` is the new value |
| 123.4 | This PC: with the month on screen, add `open = [2026-11-28]` to `calendars/会社.toml` by hand; then toggle a weekday | The hand-made open day stays; the toggled weekday is closed; nothing is refused (the change is made on the text as it is at the press) |
| 123.5 | This PC: 削除… on 会社, then 削除する | The line reads `カレンダー 会社 を削除しました（以前の内容は versions/_calendars/会社 に残しました）。`; `calendars/会社.toml` is gone; `versions/_calendars/会社/<YYYYMMDD-HHMMSS>.toml` holds its text |
| 123.6 | This PC: ＋ 新しいカレンダー; name 工場; 作成する | The box closes and the month of 工場 opens with `カレンダー calendars/工場.toml を書き込みました。`; `calendars/工場.toml` has `format = 1`. While the answer is on its way the name field, 作成する and キャンセル are off, and Enter and Esc do nothing. If the answer comes before you can see that, record this part as not reached |
| 123.7 | This PC: ＋ 新しいカレンダー; name 会社 | The box shows `KOYOMI-E4101 カレンダー 会社 はすでに存在します。…`; the box stays; no file is written |
| 123.8 | This PC: 名前の変更… on 工場; name 倉庫; 変更する | The line reads `カレンダーの名前を 工場 から 倉庫 に変更しました。`; `calendars/工場.toml` is gone; `calendars/倉庫.toml` has the same text |
| 123.9 | This PC: 名前の変更… on 倉庫; name 会社; 変更する | The box shows `KOYOMI-E4101 カレンダー 会社 はすでに存在します。…`; nothing is renamed |
| 123.10 | This PC: 祝日の表を取り込む…; paste the path of `syukujitsu.csv` with its quotes; Enter | The line reads `祝日の表を取り込みました（calendars/syukujitsu.csv、<first> 年から <last> 年まで）。`; `calendars/syukujitsu.csv` is the file byte for byte; the old one is in `versions/_calendars/syukujitsu/<YYYYMMDD-HHMMSS>.csv` |
| 123.11 | This PC: 祝日の表を取り込む… with a CSV that has a bad date (`x,y` on a line) | The field shows `KOYOMI-E4150 祝日の表の <n> 行目にある日付 <x> を読み込めません。…`; `calendars/syukujitsu.csv` is unchanged |
| 123.12 | This PC: stop the `koyomi` service (§89) so that `koyomi service` shows it stopped, with the engine stopped; toggle a day | The line reads `KOYOMI-E1062 サービスのエンジンが停止しています。koyomi service start を実行して、サービスを開始してください。`; no file under `calendars/` changes. (Without a service the window starts the engine, so this row does not apply there.) |
| 123.13 | kei, `admin` token: pick kei; toggle a weekday in 会社 | The line reads `カレンダー calendars/会社.toml を書き込みました。`; `koyomi calendar 会社 --month <YYYY-MM>` on kei shows the day closed; this PC's `calendars/会社.toml` has the same sha256 as before |
| 123.14 | kei, `admin` token: on kei, do each in turn: the inline name, 名前の変更… to 倉庫, ＋ 新しいカレンダー 工場, 削除… on 倉庫 with 削除する, 祝日の表を取り込む… | Each line is the engine's own text (as in 123.2, 123.8, 123.6, 123.5, 123.10); the files appear on kei; this PC's `calendars/` has none of them |
| 123.15 | kei, unnarrowed `edit` token: pick kei; toggle a weekday; then import a holiday table; then ＋ 新しいカレンダー with name 工場 and 作成する | Each says `KOYOMI-E2201 利用者 <利用者> のロールは edit です。この操作には admin ロールが必要です。…`, where `<利用者>` is kei's `[[http.user]]` name, followed by the action `接続先の管理者に、ロールの変更を依頼してください。`; kei's files have the same sha256 as before; the box shows the sentence and stays open |
| 123.16 | kei, `admin` token: open 会社 on the window; by hand on kei add `closed = [2026-11-25]` to `calendars/会社.toml`; in the window, toggle a weekday | The line reads `カレンダー calendars/会社.toml を書き込みました。`; kei's file has both the hand-made closed day and the toggled day; no refusal is shown |
| 123.17 | kei, `admin` token: as 123.16, then 削除… on 会社 with 削除する | The line reads `カレンダー 会社 を削除しました（以前の内容は versions/_calendars/会社 に残しました）。`; kei's `versions/_calendars/会社/` holds the file with the hand-made day |
| 123.18 | kei, `admin` token: create `calendars/工場.toml` by hand on kei; in the window, at once (before the list shows it) ＋ 新しいカレンダー, name 工場, 作成する | The box shows `KOYOMI-E4101 カレンダー 工場 はすでに存在します。…` and stays open with the name (the engine's answer and the window's own check now read the same, so the row can no longer tell whether the list had updated) |
| 123.19 | This PC, no `[[remote]]` in koyomi.toml (or a copy of it without kei): palette → 別の PC のエンジンに接続する | The status bar shows the link button `この PC ▴` (see 124.1); the box titled `別の PC のエンジンに接続する` opens with the lead `接続先の名前と、接続先の koyomi の URL（https://…）を指定してください。確認が終わるまで、何も書き込みません。`, fields 名前 (hint `例: 工場`) and URL, and the buttons 次へ and キャンセル |
| 123.20 | This PC, with kei in koyomi.toml: the status-bar list, last row 接続先を管理…; in the box 「接続先の管理」 press 接続先を追加する…. Then close the box and open the palette → 別の PC のエンジンに接続する | From the manage box: it closes and the same add box opens. The palette route opens the add box directly |
| 123.21 | This PC: in the add box, one at a time: empty name and 次へ; name `kei`; name `a b`; name `factory` with URL `http://x:8443`; name `factory` with URL `https://` alone; URL `https://a b` | The box shows `KOYOMI-E2130 接続先の名前が入力されていません。…`; `KOYOMI-E2131 接続先 kei はすでに存在します。…`; `KOYOMI-E2132 接続先の名前 a b は使用できません。…`; `KOYOMI-E2133 接続先の URL が正しくありません。…` for the last three; `接続先の指紋を読み取っています…` does not appear; koyomi.toml is unchanged |
| 123.22 | kei: name `factory`; URL of the other PC's engine (https://host:port); 次へ | The box shows `接続先の指紋を読み取っています…`, then the fingerprint in bold, and `接続先の PC で `koyomi http fingerprint` を実行して、同じものが表示されることを確認してください。異なる場合は、別の PC に接続しています。`, with the buttons 同じでした and キャンセル. There is no token field yet. Esc closes the box; koyomi.toml is unchanged (sha256 before and after) |
| 123.23 | kei: as 123.22, but press Enter at the fingerprint step, then click 同じでした | Enter does nothing; the click brings the masked token field; nothing is written yet |
| 123.24 | kei: at the token step, leave the token empty; 接続する | The box shows `KOYOMI-E2134 接続先のトークンが入力されていません。…`; nothing is written |
| 123.25 | kei: at the token step, type a wrong token; 接続する | The box shows `KOYOMI-E2107 接続先 factory がトークンを受け付けません。…`; the token field is empty; the box stays at the token step; koyomi.toml has no `factory` block |
| 123.26 | kei: at the token step, type the right token; 接続する | The box closes; the window goes to `factory` (status bar and title); koyomi.toml has one new `[[remote]]` block with name `factory`, its url, the fingerprint shown in 123.22, and `token = "secret:factory-token"`; the comments, `[vars]` and the kei block are unchanged |
| 123.27 | This PC, after 123.26: palette → 別の PC のエンジンに接続する; name `factory` | The box shows `KOYOMI-E2131 接続先 factory はすでに存在します。…` |
| 123.28 | kei: by hand, give an existing `[[remote]]` the line `token = "secret:works-token"` and store its token with `koyomi secret set works-token`; add a connection named `works` with the right token; 接続する | The box shows `KOYOMI-E2135 シークレット works-token は、ほかの接続先が使用しています。…`; koyomi.toml does not change; the existing connection still works |
| 123.29 | kei, any token: the settings screen, General page | The 記録 section is gone; one line `接続先の koyomi.toml を修正する` is in its place; the koyomi.toml button is gone; the language, theme and keys cards are as before |
| 123.30 | kei: the Notifications page | Only the one line `接続先の koyomi.toml を修正する` is on the page |
| 123.31 | kei: the Advanced page | The line `接続先の koyomi.toml を修正する`; the config folder's button is there and opens this PC's config folder; there is no state folder row; the reset row is there and works as at home (pressed twice) |
| 123.32 | kei: the settings search, one at a time: `keep_days`, `log_max_mb`, `[notify]`, `keymap.toml` | The first three find nothing; `keymap.toml` finds the Keys page |
| 123.33 | This PC: the settings screen; the search for `keep_days` | The 記録 section is there, with its rows; the Notifications page has its rows; the koyomi.toml button is on the General page; `keep_days` finds a row |
| 123.34 | kei, `admin` token: open ＋ 新しいカレンダー and type a name; leave the box open and, while a switch waits (`送信中 1 件`, as 122.12), press 作成する | The box shows `KOYOMI-E2117 接続先を切り替えている最中のため、要求を送信していません。…` and its name stays; the result line reads the same `KOYOMI-E2117` text; no file changes on kei. If the switch ends before you can press, record this row as not reached |
| 123.35 | A calendar page with a caution band showing, the window at its default size (1280×800): right-click a weekday; 名前を付ける…; 決定する; then 祝日の表を取り込む… with a bad path | The field with 決定する and the result line stay in view at the bottom of the page with no scrolling; the month scrolls above them; with neither on the page the foot takes no room |

## 124. The link list in the status bar (`ui/hosts.rs`)

Rows marked `kei` need §122's setup: another machine's engine reachable over HTTPS, as a `[[remote]]` named `kei`. Rows marked `two` also need a second `[[remote]]` named `works` (a second machine, or a copy of kei's block under that name). Every other row needs only this PC. A row starts with the window closed on `この PC` unless it says otherwise. The look (spacing, colours, the popup above the bar) is read by eye; this section does not measure it.

| # | Do | Expect |
|---|---|---|
| 124.1 | This PC, no `[[remote]]` in koyomi.toml, engine running | The status bar shows the link button `この PC ▴` (see 124.19) |
| 124.2 | kei: click the button at the left of the status bar (dot, `この PC`, ▴) | A list opens above the button: the heading `接続先`, the rows `この PC` and `kei`, then a line, then `接続先を管理…`; the button's dot is green |
| 124.3 | kei: with the list open, click outside it; open it again and press Esc | Each closes the list; the window stays on `この PC` |
| 124.4 | kei: with the list open, click the button | The list closes |
| 124.5 | kei, the engine of kei answering: open the list | `この PC` reads `接続中` with a green dot and has no button; `kei` shows a grey dot and `リモート · 確認中`, then within a few seconds a green dot and `リモート · 応答あり`, with the button 接続 |
| 124.6 | kei: press 接続 on the `kei` row | The list closes; the window goes to kei (the title reads `koyomi - kei (…)`); the button reads `kei` with a green dot; opened again, `kei` reads `リモート · 接続中` with the button 切断 and `この PC` reads `この PC のエンジン` with a grey dot and the button 接続 |
| 124.7 | kei, on kei: press 切断 on the `kei` row | The list closes; the window is on `この PC` (the title is `koyomi`) |
| 124.8 | kei, on kei: press 接続 on the `この PC` row | The window goes to `この PC`, as 124.7 |
| 124.9 | kei, on kei: stop the engine on the other machine and wait for the status bar to read `エンジンに接続できません`; open the list | The button's dot is red; the `kei` row reads `リモート · 応答なし` with a red dot and the button 切断; 切断 goes to `この PC` |
| 124.10 | kei, on `この PC`: stop the engine on the other machine; open the list | The `kei` row turns from `リモート · 確認中` to `リモート · 応答なし` with a red dot within about 5 seconds; the window does not freeze meanwhile (scroll the list, move the window, open and close the list); the button 接続 is still there; hovering the row shows the reason |
| 124.11 | kei, on `この PC`: after 124.10 press 接続 on the `kei` row | The window goes to kei and the tree says the reason as a coded line, `KOYOMI-E2101 接続先 kei に接続できません。（…）…`; 切断 is on its row |
| 124.12 | kei, on `この PC`, kei answering: open the list and leave it; stop kei's engine; wait about 40 seconds | The `kei` row changes to `リモート · 応答なし` by itself, without closing the list; start the engine and wait about 40 seconds again: it goes back to `リモート · 応答あり` |
| 124.13 | kei, on `この PC`: close the list; stop kei's engine; wait 40 seconds; open the list | The row is `リモート · 応答なし` within about 5 seconds of opening (a check runs on opening) |
| 124.14 | two: open the list on `この PC` | Both `kei` and `works` get their dot and second line within about 5 seconds each; one that is off does not hold up the other |
| 124.15 | kei, with 10 or more `[[remote]]` blocks in koyomi.toml (copies of kei's under other names): open the list | The rows scroll inside the list (the list stops at about 420 px high); `接続先を管理…` stays in view at the bottom while the rows scroll; the window does not freeze while the dead ones are asked |
| 124.16 | kei: press `接続先を管理…` | The list closes and the box `接続先の管理` opens (§125) |
| 124.17 | kei: stop the `koyomi` service (§89) so that `koyomi service` shows it stopped, with the engine of `この PC` stopped; open the window: the tree shows `KOYOMI-E1062 サービスのエンジンが停止しています。…`; open the list from the status-bar button | The tree has no button of its own; the list is there; the button's dot is red and the `この PC` row reads `応答なし`. (Without a service the window starts the engine, so this row does not apply there.) |
| 124.18 | kei: with the language set to English (Settings → General) | The heading reads `Connections`; the second lines read `Connected`, `This PC's engine`, `Remote · answering`, `Remote · no answer`, `Remote · connected`, `Remote · checking`; the buttons read `Connect` and `Disconnect` |
| 124.19 | This PC, no `[[remote]]` in koyomi.toml: open the window, then press `この PC ▴` in the status bar | The status bar shows `この PC ▴`; the list has the row `この PC` and `接続先を管理…`; pressing `接続先を管理…` opens the box `接続先の管理`, which says `接続先はまだありません。` and offers `接続先を追加する…` |

## 125. Renaming and deleting a connection (`ui/manage.rs`, `remote.rs`)

Rows need this PC with `kei` and `works` in koyomi.toml as `[[remote]]` (a copy of kei's block under another name does; the connection need not answer unless a row says so). Back up koyomi.toml first: these rows change it. A rename changes `name` only; it never touches the token or the credential store. Whether a secret is still used is read from the decoded TOML of every settings file, not from a search of the text. The look (spacing, colours) is read by eye.

| # | Do | Expect |
|---|---|---|
| 125.1 | Open the status-bar list, press `接続先を管理…` | The list closes; the box `接続先の管理` opens in the middle with one row per `[[remote]]` in the order of koyomi.toml, each with `名前を変更する` and `削除する`, and at the bottom `接続先を追加する…` at the left and `閉じる` at the right |
| 125.2 | Palette → `接続先を管理する` | The same box opens. Esc closes it |
| 125.3 | In the box press `名前を変更する` on `works` | The row turns into a field holding `works` with the focus in it, and `変更する` and `キャンセル`; the other rows are unchanged. Esc leaves the editing and keeps the box; Esc again closes the box |
| 125.4 | Rename `works` to `factory`: type it and press Enter | The status line reads `works の名前を factory に変更しました。`; the row now reads `factory`; koyomi.toml has `name = "factory"` in that block and every other line of the block (url, fingerprint, token) as before; the window's PC list shows `factory` |
| 125.5 | Rename `factory` to `Factory` (a case change) | The rename is accepted and written as typed |
| 125.6 | Rename to the same text (with spaces around it) | The editing closes and koyomi.toml is not written (its timestamp is unchanged) |
| 125.7 | Rename to an empty name; to `kei` (another connection); to `a b` | One at a time, the status line reads `KOYOMI-E2130 接続先の名前が入力されていません。…`, `KOYOMI-E2131 接続先 kei はすでに存在します。…`, `KOYOMI-E2132 接続先の名前 a b は使用できません。…`; the field stays and nothing is written |
| 125.8 | With the window on `kei` (press 接続 on its row in the list) open the box | The `kei` row shows the dim text `接続中のため、名前の変更と削除はできません`; `名前を変更する` and `削除する` are disabled and `URL を変更する` works (132.6); the other rows work. Stop kei's engine: the row stays locked while the window is still on kei |
| 125.9 | Add `[standby]` with `watch = "works"` to koyomi.toml, then rename `works` to `factory` | The status line reads `KOYOMI-E2138 接続先 works は [standby] の watch で指定されているため、変更できません。…`; koyomi.toml is as before; the row keeps its name. The same for delete, and for `[primary]` `standby` |
| 125.10 | Press `削除する` on `works` | A question `接続先を削除する` opens: `works を削除します。この PC の koyomi.toml から [[remote]] を削除します。koyomi が作成したシークレット works-token は、ほかから使われていなければ削除します。接続先の PC には何も行いません。` with `削除する` (red) and `キャンセル`; the focus is on `キャンセル` |
| 125.11 | In the question press Enter at once | Nothing is deleted: the question closes and `works` is still there. Open it again: Tab moves the focus to `削除する`, Shift+Tab back; Esc closes the question |
| 125.12 | In the question press Tab, then Enter | The status line reads `works を削除しました。` and nothing more: the secret `works-token` went with it, as the question said (`koyomi secret check works-token` prints `シークレット works-token は存在しません。`); a secret that is kept (125.14, 125.15) or could not be deleted is named in the line instead; the row is gone from the box and the status-bar list; koyomi.toml has no `works` block and its other lines and comments are as before |
| 125.13 | Delete the last connection | The box reads `接続先はまだありません。`; the status-bar link button stays as `この PC ▴`, and its list holds `この PC` and `接続先を管理…` |
| 125.14 | Put a `.toml` file in the settings folder (a job net or calendar) with a string `secret:works-token` in it, then delete `works` | The status line says the secret `works-token` was kept, with `koyomi secret delete works-token` to remove it; `koyomi secret check works-token` prints `シークレット works-token は存在します。` |
| 125.15 | Rename `works` to `factory`, then delete `factory` | The secret `works-token` is not deleted with the connection: the token still reads `secret:works-token`, so the status line names it as kept (its name is not `factory-token`) |
| 125.16 | Delete `works` (its `works-token` is deleted with it); in the credential manager create a secret `works-token` by hand; then add a connection named `works`, to a PC that answers and with its token (the check comes after those) | The add box stops with `KOYOMI-E2136 秘密 works-token はすでに存在します。…`; nothing is written |
| 125.17 | While a delete or rename is carried out (a slow disk, or press and watch), look at the box | A spinner shows, every button is disabled, Esc does nothing, and the list's other connection buttons do not switch the link until it is done |
| 125.18 | Rename `works` after it had acknowledged failures (`確認済み` in its window) | After the rename the acknowledgements of the new name are those of the old one |
| 125.19 | Open the add box, then the manage box through the palette; then the reverse | Only one of the two is open at a time |
| 125.20 | With the box open and a row's name being typed, pick another connection in the status-bar list | The box closes and the window switches; nothing is renamed |
| 125.21 | Rename `works` to `factory`, then press `削除する` on `factory` | The question reads `factory を削除します。この PC の koyomi.toml から [[remote]] を削除します。シークレット works-token は削除しません。接続先の PC には何も行いません。` |
| 125.22 | Write a connection's token as is (`token = "…"`, not `secret:`), then open `接続先を管理…` | The band at the top reads `koyomi が読むファイルに問題があります。修正するまで、何も確認できません。` with `KOYOMI-E1023 [[remote]] <name> の token が、secret:<名前> の形式で指定されていません。…` under it, and no `他のユーザーが書き換えられる場所があります。…`; the box has no rows: it reads `koyomi.toml を読み込めないため、接続先を表示できません。直すまで、名前の変更と削除はできません。` and the same `KOYOMI-E1023 …` line |
| 125.23 | Open the add box with its fields empty; press Ctrl+Shift+P, type `新しいジョブネット` and press Enter | The palette shows over the box with the typed text in its field; after Enter no `新しいジョブネット` box opens; the add box stays and its 名前 field has the focus again |
| 125.24 | Open the `新しいジョブネット` box; then palette → `別の PC のエンジンに接続する`, and palette → `接続先を管理する` | Neither connection box opens; the new job net box stays |
| 125.25 | After 125.22, close the box, write the token as `secret:<name>` again and open the box | The rows are back, each with `名前を変更する` and `削除する`; the `KOYOMI-E1023` line and `koyomi.toml を読み込めないため、…` are gone from the box |

## 126. Top tabs and the calendar switcher

| # | Do | Expect |
|---|---|---|
| 126.1 | Open the window | The tabs at the top read `ジョブネット`, `カレンダー`, `今日`, `履歴` in that order; the window opens on `今日` |
| 126.2 | Press each tab in turn | `ジョブネット` shows the net view, `カレンダー` the calendar page, `今日` the chart, `履歴` the history; the lit tab is the one pressed |
| 126.3 | Two or more calendars: open the `カレンダー` tab | Beside the calendar's name there is a small down mark; pressing it lists every calendar by name, the current one lit |
| 126.4 | Press another calendar's name in that list | The list closes and the page shows that calendar; the left list lights the same one. Pressing the current one changes nothing |
| 126.5 | Only one calendar: open the `カレンダー` tab | No mark beside the name |

## 127. Message IDs and the troubleshooting list

| # | Do | Expect |
|---|---|---|
| 127.1 | This PC: in the connection add box (palette → `別の PC のエンジンに接続する`) give a name that a `[[remote]]` already has (for example `kei`) | The line in the box begins with the ID: `KOYOMI-E2131 接続先 kei はすでに存在します。` followed by its action; nothing is written to koyomi.toml |
| 127.2 | kei, a `view` token: pick `kei`; try to toggle a weekday on a calendar | The line begins with `KOYOMI-E2201` and names the user, the role and the role needed: `KOYOMI-E2201 利用者 <name> のロールは view です。この操作には admin ロールが必要です。…`, with no `変えていない` after it |
| 127.3 | `koyomi next` with the system date past the end of the holiday table (as 24.10); also open a net in the editor and break a job so that 検査 lists problems | Each row `koyomi next` marks reads `KOYOMI-W4153 祝日の表が古くなっています（<until> まで）。それより後の日は実行されません。` without the action; the editor's heading over the problems list reads `KOYOMI-E3114 定義に誤りがあるため、保存できません。` without the action. The code uses the short form only in these two places; the status bar carries no coded text |
| 127.4 | The system date within 90 days of the table's end (as 24.9): open the window, fold the 注意 band; move the system date one day on and reopen the window; then replace `calendars/syukujitsu.csv` with a table that ends on another date within 90 days | The band's line begins with `KOYOMI-W4152 祝日の表は <until> までの分です（あと N 日）。` and has no other `あと N 日` wording than that sentence; the next day the band stays folded (the days left changed, `<until>` did not); with the other end date the band opens again |
| 127.5 | A net with `calendar = "工場"` and no `calendars/工場.toml`: open the net and look at the problems list in the jobnet panel | The line is `KOYOMI-E4104 カレンダー 工場 は存在しません。…（<file>）`: the ID first, the net's file last |
| 127.6 | Open a net in the editor, change it, change the same file by hand, press 保存: once on this PC's engine, once on `kei` | This PC: the box `ほかで変更されています` reads `KOYOMI-E3103 開いたあとに、このファイルがほかの場所で変更されています。…` and offers 読み直す, キャンセル and 上書きする. On `kei`: it reads `KOYOMI-E3104 …` (the same first sentence) and offers only 読み直す and キャンセル |
| 127.7 | Open the command palette and choose `こんな時はどうする` | A card opens in the middle of the window with the heading `こんな時はどうする` and a search field (hint `検索`) that already has the focus: typing goes into it with no click |
| 127.8 | In the field type `E2101`, then clear it and type `e2101`, then `KOYOMI-E2101`; then clear and type a word of that entry's cause (for example `ファイアウォール`); then `zzzz` | The first three show the same single entry `KOYOMI-E2101`; the word of the cause finds it too; `zzzz` shows `該当するメッセージはありません。` |
| 127.9 | With the card open and the field focused, type `j`, press Space, then an arrow key | The characters go into the field and the arrows move its cursor; the page behind the card does not move: no row is picked in the tree, no net opens, the tab does not change |
| 127.10 | With the card open press Esc; open it again and click outside the card; open it again and click on the card (an entry, the heading, the field) | Esc closes the card; the click outside closes it; the click inside does not |
| 127.11 | Open the card and scroll through the whole list | The entries are grouped under area headings in the order `設定・起動（10xx）`, `接続・認証（21xx）`, `権限（22xx）`, `ジョブネットの保存・確認（31xx）`, `ジョブの実行（32xx）`, `カレンダー（41xx）` as far as the catalogue has entries; each entry shows `KOYOMI-E2101 エラー <heading>` (`警告` for a W code), then the lines `メッセージ: …`, `原因: …`, `対処: …`; the footer reads `Esc か外側のクリックで閉じます。` |
| 127.12 | Set `language = "en"` in common.toml; open the window, then the card; then make a connection fail as in 127.1 | The card's title is `Troubleshooting`, the field's hint `Search`, the entries are in English (`KOYOMI-E2101 Error …`, with `Message`, `Cause`, `Action`, the area headings such as `Connection and authentication (21xx)`); the error line in the window begins with the same ID in English: `KOYOMI-E2131 kei already exists. …` |
| 127.13 | `koyomi remote status a` with A's engine stopped; then with `language = "en"` in common.toml | The line printed begins with the ID, `KOYOMI-E2101 接続先 a に接続できません。（…）` followed by its action (no `koyomi:` in front of a coded line); in English `KOYOMI-E2101 Cannot connect to a. (…)` |
| 127.14 | On a Debian service whose common.toml has no `language` (or `language = "auto"`), `LANG=C`: from a window on another PC provoke a refusal of the engine (a `view` token, as 127.2); then set `language = "en"` in the service's common.toml, restart it and repeat | The refusal comes in Japanese (`KOYOMI-E2201 利用者 … のロールは view です。…`) whatever `LANG` is; with `language = "en"` the same refusal comes in English (`KOYOMI-E2201 User … has the view role. This action needs the … role. …`) |
| 127.15 | `cargo run -q -p koyomi-core --example make-messages -- --check`; open `docs/troubleshooting.md` | It exits 0; the file lists every ID of the catalogue, sorted by ID and grouped by area, each with its text, cause and action |
| 127.16 | Read the catalogue tests in `crates/koyomi-core/src/msg.rs` and run `cargo test -p koyomi-core msg::` | An ID nobody raises any more stays in the list. No test names an old ID, but these tie the list `ALL` to the TOML so that dropping an entry fails: `every_id_has_all_four_fields_in_both_languages_and_the_catalogue_has_no_stray_table`, `ids_are_unique_well_formed_and_have_an_area_name`, `the_four_digits_are_unique_across_letters`, `the_doc_is_sorted_by_id`, `docs_troubleshooting_is_up_to_date` |
| 127.17 | Press `＋ 新しいカレンダー`, type a name, open `こんな時はどうする` from the command palette over the box, type in its search field and press Enter, then press Esc, then Enter | Typing and Enter filter the list and do nothing to the box under the card: no calendar is made and the name stays. Esc closes the card only. The next Enter makes the calendar |
| 127.18 | Open a net in the editor; in 「いつ」 add a `毎日` entry; leave `先行の終了を待つ（グループ/名前）` empty and type `06:00` in `待つのは何時まで`. Then save a copy of the net with `wait_until = "06:00"` and no `wait_for` in that `[[when]]` and run `koyomi check <file>` | The red line under the entry begins with `KOYOMI-E3340 wait_until に対応する wait_for が指定されていません。wait_for を指定するか、wait_until を削除してください。`; the `koyomi check` line for the saved file begins with the same `KOYOMI-E3340 …` and ends with `（<file> の <n> 行目）`, so the ID and the sentence are the same in both |
| 127.19 | Open a net in the editor; in 「いつ」 add a `1 回だけ` entry; type `2026/11/30 21:00` in `日時` | The red line under the entry reads `KOYOMI-E3358 日時 2026/11/30 21:00 を読み込めません。2026-11-30 21:00 のように、日付と時刻を空白で区切って指定してください。` (the value typed is in the sentence); with `2026-11-30 21:00` the line is gone. This ID has no counterpart in `koyomi check`, which reads the same mistake in a file as `KOYOMI-E3390` |
| 127.20 | `koyomi delete <group/name>` on an existing net; answer with `n` (then again with an empty line) | After the question `<group/name> を削除する場合は y を入力してください（前の内容は版に残ります）:` the command prints `koyomi: KOYOMI-E3802 ジョブネット <group/name> を削除しませんでした。削除する場合は、再度実行して y を入力してください。` and exits 1; `koyomi list` still shows the net and its file is unchanged. (The command is `koyomi delete`; there is no `koyomi remove`.) |
| 127.21 | A net with `{var:share}` in a job and no `share` under `[vars]` in koyomi.toml; `koyomi check <file>` | One line begins with `KOYOMI-W1069 変数 share の値が、koyomi.toml の [vars] に存在しません。この変数を使用するジョブは、実行時にエラーになります。koyomi.toml の [vars] に share を追加するか、ジョブネットの変数の名前を修正してください。` and ends with `（<file>）`; the warning does not make the check fail: the exit code is 0 and `<file>: 問題はありません。` follows. With `share` added to `[vars]` the warning is gone |
| 127.22 | Windows: in a terminal that is not elevated, `koyomi service start` for a service the current user may not start (installed by another account) | The command prints `koyomi: KOYOMI-E1053 管理者の権限が必要です。「管理者として実行」で開いたターミナルで、再度実行してください。` and exits 1; the service's state does not change. (`koyomi service install` gives the same line, as 89.2; on a service the user may start, `koyomi service start` works without administrator rights, as 89.4) |
| 127.23 | Debian, service stopped (`sudo systemctl stop koyomi`), no engine running: `koyomi status` | The command prints `koyomi: KOYOMI-E1063 サービスのエンジンが停止しています。sudo systemctl start koyomi を実行して、サービスを開始してください。` and exits 1; no engine starts. After `sudo systemctl start koyomi` the same command prints the status |
| 127.24 | `koyomi plan 2026-11 --pdf out.pdf` with `language = "en"` in common.toml; open the PDF. Then the same with the language `ja` | In English the title reads `Run plan for 2026-11` and each day's first line starts with the label `02(Mon)` (`Mon`, `Tue`, `Wed`, `Thu`, `Fri`, `Sat`, `Sun`); a day with several starts has the times of its later lines in the same column as the first. In Japanese the title reads `2026年11月 実行予定表` and the labels read `02(月)` … `(日)`, with the times in one column too. (`koyomi plan` without `--pdf` prints no weekday: `2026-11-02  09:00  <name>`) |
| 127.25 | Open the engine's web page (`/` of its HTTP) on a run that is running; press `強制終了` | A box opens with `<name> を強制終了します。実行中のジョブは停止し、未実行のジョブは実行されません。` and two buttons, `強制終了する` (red) and `キャンセル`; the focus is on `キャンセル`, so Enter alone does not stop the run; `キャンセル` closes the box and the run goes on |
| 127.26 | On the same web page, pick a net that is not approved (`未承認`) | The net's pane has the line `承認するまで実行されません。承認は koyomi の画面で行います。` and the state `未承認`; for an approved net the line is not there; for a held net the line is `ジョブネットを保留中です。時刻が来ても実行しません。解除するまで「今すぐ実行」もできません。` instead |
| 127.27 | `grep -n text-in-source scripts/verify.sh`; then `python3 scripts/text-in-source.py --check` | The first prints the line `run python3 scripts/text-in-source.py --check`; the check exits 0 and prints `total 0` (no Japanese sentence in the code outside the catalogue and `scripts/text-in-source.allow`). Putting a Japanese sentence into a `.rs` file makes it exit 1 and name the file |

## 128. The Debian package (`.deb`)

Measured on kei (Debian 13, systemd 257) with `koyomi_<version>_amd64.deb` from `scripts/package-linux.sh --deb`. "The keep list" is `/var/lib/koyomi`, every file in `/etc/koyomi/credentials` (`sha256sum`), `koyomi.toml` (`sha256sum`), the `tls/` files, the fingerprint (`koyomi http fingerprint` as `koyomi`) and the user `koyomi`.

| # | Do | Expect |
|---|---|---|
| 128.1 | Where the tar.gz is installed: `sudo ./install.sh --uninstall`, then `sudo apt install ./koyomi_<version>_amd64.deb` | apt prints `koyomi は起動しています。`; `systemctl cat koyomi` is `/usr/lib/systemd/system/koyomi.service` with `ExecStart=/usr/bin/koyomi engine --no-tray`; `systemctl is-enabled koyomi` is `enabled`, `is-active` is `active`; the keep list is unchanged; `curl -sk -H @<header with the admin token> https://127.0.0.1:8790/me` answers 200 with `"role":"admin"` |
| 128.2 | `sudo apt install --reinstall ./koyomi_<version>_amd64.deb` | `systemctl show -p MainPID koyomi` changes (the service restarted); it stays `enabled` and `active` |
| 128.3 | `sudo systemctl disable --now koyomi`, then reinstall as 128.2; then `sudo systemctl enable --now koyomi` | apt prints `koyomi のサービスは無効のため起動していません（起動するには: systemctl enable --now koyomi）。`; the service stays `disabled` and `inactive`; after the enable it is `enabled` and `active` |
| 128.4 | Put `/usr/sbin/policy-rc.d` that exits 101; `sudo systemctl stop koyomi`; reinstall as 128.2; then remove `policy-rc.d` and `sudo systemctl start koyomi` | apt prints `/usr/sbin/policy-rc.d returned 101, not running 'restart koyomi.service'` and `koyomi が起動していません。…journalctl -u koyomi -n 30 --no-pager`; the service is `enabled` and `inactive`; it starts after `policy-rc.d` is gone |
| 128.5 | With the .deb installed, `sudo ./install.sh` from an unpacked tar.gz | Exit 1 with `エラー: koyomi は .deb パッケージとしてインストールされています。更新や削除は apt / dpkg を使ってください (例: sudo apt remove koyomi)。`; nothing is written under `/usr/local` or `/etc/systemd/system`; the service keeps its PID |
| 128.6 | With the .deb installed, put any file at `/usr/local/bin/koyomi` and reinstall as 128.2; remove the file | apt exits 100 after `エラー: tar.gz でインストールした koyomi が残っています。` and `先に、展開したフォルダーで \`sudo ./install.sh --uninstall\` を実行してください。`; `dpkg -s koyomi` stays `install ok installed`; the service is `enabled` and `active` |
| 128.7 | `sudo dpkg -r koyomi` (what `apt remove` does) | Prints `koyomi のパッケージを削除しました。` and the three kept places with the `sudo rm -rf …` command, which it does not run; `dpkg-query -W -f='${Status}' koyomi` is `deinstall ok config-files`; `systemctl is-enabled koyomi` is `masked`, `is-active` `inactive`; `/usr/bin/koyomi` and `/usr/sbin/koyomi-user` are gone; the keep list is unchanged |
| 128.8 | After 128.7, `sudo ./install.sh` from an unpacked tar.gz | Exit 1 with `エラー: koyomi の .deb パッケージの登録が残っています。先に sudo dpkg -P koyomi を実行してください (データ、設定、token は残ります)。`; `/etc/systemd/system/koyomi.service` is still the link to `/dev/null`; no `/usr/local/bin/koyomi` |
| 128.9 | `sudo dpkg -P koyomi`, then `sudo ./install.sh` | After the purge `dpkg-query` knows no koyomi and neither `/etc/systemd/system/koyomi.service` nor `multi-user.target.wants/koyomi.service` exists; after `install.sh` (exit 0) the service is `enabled` and `active`, and `multi-user.target.wants/koyomi.service` points to `/etc/systemd/system/koyomi.service`; the keep list is unchanged |
| 128.10 | On a fresh machine, `sudo KOYOMI_PORT=9000 apt install ./koyomi_<version>_amd64.deb` | The URLs at the end show `:9000`; `koyomi.toml` has `port = 9000` and `lan = true` |
| 128.11 | After 128.7 (removed, not purged), `sudo apt install ./koyomi_<version>_amd64.deb` | The mask is gone: the service is `enabled` and `active`; the keep list is unchanged |

## 129. HTTPS users from the command line (`koyomi user`, `koyomi-user`)

On kei with the service running; `koyomi-user` runs `koyomi user` as the user `koyomi`. Root's `LANG` there is `en_US.UTF-8`, so the CLI's own lines are English; the engine's refusals are Japanese (common.toml's language, else Japanese). A token is only ever written to a file (`> out`), never to the screen of the test. "/me" is `curl -sk -H @<header file with the token> https://127.0.0.1:8790/me`, about 7 s after the change.

| # | Do | Expect |
|---|---|---|
| 129.1 | `sudo koyomi-user list` | One block per `[[http.user]]`: the name, then `Role:`, `Jobnets:`, `Role per jobnet:` (`none` when there is none) and `Token:` (`registered` for a stored `secret:`, `hash (in koyomi.toml)` for a `sha256:`); no token value; exit 0 |
| 129.2 | `sudo koyomi-user add ktest --role admin --nets 'test/*'` | Exit 1 with `KOYOMI-E1030 [[http.user]] ktest は role = "admin" ですが、nets と roles も指定されています。 nets と roles を削除してください。`; `koyomi.toml` is unchanged |
| 129.3 | `sudo koyomi-user add ktest --role run --nets 'test/*' > out` | Exit 0; `out` has three lines: `Added ktest: role run, jobnets test/*, role per jobnet none.`, the token alone, and `This token is shown only now. Keep a copy. …`; `koyomi.toml` gains `name = "ktest"`, `role = "run"`, `token = "sha256:<64 hex>"`, `nets = ["test/*"]`, and does not contain the token |
| 129.4 | /me with the token from 129.3 | 200 `{"nets":["test/*"],"role":"run","roles":null,"who":"ktest"}` |
| 129.5 | `sudo koyomi-user role ktest view --nets 'test/*'`; then /me | Exit 0 with `ktest: role view, jobnets test/*, role per jobnet none. It applies within a few seconds.`; /me shows `"role":"view"` |
| 129.6 | `sudo koyomi-user reset ktest < /dev/null` (no `--yes`, not a terminal) | Exit 1 with `This is not a terminal, so there is no way to ask whether to make the token of ktest again. Add --yes to do it.`; no token in the output; the token from 129.3 still works |
| 129.7 | `sudo koyomi-user reset ktest --yes > out`; then /me with the old and with the new token | Exit 0; `out` has `Made a new token for ktest. The old one stops working within a few seconds.`, the token alone and the note; the old token gets 401 `{"error":"unauthorized"}`, the new one 200 |
| 129.8 | `sudo koyomi-user list` | The block for `ktest` reads `Role: view`, `Jobnets: test/*`, `Token: hash (in koyomi.toml)` |
| 129.9 | `sudo koyomi-user remove ktest`; then /me with the token from 129.7 | Exit 0 with `Removed ktest. It applies within a few seconds.`; /me is 401 |
| 129.10 | `sudo koyomi-user add ktest2 --role view` twice; then `sudo koyomi-user remove ktest2` | The first adds (exit 0); the second exits 1 with `KOYOMI-E2241 [[http.user]] ktest2 はすでにあります。 別の名前を指定してください。…`; the remove exits 0 |
| 129.11 | After 129.2–129.10, `sha256sum koyomi.toml` and `ls <state>/versions/_settings/` | The hash is the one before 129.2; `versions/_settings/` holds one `<date>-<time>.toml` per change (six) |
| 129.12 | `ssh -t` to the machine; `sudo koyomi-user add` with no arguments; answer a name, a role by its number and whether to narrow the job nets; answer `y` | It asks the three questions in turn, shows the `[[http.user]]` it will add and asks before adding; after `y` it prints the token once with the note; `koyomi-user list` shows the user |

## 130. Editing the other PC's koyomi.toml from the window (`ui/far_config.rs`, `ConfigText` / `ConfigPut`)

Rows need the window on this PC connected to kei (`[[remote]]`) with an engine of the same version there, and a token for each role (`admin`, `edit`). Back up kei's `koyomi.toml` first and restore it after; these rows change it. "On kei" means `sudo -u koyomi` there. The file text is not written to any log.

| # | Do | Expect |
|---|---|---|
| 130.1 | Settings → 一般, with the window on kei as `admin`; as `edit`; on この PC | As `admin`: a button `接続先の koyomi.toml を直す…` in place of the line. As `edit`: the line `接続先の koyomi.toml を修正できるのは admin だけです`, no button. On この PC: the 一般 page as before (no button, no line) |
| 130.2 | As `admin`, press `接続先の koyomi.toml を直す…` | The box `接続先の koyomi.toml` opens in the middle with the note, `読み込み中…`, then kei's koyomi.toml in a monospaced field with the focus in it; `読み直す` at the left, `閉じる` and `保存` at the right |
| 130.3 | Add a comment line `# from the window` and press `保存` (then again with Ctrl+S in the field) | `書き込み中…` shows briefly; the box closes and the status line reads `koyomi.toml を書き込みました。数秒以内に反映されます。`; on kei the file has the line and every other byte as before; `versions/_settings/` gained the text as it was; `koyomi ops` on kei shows `config-put` with no text |
| 130.4 | Open the box, change a comment; on kei change the file (`sudo -u koyomi sed -i …`); press `保存` | The box stays with the typed text and `KOYOMI-E2247 koyomi.toml は読み込んだあとに変更されているため、何も書き込んでいません。…` under the field; kei's file is the one changed on kei. `読み直す` asks first (the text was changed); `破棄する` reads the file again |
| 130.5 | In the box change `[http]` `port`; then (reloaded) change your own `[[http.user]]` `role` to `edit` | Each time `KOYOMI-E2249 HTTPS からは、[http] の port・lan・ipv6・tailnet と、自分自身の [[http.user]] <you> を変更できないため、何も書き込んでいません。` under the field; the file is unchanged; the window stays connected |
| 130.6 | Add a `[[http.user]]` with `token = "secret:nothere"` and press `保存` | `KOYOMI-E2248 トークン nothere が資格情報ストアにないか空のため、koyomi.toml を書き込んでいません。…`; nothing written |
| 130.7 | Break the TOML (delete a `]`) and press `保存` | The parse error (with the line) under the field; nothing written; fix it and `保存` writes |
| 130.8 | Change the text and press Esc; Enter; Esc again and Tab, Enter | The question `変更を破棄する` with the focus on `編集を続ける`: Enter keeps the box and the text; Tab moves to `破棄する` and Enter closes the box without writing |
| 130.9 | Change the text and pick この PC in the status-bar list | The question `接続先を切り替える` comes first and says 「接続先の koyomi.toml に、保存していない変更があります。切り替えると破棄されます。」; `キャンセル` keeps the box and the text |
| 130.10 | Close the settings while the box is open | The box stays over the window until it is closed |
| 130.11 | As `admin` on kei, import (取り込み) a bundle whose koyomi.toml changes `[http]` `port`; then one that names `token = "secret:nothere"` in a new `[[http.user]]` | The plan lists the koyomi.toml item with `KOYOMI-E2249 …` (then `KOYOMI-E2248 …`) and nothing is written; kei's file is unchanged and the window stays connected. The same bundle imported on kei itself (`koyomi import` as `koyomi`) goes through as before |

## 131. HTTPS users from the window (Settings → 利用者, `ui/users.rs`)

Rows need the window on this PC connected to kei (`[[remote]]`) with an engine of the same version there, and a token for each role (`admin`, `edit`). Back up kei's `koyomi.toml` first and restore it after; these rows change it. A token shown in the box is copied only to check it with /me (as in 129) and is never written into this file or a log.

| # | Do | Expect |
|---|---|---|
| 131.1 | Settings → 利用者, with the window on kei as `admin`; as `edit`; on この PC | As `admin`: the heading `kei のエンジンの利用者`, the lead line, `読み直す`, `追加…`, and one card per `[[http.user]]` (name, role, job nets); the card of the user this window signs in as reads `<name> （この接続）` and its buttons are off with `この画面が接続に使っている利用者です。…`. As `edit`: `このエンジンの利用者を管理できるのは admin だけです。` and no list. On この PC: `この PC のエンジンの利用者` and the list |
| 131.2 | Press `追加…`; type `wtest`, pick `run`, type `test/*` in `ジョブネット`; press `追加` | While the answer is awaited the form's fields and `追加…` are off; then the box `新しいトークン` opens with `wtest のトークンです。表示されるのは今だけで、…`, the token, `コピー` and `閉じる`; the settings behind are off while it is open; `コピー` puts the token on the clipboard; /me with it answers 200 `"who":"wtest"`, `"role":"run"`, `"nets":["test/*"]`; the list gains `wtest` |
| 131.3 | With the box from 131.2 open, pick another settings page; then press Esc | The box stays over the window when the page changes; Esc closes it without a question (and leaves the settings open); the token is not shown again anywhere. With `閉じる` instead of Esc the same |
| 131.4 | On `wtest` press `役割を変える…`, pick `view`, `保存` | The card reads `view`; /me with the token from 131.2 shows `"role":"view"` within a few seconds; `ジョブネットごとの役割は、…` is shown under the form |
| 131.5 | Press `追加…` and type `zz` (do not send); on `wtest` press `トークンを作り直す…`; the question; `作り直す`; then Esc | The question `トークンを作り直す` says `wtest のトークンを作り直します。…` with the focus on `キャンセル`; after `作り直す` the token box shows a new token; Esc closes only the token box, and the add form still holds `zz` (then `キャンセル` on the form); the old token gets 401 and the new one 200 |
| 131.6 | Open `追加…` and type `wtest2`; on kei add a comment line to koyomi.toml (`sudo -u koyomi sed -i …`); press `追加`; then `追加` again, close the token box and remove `wtest2` (`削除…`) | The first `追加` shows `koyomi.toml が、ほかで変更されました。一覧を読み直しました。入力した内容は残してあります。…` and the form keeps the name and role; the second adds `wtest2` (token box); the remove takes it away |
| 131.7 | On `wtest` press `役割を変える…` and type `test/a` in `ジョブネット` (leave the role); on kei `sudo koyomi-user role wtest edit`; pick another settings page and come back to 利用者; press `新しい一覧を見ました`, then `保存` | After coming back the card reads `edit`, the form shows `edit` and `test/a`, `この利用者は、ほかで変更されました。上が今の内容です。…` is shown and `保存` is off; after `新しい一覧を見ました` and `保存` the card reads `edit` and `test/a`, and /me with the token from 131.5 shows `"role":"edit"`, `"nets":["test/a"]` |
| 131.8 | Make a token again for `wtest` (as in 131.5); with its box open, pick この PC in the status-bar list | The question `接続先を切り替える` says `新しいトークンが画面に出ています。表示されるのは 1 回だけで、…`; `キャンセル` keeps the box and the token; then close the box |
| 131.9 | On `wtest` press `削除…`; the question; `削除する` | The question `利用者を削除` says `wtest を koyomi.toml から削除します。…`; after `削除する` the card is gone and /me with the token from 131.8 is 401 |
| 131.10 | After 131.2–131.9, `diff` kei's koyomi.toml with the backup, and `ls <state>/versions/_settings/` | Only the comment line from 131.6 differs; `versions/_settings/` gained a file for each change the window made |

## 132. A connection's URL, the engine's date, and the engine's notices on this PC (`ui/manage.rs`, `remote.rs`, `ui/mod.rs` `engine_now`, `Notices`)

Rows need this PC with `kei` in koyomi.toml as `[[remote]]`, an engine of the same version on kei (protocol 49), and a token for `edit` and one narrowed by `nets`. kei's `[notify]` `system` lists `failed` (an engine keeps a notice only for an event its own `system` or `sound` lists). Back up this PC's koyomi.toml first and restore it after; rows 132.1–132.6 change it. kei answers at two URLs with the same certificate (`https://192.168.0.133:<port>` and `https://kei:<port>`, or its Tailscale name). The token is never typed, shown or written into this file. "On kei" means `sudo -u koyomi` there.

| # | Do | Expect |
|---|---|---|
| 132.1 | With the window on この PC, open `接続先を管理…` and press `URL を変更する` on `kei` | The row turns into `kei` and a field holding kei's URL with the focus in it, with `キャンセル` and `確認して変更する`; the other rows are unchanged. Esc leaves the field and keeps the box; Esc again closes the box |
| 132.2 | Type kei's other URL (the same certificate) and press Enter | `新しい URL を確認しています…` with a spinner, and every button off; then the status line of the box reads `kei の URL を https://… に変更しました。`; koyomi.toml has the new `url` in kei's block and every other byte (fingerprint, token line, comments, other `[[remote]]`) as before; the credential store is untouched |
| 132.3 | Type `http://kei:1`; then `https://` alone; then a URL where nothing answers | Each time `KOYOMI-E2124 接続先 kei の URL は変更されていません。（…）` with the reason (the URL form; the connection that failed), the field stays with the typed text, and koyomi.toml is not written (its timestamp is unchanged) |
| 132.4 | Put a wrong `fingerprint` (change one hex digit) in kei's block by hand; then change kei's URL to its other URL | After the check the box shows `kei の新しい URL が示す指紋は、保存されている指紋と異なります。…`, the fingerprint in full, `同じでした` and `キャンセル`; the field is off. `キャンセル` (or Esc) writes nothing. Again, and `koyomi http fingerprint` on kei shows the same; press `同じでした`: koyomi.toml has the new `url` and kei's real `fingerprint`, every other byte as before |
| 132.5 | Point kei's URL at another engine's HTTPS (a different certificate whose token for kei is not valid there), press `同じでした` on its fingerprint | `KOYOMI-E2124 …` with the reason (401 or the like); nothing is written; the field comes back with the typed text |
| 132.6 | With the window on kei, open the box | kei's row reads `接続中のため、名前の変更と削除はできません`; `名前を変更する` and `削除する` are off and `URL を変更する` works. Change it to kei's other URL: after `kei の URL を … に変更しました。` the window connects to kei again with the new URL (`このウィンドウは、新しい URL で接続し直します。…`); with an unsaved change in the editor the question `接続先を切り替える` comes first, and `キャンセル` keeps the old URL in use until the window connects again |
| 132.7 | Set kei's time zone 9 hours behind this PC (`sudo timedatectl set-timezone UTC`, then restart koyomi there; set it back to `Asia/Tokyo` after) and, between 00:00 and 09:00 on this PC, open the window on kei | The band says the zones differ; 今日's list, the calendar's today mark, the Gantt's now line and 過去 7 日 in 履歴 are kei's date (yesterday on this PC); the clock in the status bar and export file names keep this PC's time |
| 132.8 | Pick kei in the status-bar list and open 履歴 at once (before the first snapshot) | `エンジンの日付を待っています…` shows and the range, the presets, `検索` and CSV are off until the snapshot comes; then the history of kei's week is asked once and shown. On この PC 履歴 opens as before, without the line |
| 132.9 | With the window on kei as `edit`, make a job net fail on kei (`koyomi run <net>` with a job that exits 1) | Within a few seconds this PC shows a system notification of the failure titled `kei · <the title kei's tray would show>`, with kei's text and `開く`; its sound and focus mode follow this PC's `[notify]`. A notice kept on kei before the window connected is not shown |
| 132.10 | Press `開く` (or the notification) from 132.9 | The window comes forward and opens the run on kei. Switch to この PC and press another such notification: the window comes forward and switches to kei (the question `接続先を切り替える` comes first when there are unsaved edits; `キャンセル` leaves the window where it was) and, as soon as kei's runs are in, opens the run |
| 132.11 | Restart koyomi on kei; make a job net fail again | The notification of the new failure shows once; none of the notices from before the restart shows again |
| 132.12 | As 121.17 with `-d '{"proto": 49, "request": {"Notices": {"boot": null, "since": null}}}'`, with a `view` token, a token narrowed by `nets`, and an `edit` token | `view`: `403` (the request needs `edit`); narrowed: `403`; `edit`: `200`, and the text holds `boot`, `last` and no notes. (The window itself connects only as `edit` or `admin`, not narrowed, so it never asks as the other two) |
| 132.13 | With `koyomi tray` running on this PC, make a job fail on this PC's engine and one on kei | Two notifications: this PC's from the tray (its click opens the run on この PC as before) and kei's from the window; neither replaces the other |
| 132.14 | With the window on kei as `edit`, suspend only the window's process for about 30 s (by process id, as in 88.8), make three nets fail on kei meanwhile, then resume it. Then the same with two nets | Three: one system notification `🔔 kei の通知が 3 件あります` with `最後の通知: <the title of the last failure>`, `「開く」で今日の実行を確かめてください。` and `開く`; the failed sound once. `開く` opens 今日 on kei. Two: two notifications as in 132.9 |

## 133. Watching every connection from the window (`watch.rs`, `ui/hosts.rs`, `main.rs` `far_notices`)

Rows need this PC with `kei` and `works` in koyomi.toml as `[[remote]]` (two other PCs with an engine of the same version, protocol 49), a token for `edit` for each, and `[notify]` `system` listing `failed` on both. The window is open the whole time. Rows 133.1 to 133.4 are read with the window on この PC.

| # | Do | Expect |
|---|---|---|
| 133.1 | With the window on この PC, make a job net fail on kei (`koyomi run <net>` with a job that exits 1) | Within about 15 s this PC shows a system notification titled `kei · <the failure's title>` with `開く`; its sound and focus mode follow this PC's `[notify]`. The status bar shows `ほかの PC の未対応の失敗 1` in red. Nothing under the bell changes |
| 133.2 | Press `開く` on the notification from 133.1 | The window comes forward and switches to kei, then opens that run. With an unsaved change in the editor the question `接続先を切り替える` comes first: `キャンセル` keeps the window on この PC and opens nothing. While a switch to この PC waits on that question, a click on a kei notification is left alone: the switch to この PC goes on |
| 133.3 | Make a net fail on works as well; open the status-bar link list on この PC | Both rows read `リモート · 未対応の失敗 <n>` with a red dot, and the status bar's item is the sum. Open the failure on kei and press `対応済みにする` there, go back to この PC: within about 15 s kei's row is green (`リモート · 応答あり`) and the item counts works only |
| 133.4 | Stop kei's engine (`sudo systemctl stop koyomi`); wait up to a minute; open the list | kei's row is red, `リモート · 応答なし`, with the reason on hover; the status bar's item does not count kei. Start it again and open the list: within seconds the row is green again and a failure on kei is told again |
| 133.5 | Make kei refuse the window: put a wrong token in the credential store (or give the token `view` only) while kei's engine runs | kei's row reads `リモート · 応答あり（この画面では使えない）` in red, with the reason on hover; the window's connection to kei is not offered as working. Put the token back: the row turns green within a minute. With the window on kei, the same: once the token is put back, failures on kei are told again within about a minute |
| 133.6 | Switch the window to kei, then back to この PC; make a net fail on kei each time | Each failure is told once, as `kei · …`, on kei and off it; none is told twice and none is lost across the switches. While on kei the status bar's own count is kei's, not added to the other item |
| 133.7 | With the window on kei, make a net fail on works | The notification reads `works · …` and its click switches to works. kei's row is the current link's as before |
| 133.8 | Make three nets fail on kei within one interval (or suspend the window's process for 30 s as in 132.14) | One notification `🔔 kei の通知が 3 件あります`, as 132.14; with two nets, two notifications |
| 133.9 | Rename `works` to `home` in `接続先を管理…` | The list shows `home` with no stale row for `works`; a failure on home is told as `home · …`; clicking an old `works · …` notification does nothing. Remove `home`: its row goes, the status bar's item no longer counts it, and clicking its notification does nothing |
| 133.10 | Change kei's URL in `接続先を管理…` while the window is on この PC | kei's row keeps what it showed, then answers at the new URL within a few seconds; failures on kei are still told, none twice |
| 133.11 | Leave the window open for 10 minutes on この PC with kei and works answering; watch the engines' logs and the process list | One request of each kind per 15 s per PC (`Notices`, and `Runs` once a minute); no connection piles up on kei or works; the window's CPU stays idle |
| 133.12 | Start the window with `--keys` (a script) | No request reaches kei or works from it |
