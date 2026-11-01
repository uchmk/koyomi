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
| 3.3 | Run `koyomi startup on`, then `koyomi startup` and `koyomi engine status` | Both say サインイン時の起動: あり (the status on its last line); the Run key `uchmk.koyomi` under HKCU holds `koyomi.exe engine`; the settings' General page shows the switch on; after signing in again the engine is up with no window |
| 3.4 | Run `koyomi startup off`, then `koyomi startup oops` | The first says サインイン時には起動しない and the key is gone; the second prints the usage and exits 2 |

## 4. Approval

| # | Do | Expect |
|---|---|---|
| 4.1 | Select a net, press 確かめる… | A box lists the program, each argument on its own line, and the folder; Enter = 確かめた, Esc = やめる |
| 4.2 | `koyomi approve 経理/月次締め`, answer `y`, restart the engine | The same content is printed; the net shows no mark or word of its own (just its name) and still does after the restart |
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
| 6.3 | A file with `timeout = 60` (no unit) and one with `retry = { count = 2 }` | `timeout は "30m" のように文字列で書く…` for the first, `retry に wait が無い（retry は { count = 2, wait = "5m" } の形で書く）` for the second; neither can be approved |

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
| 8.6 | Click a circle, and click a job's row in words | The log under the picture is that job's; the circle gets a blue halo |
| 8.9 | Watch a run with one job running, one failed, and one not reached, flowing sideways and downwards | The running job's lines carry pulses moving left to right (top to bottom), and its circle sends out a fading ring; lines a result came through are thick in that result's soft colour, curving between rows; lines not reached yet are dotted; finished and running circles are filled with their symbol inside; under each name is `9:00–9:04` (`9:00–` while it runs, ` exit 3` after a bad end, in red) |
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
| 10.14 | With the default `[notify]`, `koyomi run <net> --wait` a failing net four times in a row on the same engine | Four `✕ 異常終了` notifications; `koyomi engine status` shows the same pid afterwards (the engine did not crash) |

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
| 11.11 | `calendars/jp.toml` with `holidays = "japan"`; a net with `calendar = "jp"` and `[[when]] every = "week" days = ["mon"] at = ["02:00"]`; `koyomi next <net> --count 12` | The rows skip Japanese holidays that fall on a Monday (no 2026-11-23, no 2027-01-11) |
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
| 18.3 | Put `logs/2026-01-05/` and `history/2026/01/06.toml` in the state folder, then pick 30 days | A box says 30 days are kept, 180 → 30, and that 2 days of logs and history go at the engine's next sweep; Enter writes `keep_days = 30`; within 5 s the two are gone and `engine status` says `ログ 1 日・履歴 1 日を消した` |
| 18.4 | Pick a shorter number again with nothing old there, and press Esc | The box says no days go right away; Esc keeps the file as it was; the row shows the days as before |
| 18.5 | Type 2 in Jobs at a time and press Enter | `concurrency = 2` is written at the top; a net with 3 parallel jobs runs 2 at a time within 5 s |
| 18.6 | Type 0, `abc` or 1.5 in Log size | A red note under "Records" says to enter a whole number of 1 or more; nothing is written; the field goes back to the number in the file |
| 18.7 | Write `keep_days = 45` by hand with a comment above `[notify]`, open the page, pick 60 | The row showed "45 days" among the choices; only `keep_days` changed; the comment, `[notify]` and the line order stay |
| 18.8 | Put `logs/notes/` and `logs/2026-01-05-old/` in the state folder and shorten the days | Neither is counted nor removed: only names that are a day (`2026-01-05`, `2026/01/05.toml`) are swept |
| 18.9 | Search the settings for `keep_days`, `concurrency`, `log_max_mb` or `koyomi.toml` | The results show the General page |
| 18.10 | On a test profile with runs from 3 or more days ago, write `archive = true` and `keep_days = 1` in `koyomi.toml` | Within 5 s those days leave `logs/` and `history/`; `archive/YYYY-MM.zip` in the state folder holds them under the same names (`history/2026/10/28.toml`, `logs/2026-10-28/<run>/<job>.log`); `engine status` says `…を消した、n 日分を archive/ に入れた`; today and yesterday stay |
| 18.11 | `koyomi archive`; `koyomi archive --toml` | A line per month, `2026-10  履歴 n 日・ログ n 日  <size>`, then the folder; under `--toml`: `dir`, `errors` and a `[[zip]]` per month with `month`, `path`, `bytes`, `history`, `logs` |
| 18.12 | `koyomi archive show <one of those days>`; `koyomi archive log <its run> <job>`; `koyomi archive show` of a day not there | The day's runs as `koyomi history` lists them; the job's whole log; `<day> は archive/ にありません` and exit 1 |
| 18.13 | `koyomi history --from <the first of those days>` | Those days' runs are listed again (the engine reads them from the zip), oldest first, before today's, each once |
| 18.14 | Make the month's zip read-only, then change `keep_days` so that another day of that month goes | That day stays in `logs/` and `history/`; `engine status` says `消せなかった 1 件: …YYYY-MM.zip: …`; once the zip is writable again, the next sweep puts the day in and removes it |
| 18.15 | Put a copy of a zip as `archive/2025-01.zip` and write `archive_days = 30` | At the sweep it goes and `engine status` says `古い zip 1 個を消した`; the zips of the months of the last 30 days stay |
| 18.16 | Under "Records", turn on 消す前に archive/ にしまう, then off | `archive = true`, then `archive = false`, is written at the top of `koyomi.toml`; the comments and the line order stay; searching the settings for `archive` shows the General page |
| 18.17 | With it on, shorten the days so that some go (as 18.3) | The box says the n days go into archive/ at the next sweep and can be read with koyomi archive, not that they cannot be brought back; with it off the box says as in 18.3 |
| 18.18 | `koyomi log <run> <job>` for a run whose day is only in `archive/` (18.10), with the engine running and then stopped | Read from the zip: with the engine, the last 64 KB from a line start (all of a shorter log), as for a log in `logs/`; with it stopped, the whole log; for a run in neither `logs/` nor `archive/`, the same error as before |
| 18.19 | In the window, open 履歴 with a range that holds one of those days; pick one of its runs, then a job | The day's runs are in the list with the others (each once); the job's log shows in the log pane (its end, as for a run in `logs/`); a day in a month with no zip shows nothing and no error |

## 19. Retry

| # | Do | Expect |
|---|---|---|
| 19.1 | Write a job with `retry = { count = 2, wait = "5s" }` and run `koyomi check`, then `koyomi approve` | check says OK; the approve text shows `やり直し: 2 回まで、5s おいて` under the job |
| 19.2 | Make that job `cmd.exe /c exit 3` and run it | The log shows `! やり直し (1/2): 5s 待つ`, then `! やり直し (2/2): 5s 待つ`; the job turns 異常 after about 10 s with the reason `終了コード 3、やり直し 2 回`; the run screen and the history show the same reason |
| 19.3 | Make the job fail once and succeed the second time (a flag file) | The run ends 正常; the job's reason is `やり直し 1 回`; the log has `(1/2)` and no `(2/2)` |
| 19.4 | Give the job `timeout = "2s"` and a command that sleeps 10 s | 打ち切り after 2 s, no やり直し line: a timeout is not run again; the same for `koyomi kill` during the job (強制終了) |
| 19.5 | With `wait = "60s"`, run `koyomi kill` while the log says やり直し | The job ends 強制終了 at once (not after the wait); the log ends with `! 強制終了` |
| 19.6 | Write `retry = 2`, `retry = { count = 0, wait = "5m" }`, `retry = { count = 2 }` or `retry = { count = 2, wait = 300 }` | `koyomi check` refuses each with the line number and what to write (the shape, 1 to 100, wait missing, a string like "5m") |

## 20. A passed time run once, a time that waits (`missed`, `overlap`)

| # | Do | Expect |
|---|---|---|
| 20.1 | Write `missed = "once"` at the top of a net with `at = ["02:00"]`, approve it, stop the engine, set the clock past 02:00 (or wait), and start the engine | The run starts at once with 遅れて実行（2:00 の予定を 8:12 に流しました） as its reason on the run screen, in `koyomi status` and in the history; the notification title reads `正常（遅れて実行）  <title>`; the approve text says 時刻を過ぎた回: 起動したとき最後の 1 回を流す（遅れて実行） |
| 20.2 | The same with two passed times (`at = ["02:00", "03:00"]`) | Only the 03:00 run starts late; 02:00 is 時刻を過ぎた with `2:00 に PC が止まっていました` |
| 20.3 | The same net, but unconfirmed, or with a definition folder writable by others | Nothing starts late: the passed time is 止めた with the same reason an on-time one gets |
| 20.4 | Write `overlap = "queue"` and `at = ["10:00", "10:01"]` with a job that sleeps 3 minutes | At 10:01 nothing starts; `koyomi engine status` shows `待ち: <title>  10:01 の回は前の回が終わってから流す（10:01 から待っている）`; when the 10:00 run ends, the 10:01 run starts (how 予定, planned 10:01, no 遅れて実行) |
| 20.5 | `at = ["10:00", "10:01", "10:02"]` with the same job | 10:01 waits; 10:02 is スキップ with `前の回がまだ動いていました（その次の回が待っています）` |
| 20.6 | Run `koyomi engine stop` while a time waits | The waiting time is recorded スキップ with `待っている間にエンジンが止まった`; nothing starts when the engine comes back |
| 20.7 | Write `missed = "later"` or `overlap = 1` | `koyomi check` refuses with the line number: `missed は "skip"・"once" のどれか` / `overlap は "skip"・"queue" のどれか`; `missed = "skip"` and `overlap = "skip"` are accepted, as is leaving both out |

## 21. Exit-code judge and PowerShell script jobs (`judge`, `shell`, `script`)

| # | Do | Expect |
|---|---|---|
| 21.1 | Write `judge = { warn = "1", error = "2.." }` on a job and run `koyomi check`, then `koyomi approve` | check says OK; the approve text shows `判定: 警告 1、異常 2..、他は 0 だけ正常` under the job |
| 21.2 | Make that job `cmd.exe /c exit 1` with a job after it, and run it | The job is 警告 with the reason `終了コード 1（警告）`, the next job runs, and the run ends 警告 (yellow) on the run screen, in `koyomi status`, in the history and in the notification title |
| 21.3 | The same job with `exit 2`, and with `exit 5` and no `judge` at all | Both are 異常 with `終了コード 2` / `終了コード 5`; the next job is 止めた |
| 21.4 | Give the warn job `retry = { count = 2, wait = "5s" }` and `exit 1` | No `! やり直し` line in the log: a 警告 end is not run again; `exit 2` is retried as before |
| 21.5 | Write `shell = "powershell"` and `script = '''
Write-Output "hi"
exit 4'''` with `judge = { warn = "4" }`, approve and run | The approve text shows `シェル: powershell` and the script lines under `スクリプト:`; the log has `hi`; the job is 警告 with `終了コード 4（警告）`; no profile is loaded and nothing is asked (the job ends by itself) |
| 21.6 | Put a `"`, a `$`, a `'` and a Japanese word in the script (`Write-Output "a `"b`" $env:USERNAME 'c' 日本語"`) | The log shows the line as PowerShell prints it: quotes and `$` reach the shell as written (the script is passed as `-EncodedCommand`) |
| 21.6a | A `shell = "powershell"` job with `Write-Warning 'careful'`, `Write-Verbose 'said' -Verbose`, `Write-Host 'hi'`, `Get-Item 'C:\nope\nope'` and `exit 0`, with `judge = { error_output = "^Get-Item" }` | No `#< CLIXML`, `<Objs` or `_x000D_` in the log and no 「モジュールを初めて使用するための準備」 line; `! 警告: careful`, `! 詳細: said`, `hi` (once) and the error's lines (`! Get-Item : …`, `! 発生場所 行:4 文字:1` …) as PowerShell prints them; the job is 異常 with the reason `出力に異常の行があった: Get-Item : …` |
| 21.7 | Write `judge = { warn = "5..2" }`, `judge = { ok = "0" }`, `judge = "1"`, `shell = "cmd"`, `shell = "powershell"` with no `script`, `script` with no `shell`, or both `run` and `shell` | `koyomi check` refuses each with the line number and what to write (start not above end, unknown field, the shape, `"powershell" のどれか`, `shell には script が要る`, `script には shell = "powershell" が要る`, `run と shell は両方書けない`) |

## 22. Rerun and start at a job (`rerun`, `run --from`)

| # | Do | Expect |
|---|---|---|
| 22.1 | Run a net whose second job fails (`exit 3`), then `koyomi rerun <run>` | A new run with the reason `再実行（<run>）`; the first job is ✓ with the reason `前の回の結果` and keeps its old times, and no new log for it appears under `logs/<day>/<new run>/`; the failed job runs again and has a log; `koyomi status --today` lists both runs |
| 22.2 | `koyomi rerun <run> --from <first job>`, then `--from x` | The first job runs again too (a new log, the reason `再実行（<run> を <first job> から）`); `--from x` is refused with `ジョブがない: x` and nothing starts |
| 22.3 | `koyomi rerun` while the run is still going; then a run where everything passed, without `--from` | `<run>: まだ終わっていない`; `<run>: 全部済んでいる（流し直すなら --from <ジョブ>）`; nothing starts either time |
| 22.4 | Change the net's file (add a job) and `koyomi approve` it, then `koyomi rerun` the old run | The rerun uses the old copy (`logs/<day>/<run>/definition.toml`): the added job is not in it, and `definition.toml` under the new run is the old text |
| 22.5 | Change the net's file without approving, then `koyomi rerun` | Refused with `未確認のため流さない: <file>（koyomi approve <group/name>）` |
| 22.6 | `koyomi run <group/name> --from <second job>` | The jobs before it are ⊘ スキップ with the reason `<second job> から流したので飛ばした`, the run line reads `<second job> から`, and only the later logs exist |
| 22.7 | In the window, open a run that is over and open 「再実行 ▾」; press 「止まったところから」; then pick a job in the flow and open the menu again | One 「再実行 ▾」 button; 「止まったところから」 starts a new run that opens (selected in 今日); with a job picked the menu also has 「<job> から再実行」, 「<job> の次から（<job> はスキップ）」 and 「（<job> は警告終了）」, and without one 「選んだジョブから再実行」 is grey; the button is grey while the top warning is shown |
| 22.8 | `koyomi rerun` with yesterday's run id (from `koyomi history`), and with an id that never was | The rerun starts for yesterday's business day (`--date` is the old run's date); the unknown id is refused with `実行がない: <id>` |
| 22.9 | `koyomi rerun <run> --next <first job>`, then `--next <first job> --warn`, then `--next <last job>` and `--warn` alone | The first job does not run (no log) and is ⊘ スキップ (then ⚠ 警告) with the reason `次から再実行したのでスキップにした` (`…警告終了にした`); the run line reads `再実行（<run> を <first job> の次から）` and the jobs after it run; `--next <last job>` with nothing else to run is refused with `<job> の後ろに流すジョブがない`; `--warn` alone is refused with `--warn goes with --next` |

## 23. Hold, skip, pause (`hold`, `release`, `skip`, `pause`, `resume`)

| # | Do | Expect |
|---|---|---|
| 23.1 | `koyomi hold <group/name>` on a confirmed net with a time a minute away, then `koyomi list`, the window's tree, and wait for the time | `保留した: <name>（koyomi release <group/name> で戻す）`; the list and the tree show `⏸ <name>  保留`; when the time comes, 今日 and `koyomi status --today` have a ⏸ 保留 run with the reason `保留中（koyomi release <group/name>）`, no job ran, no log folder appeared, and no notification was sent; `koyomi engine status` has the line `保留中: <group/name>（koyomi release … で戻す）` |
| 23.2 | While held, `koyomi run <group/name>` and the window's 今すぐ流す; then `koyomi hold` again | `保留中のため流さない: <group/name>（koyomi release <group/name>）`, the button is grey and the detail shows the ⏸ 保留中 line; the second hold is refused with `もう保留している: <group/name>` |
| 23.3 | `koyomi release <group/name>` (and once more), then wait for the next time | `保留を解いた: <group/name>`, then `保留していない: <group/name>`; the mark goes back to ✔ and the next time runs as usual |
| 23.4 | Run a net a → b → c where a sleeps 10 seconds; while a runs, `koyomi skip <run> b` (also the window: pick b in the flow and press 「b をスキップ」) | `<run> の b を飛ばした`; b is ⊘ スキップ with the reason `人がスキップした` and no log; c runs after a ends; the run ends ✔ and `koyomi history` shows b's skip |
| 23.5 | `koyomi skip <run> a` while a runs; `koyomi skip <run> c` after the run is over; `koyomi skip <run> x` | `a: もう動いている`, `<run>: もう終わっている`, `ジョブがない: x`; the skip button is grey when the picked job runs or has ended |
| 23.6 | `koyomi pause`, then `koyomi list`, `koyomi engine status`, the window, and wait for a time; then `koyomi run <group/name>` | `スケジュールを止めた（koyomi resume で戻す）`; the list starts with `⏸ スケジュールを止めている（koyomi resume で戻す）`, engine status has the same line, and the window shows the band 「⏸ スケジュールを止めています…」 with the スケジュールを戻す button while the left panel's スケジュールを止める button is gone; the time becomes a ⏸ 保留 run with the reason `スケジュールを止めている（koyomi resume）` and nothing runs; `koyomi run` still starts the net |
| 23.7 | `koyomi pause` again, then `koyomi resume` (CLI or the band's button), then `koyomi resume` again | `もう止めている（koyomi resume で戻す）`; `スケジュールを戻した` and the band goes away; `止めていない` |
| 23.8 | Hold a net and pause, then `koyomi engine stop` and `koyomi engine start`; look at `state_dir("koyomi")/held.toml` | The file has `paused = true` and `held = ["<group/name>"]`; after the restart the hold and the pause are still on (list shows both) |
| 23.9 | Write `held = 1` into `held.toml` while the engine runs (or before `engine start`), then wait for a time | The warning band and `koyomi list` say `held.toml が読めない（…）。直すか退けるまで流さない`, nothing runs, `koyomi hold`/`pause` are refused with the same line; fixing the file clears it at the next scan |
| 23.10 | Queue a time behind a running run (`overlap = "queue"`), then `koyomi hold <group/name>` (or `koyomi pause`) before the run ends | The waiting time turns into a ⏸ 保留 run with the reason `待っている間に保留した（koyomi release …）` (`待っている間にスケジュールを止めた（koyomi resume）`), and `engine status` no longer lists it under 待ち |
| 23.11 | Net a → b → c and a branch `side` beside them (all quick echoes). `koyomi hold <group/name> b`, then `koyomi hold <group/name> b` again and `koyomi hold <group/name> x`; `koyomi run <group/name>` | `保留した: <name> の b（そこに着いた回は待つ。koyomi release <group/name> b で戻す）`; then `もう保留している: <group/name> の b` and `<group/name> にジョブがない: x`; the run does a and side, then b is ⏸ 保留 with the reason `保留中（koyomi release <group/name> b）`, b has no log, c stays 待ち and the run is not over; `koyomi engine status` lists `保留中: <group/name> の b` |
| 23.12 | While the run waits at b, `koyomi release <group/name> b` (then once more) | `保留を解いた: <group/name> の b`, then `保留していない: <group/name> の b`; the same run goes on: b and c run and the run ends ✔ (no new run appears) |
| 23.13 | In the window: open the net, click b in the flow and press 「b を保留」; look at the tree; start the net, open the run and pick b; press 「b の保留を解く」 | The station b shows ⏸ with the line 「⏸ のジョブは保留中です…」 under the flow; the tree row has `⏸ ジョブを保留`; in the run detail b waits as ⏸ and the button reads 「b の保留を解く」; pressing it lets the run go on and the marks go away |
| 23.14 | Hold a job `sub` that calls a child jobnet (`use`), then run the parent | The child's jobs do not start; `sub` stays 待ち and the jobs inside it show ⏸ 保留; releasing `sub` runs the child and the parent goes on |
| 23.15 | Look at `held.toml` after 23.11; `koyomi engine stop` and `start`; an old `held.toml` with only `held = ["<group/name>"]` | It has `[[jobs]]` with `net = "<group/name>"` and `job = "b"`; the hold is still on after the restart; the old file still reads (the net is held, no jobs) |
| 23.16 | Open an over run that ended ✔ with no job picked | The button row shows the small dim line 「ジョブを選ぶとそこから再実行」 after 比べる |

## 24. Calendar (`calendars/<name>.toml`, `calendar`, `business_day`, `on_closed`, `koyomi calendar`)

Set `KOYOMI_DATE` where a step needs a day that is not today. `app_dir("koyomi")` is the settings folder.

| # | Do | Expect |
|---|---|---|
| 24.1 | Write `calendars/会社.toml` with `format = 1`, `holidays = "japan"`, `weekly_off = ["sat", "sun"]`, `closed = [{ date = 2026-11-20, name = "創立記念日" }]`, `open = [2026-11-28]`; then `koyomi calendar 会社 --month 2026-11` | The heading `会社 2026-11（営業日 19 日）`, a weekday header 月〜日, the weeks with `( 3)`, `( 7)( 8)`, `(20)`, `(23)` and `[28]`, then the lines `3 文化の日`, `20 創立記念日`, `23 勤労感謝の日`, `28 休日出勤`, `祝日の表: 2027-12-31 まで` |
| 24.2 | `koyomi calendar 工場` (no such file), `koyomi calendar 会社 --month 2026-13` | `カレンダーがない: 工場` (exit 1); `--month takes YYYY-MM` |
| 24.3 | A net with `calendar = "会社"` and `[[when]] every = "month", business_day = 5, at = ["09:00"]`; `koyomi next 経理/<name>`, the window's detail | The next times are the 5th business day of each month (2026-11: 11-09, since 11-03 is 文化の日 and the weekend is off); the window shows the same rows under 次の 10 回 |
| 24.4 | `business_day = -1`, then `day = 20, shift = -3` (same net), `koyomi next` | The last business day of each month (2026-11-30, 2026-12-31 since 12-31 is not in `closed`); 3 business days before the 20th (2026-11-17 since 11-20 is closed: 19→18→17) |
| 24.5 | `calendar = "会社"`, `[[when]] every = "week" days = ["sat"] at = ["02:00"]` and `[[when]] every = "day" at = ["03:00"]`, `koyomi next`; then `on_closed = "prev"`, `"next"`, `"run"` on the first | `skip` is the default: no 02:00 at all, and Saturday, Sunday and 文化の日 have no 03:00; `prev` adds 02:00 on the Friday before (2026-11-06), `next` on the Monday after (2026-11-02, 11-09), `run` on the Saturday itself; a moved time that meets one already on that day runs once |
| 24.6 | `business_day = 5` in a net without `calendar` and no `_group.toml` naming one; `koyomi check <file>` and `koyomi list` | `calendar が要る（business_day・shift・on_closed を使うとき）`; the net is ✕ 無効 and does not run |
| 24.7 | Write `jobnets/経理/_group.toml` with `format = 1` and `calendar = "会社"`; a net of 経理 without `calendar`; `koyomi check` the net and the window's detail | The net counts by 会社 (its 次の 10 回 skip the weekend); `koyomi check jobnets/経理/_group.toml` says `_group.toml: 問題なし`; write `x = 1` into `_group.toml`: every net of 経理 is ✕ 無効 with `_group.toml: 知らない欄 x` |
| 24.8 | `calendar = "工場"` (no file), then write `calendars/工場.toml` with `holidays = 1` | `カレンダーがない: calendars/工場.toml`, then `calendars/工場.toml が読めない（…）`; the window's band 「カレンダーの注意」 and `koyomi list`'s `注意:` line show `calendars/工場.toml: …`; nothing else stops |
| 24.9 | `KOYOMI_DATE=2027-10-15` (fewer than 90 days left), `koyomi list`, the window, `koyomi check calendars/会社.toml` | The band and the `注意:` line read `祝日の表は 2027-12-31 までの分です（あと 77 日）`; `koyomi check` prints the same line and exits 0 |
| 24.10 | `KOYOMI_DATE=2028-01-05`, a net on 会社 with `every = "day"`, `at` a minute away; `koyomi check calendars/会社.toml`, `koyomi next`, wait for the time | The band reads `祝日の表が古い（2027-12-31 まで）。それより後の日は流さない`; `koyomi check` prints it and exits 1; `koyomi next` marks each row `祝日の表が古い`; when the time comes, 今日 has a 🔒 run with the reason `祝日の表が古い（2027-12-31 まで）`, no job ran, and a notification was sent |
| 24.11 | In the window, pick 会社 under カレンダー; use ◀ ▶; double-click a Saturday, a holiday, a weekday, and each again | The head reads `会社 ◀ 2026年11月 ▶ 営業日 19 日`; the weekly-off boxes run 日 to 土; the month table starts on Sunday, one framed cell per day: days off sit on a darker fill, holidays in the gold accent, closed days in magenta, `open` days in the done colour, each named day's name under its date in the cell (cut to 3 characters and … when longer; the full name on hover), today outlined in blue; a legend of those colours under it, then the names list; the double-clicks write `calendars/会社.toml` (`open = [...]` / `closed = [...]` appear and go), the engine reads it again and the table follows within a few seconds; `<file> に書きました` under the month |
| 24.11b | In 会社's month, look before clicking; click 11/7 (a Saturday), then a weekday, then a day of another month after ▶; right-click a weekday and choose この日を休業にする; on a picked day press the box's button; Tab to a day and press Enter | Beside the month a box shows today first: `この日に動くもの（<月>/<日> <曜>・営業日 <n>）` (n counts the business days of the month up to it), each approved net's start that day as `HH:MM  <name>` (nets on any calendar), or 「この日に始まるものはありません」; a click moves the box and the outline (white; today stays blue, thicker when picked) to that day and writes nothing; a day off shows its name or 休み in the head; the menu item and the box's button (この日を休日出勤にする / この日を休業にする / この日を元に戻す) toggle the day as a double-click does; Enter on a day only picks it |
| 24.11a | Pick 会社 under カレンダー, go to a month with approved nets on it, press 「この月の予定を印刷…」 | `書き出し中…` at once (the window does not stop), then `印刷に回した（PDF は <state>\exports\plan-<年>-<月>.pdf）` or, where the OS cannot send it, the PDF opens and `PDF を開いた（印刷はそこから）: …`; the PDF matches `koyomi plan <年>-<月> --pdf` for that month |
| 24.12 | Make `calendars/会社.toml` read-only, click a day | `calendars/会社.toml に書けない: …` in the error colour; the table does not change |
| 24.13 | Pick a net that uses 会社 in the tree, then pick 会社 under カレンダー | The calendar view lists the net under 「このカレンダーで流す定義」 and its next 10 times under it, counted by 会社 |
| 24.14 | `koyomi approve` a net on 会社 with `every = "day"` | The week's count in the approval box and in the CLI skips the weekend (5 times, not 7) |
| 24.15 | With `calendars/` empty: look under the tree, open the カレンダー tab; then 「＋ 新しいカレンダー」 (also from the heading's right-click and the palette's 「新しいカレンダー…」); Enter with no name, then the name 会社 when it exists, then `a/b`; then the name 工場, untick 日本の祝日, tick only 日 and 作る | The heading カレンダー and 「カレンダーはまだありません」 show, and the tab opens the box; `名前を入れてください`, `もうあります: 会社`, the bad-name line; `calendars/工場.toml` holds `format = 1`, `holidays = "none"`, `weekly_off = ["sun"]`, the month view of 工場 opens at once (「（読み込んでいます）」 until the engine reads it), and `koyomi calendar 工場` shows only Sundays off |
| 24.16 | Open 工場 in the カレンダー view; untick 日 and tick 土; then 名前を変える… (also from the row's right-click) with the name 工場, then 倉庫; with a net `calendar = "倉庫"`, 消す…, press Enter once, then 消す… again and はい | `calendars/工場.toml` says `weekly_off = ["sat"]` and the month follows; `もうあります: 工場`, then the file is `倉庫.toml` with the same content; the delete box lists the net's file under 「calendar = "倉庫" と書いた所は直さない…」 and starts on いいえ, so Enter deletes nothing; はい removes the file, keeps it as `versions/_calendars/倉庫/<日時>.toml`, and the net turns invalid until fixed |
| 24.17 | Open a net in the editor, its ジョブネット pane; open the カレンダー list and pick 「新しいカレンダー…」; name it 夜間 and 作る; then 保存 | The list shows （なし）, the calendars, and 「新しいカレンダー…」 at the end; the box opens over the editor, which stays; `calendars/夜間.toml` is written, the pane's カレンダー says 夜間 and the net is unsaved (Ctrl+Z takes it back); after 保存 the file says `calendar = "夜間"` |
| 24.18 | Pick 会社 under カレンダー; right-click a weekday, then a Saturday, then a closed day, and choose 名前を付ける… on each; type a name and press Enter (or 決める); once with the field emptied. Then 祝日の表を取り込む…, paste the path of the Cabinet Office's `syukujitsu.csv` (with its quotes, as コピー as path gives it) and press Enter; once more with a CSV that has a bad date | The field under the month shows `<日> の名前` with the name the day has; a weekday and the Saturday go into `closed` under that name (still off), a closed day keeps its place with the new name, an emptied name leaves the day closed with none; the names list follows. The table is written to `calendars/syukujitsu.csv` byte for byte, `… に書きました（<first> 年から <last> 年）` shows, the one there before is in `versions/_calendars/syukujitsu/<日時>.csv`, and the holiday line follows; the bad CSV says `<path> を取り込めない: <n> 行目: 日付が読めない: …` and nothing is written. Esc or やめる closes the field |
| 24.19 | Pick 会社 under カレンダー and click a weekday; at the right of the head press 日, then ◀ and ▶, then 週, then 月; close the window on 日 and open it again | The switch reads 日 \| 週 \| 月 in that order. 日 shows that one day as a timetable (its weekday, date and name on top, a row for each time something starts, or 「この日に始まるものはありません」); the tag says what the day is (営業日 3, 文化の日, 休み), the head says `11/4(水)`, and ◀ ▶ move one day. 週 shows its seven days, Sunday first, and 月 the month. The window opens again on 日 |

## 25. Parts of the flow (lines `on` / `code`, branch, join, time, file, notify, child jobnet)

`koyomi check <file>` reads the lines and parts; the window's 流れ draws a job as a circle and a part as a square (`【分岐】` and the like before its name in `koyomi show` and the notifications).

| # | Do | Expect |
|---|---|---|
| 25.1 | A net: `a` (fails, exit 3), `b` with `after = [{ job = "a", on = "error" }]`, `c` with `after = ["a"]`; run it | `b` runs, `c` ends `－ 実行なし`（理由 `a が異常終了`）; the run is ✕ 異常（`a` failed）even though the error line led on; the thick line a→c stays in the border colour, the thin line a→b takes `a`'s colour |
| 25.2 | `d` with `after = [{ job = "a", on = "any" }]` after a failing and after a passing `a` | `d` runs both times; `koyomi show` of the net lists `線: a (any)` under `d` |
| 25.3 | `a` ends by `timeout = "1s"`, then by `koyomi kill`; `b` has `on = "error"`, `c` has `on = "ok"` | Both times `b` runs and `c` is 実行なし: 打ち切り and 強制終了 count as abnormal on a line |
| 25.4 | `p` with `kind = "branch"`, `after = [{ job = "a", on = "any" }]` (a plain `after = ["a"]` follows only a 0 from `a`); `x` with `after = [{ job = "p", code = "0" }]`, `y` with `code = "1..3"`, `z` with `code = "else"`; `a` exits 0, 2, 9 in turn | Exactly one of x / y / z runs each time (0 → x, 2 → y, 9 → z); the others are 未実行 (`p の線の条件に合わなかった`); `p` reads `【分岐】p` and ends ○ with `a` 's code as its reason |
| 25.5 | `j` with `kind = "join"`, `wait = "all"`, `after = ["a", "b"]` (a 1 s, b 5 s), `c` after `j`; then `wait = "any"` | `all`: `c` starts after `b` ends; `any`: `c` starts right after `a`, `b` keeps running and the run ends when `b` does |
| 25.6 | `k` (a plain job) with `after = ["a", "b"]` | `k` waits for both, like `wait = "all"`: a job with 2 or more lines waits for all of them |
| 25.7 | `t` with `kind = "time"`, `at` 2 minutes ahead, `after = ["a"]`, `c` after `t`; then `at = "00:00"` | `t` reads 実行中 `【時刻待ち】t` with the reason `HH:MM を待っています` and a dashed line; at the time it ends ○ `HH:MM になった` and `c` runs; `00:00`: it ends at once with `00:00 を過ぎていた` |
| 25.8 | `t` with `days = 1`, `at = "00:00"`; `koyomi kill <run> t` | The reason reads `YYYY-MM-DD 00:00 を待っています` (tomorrow); kill ends it ✕ 強制終了, the run ends ✕ 異常, `c` is 実行なし |
| 25.9 | `f` with `kind = "file"`, `file = 'C:\in\*.csv'`, `settle = "3s"`, `timeout = "10m"`, `c` after `f` running `cmd /c echo %KOYOMI_FILE%`; drop `note.txt`, then `Sales.csv` and keep writing it for 5 s | `f` waits (`C:\in\*.csv を待っています`), `note.txt` changes nothing; `Sales.csv` ends `f` 3 s after it stops changing: ○ `C:\in\Sales.csv が来た`; `c`'s log shows `C:\in\Sales.csv`; without `settle` it ends as soon as the file is there |
| 25.10 | `f` with `timeout = "5s"` and no file | `f` ends ✕ 打ち切り（`打ち切り（5 秒 を超えた）`）after 5 s, the run ends ✕ 異常 |
| 25.11 | `n` with `kind = "notify"`, `after = [{ job = "a", on = "error" }]`, and `to = "x"` on it; then without `to`; `a` fails | `koyomi check`: `to は kind = … にだけ書く` (the part has no `to`); without it the notification arrives as `✕ 異常  <title>: n` with the line of `a` and the run's id and day; `n` ends ○ `通知した` at once; the run is ✕ 異常 (`a` failed) |
| 25.12 | `n` after a passing `a` | The notification is a `done` one (`○ 正常  <title>: n`) with the run's line |
| 25.13 | `jobnets/共通/バックアップ.toml` (confirmed, jobs `x`, `y`); a net with `c` `kind = "jobnet"`, `use = "共通/バックアップ"`, `after = ["a"]`, `b` after `c`; run it | The run lists `a`, `c/x`, `c/y`, `【子ジョブネット】c`, `b` in that order; `c/x` and `c/y` run after `a`, `b` after them; `c` ends with the child's result; the child's own `[[when]]` is not used |
| 25.14 | `use = "共通/無い"`; then make `バックアップ.toml` invalid (`x = 1`); then leave it unconfirmed (edit it) | `koyomi check` and `koyomi list`: `子ジョブネットがない: 共通/無い`, `子ジョブネット 共通/バックアップ に定義の誤りがある`; the parent is ✕ 無効 and does not run; unconfirmed: `koyomi run` says `子ジョブネット 共通/バックアップ が未確認` and nothing starts |
| 25.15 | `バックアップ.toml` gets a `kind = "jobnet"` part with `use` = the parent | Both are ✕ 無効 with `子ジョブネットの呼び合いが輪になっている: 共通/バックアップ → <parent> → 共通/バックアップ` (the ids in the order they call, from the net it uses first: the parent's line starts at `共通/バックアップ`, `バックアップ`'s at `<parent>`) |
| 25.16 | A part with no `after` (`kind = "join"`) and a job nothing follows that has `kind = "time"` | `孤立した部品: j（after が無い）` and `孤立した部品: t（後に続くものが無い）`; `after = ["a", "b"]` with `b` after `a` after `b`: `after が輪になっている: a → b → a` |
| 25.17 | `koyomi show` / the approval box of a net with every kind of part | Each part is headed by its kind (`分岐 p`, `待ち合わせ j` …) with `線:` per line, `待ち方: 全部が終わってから`, `時刻: 06:00（1 日後）`, `待つファイル:`, `落ち着くまで:`, `使う定義: 共通/バックアップ`; a job's lines read `線: a (error)` |
| 25.18 | `koyomi run` a net with 3 running jobs and `concurrency = 2`, one of them a `kind = "time"` part | The part waits without taking a slot: the two jobs run and the part is 実行中 at the same time |

## 26. Starts by events (`[[when]] file = …` + `settle`, `on_start = true`)

A `[[when]]` with `file = 'D:\受信\*.csv'` starts the net when such a file comes (`ito-watch`, and a look every 5 s besides); `settle = "10s"` waits until the size and change time have kept for that long. `on_start = true` starts the net when the engine starts (the PC's start or sign-in). Each is an event, not a time: `koyomi next` and the window list it as a row with no time.

| # | Do | Expect |
|---|---|---|
| 26.1 | A net with `[[when]] file = 'D:\受信\*.csv'` and a job echoing `%KOYOMI_FILE%`; `koyomi approve` it; copy `sales.csv` into `D:\受信` | A run starts within a few seconds; `koyomi show` says `ファイル`（`D:\受信\sales.csv が来た`）; the job's log is the path |
| 26.2 | The same with `settle = "10s"`; write the file slowly (append every 3 s for 20 s, then stop) | No run while it is being written; a run starts about 10 s after the last write |
| 26.3 | Copy `note.txt` into `D:\受信` | Nothing starts: only `*.csv` counts |
| 26.4 | Two files are already in `D:\受信` when the net is approved (or the engine starts) | No run; the history has a ⊘ スキップ row `見張り始めたときに既に 2 個あった（D:\受信\*.csv）: 流さない`; a file copied in afterwards starts a run |
| 26.5 | Copy `b.csv` in while the run for `a.csv` is still going | `b.csv` waits; its run starts right after the first run ends (one run per file, in name order) |
| 26.6 | Overwrite `a.csv` (new size or time) after its run ended | A run starts again; a file left as it is does not start another, and the engine's restart does not run it again |
| 26.7 | Delete `D:\受信` before the engine starts, then make it and copy a file in | `koyomi check` (or the window) says `待つファイルのフォルダーを見張れない（5 秒ごとに見る）`; the file still starts a run within about 5 s |
| 26.8 | The net has `calendar = "会社"` and the file comes on a Sunday | No run; a ⊘ スキップ row `… が来たが、休業日なので流さない（on_closed = "run" で流せる）`; with `on_closed = "run"` in the `[[when]]` the run starts |
| 26.9 | The `[[when]]` has `except = [<today>]` or `start = <tomorrow>` | No run; a ⊘ スキップ row `… が来たが、except か start / end の外なので流さない` |
| 26.10 | `koyomi hold` the net (or `koyomi pause`), then copy a file in | No run; a ⏸ 保留 row with the reason, as for a time; `koyomi release` / `resume` does not run it afterwards (the file counts as recorded) |
| 26.11 | A net with `[[when]] on_start = true`; `koyomi approve`; `koyomi engine stop` and start the engine again (or sign in again) | A run starts once; `koyomi show` says `起動`（`起動したとき`）; the engine's running on does not start it again |
| 26.12 | The same net, unapproved, when the engine starts; and with a calendar on a closed day | 🔒 `未確認のため流さなかった（koyomi approve …）`; ⊘ スキップ `起動したが、休業日なので流さない（on_closed = "run" で流せる）` |
| 26.13 | `koyomi next` and the net's 予定 in the window for a net with both events and `every = "day"` | The rows `ファイルが来たとき: D:\受信\*.csv（10 秒 変わらなくなってから）` and `PC の起動・サインインのとき（エンジンの起動）` come first, then the times; `koyomi review` of it shows the same two rows under 今週 |
| 26.14 | `koyomi check` a `[[when]]` with `file = 'in\*.csv'`, with `file = 'D:\*\a.csv'`, with `file = …` and `on_closed = "prev"`, and with `on_start = false` | `file は絶対パスで書く（例: …）`; `file の * / ? はファイル名の部分だけ（フォルダーには使えない）`; `file / on_start では on_closed は "skip" か "run"`; `on_start は true だけ（使わないなら [[when]] ごと消す）` |

## 27. Links between jobnets (`[[when]] after_jobnet = …` + `on`)

A `[[when]]` with `after_jobnet = "経理/売上取込"` starts the net when a run of that net ends as `on` says: `on = "ok"` (the default: 正常終了 or 警告終了), `"error"` (異常終了, 打ち切り, 強制終了) or `"any"`. However the run before was started (a time, the window, `koyomi run`, `koyomi rerun`, a file, or another net before it), its business day is handed on as `KOYOMI_DATE`. A net starts this way at most once a business day; a second end on the same day is recorded as ⊘ スキップ and told of, so a person runs it by hand if it is wanted.

| # | Do | Expect |
|---|---|---|
| 27.1 | Net `g/a` (one job echoing hi) and net `g/b` with `[[when]] after_jobnet = "g/a"` and a job echoing `%KOYOMI_DATE%`; `koyomi approve` both; `koyomi run g/a` | When `g/a` ends 正常終了, a run of `g/b` starts within a few seconds; its history row's reason is `g/a が終わった（正常終了）` (and `how = "after"` in the history file); its job's log is today's date |
| 27.2 | `koyomi rerun` a run of `g/a` whose business day was 2 days ago | `g/b` starts again, with that run's business day (`KOYOMI_DATE` is 2 days ago, and the history row shows that date) |
| 27.3 | `g/a`'s job made to fail (`exit 1`); `koyomi run g/a` with `g/b` on `on = "ok"` (default), then with `on = "error"`, then `on = "any"` | With the default, nothing starts (no row at all for `g/b`); with `"error"` and `"any"`, `g/b` starts with `g/a が終わった（異常終了）` |
| 27.4 | `g/a`'s job ends 警告終了 (`warn_codes`), `g/b` on `on = "error"` and then on `on = "ok"` | `"error"`: nothing; `"ok"`: `g/b` starts (`g/a が終わった（警告終了）`) |
| 27.5 | `koyomi run g/a` twice on the same business day (after 27.1) | The second time `g/b` does not start: a ⊘ スキップ row `g/a が終わった（正常終了）が、同じ業務日（<date>）に既に流した（<run id>）: 後続は流していない。要るなら手で流す（koyomi run g/b）`, and a notification says so |
| 27.6 | After 27.5, `koyomi run g/b` by hand (and from the window) | It runs: only the automatic start is once a day |
| 27.7 | `g/b` given a job that sleeps 60 s; `koyomi run g/a` twice quickly (two different runs of `g/a` ending while `g/b` is still running, with the second `g/a` run `rerun` from another day) | The second end: ⊘ スキップ `… が、前の回がまだ動いているので流さない`; nothing queues |
| 27.8 | `g/b` unapproved; `koyomi hold g/b`; `koyomi pause`; each time `koyomi run g/a` | 🔒 `未確認のため流さなかった（koyomi approve …）`; ⏸ 保留 with the reason; ⏸ with the pause's reason — as for a time |
| 27.9 | `g/b` with `calendar = "会社"`, and `koyomi rerun` a run of `g/a` whose business day is a Sunday | ⊘ スキップ `g/a が終わった（正常終了）が、休業日なので流さない（on_closed = "run" で流せる）`; with `on_closed = "run"` in the `[[when]]`, `g/b` runs with that Sunday as `KOYOMI_DATE` |
| 27.10 | `koyomi check` with `after_jobnet = "g/none"`; and with `g/a` after `g/b` and `g/b` after `g/a` | `先行のジョブネットがない: g/none`; `after_jobnet が輪になっている: g/a → g/b → g/a` on both (the window shows them as ✗ 誤り and does not run them); a net after `g/a` that is not itself in the loop is fine |
| 27.11 | `koyomi check` a `[[when]]` with `after_jobnet = "売上取込"`; with `on = "sometimes"`; with `every = "day"` and `on = "ok"`; with `after_jobnet = …` and `on_closed = "prev"` | `after_jobnet は "グループ/名前" で書く（例: …）`; `on は "ok"・"error"・"any" のどれか`; `on は day では使わない`; `after_jobnet では on_closed は "skip" か "run"` |
| 27.12 | `koyomi next`, the net's 予定 in the window, and `koyomi show` for `g/b` | A row with no time: `g/a が終わったとき（正常終了・警告終了のとき）` (`異常終了のとき` / `どう終わっても` for the other `on`s) |
| 27.13 | Three nets `g/a` → `g/b` → `g/c` (each after the one before); `koyomi rerun` a run of `g/a` from 3 days ago | `g/b` and then `g/c` run, each with `g/a`'s business day; the history has the three rows with that date |

## 28. History search and the rest of the CLI (`history`, `show`, `list --group`, `--help`, `--toml`)

The window's 履歴 (a row under 今日) and `koyomi history` show the runs that started on a range of days (the last 7 when none is given, at most a year; days the sweep put into `archive/` are read from there, 18.19), narrowed by jobnet and state on the asking side. Every command takes `--help` (its own lines of the usage, exit 0) and `--toml` (the answer as TOML for another program: the commands that answer with data print it as such, the rest print `ok = true|false` and the `lines` they would have said).

| # | Do | Expect |
|---|---|---|
| 28.1 | Some runs over the last days (ok, error, skipped); `koyomi history` | The runs of the last 7 days up to today, oldest first, each as `status --today` prints it (the mark, the id, the title, the date, the times, the jobs under it) |
| 28.2 | `koyomi history --from 2026-10-01 --to 2026-10-15`; `--from` alone; `--to` alone | Those days only; `--from` alone goes up to today; `--to` alone covers the 7 days up to it |
| 28.3 | `koyomi history --status error`; `--status ok g/a`; `koyomi history g\a` | Only the runs of that state; only that net's; `\` reads as `/` |
| 28.4 | `koyomi history --status nope`; `--from 2026/10/01`; `--from 2026-10-20 --to 2026-10-01`; `--from 2024-01-01` | `--status takes one of ok, warn, …` and the usage (exit 2); `--from takes YYYY-MM-DD` (exit 2); `koyomi: 範囲が逆: …` (exit 1); `koyomi: 範囲は 1 年まで` (exit 1) |
| 28.5 | `koyomi history --status held` with no held run in the days | `該当する実行はありません` (exit 0) |
| 28.6 | `koyomi history --toml`, `koyomi status --toml`, `koyomi list --toml`, `koyomi show g/a --toml` | The runs as `[[run]]` tables, each with its `[[run.jobs]]`; the snapshot (`[[defs]]`, `warnings`, `held`, `paused`, …); the definition as the engine read it (`id`, `file`, `standing`, `[net]`, `[[problems]]`, where `line = 0` is a problem of the whole file such as a missing child) |
| 28.7 | `koyomi next g/a --toml`; `koyomi calendar 会社 --toml`; `koyomi log <run> <job> --toml` | `unconfirmed = …` and `[[next]]` with `at = "2026-11-02 09:00:00"` or `event = "…"`; `calendar`, `month`, `business_days` and a `[[day]]` per date with `kind = "business" / "open" / "closed" / "holiday" / "weekly_off"` and `name` when it has one; `run`, `job`, `text` |
| 28.8 | `koyomi hold g/a --toml`; `koyomi engine status --toml` with the engine down; `koyomi check bad.toml --toml` | `ok = true` + `lines = ["…"]`; `ok = false` + `lines = ["koyomi: エンジンは動いていない"]` (exit 1); `ok = false` and the problems as `lines` (exit 1) |
| 28.9 | `koyomi approve g/a --toml` on an unconfirmed net | The preview and `確かめたなら y を入力` are still said as text (a person answers); after `y`, `ok = true` + `lines` |
| 28.10 | `koyomi list --help`, `koyomi history -h`, `koyomi --toml list --help`, `koyomi pause --help` | Only that command's lines of the usage under `usage:`, then `every command takes --help …` (exit 0); `pause --help` prints the `pause \| resume` line |
| 28.11 | `koyomi list --group 経理`; `koyomi list --group nothing`; `koyomi list --toml --group 経理` | Only that group's nets in the tree (the warnings and notes still above); `koyomi: グループがない: nothing` (exit 1); the snapshot with only those defs |
| 28.12 | `koyomi show g/a` (confirmed, unconfirmed, held, and ✗ 誤り) | The file and its standing (`⏸ 保留` added when held), then what `approve` shows (the jobs, the times, the next week) without the question or the diff; a ✗ net prints its problems and nothing under them; exit 0 |
| 28.13 | The window: click the 履歴 tab | The right side shows 履歴: 開始日 (7 days ago), 終了日 (today), ジョブネット = すべて, 状態 = すべて, 検索; the rows of those days newest first (`2026-10-29  ✓ 正常終了  売上取込  09:00:00  09:00:12  12 s`, coloured by state); the count above; the tree's and 今日's picks are off |
| 28.14 | Change 開始日 to a date and press 検索; type `2026/10/1` and press 検索 | The rows of the new days; `日付は YYYY-MM-DD で書く` in red and the rows stay |
| 28.15 | Pick a net in ジョブネット, then a state in 状態 | The rows narrow at once (no 検索 needed); the count follows; `該当する実行はありません` when nothing matches |
| 28.16 | Click a row in 履歴 | Under the rows: the run's picture, its jobs' logs on click, 再実行 and the rest as under 今日; the row stays marked |
| 28.17 | While 履歴 is open: click a net in the tree, a run in the 今日 table, or a calendar; then the 履歴 tab again | Each closes 履歴 and shows its own detail; 履歴 opens again on the last 7 days with nothing narrowed |
| 28.18 | Narrator / NVDA over 履歴 | The 履歴 row, the 検索 button and each run row are read (row: the date, the state, the title, the times) |
| 28.19 | `koyomi --keys` report with 履歴 open | A `history: 2026-10-23..2026-10-30 * * 12` line (the days, the net or `*`, the state key or `*`, the rows shown) |
| 28.20 | A job whose reason has line breaks (a `[[when]] eventlog` net started by an event whose message has several lines, e.g. from PowerShell `eventcreate /L APPLICATION /SO MyApp /T ERROR /ID 100 /D "line 1$([char]10)line 2"`); `koyomi history`, `koyomi show <net>` and `koyomi run <net> --date <a week ago>..<today> --only missing --dry` | Each run and job is one row: the reason's line breaks read as spaces. `koyomi show <run> <job>`'s 理由 keeps them |

## 29. Editor (`ui/canvas.rs`, `koyomi new` / `versions` / `restore`)

The 編集 button on a jobnet's detail opens the editor over the picture: a canvas with the parts, a shelf of the seven parts on the left, and a pane on the right for the part or the net that was double-clicked. The editor writes the whole file back from what it shows (comments in the file are lost; the previous text is kept under `versions/`), the engine checks the text before it is written, and a save from the editor counts as confirmed. Parts the person did not move get their place from 整列 and are not written.

| # | Do | Expect |
|---|---|---|
| 29.1 | Select a jobnet in the tree, click 編集 | The canvas replaces the picture: the parts where the file (or 整列) puts them, the shelf on the left, 保存 / 整列 / 元に戻す / やり直す / ジョブネット / ← at the top; the tree stays usable |
| 29.2 | A jobnet whose file does not read (a `問題` row) | 編集 is greyed out; nothing opens |
| 29.3 | Drag a part from the shelf onto the canvas; click a part in the shelf | The part appears where it was dropped with a new id (`job2`, `notify1` …) and is selected; a click puts it after the cursor's part and links it with 成功したら |
| 29.4 | Drag from the right port of a part to another part | A line (成功したら) from the first to the second; dropping on nothing or on the same part draws no line |
| 29.5 | Right-click a line | A menu: 成功したら / 失敗したら / どちらでも / 線を消す; the choice changes or removes the line |
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
| 29.16 | Edit the file by hand (or `koyomi restore`) while the editor is open, then 保存 | A box: ほかで変わっています, with the differences between what the editor started from and the file now; 読み直す drops the editor's changes and loads the file, やめる keeps editing, 上書きする saves over it (the hand edit is kept as a version); Escape = やめる |
| 29.17 | With unsaved changes: ← , the tree, a run in the 今日 table, 履歴, or the palette's 閉じる | A box: 変更を捨てて閉じますか; 捨てて閉じる leaves, やめる (the default, Escape) stays |
| 29.18 | Without changes: ← | The editor closes at once and the picture is back |
| 29.19 | Double-click a child jobnet part (or select it and Enter) | Its net opens in the editor on top; ← comes back to the parent (asking first when the child has changes) |
| 29.20 | Ctrl+P while editing | The palette lists 保存 / 整列 / 横と縦 / 元に戻す / やり直す / 閉じる above the usual commands; they are gone when the editor is closed |
| 29.21 | `koyomi new 経理/新規` | `jobnets/経理/新規.toml` from the template (one job, no `[[when]]`), the net appears unconfirmed; `new` on an existing file refuses |
| 29.22 | `koyomi versions 経理/新規`; `koyomi restore 経理/新規 <time>` | The versions newest first; restore writes that version back (today's text becomes a version too) and the net is unconfirmed until `approve` |
| 29.23 | Narrator / NVDA over the editor | The toolbar buttons, the shelf's parts, the pane's fields and the box buttons are read |
| 29.24 | `koyomi --keys` report with the editor open | An `editor: 経理/新規 dirty=false parts=1 selected=[] cursor=- pane=net problems=0 connect=false` line |
| 29.25 | Open a net that is not confirmed (edited by hand, or after `koyomi restore`) | A 未確認 badge beside the title, its hover saying the net does not run when its time comes and that 保存 confirms it; no badge on a confirmed or a new net, and none after 保存 |
| 29.26 | 検査 with nothing wrong; then name a calendar that is not there (`calendar = "ない"`) or a child jobnet that is not there, 検査 again | `検査: 問題なし` in green under the toolbar; then `検査: 直すところがあります` in red with the same lines a save would refuse with (`<group>/<name>.toml: カレンダーがない …`); nothing is written and the net stays as it was |
| 29.27 | Look under the canvas | A legend: 成功したら ✓ (solid green), 失敗したら (dashed red), どちらでも *, 終了コード, それ以外 else, then each part's mark (↻ ▣ ◇ ⊕ 🕑 📄 ✉ 👤 and the chores); the same marks are inside the stations on the canvas (not 【) |
| 29.28 | Under the legend, with `[[when]]` every day at 09:00; then add a `quiet` span over one of them; then remove all `[[when]]` | この先 7 日の開始 with `7 回` and a strip of 7 days (dates at the dividers, a tick at each 09:00; hover a tick for its date and time); the quiet one moves or goes as `on_quiet` says; with no `[[when]]` it says the net runs only by hand |
| 29.29 | A net with many parts, zoomed in; look at the bottom-right corner of the canvas; click and drag there | A small picture of every part as a dot (red for a part with a problem, blue for the selected) and a blue frame for what is in view; a click or a drag brings that place to the middle; on a small canvas or with one part it is not drawn |
| 29.30 | Open the editor; look at the right; double-click a job; empty its 実行; press Escape; press › at the pane's top | The pane is there from the start with the net's fields (基本設定 / 動かし方 / いつ / 次の 10 回); each label has a red `*` when it must be filled or `(任意)` when it may be empty; the job's fields come in sections (基本設定 / 実行コマンド・スクリプト / タイムアウトと再試行 / 結果の判定 / 別のアカウントで動かす) with 保存 at the bottom right and この部品を消す at the bottom left; the empty 実行 turns red; Escape goes back to the net's fields; › folds the pane to a strip at the right edge and a click on the strip opens it again |
| 29.31 | Open a net that calls a child net; double-click the child to go into it; press 戻る at the right of the header | The child's canvas opens; 戻る (‹) is at the right of the header beside ジョブネット and takes you back to the parent, the same as 戻る at the left; at the top net there is no 戻る at the right |

## 30. Import, export and the daily backup (`koyomi export` / `import`, `backups/`)

| # | Do | Expect |
|---|---|---|
| 30.1 | `koyomi export -o out.toml --all` with nets, a group file, a calendar and `koyomi.toml` that has `webhook = "secret:x"` | One TOML with `[[calendar]]`, `[[group]]`, `[[jobnet]]` (each file's text as it is, comments kept) and `[settings]` (`koyomi.toml` as a table: its comments are not kept) whose webhook addresses are `***` (a `secret:<name>` stays, since it is only a name); the count is printed |
| 30.2 | `koyomi export -o out.toml` again; `-n 経理/月次締め` alone; `--group 経理`; `--calendars`; `--settings` | The second write is refused (`もうある`); a net comes with its group file, calendar and child nets; a group brings its nets; the other flags pick only those |
| 30.3 | `koyomi import out.toml --dry-run` on a PC where one net differs | The plan: `追加`/`上書き`/`同じ` per thing with a `+`/`-` diff under what differs; exit 1 and nothing written |
| 30.4 | `koyomi import out.toml` with a difference and no `--on-conflict` | Refused with the three choices named; nothing written |
| 30.5 | `koyomi import out.toml --on-conflict overwrite` | The net is replaced, its old file is under `versions/<group>/<name>/`, and it is `未確認` in the tree until approved; new nets come in unconfirmed too |
| 30.6 | `--on-conflict rename` and `--on-conflict skip` | rename adds the different one under a new name beside the old; skip leaves it and brings in the rest; a bundle with a missing child net or a broken file is refused whatever the choice |
| 30.7 | `koyomi import out.toml --toml` | The plan as TOML (`lines`, `conflicts`, `problems`, `undecided`, `writes`, `applied`) |
| 30.8 | Palette → 書き出す | Under the detail: `書いた: <state>/exports/<YYYYMMDD-HHMMSS>.toml（N 件…）` and the file is there |
| 30.9 | Palette → 取り込む…, type a wrong path, Enter | The box keeps the field with `…を読めない` in red; Esc closes it |
| 30.10 | Type the path of a bundle whose nets differ, Enter | The plan box (the lines of 30.3, cut after 30 with `あと N 行`) with 上書きする / やめる / 名前を変えて足す / 違うものは飛ばす; Enter on the first frame = やめる, Tab moves, Esc = やめる |
| 30.11 | Pick 上書きする | `取り込んだ（ジョブネットは未確認…）` under the detail; the tree shows the nets `未確認`; old files are under `versions/` |
| 30.12 | Type a bundle with nothing different, Enter | 取り込む / やめる; a bundle that is all the same says `書くものはありません` with one 閉じる button |
| 30.13 | Start the engine, wait, `koyomi engine status` | A `バックアップ: <time>、backups/<YYYY-MM-DD>.toml に N 件（7 日分を残す）` line; the file is a bundle `import` reads; an empty settings folder says `定義がないので書かなかった` and leaves the old day files |
| 30.14 | Narrator / NVDA over the 取り込む box | The field is read as ファイル, the buttons by their labels, the plan's lines as text |

## 31. A new jobnet from the window

| # | Do | Expect |
|---|---|---|
| 31.1 | Start `koyomi` with no definitions | The tree shows `＋ 新しいジョブネット` above `定義がありません…`; the message names the button |
| 31.2 | Press `＋ 新しいジョブネット` | The 新しいジョブネット box opens in the middle with グループ and 名前; the keyboard is in 名前; the window behind takes no clicks |
| 31.3 | Press 作る with a field empty; then type `a/b`, `..`, `a:b` or `締め.` as the name | `グループと名前を入れてください` and `/ \ : * ? " < > \| と、末尾の . は使えません` in red; the box stays open |
| 31.4 | Type a group and a name a net already has (try other upper/lower case) | `もうあります: <group/name>`; the box stays open |
| 31.5 | Type `テスト` and `新規1`, Enter | The box closes; the editor opens titled `編集 テスト/新規1（新規・未保存） *` with one job `初のジョブ`; no file is written yet |
| 31.6 | Edit the job, Ctrl+S | `jobnets/テスト/新規1.toml` is written; the title loses `（新規・未保存）` and `*`; the tree shows the net under テスト as confirmed, not `未確認` |
| 31.7 | Right-click a group heading in the tree → このグループに新しいジョブネット | The box opens with グループ filled in; the グループ list (今あるグループ) picks another one |
| 31.8 | Ctrl+N, and palette → 新しいジョブネット… (the hint says Ctrl+N) | Both open the box; Ctrl+N does nothing while the editor is open; Tab moves between the fields, Esc closes the box |
| 31.9 | Open the box while the editor has unsaved changes, fill it, Enter | `編集中の変更を保存するか閉じてから作ってください`; nothing is lost |
| 31.10 | Make `テスト/新規2`, then before saving write `jobnets/テスト/新規2.toml` by hand, then Ctrl+S | `もうある（ほかで作られた）: テスト/新規2。閉じて、別の名前で作り直す`; the hand-written file is unchanged; no overwrite button |
| 31.11 | Make a net and press 閉じる without saving | The discard question comes first, with the keyboard on やめる (Enter keeps the editor); Tab, then Enter on 捨てて閉じる leaves no file |
| 31.12 | `koyomi --keys "<Wait:2500><C-n><Wait:400>shime<Enter><Wait:300><State:b><S-Tab>keiri<Enter><Wait:500><State:c><Quit>"` | `b.txt` has `make: /shime (グループと名前を入れてください)`; `c.txt` has `editor: keiri/shime dirty=true parts=1 … new=true` (with no nets yet; once a group exists, its list sits between the two fields, so press `<S-Tab>` twice) |
| 31.13 | Narrator / NVDA over the box | The fields are read as グループ and 名前, the buttons by their labels; the error is read as text |


## 32. 日付の埋め込み

| # | Do | Expect |
|---|---|---|
| 32.1 | A job `run = ["cmd.exe", "/c", "echo", "{date}", "{month-1}"]`; `koyomi run <net> --date 2026-01-15 --wait`, then `koyomi log` | The log has `2026-01-15 202512` |
| 32.2 | A net with `calendar = "会社"` (土日 off) and `{date-1bd:%m%d}`; run it with `--date 2026-11-02` | The log has `1030` (the Friday) |
| 32.3 | The same `{date-1bd}` in a net with no calendar; `koyomi run` | Refused: `… 営業日で動かすには calendar が要る`; no run starts |
| 32.4 | Write `{date-1x}` in `run`; `koyomi check` | A problem with the line number: `run: {date-1x}: 単位が読めない…` |
| 32.5 | A PowerShell `script` with `@{ a = 1 }` and `{ … }` | Runs as written; the braces are not touched |
| 32.6 | `koyomi rerun` a run of an earlier day | The tokens are written for that run's business day, not today |
| 32.7 | `koyomi run <net> --date 2026-11-02 --dry` | Each job prints as `id: command @ dir` with the dates written out; no run starts (`koyomi status` shows nothing new) |

## 33. 環境変数と秘密

| # | Do | Expect |
|---|---|---|
| 33.1 | `echo hunter2 \| koyomi secret set db`, then `koyomi secret check db` | `秘密 db はある`; the value is not shown anywhere |
| 33.2 | A job `run = ["cmd.exe", "/c", "echo", "%DB_PASS%"]` with `env = { DB_PASS = "secret:db" }`; run it, then `koyomi log` | The log has `***`, not `hunter2` |
| 33.3 | The same job after `koyomi secret delete db` | The job fails with `秘密がない: db（koyomi secret set db）` |
| 33.4 | `env = { OUT = "{month-1}" }` and a job that echoes `%OUT%` | The log has last month as `YYYYMM` |
| 33.5 | `env = { KOYOMI_X = "1" }` or `env = { 1A = "x" }`; `koyomi check` | A problem with the line number |
| 33.6 | Open the review of a job with `env` | `環境変数 DB_PASS: secret:db` is listed; no secret value |

## 34. エンジンが落ちていたことの通知

| # | Do | Expect |
|---|---|---|
| 34.1 | Start the engine, kill its process (`taskkill /f`), wait 2 minutes, start it again | A notification `エンジンが落ちていた`, and a band at the top of the window with the span |
| 34.2 | `koyomi engine stop`, wait 11 minutes, start it again | `エンジンが止まっていた` is told the same way |
| 34.3 | `koyomi engine stop` and start it again within a minute | Nothing is told |
| 34.4 | The band an hour after the start | It is gone |

## 35. 実行前の点検

| # | Do | Expect |
|---|---|---|
| 35.1 | A job `run = ["C:/nope/x.exe"]`; `koyomi check` the file | `<job> の実行ファイルが見つからない: …` and exit code 1 |
| 35.2 | A job with `dir = "C:/nope"` | `作業フォルダーがない` |
| 35.3 | A `kind = "file"` part watching `C:/nope/*.csv` | `見張るフォルダーがない: C:/nope` |
| 35.4 | `run = ["cmd.exe", ...]` with `dir = "C:/work/{month}"` | No complaint about the dir (it has no name until the day) |
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
| 39.1 | Net B: `[[when]] every="day" at=["HH:MM"] wait_for="g/A"`; A runs daily and is still running at HH:MM | B does not start at HH:MM; it starts right after A ends, with the same business day |
| 39.2 | Add `wait_until` a few minutes after HH:MM; keep A running past it | B is recorded as 異常 with the reason (wait_until), and a notification comes |
| 39.3 | A is not scheduled that day (e.g. `days` excludes today) | B starts at HH:MM without waiting |
| 39.4 | In the editor open B's `[[when]]` | Fields 先行の終わりを待つ / 待つのは何時まで show and save to the file |

## 40. ステータスバー

| # | Do | Expect |
|---|---|---|
| 40.1 | Open the window with the engine running | The bar shows `次の回 MM-DD HH:MM` (the soonest time of any net not on hold) |
| 40.2 | Start a run | `実行中 N` appears in green while it runs and goes when it ends |
| 40.3 | `koyomi pause` | `スケジュール停止中` shows in red; `koyomi resume` clears it |
| 40.4 | Stop the engine while the window is open | The bar says the engine is not reachable / starting |
| 40.5 | §102's two engines (102.1); open a window from each console (`koyomi`) | Within 30 s, after the engine word: B `待機中: a を見張っている` in the colour of 未承認; A `b が待機している` in grey. Your usual profile (neither section) shows no such words |
| 40.6 | Close A's window (it would start A's engine again); do 102.4 and watch B's window; open A's window again after 102.7 starts A | B: `a から引き継いで流している（YYYY-MM-DD HH:MM から）` in green. A, until B gives them back: `b が引き継いでいる` (or `b に聞いている`) in the colour of 未承認. Then, within 30 s: B `待機中: a を見張っている（YYYY-MM-DD HH:MM に戻した）`, A `b が待機している`. While an engine is down or starting, its window shows none of these |

## 41. 出力とファイルでの判定

| # | Do | Expect |
|---|---|---|
| 41.1 | A job that prints `ERROR x` and exits 0, with `judge = { error_output = "^error" }` | The job ends 異常, reason `出力に異常の行があった: ERROR x`; it is retried if `retry` is set |
| 41.2 | A job that exits 3 and leaves `C:/out/{date}.csv`, with `judge = { ok_file = "C:/out/{date}.csv" }` | The job ends 正常; the log says the file made it normal |
| 41.3 | Same job with the file missing | The job ends 異常 (exit 3) |
| 41.4 | In the editor open the job | Fields 出力にあれば失敗 / あれば成功にするファイル show and save; a bad regex is refused on save |

## 42. 時刻を 1 回だけずらす

| # | Do | Expect |
|---|---|---|
| 42.1 | `koyomi shift g/a 09:00 09:30` (a net with a time at 09:00 later today) | It answers that the time was moved; at 09:00 nothing starts; at 09:30 the net runs once with planned time 09:00 |
| 42.2 | The next day | The net runs at 09:00 as the definition says (the file was not changed) |
| 42.3 | `koyomi shift g/a 09:00 09:30` then `koyomi shift g/a 09:00 --cancel` | The move is taken back; 09:00 runs as usual |
| 42.4 | A time the net does not have, or a new time before the old one or already past | It is refused with the reason |
| 42.5 | `koyomi skip-next g/a` (an approved net with a time later today) | It answers which time it skips; `held.toml` has a `[[skip]]` with it; at that time nothing starts and the history shows the time as スキップ「次の 1 回だけスキップ」; the time after runs as usual |
| 42.6 | `koyomi skip-next g/a` twice, then `koyomi skip-next g/a --cancel` | The second skips the time after the first; `--cancel` takes both back and the times run; `--cancel` with nothing skipped is refused |
| 42.7 | `koyomi rename g/a g/b` with a time skipped | The skip moves to g/b and still holds |

## 43. 操作の記録

| # | Do | Expect |
|---|---|---|
| 43.1 | `koyomi hold g/a`, `koyomi release g/a`, then `koyomi ops` | Two lines with the time, `hold` / `release` and `g/a`, oldest first |
| 43.2 | Run a net by hand, save one in the editor, then `koyomi ops` | `run` and `save` lines appear |
| 43.3 | A refused operation (hold a net that is already held) | No line is written |
| 43.4 | `koyomi ops --from <yesterday>` after restarting the engine | The earlier lines are still there; `koyomi history` is unchanged |

## 44. 使っている所を探す

| # | Do | Expect |
|---|---|---|
| 44.1 | `koyomi uses <calendar name>` | Lines `calendar   g/x` for each net that counts by it |
| 44.2 | `koyomi uses g/a` where g/b has a child-net job `use = "g/a"` and `after_jobnet = "g/a"` | `child net  g/b (job …)` and `follows    g/b (on Ok)` |
| 44.3 | `koyomi uses robocopy` (a job runs `C:/tools/Robocopy.exe`) | `program    g/x (job …)`; the folder and `.exe` do not matter |
| 44.4 | `koyomi uses nothing-here` | `(not used)` |

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

## 49. 回復の通知

| # | Do | Expect |
|---|---|---|
| 49.1 | Run a net whose job exits 3, then change it to exit 0 and run it again | The second end shows "Back to normal" naming the abnormal run, after the usual ok end |
| 49.2 | Run it ok once more | No recovery notice (the run before it was ok) |

## 50. ジョブの間で値を渡す

| # | Do | Expect |
|---|---|---|
| 50.1 | Job `a` writes `rows = 5` to the file named by `KOYOMI_OUT`; job `b` (after `a`) prints the file named by `KOYOMI_IN` | b's log shows `[a]` and `rows = 5` |
| 50.2 | Make `a` write text that is not TOML | b's `KOYOMI_IN` has no `[a]` table; the run still goes on |

## 51. 排他（lock）

| # | Do | Expect |
|---|---|---|
| 51.1 | Give two nets `lock = "db"`; start the first (a slow job), then start the second by hand | The second is refused with a message naming the lock and the net that holds it |
| 51.2 | Start a net without `lock` meanwhile | It starts normally |
| 51.3 | Wait for the first to end, start the second again | It starts |
| 51.4 | Give two nets `lock = { name = "db", shared = true }` and one `lock = "db"`; start both shared nets (slow jobs), then the plain one by hand | Both shared nets run together; the plain one is refused with 「排他「db」は … が共有で使っている」 |
| 51.5 | While the plain `lock = "db"` net runs, start a shared one by hand | Refused with 「共有「db」は … が排他で使っている」 |
| 51.6 | Put `[locks]` `"api" = 2` in koyomi.toml; give three nets `lock = "api"` (slow jobs) and start them one after another | The first two run together; the third is refused with 「排他「api」は 2 本まで。… が使っている」 |
| 51.7 | Give the third of 51.6 a schedule a minute ahead while the other two still run | At its time the day shows it 🔒 止めた, with the same reason |
| 51.8 | Write `"api" = 0` (or `"a/b" = 2`) under `[locks]` | The engine shows 「koyomi.toml が読めない（[locks] の "api" は同時に持てる数で、1 以上 …）。直すまで流さない」 (or the line about the name) |
| 51.9 | Save a net with `lock = { name = "db", shared = true }` from the editor | The file still has the same `lock` line |

## 52. 人の確認待ち

| # | Do | Expect |
|---|---|---|
| 52.1 | Add an "approval" part (shelf) between two jobs and start the net | The run waits at the approval part and a notification arrives |
| 52.2 | Press "承認する" on the run's toolbar | The part ends 正常 and the next job runs |
| 52.3 | Start again and press "却下する" (or `koyomi approve-step <run> <job> --no`) | The part ends 異常, the run fails and the next job does not run |

## 53. 期間をまとめて流す

| # | Do | Expect |
|---|---|---|
| 53.1 | `koyomi run <net> --date 2026-05-01..2026-05-03 --dry` | The commands for each of the three days are printed |
| 53.2 | Run it without `--dry` | The days run one after another, each started after the last ended |
| 53.3 | Make one day fail, then run the range | It stops at that day with a non-zero exit |
| 53.4 | `--date 2026-05-03..2026-05-01` | Refused: the range ends before it starts |
| 53.5 | For a net that runs daily, with runs on some days of the last week and one of them ended 異常: `koyomi run <net> --date <a week ago>..<today> --only missing --dry` | Each day is listed: 流す for the days with no run (実行の記録がない, or only a 時刻を過ぎた/保留/未確認で止めた record, or a スキップ whose reason is 待っている間にエンジンが止まった: stop the engine while a time waits for the run before it), 流さない with the record's state and id for a day whose last record is any other スキップ and for the others; nothing starts |
| 53.6 | The same with `--only failed --dry` | The day that ended 異常 is now 流す too; a day off the net's calendar is 流さない 予定のない日 |
| 53.7 | Run `--only failed` without `--dry` | One line says how many days run and which, then only those days run in turn; with none to run it says 流す日はない and exits 0 |
| 53.8 | `koyomi run <net> --only missing` (no range) | Refused: `--only` goes with `--date A..B` |
| 53.9 | With `keep_days = 7`, `koyomi run <net> --date <30 days ago>..<today> --only missing --dry` | The days older than 7 days with no record are 流さない 記録が残っていない（keep_days より前）, and one line says 記録が残っていない日は流さない。範囲の日を全部流すなら --only を外す; with `archive = true` and those days in `archive/`, their runs are read from the zip and the days are listed as in 53.5 |
| 53.10 | On a standby PC (`[standby]`, not taken over): `koyomi run <net> --date A..B --only missing` | Refused: 待機中の PC では --only を使えない（流しているのは <primary> で、実行の記録もそちらにある）。<primary> で流す; nothing starts. After it has taken over, it picks the days as in 53.5 |

## 54. 平均の所要時間と終了の予想

| # | Do | Expect |
|---|---|---|
| 54.1 | Run a net that takes a minute a few times, then start it once more and run `koyomi status` | Under the run: "いつもは …、HH:MM:SS ごろに終わる見込み" |
| 54.2 | Run `koyomi status` after the run has gone past the average | Only "いつもは …" shows, with no expected time |
| 54.3 | Run `koyomi status` for a net that never passed | No average line |

## 55. MCP（Claude Code から）

| # | Do | Expect |
|---|---|---|
| 55.1 | `claude mcp add koyomi -- koyomi mcp`, start the engine, then ask Claude Code for today's runs | `koyomi_status` answers without asking |
| 55.2 | Ask it to run a net | Claude Code asks first; the first call only lists the commands, and nothing starts until it is called again with `confirm = true` |
| 55.3 | Ask it to stop a running run | Same: nothing stops without `confirm = true` |
| 55.4 | Stop the engine and ask for the runs | The tool answers with an error saying the engine is not running |

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
| 57.1 | `koyomi plan` with the engine running | Each day of this month with the times approved nets start; days with none show `-` |
| 57.2 | `koyomi plan 2026-02 --pdf out.pdf`, open the PDF | A4 pages titled with the month, text selectable, page numbers at the foot |
| 57.3 | `koyomi plan --print` | The PDF path is printed, and the PDF goes to the printer (or opens in the viewer when it has no print verb) |
| 57.4 | A net on a calendar that closes a day | That day shows no start for the net |

## 58. ネット間のつながり（map）

| # | Do | Expect |
|---|---|---|
| 58.1 | `koyomi map` with a net that follows another (`after_jobnet`) and one that uses a child net | One line each: `g/a -> g/b (after ok)`, `g/a -> g/b (child)` |
| 58.2 | `koyomi map --dot \| dot -Tsvg > map.svg`, open it | The nets as boxes, arrows from the first to the dependent, labelled |
| 58.3 | `koyomi map --mermaid`, paste into a Mermaid viewer | A left-to-right flowchart of the same links |
| 58.4 | With no links | `(no links)` |


## 59. 出力の文字コード（encoding）

| # | Do | Expect |
|---|---|---|
| 59.1 | A job that prints a Shift_JIS file (`run = ["cmd.exe", "/c", "type", 'sjis.txt']`) with `encoding = "sjis"`, run it | The log shows the Japanese text, not mojibake |
| 59.2 | The same job with `encoding = "utf8"` | The log does not show the Japanese text (the bytes are not UTF-8) |
| 59.3 | No `encoding` (auto), a program that prints UTF-8, then one that prints in the console code page | Both read right |
| 59.4 | `encoding = "cp932"` | The problem 「encoding は "auto"・"sjis"・"utf8" のどれか」 on that line |
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
| 61.1 | Run a net with a job that takes a few minutes, then `powercfg /requests` in an administrator prompt | Under SYSTEM: `koyomi: ジョブを実行中` |
| 61.2 | After the run ends, `powercfg /requests` again | The koyomi line is gone |
| 61.3 | Set sleep to 1 minute on a laptop, run a job of 5 minutes without touching the PC | The PC stays awake until the job ends, then sleeps after a minute |
| 61.4 | A net that only waits (a time part, an approval) | No hold: the PC may sleep |

## 62. 眠っている PC を前もって見せる・PC を起こして流す

| # | Do | Expect |
|---|---|---|
| 62.1 | Set sleep to 30 minutes, open a net whose next times are tonight | Times more than 30 minutes away read `この PC は 30 分で眠る設定。起こさなければ動かない`; nearer ones have no mark |
| 62.2 | Same, under 今日 on the left | The coming times today carry the same mark |
| 62.3 | Set sleep to never | No mark anywhere |
| 62.4 | In the editor's net pane, check 「PC が眠っていても起こして流す」 and save | The file has `wake = true`; the next times read `PC を起こして流す` |
| 62.5 | `koyomi status` | `PC を起こす: <時刻の 2 分前>（<ネット>）` |
| 62.6 | `powercfg /waketimers` in an administrator prompt | A timer by koyomi's engine for that time |
| 62.7 | Put the PC to sleep before the time | It wakes 2 minutes before, the net runs, and the PC sleeps again afterwards |
| 62.8 | Turn wake timers off in the power plan (Advanced settings → Sleep → Allow wake timers: Disable), wait a minute | The marks read `PC を起こして流す設定だが、起こせない: 電源プランで復帰タイマーが無効`; `koyomi status` says the same |
| 62.9 | On a laptop on battery with wake timers allowed only when plugged in | The reason reads バッテリー… |

## 63. タスクトレイの常駐アイコン

| # | Do | Expect |
|---|---|---|
| 63.1 | Start koyomi, close the window | The koyomi calendar icon stays in the task tray with a green ✔ badge; hovering reads `✔ koyomi: 次 HH:MM` (or `異常なし`); the right-click menu starts with the same words greyed |
| 63.2 | Run a net whose job takes a minute | The badge turns blue … and the tip reads `実行中 1` |
| 63.3 | Run a net that fails | The badge turns red ✖ and the tip reads `異常 1`; after the same net runs and ends 正常, it is green again |
| 63.4 | Left-click the icon while the window is closed | The window opens |
| 63.5 | Minimise the window, then left-click the icon | The window comes back to the front |
| 63.6 | Right-click → 今日の一覧 | The window comes forward with ガント in the detail (unless a net is open in the editor) |
| 63.6a | Right-click with nothing failed today; then run two nets that fail, mark one run 対応済み in the window, close the window, right-click → 未対応を見る (2); do it again after marking the other | 未対応を見る (0) is greyed at first; after the failures it reads 未対応を見る (2) and the window comes forward on the newest failed run not marked 対応済み (its picture and log in the detail); with every one marked it opens ガント instead. With a net open in the editor the window only comes forward. `koyomi tray` (with `engine --no-tray`) shows the same item and count |
| 63.6b | Right-click → 次の 1 回だけスキップ… | The window comes forward with the 次の 1 回だけスキップ box: each approved net on a schedule with its next time. 飛ばす fills 飛ばす回 with that time and moves 次の回 to the one after; 戻す clears it; Esc or 閉じる closes the box. A held or unapproved net is not listed. With a net open in the editor the window only comes forward |
| 63.7 | Right-click → スケジュールを止める | The badge turns grey –, the tip reads `スケジュールを止めている`, the window's band says the schedule is paused; the menu now reads スケジュールを戻す |
| 63.8 | Right-click while a job runs | 終了（ジョブ 1 件が実行中） is greyed |
| 63.9 | Right-click → 終了（エンジンを止める） with nothing running | The icon goes away; `koyomi engine status` says the engine is not running |

## 64. 手で流すときに値を渡す（`[params]`、`run --set`）

| # | Do | Expect |
|---|---|---|
| 64.1 | A net with `[params]` `month = "{month-1:%Y-%m}"` and `dept = "all"`, and a job `run = ["cmd.exe", "/c", "echo", "{param:dept}", "{param:month}"]`; `koyomi run <net> --dry` | The job's line shows `all` and last month as `YYYY-MM` |
| 64.2 | `koyomi run <net> --set dept=経理 --set month=2026-09 --wait`, then `koyomi log` | The log has `経理 2026-09` |
| 64.3 | `koyomi run <net> --set nope=1` | Refused: `値 nope は [params] にない（あるのは dept・month）`; nothing runs |
| 64.4 | `koyomi rerun <that run> --from <job>` | The log has `経理 2026-09` again (the values of the first run) |
| 64.5 | The net's scheduled time comes | The job runs with the defaults |
| 64.6 | In the window, select the net and press 今すぐ実行 | A box 値を決めて実行 lists dept and month with the defaults filled in; change dept, press Enter, and the run's log has the new dept |
| 64.7 | Press 今すぐ実行 on a net without `[params]` | It starts at once, with no box |
| 64.8 | Write `{param:x}` in a job of a net whose `[params]` has no `x`; open the window | The net shows the problem `{param:x}: [params] にない` and cannot be approved |

## 65. 通知に失敗したジョブのログの末尾

| # | Do | Expect |
|---|---|---|
| 65.1 | A job `run = ["cmd.exe", "/c", "for /l %i in (1,1,15) do @echo line %i & exit /b 3"]`; `koyomi run <net> --wait` | The system's notification has a last line `<job>: line 15` under the job's line |
| 65.2 | With a webhook on for `failed` (e.g. ntfy), the same run | The message ends with `<job> のログの末尾` and `line 6` … `line 15` |
| 65.3 | A job with `env = { PW = "secret:db" }` that echoes `%PW%` and exits 1 | Both the notification and the webhook show `***`, not the secret |
| 65.4 | A run that ends 正常, or a job that fails before writing anything | The notice is as before, with no log lines |

## 66. タスク スケジューラの XML の取り込み

| # | Do | Expect |
|---|---|---|
| 66.1 | `schtasks /create /tn "\koyomi-test\夜間" /tr "cmd.exe /c echo hi" /sc daily /st 02:30 /f`, then `schtasks /query /xml /tn "\koyomi-test\夜間" > t.xml` and `koyomi import t.xml --dry-run` | `追加: ジョブネット koyomi-test/夜間`; nothing is written (the file is UTF-16 and reads) |
| 66.2 | `koyomi import t.xml`, then `koyomi show koyomi-test/夜間` and `koyomi next koyomi-test/夜間` | The net has `every = "day"` `at = ["02:30"]` and one job `run = ["cmd.exe", "/c", "echo", "hi"]`; it is 未確認, and after approving `next` shows 02:30 each day |
| 66.3 | A task with `/sc weekly /d MON,FRI /st 08:00 /ri 15 /du 10:00` | `every = "minutes"`, `n = 15`, `from = "08:00"`, `to = "17:59"`, `days = ["mon", "fri"]` |
| 66.4 | A task with `/sc monthly /mo LAST /d TUE` or `/sc daily /mo 2`, or an event trigger | The plan ends with `注意:` lines saying which trigger did not come over; the rest comes in |
| 66.5 | A task that runs as SYSTEM, or with `%windir%` in its command | `注意:` lines on the user and on `%…%` |
| 66.6 | The window's 取り込む box with the same `t.xml` | The same plan and notes as the CLI |
| 66.7 | An XML that is not a task (`<a/>`), and a task with only a ComHandler action | `取り込めない: タスク スケジューラの XML ではない（一番外が <a>）` / `取り込めない: 取り込めるプログラム（<Exec> の <Command>）が無い` |
| 66.8 | From cmd, `schtasks /query /xml /tn "\koyomi-test\夜間" > t2.xml` (the console code page, CP932, with Japanese in it), then `koyomi import t2.xml --dry-run` | It reads, and the plan is the same as 66.1 |

## 67. 遅延の監視

| # | Do | Expect |
|---|---|---|
| 67.1 | A net with `late_end = "20s"` and a job `run = ["ping", "-n", "40", "127.0.0.1"]`; approve it and `koyomi run <net>`; wait 25 s | One notification `⏰ 遅れ  <name>` / `<name>: <time> を過ぎても終わっていない`; `koyomi status --today` shows `⏰終了の遅れ` on the run, and only one notification comes however long it runs |
| 67.2 | `koyomi show <net>` for that net | A line `終わらなければ知らせる: 20s` |
| 67.3 | A net with `[[when]] every = "minutes"` `n = 1`, `overlap = "queue"`, `late_start = "30s"` and a job that takes 3 minutes | When the next minute's time has waited 30 s, one notification `<name>: <time> になっても始まっていない`; when that time starts later, the run is marked `⏰開始の遅れ` and not told again |
| 67.4 | A net that ended 正常 at least 3 times taking about 1 minute; make its job take 3 minutes and run it | After 2 minutes (the usual time ×2, at least the usual time + 1 minute), one notification `<name>: いつもは 1分、もう 2分 動いている`, and the mark `⏰いつもより長い` |
| 67.5 | The same net with `late_slow = false` | No notification for taking long |
| 67.5b | `concurrency = 1`; three nets with `priority = 1`, `3` and `5` (a 1-minute job each) started by hand in that order while a fourth runs | When the fourth ends, the `priority = 5` net starts, then 3, then 1; waiting jobs say `同時に動かす数（1）の空きを待っています`. Approving a net with `priority = 5` shows `空きの枠を先に取る（priority = 5）` |
| 67.6 | `late_end = "6"` or `late_start = "25:00"` in a net; `koyomi check <file>` | `late_end: "06:00"（時刻）か "30m"（長さ）のように書く: "6"` / an error on the time; the net cannot be approved |
| 67.7 | Stop and start the engine after 67.1 | The history keeps the mark (`kind = "late"`); the run is closed as before, and no late notice is sent again |

## 68. 2 つの実行を比べる

| # | Do | Expect |
|---|---|---|
| 68.1 | A net a → b where b prints `start` / `loaded 10` / `end` when a file is there, and `start` / `no such file` and exits 3 when not; run it once with the file and once without; `koyomi compare <run-ok> <run-error>` | `A` and `B` lines as `history` prints the runs, `所要 … → …（±…）`; under ジョブ, `= a …` and `≠ b ✓ 正常終了 … 終了コード 0  →  ✕ 異常終了 … 終了コード 3`; then `ログの違い: b` with ` start`, `-loaded 10`, `-end`, `+no such file` (a's log is the same and is not shown) |
| 68.2 | The same with `--toml` | `a`, `b`, `a_seconds`, `b_seconds` and a `[[job]]` each with `differs`, `[job.a]` / `[job.b]` (`state`, `seconds`, `exit`) and `log_diff` only on b |
| 68.3 | Compare two runs of different nets, one with a job the other lacks | `別のジョブネットです: … と …`; the job only one run has shows `（なし）` on the other side and `≠` |
| 68.4 | `koyomi compare <run> 20261030-999999-00` | `koyomi: 実行がない: 20261030-999999-00`, exit 1 |

## 69. 流さない時間帯

| # | Do | Expect |
|---|---|---|
| 69.1 | A net with `[[when]] every = "minutes"` `n = 2` and `quiet = ["<now>-<now + 5 min>"]` (a span that starts now); approve it | `koyomi show <net>` has a line `流さない時間帯: …（来た回は終わってから流す）`; no run starts in the span; at its end one run starts (`koyomi status --today`), its planned time the first one in the span; the later times in the span are recorded `⊘ スキップ` with `流さない時間帯。H:MM の回が終わりを待っている` |
| 69.2 | The same with `on_quiet = "skip"` | Each time in the span is recorded `⊘ スキップ` with `流さない時間帯（HH:MM-HH:MM）`; the times after the span start as usual |
| 69.3 | 69.1 with `koyomi hold <net>` while the time waits, then let the span end | The time is recorded `⏸` (保留中), not started |
| 69.4 | `koyomi run <net>` in the span | The run starts at once (by hand is not held back) |
| 69.5 | `quiet = ["12:00"]`, `quiet = ["12:00-12:00"]` or `on_quiet = "later"`; `koyomi check <file>` | `quiet: "12:00-13:00" のように書く: "12:00"` / `quiet: 始まりと終わりが同じ: …` / `on_quiet は "wait"・"skip" のどれか`; the net cannot be approved |
| 69.6 | `quiet = ["23:00-01:00"]` on a net at 00:30 | The 00:30 time starts at 01:00 |
| 69.7 | 69.1 with a job that takes 2 minutes, the span ending on one of the net's own times | At the end the waited time starts first; the time due at the end is recorded `⊘ スキップ` `前の回がまだ動いていました` (`overlap = "queue"`: it runs after) |

## 70. 通知の宛先

| # | Do | Expect |
|---|---|---|
| 70.1 | `koyomi.toml`: `[[notify.webhooks]]` three times — one with no `name`, one `name = "経理"`, one `name = "ops"` (each a test URL that shows what it got); a failing net with no `notify_to` | Its `failed` notice reaches the place with no name only |
| 70.2 | The same net with `notify_to = ["経理"]` | `koyomi show <net>` has `通知の宛先: 経理`; the notice reaches 経理 only; the system's notification still shows |
| 70.3 | A net with `notify_to = ["経理"]` and a `kind = "notify"` part with `to = ["ops"]` | The part's notice reaches ops only; the run's end reaches 経理 only |
| 70.4 | `notify_to = ["夜間"]` (no place has it); `koyomi check <file>` | `注意: koyomi.toml の [notify] にその名前の通知先がない: 夜間`; the file still reads (a warning) |
| 70.5 | `notify_to = "経理"`, `notify_to = ["経理", "経理"]`, or `to = ["ops"]` on a plain job; `koyomi check <file>` | `notify_to は通知の場所の名前の配列で書く…` / `notify_to: 同じ名前が 2 つある: 経理` / `to は kind = "notify" にだけ書く`; the net cannot be approved |
| 70.6 | Stop the engine for a while with webhooks named and not, then start it | The outage notice reaches every place |

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
| 72.1 | Download `syukujitsu.csv` from the Cabinet Office's "国民の祝日について" page and put it, as it is (Shift_JIS), in `calendars/`; `koyomi check calendars/syukujitsu.csv` | `問題なし（…〜… 年の祝日 N 日。祝日は <last year>-12-31 まで分かる）` |
| 72.2 | A calendar with `holidays = "japan"`; the window's calendar page for a year in the file | The holidays and their names are the file's |
| 72.3 | Edit the file so its last line is `2028/1/1,元日` (save as UTF-8); wait a few seconds | The band's "祝日の表は … まで" goes away or moves to 2028-12-31; `koyomi check calendars/<name>.toml` says no caution |
| 72.4 | Write a line `x,y` in the middle of the file | The band says `calendars/syukujitsu.csv: N 行目: 日付が読めない: x`; nets run on the built-in rules |
| 72.5 | Remove the file (with today near the end of 2027) | The band says the table lasts until 2027-12-31 and names `syukujitsu.csv` as the way to carry it on |
| 72.6 | `koyomi export -o out.zip --calendars` with the file in `calendars/` | The bundle has the calendars' `.toml` but not `syukujitsu.csv` |

## 73. CSV の書き出し

| # | Do | Expect |
|---|---|---|
| 73.1 | `koyomi list --csv jobs.csv`; open it in Excel | One row per job of every definition, with the heads グループ・ジョブネット・名前・確認・カレンダー・ジョブ…; Japanese is not garbled |
| 73.2 | `koyomi history --from <a week ago> --csv runs.csv` | One row per run, oldest first; 秒 is the run's length; 異常のジョブ names the jobs that ended 異常 |
| 73.3 | `koyomi plan --csv plan.csv` | Every day of this month; a day with no start is one row with only its date and weekday |
| 73.4 | `koyomi plan --csv -` | The table on the console with no BOM, and no file name said |
| 73.5 | A net whose name has a comma; `koyomi list --csv -` | The name is in double quotes; Excel shows it in one cell |
| 73.6 | Set the language to English in Settings; `koyomi plan --csv -` | The heads are Day,Weekday,Time,Jobnet |
| 73.7 | `koyomi history --from <a week ago> --csv runs.xlsx`; open it in Excel | A workbook: bold frozen heads; 日付・開始・終了 are dates (sortable), 秒 is a number |
| 73.8 | A job whose id starts with `=`; `koyomi list --csv jobs.csv` and open it in Excel | The id shows as text with a leading `'`, not as a formula |

## 74. ジョブネットの名前を変える・複製・消す

| # | Do | Expect |
|---|---|---|
| 74.1 | Right-click a net in the tree → "Rename or move…"; pick another group, Enter | The net is under the new group, still confirmed and (if held) still held; Compare versions on it shows its old file |
| 74.2 | A net another net uses as a child net; rename it without the checkbox | The result line lists the nets that use it and says to use `--fix`; nothing moved |
| 74.3 | The same with "Point them at the new name too" ticked | Both moved along; the user's file has the new name in `use`, its comments kept, and a version of its old file |
| 74.4 | `koyomi rename 経理/月次 総務/月次 --fix` | `名前を変えた: 経理/月次 → 総務/月次（参照も直した: …）` |
| 74.5 | "Copy…" on a net, Enter | A `<name>-2` net appears, unconfirmed; the original is not touched |
| 74.6 | "Delete…" on a net; Enter at once | Nothing is deleted (the box starts on Cancel) |
| 74.7 | "Delete…", Tab, Enter | The net goes from the tree; `koyomi versions <id>` lists its file, and `koyomi restore <id> <version>` brings it back unconfirmed |
| 74.8 | `koyomi delete <id>` on a net another net uses | The nets that use it are printed; after `y` the engine refuses and says to change them first |
| 74.9 | Rename a net while it runs | `実行中のため変えられない` |
| 74.10 | With a net open in the editor, right-click a net | The three items are greyed out ("Close the editor first") |

## 75. 繰り返しの部品（`kind = "loop"`）

| # | Do | Expect |
|---|---|---|
| 75.1 | A job with `kind = "loop"`, `file = 'C:\in\*.csv'`, `run = ["cmd.exe", "/c", "echo %KOYOMI_INDEX% %KOYOMI_FILE%"]`; put `b.csv` and `a.csv` in `C:\in`; start the net | The job ends 正常; its log has `▶ 1/2: …a.csv`, `1 …a.csv`, `▶ 2/2: …b.csv`, `2 …b.csv` |
| 75.2 | The same with no file in the folder | 正常 at once; the log says no file matched and nothing ran |
| 75.3 | `count = 3` instead of `file` | Three passes, `KOYOMI_INDEX` 1, 2, 3 |
| 75.4 | `count = 3` with a command that exits 3 on the second pass | 異常, the reason `2/3 回目で異常: 終了コード 3`; the third pass does not run |
| 75.5 | Add `retry = { count = 1, wait = "1s" }` to 75.4 | The second pass is tried again once before the job ends 異常; the reason ends with `やり直し 1 回` |
| 75.6 | Stop the job (強制終了) during a pass of a long loop | The pass is killed and no later pass starts |
| 75.7 | Write both `file` and `count`, or neither | The definition does not load; the problem names the line and says to write one of them |
| 75.8 | In the editor, drag 繰り返し from the shelf onto the canvas | A circle (it runs a process) with the file and count fields over the command fields; Save writes `kind = "loop"` |
| 75.9 | `koyomi review <net>` with a loop | The loop's line shows `このファイルごとに 1 回` or `くり返す回数: 3 回` above its command |

## 76. 外への生存の合図（`[ping]`）

| # | Do | Expect |
|---|---|---|
| 76.1 | Make a check on healthchecks.io (or Uptime Kuma's push monitor); `echo <its URL> \| koyomi secret set ping`; write `[ping]` with `url = "secret:ping"` and `every = "1m"` in koyomi.toml; start the engine | Within a minute the watcher shows a ping; then one each minute |
| 76.2 | Stop the engine (`koyomi engine stop`), or pull the PC's network | The pings stop; the watcher tells of it after its grace time |
| 76.3 | `url = "https://…"` in plain text, `hide_webhooks` not written | No ping goes; the window's notes say the address is in plain text and how to move it to the credential store |
| 76.4 | Add `hide_webhooks = false` to `[notify]` | The plain address is pinged |
| 76.5 | `url = "secret:nope"` (not in the store) | No ping goes; the notes say `nope` is not in the credential store |
| 76.6 | `every = "30s"`, or `url = "hc-ping.com/x"` | koyomi.toml does not read; the engine says why and runs nothing until it is fixed |
| 76.7 | Remove `[ping]` while the engine runs | The pings stop within a minute, with no note |

## 77. 月の成績（report）

| # | Do | Expect |
|---|---|---|
| 77.1 | `koyomi report` after a few runs | The month, then a line per net: runs, pass rate, usual time, and the jobs that failed most |
| 77.2 | `koyomi report 2026-09 --csv r.csv` | A CSV with one row per net, rates in percent and averages in seconds |
| 77.3 | History view, open 成績 | The same lines for the rows the filter shows |

## 78. システムのイベントログ（Application ログ、ソース `koyomi`）

| # | Do | Expect |
|---|---|---|
| 78.1 | Let a net end ✓, then a net that fails (`exit 1`) | Event Viewer → Windows ログ → Application, source `koyomi`: id 1000 (情報) and id 2000 (エラー) with the title and the failed jobs |
| 78.2 | A net stopped at its `timeout`; one stopped with 強制終了 | id 2001 and id 2002 (エラー) |
| 78.3 | The failed net runs well on the next start | id 1100 (情報) after the 1000 |
| 78.4 | A net held with ⏸ | Nothing is written (a person did it) |

## 79. 動かす条件（`require`）

| # | Do | Expect |
|---|---|---|
| 79.1 | Write `require = ["ac"]` in a net; unplug the laptop; run it | The job waits (待機) with the reason 「AC 電源につながるのを待っています」; no slot is taken |
| 79.2 | Plug the AC in again | The job starts within a few seconds |
| 79.3 | `require = ["network"]`, turn the network off, then on | Waits with 「ネットワークにつながるのを待っています」, then starts |
| 79.4 | `require = ["idle:1m"]`, keep using the mouse | Waits with 「PC が 1 分使われなくなるのを待っています」; starts after a minute untouched |
| 79.5 | `require = ["wifi"]` | The net is invalid, with the message 「require は …」 |

## 80. よくある作業の部品（`kind = "delete"` / `"zip"` / `"copy"` / `"empty"`）

| # | Do | Expect |
|---|---|---|
| 80.1 | A folder with an old `a.log` (made 40 days ago), a new `b.log` and an old `c.txt`; a net with `kind = "delete"`, `path` = that folder, `match = "*.log"`, `keep_days = 30`; run it | Only `a.log` goes; the log says `deleted: …a.log` and `1 file(s) deleted` |
| 80.2 | `kind = "zip"`, `from = 'D:\out\*.csv'`, `into = 'D:\arc\{date}.zip'` | `D:\arc\<today>.zip` holds the csv files; run again: replaced, ✓ |
| 80.3 | `kind = "copy"` from a folder with subfolders into a new folder, then run again | All copied ✓; the second run copies nothing and is still ✓; a file only in `into` stays |
| 80.4 | `kind = "copy"` with `into` on a share that does not exist | ✕ (robocopy 8 or more); `retry` tries again |
| 80.5 | `kind = "empty"`, `path` = a folder with files and subfolders | The folder stays, empty; the log says how many items went |
| 80.6 | `path = 'C:\'` on `empty`, `keep_days = 0` on `delete`, `into = 'a.7z'` on `zip`, `run` on a chore | Each is refused when read, with the line and why |
| 80.7 | The editor: drag 古いファイルを消す from the parts; fill フォルダー and 名前 | The pane shows フォルダー・名前・何日より古いものを消すか, timeout and retry; saving writes `kind = "delete"`; `D:\` there shows the error and is not applied |
| 80.8 | Approve a net with a chore | The confirm box shows each chore's paths and days instead of a command |

## 81. ログの行・イベントログで起動する（`[[when]] log` / `eventlog`）

| # | Do | Expect |
|---|---|---|
| 81.1 | A net with `[[when]] log = 'D:\app\app.log'` and `contains = "ERROR"`; `koyomi approve`; append `ok` and then `ERROR disk full` to the file | A run starts within a few seconds (only for the second line); `koyomi show` says `ログ`（`D:\app\app.log に行が書かれた: ERROR disk full`） |
| 81.2 | The file already has `ERROR` lines when the net is approved (or the engine starts) | Nothing starts: only lines written after the watch starts count |
| 81.3 | Append three `ERROR` lines at once | One run; the reason ends with `（ほか 2 件）` |
| 81.4 | Append an `ERROR` line while the run is still going | No second run; a ⊘ スキップ row `… に行が書かれたが、前の回がまだ動いているので流さない: …` |
| 81.5 | Rotate the log (rename `app.log` to `app.1.log` and make a new `app.log`), then append an `ERROR` line to the new file | A run starts: the new file is read from its start |
| 81.6 | `[[when]] log = 'D:\nothere\app.log'`; then make the folder and the file with an `ERROR` line | Nothing is said while the file is not there; the new file is read from its start and the run starts |
| 81.7 | `[[when]] eventlog = "Security"`, `ids = [4625]` (run the engine as administrator); sign in with a wrong password once | A run starts; `koyomi show` says `イベントログ Security に記録された: ID 4625 …` |
| 81.8 | `[[when]] eventlog = "Application"`, `source = "MyApp"`; `eventcreate /L APPLICATION /SO MyApp /T ERROR /ID 100 /D "boom"` | A run starts; an event from another source does not start one |
| 81.8a | `[[when]] eventlog = "Application"`, `ids = [1000, 2000]` and no `source`; run any net by hand so it ends ✓ (koyomi writes ID 1000, source `koyomi`) | The net does not start: koyomi's own records are not counted when no `source` is written. Then write `source = "koyomi"` and `ids = [2000]`, and run another net that fails: this net starts (named on purpose, koyomi's records count); it ends ✓ (1000), so it does not start itself again |
| 81.9 | `eventlog = "NoSuchLog"`, or `Security` with the engine not as administrator | The window's 注意 says `<net>: イベントログ … を読めない（…）`; the net is still valid |
| 81.10 | The net has `calendar = "会社"` and the line comes on a closed day; or the net is held | ⊘ スキップ `… が、休業日なので流さない（on_closed = "run" で流せる）`; ⏸ 保留 with the reason, not run afterwards |
| 81.11 | The editor: add a `[[when]]` of ログに行が出たら and one of イベントログに出たら | The fields are ログのファイル・含む文字, and イベントログ・イベント ID・ソース; except / start / end / on_closed are not shown; saving writes `log` / `contains` and `eventlog` / `ids` / `source` |
| 81.12 | `koyomi next` for the net | The rows `ログに「ERROR」の行が書かれたとき: D:\app\app.log` and `イベントログ Security に記録されたとき（ID 4625）` come first |
| 81.13 | `koyomi check` a `[[when]]` with `log = 'app.log'`, with `log = 'D:\*.log'`, with `log = …` and no `contains`, with `eventlog = "Application"` and neither `ids` nor `source`, with `ids = [70000]` | `log は絶対パスで書く（例: …）`; `log は 1 つのファイルのパス（* / ? は使えない）`; `contains が要る（例: contains = "ERROR"）`; `eventlog には ids か source が要る（例: ids = [4625]）`; `ids はイベント ID（0〜65535）の配列で書く（例: ids = [4625]）` |

## 82. ファイルを送る・受け取る部品（`kind = "send"` / `"fetch"`）

| # | Do | Expect |
|---|---|---|
| 82.1 | `kind = "send"`, `from = 'D:\out\*.csv'`, `into = '\\nas\share\in\{date}'`; run it | The csv files are in the share's folder for today (made when missing); the log says `sent: …` for each; ✓ |
| 82.2 | `kind = "fetch"`, `from = '\\nas\share\out\*.csv'`, `into = 'D:\in'`; run it; then once more with no csv in the share | ✓ and the files are in `D:\in`; the second run is ⚠ 警告 with `no file to fetch: …` |
| 82.3 | An SFTP server you can reach with a key; `koyomi sftp hostkey host` prints `host_key = "SHA256:…"`; `kind = "send"`, `into = "sftp://user@host/in"`, `key = 'C:\keys\id_ed25519'` and that `host_key` | ✓; the files are in `/in` on the server; the log says `sent: <name> (<n> bytes)` for each; `known_hosts` is not used |
| 82.4 | `kind = "fetch"`, `from = "sftp://user@host/~/out/*.csv"`, `into = 'D:\in'` | ✓; each csv under the home's `out` is in `D:\in`; a folder that matches is not fetched; no match is ⚠ 警告 |
| 82.5 | The same send with a wrong `host_key`, to a wrong port, or with a key the server does not take; then `password = "secret:sftp"` instead of `key` after `koyomi secret set sftp` | ✕ with exit code 2 and `! <why>` in the log; nothing asks anything; `retry` tries again; with the password it is ✓ and the log never shows it |
| 82.6 | `koyomi check` a send with `from = "sftp://…"`, a fetch with `into = "sftp://…"`, `into = "sftp://-oProxyCommand=x/in"`, `from = "sftp://h/out/"` on fetch, `key` on `copy`, an sftp:// send without `host_key`, `password = "plain"`, both `key` and `password` | Each is refused with the line and why; the missing `host_key` names `koyomi sftp hostkey` |
| 82.7 | The editor: drag ファイルを送る and ファイルを受け取る from the parts | The pane shows 送るもの・送り先・SSH の秘密鍵 (受け取るもの・置くフォルダー・SSH の秘密鍵), timeout and retry; and ホスト鍵・パスワード; saving writes `kind = "send"` / `"fetch"` with `from`・`into`・`key`・`host_key`・`password` |
| 82.8 | Approve a net with a send | The confirm box shows 元・先・SSH の鍵・ホスト鍵・パスワード instead of a command |

## 83. HTTP で呼ぶ部品（`kind = "http"`）

| # | Do | Expect |
|---|---|---|
| 83.1 | `kind = "http"`, `url = "https://example.com/"`; run it | ✓; the log starts with `HTTP 200` and then the start of the page (cut at 4000 characters with ` ...`) |
| 83.2 | `url = "https://example.com/nope"`; run it; then add `expect = [404]` and run again | The first is ✕ with exit code 1 and `HTTP 404`; the second is ✓ |
| 83.3 | `url = "https://no-such-host.invalid/"` with `retry = { count = 1, wait = "1m" }` | ✕ with exit code 2 and `no answer: …` (why) in the log; it is tried once more after a minute |
| 83.4 | A test API you can see (e.g. a local echo server): `method = "POST"`, `body = '{"day": "{date}", "名前": "テスト"}'`, `headers = { Authorization = "secret:api", X-Run = "{date}" }` with the secret `api` set | The server gets the body as UTF-8 JSON with today's date filled in, `Content-Type: application/json; charset=utf-8`, the Authorization value of the secret, and `X-Run` as written (`{date}` is not filled in a header); the job's log shows neither the secret nor the script |
| 83.5 | The same with `content_type = "text/plain"` | The server gets `Content-Type: text/plain` |
| 83.6 | `koyomi check` with `url = "ftp://x"`, `method = "get"`, `body` with `GET`, `expect = [99]`, `expect = []`, `headers = { Content-Type = "x" }`, `content_type` without `body`, `url` on `copy` | Each is refused with the line and why |
| 83.7 | The editor: drag HTTP で呼ぶ from the parts | The pane shows URL・メソッド・送る中身・ヘッダー (one `名前: 値` a line)・中身の種類・正常とする状態コード, timeout and retry; saving writes `kind = "http"` with those fields (`method` only when not GET); a header line without `:` or a code that is not a number is refused |
| 83.8 | Approve a net with an http | The confirm box shows `POST https://…`, the body, the header names (not their values) and the codes instead of a command |

## 84. 別のアカウントで動かす（`run_as`）

Make a test account first: `net user koyomi-test <password> /add` (as administrator), and `koyomi secret set koyomi-test` with the same password.

| # | Do | Expect |
|---|---|---|
| 84.1 | A job `run = ["C:/Windows/System32/whoami.exe"]` with `run_as = { user = "koyomi-test", password = "secret:koyomi-test" }`; approve and run it | ✓; the log has `▶ koyomi-test として動かす` and then `<computer>\koyomi-test` |
| 84.2 | The same with `run = ["C:/Windows/System32/cmd.exe", "/c", "echo %KOYOMI_JOB% %USERPROFILE%"]` | The job's id and the test account's profile folder |
| 84.3 | `koyomi secret delete koyomi-test` (or a wrong secret name), then run | ✕ with `koyomi-test のパスワードの秘密がない: … （koyomi secret set …）`; nothing is started |
| 84.4 | Set the secret to a wrong password; run | ✕ with Windows' reason (the user name or password is incorrect) in the log; the password is not in the log |
| 84.5 | A job that prints the password (`cmd /c echo <password>`) as the account | The log shows `***` |
| 84.6 | A long job (`ping -n 600 127.0.0.1`) as the account; stop it from the window; and one with `timeout = "5s"` | 強制終了 / 打ち切り; no `ping.exe` is left running (Task Manager, user koyomi-test) |
| 84.7 | An administrator account as `run_as`, running `net session` | It fails with access denied: there is no elevation |
| 84.8 | A `kind = "copy"` to a share only koyomi-test can write, with `run_as` | ✓; the files are in the share |
| 84.9 | `koyomi check` with `password = "hunter2"`, `password = "secret:"`, no `user`, `user = "a b"`, an unknown key, `run_as` on a `"time"` part | Each is refused with the line and why; the plain password one says to use `koyomi secret set` |
| 84.10 | The editor: a job's pane, fill 実行のアカウント and the secret name; save | The file has `run_as = { user = …, password = "secret:…" }`; only one of the two filled is refused |
| 84.11 | Approve a net with `run_as` | The confirm box shows `実行のアカウント: CORP\batch（パスワードは秘密 batch）`, and no password |

## 85. 繰り返しの回を並べて動かす（`max`）

| # | Do | Expect |
|---|---|---|
| 85.1 | `concurrency = 4`; a loop `count = 6`, `max = 3`, `run = ["cmd.exe", "/c", "ping -n 6 127.0.0.1 > nul & echo %KOYOMI_INDEX%"]`; run it | It ends in about 10 s (not 30); Task Manager shows 3 `ping.exe` at a time; the log starts with `▶ 6 回を 3 つずつ並べて動かす` and has each `▶ i/6` once, each followed by its index |
| 85.2 | `concurrency = 2`; the same loop with `max = 3`, and another net's job started while it runs | The loop runs 2 at a time (`▶ 6 回を 2 つずつ…`); the other job waits with `同時に動かす数（2）の空きを待っています` until the loop ends |
| 85.3 | `file = 'C:\in\*.csv'` with 5 files, `max = 2`, a script that echoes `%KOYOMI_FILE%` | ✓; each file once, under its own heading |
| 85.4 | `count = 6`, `max = 2`, where pass 1 exits 3 at once and the others sleep 5 s | ✕ with `1/6 回目で異常: 終了コード 3`; pass 2 ran to its end; passes 3–6 did not start |
| 85.5 | `count = 4`, `max = 2`, `ping -n 600 127.0.0.1`; stop it from the window | 強制終了 at once; no `ping.exe` is left; the log has `! 強制終了` under both running passes |
| 85.6 | `count = 4`, `max = 2`, `timeout = "5s"`, `ping -n 600 127.0.0.1` | ✕ 打ち切り after about 5 s; no more passes start |
| 85.7 | `koyomi check` with `max = 0`, `max = 17`, `max = "2"`, `max` on a `"job"` | Each is refused with the line and why |
| 85.8 | The editor: a loop's pane, fill max with 3; save; then empty it | The file has `max = 3`, then no `max`; `0` or `x` is refused |
| 85.9 | Approve a net with `max = 3` | The confirm box shows `並べて動かす数: 3` under the loop |

## 86. 長さの書き方（D4）

| # | Do | Expect |
|---|---|---|
| 86.1 | Run a net whose job takes about 1 minute 5 seconds; `koyomi show <run>` | The run and the job lines read `1分5秒`, not `0:01:05` |
| 86.2 | A job of a few seconds; open the run in the window and pick the job | 所要 reads `3秒`; a 41-minute one reads `41分`; a 65-minute one `1時間5分` |
| 86.3 | Set `lang = "en"` in common.toml; repeat 86.1 | `1 min 5 s` |
| 86.4 | `koyomi compare <a> <b>` for two runs that took 1 min and 1 min 10 s; `koyomi report` | `1分  →  1分10秒  (+10秒)`; いつもは in words |

## 87. Start over HTTP (`[http]`, TODO 82)

| # | Do | Expect |
|---|---|---|
| 87.1 | `koyomi secret set koyomi-http` (a long random value); koyomi.toml `[http]` `port = 8790`, `token = "secret:koyomi-http"`; wait a few seconds; `koyomi status` | The status has `HTTP: 127.0.0.1:8790 で待ち受けている`; `netstat -an` shows 127.0.0.1:8790 only, not 0.0.0.0 |
| 87.2 | `curl -i -X POST http://127.0.0.1:8790/runs -d '{"net":"g/a"}'` with no token, then with `-H "Authorization: Bearer wrong"` | 401 both times; no run starts; the engine's log has `http: POST /runs 401 …` and never the token |
| 87.3 | The same with the right token and `-d '{"net":"g/a","params":{"day":"2026-10-31"}}'` | 202 and `{"run":"<id>"}`; the run goes with that param; 操作の記録 has a by-hand run |
| 87.4 | `curl -H "Authorization: Bearer …" http://127.0.0.1:8790/runs/<id>` while it runs and after it ends; then a run id of yesterday | JSON with `state`, `jobs`, `"over": false`, then `true`; yesterday's run is found too |
| 87.5 | POST a net that is held, unconfirmed or does not exist; POST `{}`, `{"net":"g/a","dry":true}`, `not json`; GET `/runs/nope`; `PUT /runs` | 422 with the same reason `koyomi run` gives; 400 for the bodies; 404; 405 |
| 87.6 | Write `token = "plain"`, then `port = 0`, then `host = "0.0.0.0"` under `[http]` | Each is refused when koyomi.toml is read (the band says why); nothing listens |
| 87.7 | `koyomi secret delete koyomi-http` and restart the engine; or take 8790 with another program first | A note says it cannot listen and why; it is tried again every 60 s; once fixed it listens without a restart |
| 87.8 | Remove `[http]` while the engine runs | It stops listening within a few seconds; `koyomi status` has no HTTP line |
| 87.9 | `ipv6 = true`; `curl -g "http://[::1]:8790/runs/<id>"` with the token | Answers the same as on 127.0.0.1 |

## 88. The tray apart from the engine (`koyomi engine --no-tray`, `koyomi tray`, TODO 86)

| # | Do | Expect |
|---|---|---|
| 88.1 | `koyomi engine --no-tray` | No koyomi icon in the task tray; `koyomi engine status` answers |
| 88.2 | Then `koyomi tray` | The icon appears with the engine's color and tooltip; a second `koyomi tray` says `トレイはもう動いている` and exits 1 |
| 88.3 | Run a net that ends ✕ with `[notify] system` on for failures | The tray shows the OS notification within about a second; the engine shows none itself |
| 88.4 | Right-click the icon | The last item reads `トレイを閉じる（エンジンは動き続ける）` and is enabled even while a job runs; 一時停止 pauses the engine (`koyomi engine status` says so) |
| 88.5 | `koyomi engine stop` while the tray is up | The icon turns grey with `koyomi: エンジンは動いていない`; start the engine again with `--no-tray` and it comes back without old notifications |
| 88.6 | Pick トレイを閉じる | The tray exits; `koyomi engine status` still answers |
| 88.7 | `koyomi engine` (with its own tray), then `koyomi tray` | `koyomi tray` says the engine has its own tray and exits 1 |

## 89. The engine as a Windows service (`koyomi service`, TODO 86)

| # | Do | Expect |
|---|---|---|
| 89.1 | `koyomi service status`; `koyomi service start`; `koyomi service run` (not elevated, nothing installed) | `サービスは入っていない`; `入っていない` and exit 1; `サービスとして起こされたときだけ動く` and exit 2 |
| 89.2 | `koyomi service install` in a terminal that is not elevated | It asks for the password, then says `管理者の権限が要る。「管理者として実行」…`; nothing is installed |
| 89.3 | `koyomi engine stop`; `koyomi startup on`; then `koyomi service install` in an elevated terminal, giving your Windows password | It says it installed `koyomi-<user>` and started it; `services.msc` shows `koyomi (<user>)`, 自動（遅延開始）, logged on as you; `koyomi startup` says `トレイだけ` |
| 89.4 | In a terminal that is not elevated: `koyomi service stop`, `koyomi service status`, `koyomi service start` | Each works without an administrator; status shows 止まっている then 動いている（pid …） |
| 89.5 | With the service running: `koyomi engine status`; open the window; `koyomi run g/a` | The window and the CLI talk to the service's engine; no second engine starts; the run goes |
| 89.6 | `koyomi service stop` while a long job runs | The job's process tree ends (as with `koyomi engine stop`); the run ends 強制終了; `engine.log` in the settings folder has the engine's lines with times |
| 89.7 | With the service stopped, open the window or `koyomi status` | `サービスのエンジンが止まっている。koyomi service start で始める`; no engine is started by hand |
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
| 90.1 | `curl -H "Authorization: Bearer T" http://127.0.0.1:8790/health`; then the same with no token | `{"ok": true, "version": …, "proto": …, "status": "<koyomi status's line>"}`; 401 without the token |
| 90.2 | `GET /nets` with the token, with one net held and one unconfirmed | Each net with `id`, `group`, `name`, `standing` (`confirmed`/`unconfirmed`/`invalid`) and `held`; no definition text; `paused` follows `koyomi pause` |
| 90.3 | `GET /runs`; then `GET "/runs?from=2026-10-30&to=2026-10-31"`; then `?from=bad` and `?to=2026-10-31` | Today's and running runs, each with `"over"`; the two days' runs, oldest first; 400 for both bad ones |
| 90.4 | `GET "/runs/<id>/log?job=<job>"` for a job whose command has a registered secret in its arguments | The log's tail as `koyomi log` shows it, the secret as `***`; without `job` 400; an unknown job 404 |
| 90.5 | `POST /nets`, `DELETE /health` | 405; nothing changes |

## 91. メールを受けて起動する（`[[when]] mail`）

| # | Do | Expect |
|---|---|---|
| 91.1 | `koyomi secret set m` (the mailbox's password); a net with `[[when]] mail = "imaps://me@example.com@imap.example.com/INBOX"`, `password = "secret:m"`, `subject = "請求"`; `koyomi approve`; send a mail with the subject `11月の請求書` to the mailbox | A run starts within about a minute; `koyomi show` says `メール`（`imaps://… にメールが来た: 差出人 <…>「11月の請求書」`） |
| 91.2 | The mailbox already has mail with `請求` when the net is approved (or the engine starts) | Nothing starts: only mail that comes after the watch starts counts |
| 91.3 | Send three matching mails within the same minute | One run; the reason ends with `（ほか 2 件）` |
| 91.4 | `sender = "billing@"`, no `subject`; send one mail from `Billing <BILLING@corp.example>` and one from another address | Only the first starts a run (any case) |
| 91.5 | Send a matching mail while the run is still going | No second run; a ⊘ スキップ row `… にメールが来たが、前の回がまだ動いているので流さない: …` |
| 91.6 | Open the mailbox in a mail client after the run | The mail is still unread and in its folder: koyomi does not mark, move or delete mail |
| 91.7 | `password = "secret:nothere"`; or a wrong password with `koyomi secret set`; or `/NoSuchFolder` | The window's 注意 says `<net>: メールボックス … を読めない（…）`; the net is still valid; after fixing it the warning goes within about a minute; the password never appears in the log or the warning |
| 91.8 | The net has `calendar = "会社"` and the mail comes on a closed day; or the net is held | ⊘ スキップ `… が、休業日なので流さない（on_closed = "run" で流せる）`; ⏸ 保留 with the reason, not run afterwards |
| 91.9 | The editor: add a `[[when]]` of メールが来たら | The fields are メールボックス・パスワード・差出人・件名; except / start / end / on_closed are not shown; saving writes `mail` / `password` / `sender` / `subject`; a password not starting with `secret:` is refused |
| 91.10 | `koyomi next` for the net; approve it | The row `メールが来たとき: imaps://…（件名「請求」）` comes first; the confirm box shows the same |
| 91.11 | `koyomi check` a `[[when]]` with `mail = "imap://me@host"`, with `mail = "me@host"`, with `password = "hunter2"`, with no `password`, with neither `sender` nor `subject` | `mail は imaps:// だけ…`; `mail は "imaps://ユーザー@サーバー/フォルダー" で書く…`; `password は "secret:<名前>" で書く…`; `mail には password = "secret:<名前>" が要る…`; `mail には sender か subject が要る…` |

## 92. Another PC over the LAN (`[http] lan = true`, `[[remote]]`, `koyomi remote`; TODO 80)

PC A runs the engine with `[http]` as in §87 plus `lan = true`; PC B calls it. `T` is A's token.

| # | Do | Expect |
|---|---|---|
| 92.1 | On A: `koyomi http fingerprint`; start the engine; `koyomi status` | 64 hex digits in pairs with `:`; the status says `HTTP: LAN の :8790 で TLS で待ち受けている（指紋 …）` with the same fingerprint; the certificate is in the settings folder's `tls`, and the same after a restart |
| 92.2 | From B: `curl http://A:8790/health`; `curl -k -H "Authorization: Bearer T" https://A:8790/health` | Plain HTTP gets no answer; over TLS the same JSON as §90.1 |
| 92.3 | On B: `koyomi remote peek https://A:8790` | `fingerprint = "…"`, the same as 92.1 shows on A; an address with nothing listening says it cannot reach it |
| 92.4 | On B: `koyomi secret set a-token` (T); koyomi.toml gets `[[remote]]` with `name = "a"`, `url = "https://A:8790"`, the fingerprint and `token = "secret:a-token"`; `koyomi remote` | `a  https://A:8790  <A's status line>` |
| 92.5 | On B: `koyomi remote status a`, `remote nets a`, `remote runs a` | A's status line and version; A's nets with their standing and 保留; today's runs, the running ones marked 動いている |
| 92.6 | On B: `koyomi remote run a <group/name>` | `a: <group/name> を流した（<run id>）`; the run shows on A as started over HTTP |
| 92.7 | On B: change one digit of the fingerprint; `koyomi remote status a` | `証明書が koyomi.toml の fingerprint と違う（今見えたのは …）…`; nothing is sent |
| 92.8 | On B: `koyomi secret set a-token` with a wrong value; or delete it | `token が合わない…`; `token の秘密 a-token がない…` |
| 92.9 | `koyomi check` koyomi.toml with `url = "http://A:8790"`, a short fingerprint, `token = "T"`, two `[[remote]]` with the same name | Each refused: `url は https://<PC>:<port> で書く…`, `fingerprint は SHA-256…`, `token は secret:<名前> で書く`, `name … が 2 つある` |
| 92.10 | `koyomi remote status nope`; `koyomi remote frob` | `nope: [[remote]] にない（あるのは a）`; the usage line, exit 2 |

## 93. Standby for another PC (`[standby]`, `koyomi standby [giveback]`; TODO 87)

PC A runs nets as in §92 (`lan = true`). PC B has the same nets confirmed, `[[remote]]` `a` as in §92.4, and `[standby]` with `watch = "a"`, `every = "5s"`, `misses = 2`.

| # | Do | Expect |
|---|---|---|
| 93.1 | Start B's engine; `koyomi status` and `koyomi standby` on B | `待機中: a を見張っている。こちらではスケジュールとイベントで流さない`; the window's cautions show the same line |
| 93.2 | Let a net's time pass on both | It runs on A only; B's history has no row for it |
| 93.3 | On B: `koyomi run <net>` | It runs on B (by hand still goes) |
| 93.4 | Stop A's engine; wait about 15 s | B tells `koyomi: 待機から引き継いだ` with `a が答えなくなった（つながらない: …）ので、… からこの PC で流す。向こうで動いていた回は引き継いでいない…`; `koyomi status` says `a から引き継いで流している（… から。koyomi standby giveback で戻す）`; the next time runs on B |
| 93.5 | Restart B's engine | Still taken over at once (`standby.toml` in the state folder has `active = true`) |
| 93.6 | Start A's engine again | B tells `koyomi: 両方で流している` once, and stays taken over |
| 93.7 | On B: `koyomi standby giveback`; again | `a に戻した。また見張る`, back to 93.1; the second says `引き継いでいない（a を見張っている）`; `koyomi ops` shows `giveback` |
| 93.8 | On B: change one digit of `a`'s fingerprint; stop A | A note `[standby] a: 証明書が fingerprint と違う…`; B does not take over while the note shows (only no connection counts) |
| 93.9 | `koyomi check` koyomi.toml with `watch = "nope"`, `every = "1s"`, `misses = 0` | Each refused: `[standby] の watch "nope" は [[remote]] の name で書く`, `every は 5s から 10m まで`, `misses は 1 から 20 まで` |
| 93.10 | Remove `[standby]` from B's koyomi.toml | B runs its schedules again; `koyomi standby` says `koyomi.toml に [standby] がない` |

## 94. People and roles over HTTP (`[[http.user]]`; TODO 83)

`[http]` as in §87 plus `[[http.user]]` `sato` (`role = "view"`, `token = "secret:sato"`) and `ito` (`role = "run"`, `token = "secret:ito"`); `koyomi secret set sato` / `ito` with `S` / `I`. `T` is `[http]`'s token.

| # | Do | Expect |
|---|---|---|
| 94.1 | `curl -H "Authorization: Bearer S" http://127.0.0.1:8790/runs` | 200 and the runs |
| 94.2 | `curl -X POST -H "Authorization: Bearer S" -d '{"net":"<group/name>"}' http://127.0.0.1:8790/runs` | 403 `sato は view のロール。これには run が要る`; nothing starts |
| 94.3 | The same POST with `I`; then with `T` | 202 both times; `koyomi ops` shows the two `run` lines ending `（HTTP: ito）` and `（HTTP: default）` |
| 94.4 | Run a net from this PC's window and with `koyomi run` | Their `koyomi ops` lines have no `（HTTP: …）` |
| 94.5 | Remove `token` from `[http]` (keep the users); restart the engine | It listens; `T` gets 401, `S` and `I` as before |
| 94.6 | `koyomi check` with `role = "admin"`, `name = "default"`, two users named `sato`, `token = "S"`, and `[http]` with neither `token` nor users | Each refused with its reason |
| 94.7 | `koyomi secret delete sato` (or never set it); restart | The status says `[http] で待ち受けられない（token の秘密 sato: …）` and tries again every 60 s |

## 95. The Web UI (`http://127.0.0.1:<port>/`; TODO 81)

`[http]` and `[[http.user]]` as in §94 (`S` view, `I` run, `T` default). Open the engine's address in a browser.

| # | Do | Expect |
|---|---|---|
| 95.1 | Open `http://127.0.0.1:8790/` | The サインイン form; no other request than `/`, `/theme.css`, `/app.css`, `/app.js`, `/me` (the browser's network tab) |
| 95.2 | Sign in with a wrong token | トークンが合いません; still signed out |
| 95.3 | Sign in with `S` | The header shows `sato（view）` and 稼働中; at the left ジョブネット一覧 by group, each net with the mark of its last run today; in the middle 今日 with today's date; no 今すぐ流す or 強制終了 anywhere; the token field is empty |
| 95.4 | Look at 今日 with nets that start later today and runs that ended, failed and run now | As the window's 今日: a ○ 待ち row for each start still to come (予定 and 次の回), then the runs newest first (開始・終了・時間, `…` while running); rows tinted by state; a chip per state that has rows (`✕ 異常終了 1` …) |
| 95.5 | Press a chip; press it again | Only that state's rows; then all again |
| 95.6 | Click a run row | The pane at the right: its state, `id · date · times · time taken`, the reason, and its jobs with state, times and exit code |
| 95.7 | ログ on a job of it | The log tail under the jobs, secrets as `***`; while the run goes it follows |
| 95.8 | Click a 待ち row, or a net in the tree | The pane shows the net: its id, 次の回, 今日の予定 and today's runs of it (a click opens the run) |
| 95.9 | Reload the page | Still signed in (the cookie); refreshes every 5 s; the picked row and the focus stay through a refresh |
| 95.10 | 履歴, then pick a day | That day's runs; no 次の回 column; 今日 comes back to today's |
| 95.11 | In the window's 設定, set the theme to a dark one, then a light one, then system, reloading the page each time; then put `high_contrast = true` in common.toml and reload | The page takes the window's colours each time, also with a page-darkening add-on such as Dark Reader on; with system it follows the browser's light or dark; with high_contrast the stronger colours |
| 95.12 | `curl -i http://127.0.0.1:8790/theme.css` | 200 `text/css` with `:root {` and `--bg`; no token needed |
| 95.13 | サインアウト, then sign in with `I` | `ito（run）`; on a picked confirmed net 今すぐ流す (greyed on 未承認 and held ones) |
| 95.14 | 今すぐ流す a net, then 今すぐ流す in the question | A new running run in 今日, picked in the pane; `koyomi ops` shows `run … （HTTP: ito）` |
| 95.15 | Pick a running run, 強制終了 | The question starts on やめる; with 強制終了する its jobs stop (■ 強制終了); `koyomi ops` shows the kill with `（HTTP: ito）` |
| 95.16 | `curl -i http://127.0.0.1:8790/` | 200 with `Content-Security-Policy` (`script-src 'self'`, `style-src 'self'`, `frame-ancestors 'none'`), `X-Content-Type-Options: nosniff`, `Cache-Control: no-store` |
| 95.17 | With a signed-in cookie, `curl -X POST --cookie "ito_session=…" -d '{"net":"<group/name>"}' http://127.0.0.1:8790/runs` (no `X-Ito-Csrf`) | Refused; nothing starts |
| 95.18 | Put a job's title or log line `<img src=x onerror=alert(1)>` and view it | Shown as text; no dialog |
| 95.19 | With `[http] lan = true`, open `https://<PC>:<port>/` from another PC (accept the self-signed certificate after checking its fingerprint) | The same page; the cookie is `Secure` |
| 95.20 | Pause the schedule (`koyomi engine pause`); stop the engine | スケジュール停止中 in the header and a band over 今日; then エンジンに繋がらない in red, the page stays |
| 95.21 | With `[standby]` (or `[primary]`) on the engine; then §102's take-over and give-back | The header says which PC runs the schedules in the window's words and colours (40.5, 40.6): `待機中: … を見張っている` amber, `… から引き継いで流している（YYYY-MM-DD HH:MM から）` green, `… が待機している` grey; after a give-back the standby's chip ends `（YYYY-MM-DD HH:MM に戻した）` |
| 95.22 | Make the window narrower than 900 px | The tree, 今日 and the pane stack in one column; nothing is cut off |

## 96. Another PC in the window (the PC list; TODO 80)

Two PCs: `B` with `[http] lan = true` and a `[[http.user]]` `I` of role `run` (and `S` of role `view`); this PC with `[[remote]]` `name = "b"` for B (`koyomi secret set` holds `I`'s token).

| # | Do | Expect |
|---|---|---|
| 96.1 | Open the window with no `[[remote]]` | No PC list at the top |
| 96.2 | With `[[remote]]` `b`, open the window | `PC [この PC ▾]` at the top; this PC's page as before |
| 96.3 | Pick `b` | `b のエンジン`; B's jobnets (承認済み・未確認・保留) and today's runs; the tree and tabs are gone |
| 96.4 | Pick a run, then one of its jobs | Its jobs with marks; the end of the job's log, secrets as `***` |
| 96.5 | 今すぐ流す on a net, then やめる | Nothing starts |
| 96.6 | 今すぐ流す, then 今すぐ流す in the question | `g/n: <run id>`; within 5 s the run is in the list; on B, `koyomi ops` shows `（HTTP: ito）` |
| 96.7 | 強制終了 on a running run, then 強制終了する | B's jobs stop; the run shows 強制終了 |
| 96.8 | Put `S`'s token in the secret instead and press 今すぐ流す | The refusal (403) in red; nothing starts |
| 96.9 | Stop B's engine | つながらない … in red under ジョブネット; the window stays usable; start it again and 読み直す |
| 96.10 | Change B's certificate fingerprint in `[[remote]]` | 証明書が koyomi.toml の fingerprint と違う …; nothing is sent |
| 96.11 | With `b` shown, open 次の 1 回だけスキップ from the tray | The window comes back to この PC with the box |
| 96.12 | Pick この PC | This PC's page, as it was |
| 96.13 | Let a run on B fail, pick it, 再実行, then やめる | Nothing starts |
| 96.14 | 再実行, then 再実行 in the question | `<run> → <new run>`; the new run starts from the failed job, the passed ones carried over; on B, `koyomi ops` shows `rerun … （HTTP: ito）` |
| 96.15 | With `S`'s token, 再実行 | The refusal (403) in red; nothing starts |
| 96.16 | On B, a confirmed net with `[params]` `month = "{month-1:%Y-%m}"` and `dept = "all"`; here, 今すぐ流す on it | Above the question, a field for each value with its default in it |
| 96.17 | Change `dept` to `経理`, leave `month`, then 今すぐ流す in the question | A run starts on B; its job sees `dept = 経理` and last month for `month` |
| 96.18 | 今すぐ流す on a net without `[params]` | Only the question, no fields |
| 96.19 | On B's net row, ジョブネットを保留, then ジョブネットを保留 in the question | The line says `保留した: …`; within 5 s the row shows 保留 and the button turns to ジョブネットの保留を解く; on B, `koyomi ops` shows `hold … （HTTP: ito）` |
| 96.20 | ジョブネットの保留を解く and confirm | `保留を解いた: …`; the 保留 mark goes |
| 96.21 | 次の回を飛ばす on a confirmed net with a time later today, confirm | The line names the time it skips; on B, `koyomi skip-next` would list it and nothing starts at that time |
| 96.22 | 飛ばしを戻す and confirm; then 飛ばしを戻す again | `… を飛ばすのをやめた`; the second shows the refusal (`… は飛ばしていない`) in red |
| 96.23 | With `S`'s token, any of the four | The refusal (403) in red; nothing changes on B |

## 97. Rerun, skip and hold from the Web UI (TODO 81)

As §95, signed in with `I` (run) unless said.

| # | Do | Expect |
|---|---|---|
| 97.1 | Pick a net, ジョブネットを保留, then ジョブネットを保留 in the question | The tree shows ⏸ and the button turns to ジョブネットの保留を解く, with the note that it does not run; its next scheduled time does not start; `koyomi ops` shows `hold … （HTTP: ito）` |
| 97.2 | ジョブネットの保留を解く and confirm | The ⏸ mark goes; `koyomi ops` shows `release … （HTTP: ito）` |
| 97.3 | Let a run fail, pick it, 再実行 ▾ → 止まったところから（済んだジョブは引き継ぐ）, confirm | A new run that starts from the failed job (passed jobs carried over), picked in the pane |
| 97.4 | 再実行 ▾ → `<middle job> から再実行`, confirm | A new run that starts from that job |
| 97.5 | 再実行 ▾ on a failed run → `<failed job> の次から（… はスキップ）`; again with `（… は警告終了）` | New runs that start after that job, which ends ⊘ (then △) |
| 97.6 | While a run waits on a long job, スキップ on a later job that has not started, confirm | The job ends as ⊘ and the next ones go when their turn comes |
| 97.7 | Any of these, then やめる (or Esc) in the question | Nothing is sent |
| 97.8 | Sign in with `S` (view) | No 今すぐ流す, 保留, 再実行 ▾, スキップ, 次の回を飛ばす or 強制終了 buttons; ログ is there |
| 97.9 | `curl -X POST -H "Authorization: Bearer <S>" -d '{}' http://127.0.0.1:8790/runs/<id>/rerun` | 403; nothing starts |
| 97.10 | `curl -X POST -H "Authorization: Bearer <I>" -d '{"from":"a","next":"b"}' http://127.0.0.1:8790/runs/<id>/rerun` | 400 (`from と next はどちらか 1 つ`) |
| 97.11 | `curl -X POST -H "Authorization: Bearer <I>" -d '{"job":"nope"}' http://127.0.0.1:8790/runs/<id>/skip` | 422 with the engine's reason |
| 97.12 | 今すぐ流す on a net with `[params]` | One question with a field per value, its default filled in; やめる sends nothing |
| 97.13 | Change one value, keep the others, 今すぐ流す | The run gets the changed value, the others take their defaults |
| 97.14 | `curl -H "Authorization: Bearer <S>" http://127.0.0.1:8790/nets` | Each net has `params` with its names and defaults (`{}` when none), and `today` (the starts still to come today), `next`, `then` (the one after today's last) and `skips`; a held or unapproved net has no `today` and `next` null |
| 97.15 | `curl -X POST -H "Authorization: Bearer <I>" -d '{"net":"g/a"}' http://127.0.0.1:8790/nets/skip-next` (an approved net with a time later today) | 200 with `"skip": true` and the time it skips, as `koyomi skip-next g/a` says; at that time nothing starts; `koyomi ops` shows `skip-next g/a （HTTP: ito）` |
| 97.16 | The same with `{"net":"g/a","cancel":true}`; then once more | 200 with `"skip": false` and the time runs; the second is 422 (nothing skipped) |
| 97.17 | The same POST with `<S>`; `GET /nets/skip-next`; `{"net":"g/a","job":"b"}` | 403; 405; 400 |
| 97.18 | In the page, pick the net, 次の回を飛ばす, 飛ばす | The line names the time it skips; the pane lists it under 飛ばす回 and shows 飛ばしを戻す; 飛ばしを戻す, 戻す takes it back |

## 98. Definitions from the `edit` role (`GET /nets/text`, `POST /nets/propose`; TODO 83)

As §94, plus `[[http.user]]` `kato` (`role = "edit"`, `token = "secret:kato"`; `koyomi secret set kato` with `K`). The window is open on this PC.

| # | Do | Expect |
|---|---|---|
| 98.1 | `curl -H "Authorization: Bearer I" "http://127.0.0.1:8790/nets/text?net=g/a"` | 403 `ito は run のロール。これには edit が要る` |
| 98.2 | The same with `K` | 200 with `text` (the file as it is), `sha256` and `standing` |
| 98.3 | `curl -X POST -H "Authorization: Bearer K" -d '{"net":"g/a","text":"<the text with a job changed>","base":"<sha256 from 98.2>"}' http://127.0.0.1:8790/nets/propose` | 200, `"confirmed": false`; the file on disk is the new text; the window shows `g/a` as 未確認 and its confirm box shows the change |
| 98.4 | Wait for `g/a`'s next scheduled time, or `koyomi run g/a` | It does not run the new text (refused as unconfirmed) |
| 98.5 | Confirm `g/a` in the window | It runs again as scheduled, with the new text |
| 98.6 | Send 98.3 again with the old `base` | 409 `ほかで変わっています: g/a`; the file is not touched |
| 98.7 | Send a text that does not read (`format = 1` and a job without `run`) | 422 `保存できない: …`; the file is not touched |
| 98.8 | Send a new net `g/new` without `base` | 200; `jobnets/g/new.toml` is there, 未確認 in the window |
| 98.9 | `koyomi ops` | `propose g/a （HTTP: kato）` and `propose g/new （HTTP: kato）` |
| 98.10 | `koyomi versions g/a` | The text before 98.3 is kept as a version |

## 99. A standby copies the definitions (TODO 87)

As §93, plus on A `[[http.user]]` `b` (`role = "edit"`, `token = "secret:b"`) whose token is the one B keeps as `koyomi secret set a`. A has `g/a` confirmed; B has none of A's nets.

| # | Do | Expect |
|---|---|---|
| 99.1 | Start B's engine; wait a few seconds | B says `koyomi: [standby] 写した（未確認。この PC の窓で確認するまで動かない）: g/a` for each confirmed net of A; B's window shows them 未確認 |
| 99.2 | Confirm them in B's window | They show 確認済み; nothing more is copied (the hashes match) |
| 99.3 | Change and confirm `g/a` on A; wait up to 5 minutes | B copies `g/a` again, 未確認, the confirm box shows the change; `koyomi versions g/a` on B keeps the one before |
| 99.4 | On A, change `g/a` but do not confirm it | B does not copy it |
| 99.5 | On A make `b`'s role `view`; restart B | B's cautions and `koyomi standby` show `[standby] a: 定義を写せない: 向こうでこの token に edit のロールが要る…`; watching goes on |
| 99.6 | A net on A that calls a child net (`kind = "net"`), both new to B | Both land on B in one copy (the child first) |
| 99.7 | Stop A so that B takes over; leave `g/a` unconfirmed on B | B does not run `g/a` (unconfirmed); it copies no more, and the copy's caution from 99.5 is gone |
| 99.9 | Start A again; `koyomi standby giveback` on B | B watches again and copies at once (no 5-minute wait) |
| 99.8 | A net only on B | Still there after copies |

## 100. Nets per person over HTTP (`nets`; TODO 94)

As §98, plus `nets = ["経理/*"]` on `kato` (`K`). Nets `経理/a` and `人事/b`, both confirmed, each with a run today.

| # | Do | Expect |
|---|---|---|
| 100.1 | `curl -H "Authorization: Bearer K" http://127.0.0.1:8790/me` | `"nets": ["経理/*"]`; with `S`, `"nets": null` |
| 100.2 | `GET /nets` and `GET /runs` with `K` | Only `経理/a` and its runs; with `S`, both nets and all runs as before |
| 100.3 | `GET /runs/<人事/b's run>` and `GET /runs/<人事/b's run>/log?job=…` with `K` | 404 both times, as for a run that is not there |
| 100.4 | `POST /runs` `{"net":"人事/b"}` with `K`; also `POST /nets/hold` and `/nets/skip-next` with it | 422 `定義がない: 人事/b`; nothing starts or changes; `koyomi ops` has no line for them |
| 100.5 | `POST /runs/<人事/b's run>/rerun` and `/stop` with `K` | 422 `実行がない: …`; nothing starts or stops |
| 100.6 | `GET "/nets/text?net=人事/b"` with `K`; then `net=経理/a` | 404 `定義がない: 人事/b`; 200 with the text |
| 100.7 | `POST /nets/propose` with `K` for a new `人事/new`; then for a new `経理/new` | 403 `nets の外なので出せない: 人事/new（出せるのは 経理/*）`, no file; 200, `経理/new` is 未確認 in the window |
| 100.8 | `GET /health` with `K` | `koyomi engine pid N` only (no net names after it); with `S`, the whole line |
| 100.9 | Open the Web UI and sign in with `K` | Only `経理/a` and its runs |
| 100.10 | `koyomi check` with `nets = []`, `nets = ["経理"]` and `nets = ["*/*/x"]` | Each refused: `[[http.user]] kato の nets を空にしない…`, `… nets は <グループ>/<名前> で書く…` |
| 100.11 | This PC's window and `koyomi runs` | Both nets and all runs, as before |

## 101. Run a command on another PC (`kind = "ssh"`; TODO 95)

An SSH server you can reach that is already set up (e.g. Windows' OpenSSH Server on this PC as `ssh://<you>@localhost`; this check does not install or turn one on), and a key it takes in `~\.ssh\id_ed25519`. Commands below are for a Windows server (`cmd`); on Linux use the `sh` in brackets.

| # | Do | Expect |
|---|---|---|
| 101.1 | `koyomi sftp hostkey ssh://localhost`, then `koyomi sftp hostkey localhost` | `host_key = "SHA256:…"`, the same both times |
| 101.2 | `kind = "ssh"`, `on = "ssh://<you>@localhost"`, `command = "echo こんにちは & ping -n 4 127.0.0.1 & echo done"` (`echo こんにちは; sleep 3; echo done`) with that `host_key`; run it and watch its log | The lines come as they are printed, not all at the end; こんにちは is not garbled; ✓ |
| 101.3 | `command = "echo oops 1>&2 & exit 3"` (`echo oops >&2; exit 3`); then add `judge = { warn = "3" }` | ✕ with exit code 3 and `! oops` in the log; with the judge, ⚠ 警告 |
| 101.4 | A wrong `host_key`; then `on = "ssh://<you>@localhost:2"`; then `key` = a key the server does not take; also with `retry = { count = 1, wait = "1m" }` | ✕ each time with exit code 255, the why in Japanese as the reason (`localhost: …`) and on the log's last line after `! `; nothing asks anything; the retry tries once more |
| 101.5 | `password = "secret:ssh"` instead of the key, after `koyomi secret set ssh` (a server that takes passwords) | ✓; neither the log nor the confirm box shows the password |
| 101.6 | `env = { DAY = "{date}" }` and `command = "echo %DAY%"` (`echo $DAY`) | When the server's `AcceptEnv` takes `DAY`: today's date. When it does not (the default): `! localhost: env を受け付けなかった: DAY（相手の sshd_config の AcceptEnv）` and the job goes on |
| 101.7 | `command = "ping -n 120 127.0.0.1"` (`sleep 120`) with `timeout = "10s"`; then without the timeout, 強制終了 it | 打ち切り, then 強制終了, each within about 10 s of the ask (an ssh job is asked first). Under `run_as`, where the ask cannot reach `koyomi ssh run` (DECISIONS.md), the command on the server may go on: look with Task Manager (`ps`) |
| 101.8 | Linux only: `command = "kill -9 $$"` | ✕ with exit code 255 and `localhost: 相手のコマンドがシグナル KILL で終わった` as the reason |
| 101.9 | `koyomi check` with `on = "db1"`, `on = "ssh://db1/tmp"`, no `command`, an 8001-character `command`, no `host_key`, `password = "plain"`, both `key` and `password`, `on` on a `copy`, `encoding` on an ssh | Each is refused with the line and why in Japanese; the missing `host_key` names `koyomi sftp hostkey` |
| 101.10 | The editor: drag 別の PC で動かす（SSH） from 基本 | The pane shows 動かす PC・コマンド・ホスト鍵・SSH の秘密鍵・パスワード, 結果の判定, timeout and retry; saving writes `kind = "ssh"` with `on`・`command`・`host_key` (and `key`・`password`・`judge` when set); an `on` without `ssh://` is refused |
| 101.11 | Approve a net with an ssh | The confirm box shows 動かす PC, コマンド (lines joined with ⏎) and the host key instead of a script |
| 101.12 | A Linux server: `command = "sleep 600"`; run it with the release `koyomi.exe` engine on Windows (a window program, no console), and once the log shows it started, 強制終了 it; then the same with an engine that has a console (`cargo run -p koyomi -- engine`, or the engine on Linux or macOS), and with `run_as`. Then stop one while it connects: on the server `nc -lk 2222` (takes the connection, never answers), `on = "ssh://<you>@<server>:2222"`, run it with the release `koyomi.exe` and 強制終了 it within 10 s | The release `koyomi.exe` and the console alike: 強制終了 within a few seconds, the reason `強制終了、<server>: 止めた（コマンドが始まっていれば TERM で止まるよう頼んだ）`, `pgrep -f "sleep 600"` on the server finds nothing, and the job's log has no `Entering debug mode` or `[DBG]` lines. `run_as`: 強制終了 at once with no 頼んだ. While it connects: 強制終了 within about 1 s (not 10 s), with the same reason |

## 102. Hand back and take over runs (`give_back`, `[primary]`, `takeover = "rerun"`; TODO 96)

Two engines on this PC, A and B, each in its own temp folders, so the real profile is not touched. Open two consoles. In A's: `$env:KOYOMI_CONFIG_HOME = "$env:TEMP\koyomi-t96\a\config"`, `$env:KOYOMI_STATE_HOME = "$env:TEMP\koyomi-t96\a\state"`, `$env:KOYOMI_ADDRESS = '\\.\pipe\koyomi-t96-a'`; in B's the same with `b`. Run every command below in the console of the engine it names.

- Keep two tokens once: `koyomi secret set t96-a` and `koyomi secret set t96-b` (any values; the store is shared by both).
- A: `[http] lan = true`, `port = 8791`, `[[http.user]]` `b` (`role = "edit"`, `token = "secret:t96-b"`); `[[remote]]` `b` (`url = "https://localhost:8792"`, B's `koyomi http fingerprint`, `token = "secret:t96-a"`); `[primary]` `standby = "b"`, `every = "5s"`, `misses = 2`.
- B: `[http] lan = true`, `port = 8792`, `[[http.user]]` `a` (`role = "run"`, `token = "secret:t96-a"`); `[[remote]]` `a` (`url = "https://localhost:8791"`, A's fingerprint, `token = "secret:t96-b"`); `[standby]` `watch = "a"`, `every = "5s"`, `misses = 2`, `give_back = "auto"`.
- Nets on A, confirmed, and copied to B and confirmed there (§99): `t/r` with `takeover = "rerun"` and `t/l` without, each with job `one` (`cmd /c exit 0`) then `two` (`cmd /c ping -n 120 127.0.0.1 > nul`); `t/clock` with a time every few minutes.

| # | Do | Expect |
|---|---|---|
| 102.1 | Start B's engine, then A's; `koyomi standby` on both | A: `[primary] b が待機している。この PC で流す`; B: `待機中: a を見張っている…`. `t/clock` runs on A only |
| 102.2 | `koyomi check` with `[standby]` and `[primary]` both, `give_back = "sometimes"`, a net with `takeover = "maybe"` | Each is an error (`[standby] と [primary] は一緒に書けない…` for the first) |
| 102.3 | Stop B; restart A; `koyomi standby` on A at once, then after 10 s | First `[primary] b に聞いている…スケジュールとイベントは待たせる`; then `koyomi: 待機の PC` `b が答えないので、この PC で流す` and A runs. Start B again |
| 102.4 | On A `koyomi run t/r` and `koyomi run t/l`; once both are in `two` and 5 s have passed, stop A's engine process (`Stop-Process` on the pid you started) | After about 10 s B says `koyomi: 待機から引き継いだ` ending `a で動いていた 2 回: 続きから流す 1 回、引き継いでいない <id>（koyomi rerun <id> で続きから流せる）` |
| 102.5 | B's window: the runs of `t/r` and `t/l` | `t/r` runs `two` (`one` shows it ended ok on A, not run again); `t/l` is ↷ with `a で動いていた・引き継いでいない（<A's id> / …）`, `two` 未実行 with `a で動いていた（終わったかは分からない）` |
| 102.6 | On B `koyomi rerun <t/l's id>` | `t/l` runs from `two` on B |
| 102.7 | Start A again (same console) | A says `koyomi: 待機の PC` `b が引き継いでいる。返されるまで…`; `t/clock` does not run on A; B says `koyomi: 引き取りを待っている`; after about 10 s B says `koyomi: 戻した` (`a が引き取りに来たので戻した。また見張る`) and A `b から戻った。この PC で流す` |
| 102.8 | B's `standby.toml` in its state folder; `koyomi ops` on B | One line each for the takeover and the hand-back; ops has `handover-release` |
| 102.9 | Repeat 102.4 and 102.7 with `koyomi run t/l` started on B just before A comes back | The run on B goes on to the end after the hand-back; A runs the schedules |
| 102.10 | Make `t/clock`'s next time 20 s after you start A again in 102.7 | B answers 409; A's `koyomi standby` shows `[primary] b: 時刻が近い（t/clock HH:MM:SS）。少しあとで`; it is handed back on a later ask, and `t/clock` runs once (on one of them) |
| 102.11 | `give_back = "manual"` on B (restart B); repeat 102.4, then start A | A waits; B's `koyomi standby` says `…戻すのは koyomi standby giveback`; `koyomi standby giveback` on B says `a が引き取りを待っているので、次に聞いてきたら戻す`, and within 5 s A runs. `koyomi ops` on B has `giveback` |
| 102.12 | Take `[primary]` out of A; with `give_back = "auto"` repeat 102.4, then start A | A runs at once; B says `koyomi: 両方で流している` (`…a が 2 回続けて答えたらこちらは止める`) and after about 10 s `koyomi: 戻した` |
| 102.13 | Put `[primary]` back; on B give `a` `nets = ["t/*"]`; repeat 102.4, then start A | A stays waiting; its cautions show `[primary] b: 引き取れない: 向こうでこの token に run のロールが要る（nets のない利用者で）`. Take `nets` out: A takes them back |
| 102.14 | Clean up | Stop both engines (only the two started here), `koyomi secret delete t96-a` and `t96-b`, remove `%TEMP%\koyomi-t96` |

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
| 103.8 | Kill the engine (`taskkill /f` on the pid you started), start it again, click `エンジンが落ちていた` | Nothing opens |

## 104. Themes and a narrow window (TODO 97)

Setup: a few confirmed nets, one with a long name (`経理/とても長い名前の月次締めのジョブネット`) left unconfirmed and held, one with a long command and work folder. Take the shots with `--keys "<Theme:dark>…<Shot:…>"` as well as by hand.

| # | Do | Expect |
|---|---|---|
| 104.1 | With the window open, write `high_contrast = true` in common.toml, in the dark and then the light theme | The window turns to the stronger colours at once (as the Web UI does); removing the line goes back |
| 104.2 | Narrow the tree at the left until the long name does not fit | The name ends in `…`; `未確認` and `保留` stay whole at the right; hovering the row shows the whole name |
| 104.3 | Narrow the window until the net list, a run's jobs and the net page's jobs no longer fit | The word columns (name, command, folder, reason) end in `…` and the table stays inside the window; ID, kind and times stay whole; the heads end in `…` too, and hovering a cut cell or head shows all of it |
| 104.4 | Narrow the window on a calendar page, in the editor with nothing to save, and on the PC list | Each row of buttons wraps to a new line instead of pushing the page off the right side |
| 104.5 | Look at the headings of the net list, the net page, a run's jobs and the right pane | All the same size and weight; a screen reader reads them as headings |

## 105. A role per net over HTTP (`roles`; TODO 101)

As §100 without `nets` on `kato`, plus `roles = { "経理/*" = "run" }` on `sato` (`S`, `role = "view"`) and `roles = { "人事/*" = "view" }` on `ito` (`I`, `role = "run"`). Nets `経理/a` and `人事/b`, both confirmed, each with a run today.

| # | Do | Expect |
|---|---|---|
| 105.1 | `curl -H "Authorization: Bearer S" http://127.0.0.1:8790/me` | `"role": "view"`, `"roles": {"経理/*": "run"}`; with `K`, `"roles": null` |
| 105.2 | `GET /nets` with `S` | Both nets; `経理/a` has `"role": "run"`, `人事/b` `"role": "view"`; with `K`, no `role` on the nets |
| 105.3 | `POST /runs` `{"net":"経理/a"}` with `S`; then `{"net":"人事/b"}` | 202 and it runs; then 403 `人事/b では view のロール。これには run が要る`, nothing starts |
| 105.4 | `POST /runs/<人事/b's run>/stop` and `POST /nets/hold` `{"net":"人事/b"}` with `S` | 403 both times; nothing stops or is held. With `経理/a` (and a run of it still going) both are done |
| 105.5 | `GET /runs/<人事/b's run>/log?job=…` with `S` | 200 with the log (view is enough) |
| 105.6 | `GET "/nets/text?net=経理/a"` with `S` | 403 `sato は run のロール。これには edit が要る` (the strongest of `S`'s roles is still not edit) |
| 105.7 | `POST /runs` `{"net":"人事/b"}` with `I`; then `{"net":"経理/a"}` | 403 `人事/b では view のロール…`; then 202 |
| 105.8 | On the standby of §99, `POST /standby/release` with `S`; then with `I` | 403 `ネットごとでないものは view のロール。これには run が要る`; then 200 |
| 105.9 | Open the Web UI and sign in with `S` | Next to the name, `sato（view、経理/* は run）`. `経理/a` and its run have 今すぐ流す, 保留, 強制終了 / 流し直す and スキップ; `人事/b` and its run have none of them |
| 105.10 | `koyomi check` with `roles = {}`, `roles = { "経理" = "run" }` and `roles = { "経理/*" = "admin" }` | Each refused: `[[http.user]] sato の roles を空にしない…`, `… roles のキーは <グループ>/<名前> で書く…`, `… roles の 経理/* の role は view・run・edit のどれか…` |
| 105.11 | This PC's window and `koyomi run 人事/b` | Both work as before |

## 106. The working folder of a job without `dir` (`work_dir`) and a month's `day = 29`–`31`

Net `経理/a` with job `p` (`run = ["cmd", "/c", "cd"]`, no `dir`) and job `q` (the same `run`, `dir = "out"`). Net `経理/b` that uses `共通/c`, whose job `r` is `cmd /c cd` with no `dir`. Make the folder `jobnets/経理/out`. Both nets confirmed.

| # | Do | Expect |
|---|---|---|
| 106.1 | No `work_dir` in koyomi.toml; run `経理/a` | `p`'s log shows the engine's own folder, as before; `q` ends in ✕ (no `out` there) |
| 106.2 | Write `work_dir = "definition"`; within 5 s run `経理/a` again | `p`'s log shows `…\jobnets\経理`, `q`'s `…\jobnets\経理\out`; both ✓ |
| 106.3 | Run `経理/b` | `r`'s log shows `…\jobnets\共通` (the child's own folder, not `経理`) |
| 106.4 | `koyomi run 経理/a --dry` | `p: cmd /c cd @ …\jobnets\経理` and `q: … @ …\jobnets\経理\out` |
| 106.5 | Write `work_dir = 'D:\batch'` (make `D:\batch` but not `D:\batch\out`) and run `経理/a` | `p`'s log shows `D:\batch`; `q` ends in ✕ |
| 106.6 | Remove `D:\batch`; `koyomi check jobnets/経理/a.toml` | `q の作業フォルダーがない: D:\batch\out` and `dir のないジョブの作業フォルダーがない: D:\batch（koyomi.toml の work_dir）`; exit 1 |
| 106.7 | Write `work_dir = "batch"`; `koyomi check koyomi.toml` | Refused: `work_dir は "definition"（定義のあるフォルダー）か絶対パスで書く（例: work_dir = 'D:\batch'）: batch` |
| 106.8 | A net with `[[when]]` `every = "month"`, `day = 31`, `at = ["21:00"]`; `koyomi check` it | `注意: [[when]] の day = 31 は 31 日のない月（2・4・6・9・11 月）には流れない。月末に流すなら day = "last"`, then `問題なし`; exit 0 |
| 106.9 | The same with `day = 30`, `day = 29` and `day = "last"` | `（2 月）`; `（うるう年でない 2 月）`; no word |
