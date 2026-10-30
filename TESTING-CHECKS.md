# Checklist

308 checks in TESTING.md, 304 left for a person.

## 1. Start

- [ ] **1.1** Run `koyomi` → A window of 1280x800 opens with the title "koyomi" 〔未訳〕
- [ ] **1.2** Move and resize the window, close it and start it again → The window comes back at the same place and size 〔未訳〕
- [ ] **1.3** Press `F12` → The browser opens the repository's new issue form with the version and OS filled in 〔未訳〕

## 2. Definitions

- [ ] **2.1** Put `経理/月次締め.toml` and `共通/バックアップ.toml` under `jobnets/` and open the window → The left tree shows 共通 and 経理 with `▲ バックアップ  未確認` and `▲ 月次締め  未確認`; the Japanese names are letters, not boxes 〔未訳〕
- [ ] **2.2** Run `koyomi list` and `koyomi list --toml` with the engine stopped → The engine starts; the same list as the tree is printed 〔未訳〕

## 3. Engine

- [ ] **3.1** Open the window with no engine running, then close the window → The engine starts by itself and is still running after the window is closed (`koyomi engine status` answers) 〔未訳〕
- [ ] **3.2** Run `koyomi engine` while one is up, then `koyomi engine stop` → The first prints "もう動いている" and exits 1; stop stops the running one 〔未訳〕
- [ ] **3.3** Run `koyomi startup on`, then `koyomi startup` and `koyomi engine status` → Both say サインイン時の起動: あり (the status on its last line); the Run key `uchmk.koyomi` under HKCU holds `koyomi.exe engine`; the settings' General page shows the switch on; after signing in again the engine is up with no window 〔未訳〕
- [ ] **3.4** Run `koyomi startup off`, then `koyomi startup oops` → The first says サインイン時には起動しない and the key is gone; the second prints the usage and exits 2 〔未訳〕

## 4. Approval

自動テスト済みなので下には出していない: 4.5

- [ ] **4.1** Select a net, press 確かめる… → A box lists the program, each argument on its own line, and the folder; Enter = 確かめた, Esc = やめる 〔未訳〕
- [ ] **4.2** `koyomi approve 経理/月次締め`, answer `y`, restart the engine → The same content is printed; the net shows `✔ 確認済み` and still does after the restart 〔未訳〕
- [ ] **4.3** `koyomi approve 経理/月次締め` and answer anything but `y` → It stays `▲ 未確認` and the exit code is 1 〔未訳〕
- [ ] **4.4** In the box press やめる → The net stays `▲ 未確認` 〔未訳〕
- [ ] **4.6** Open the box on a net with a long diff, close it, open it on a one-job net; then make the window small → The title, the rows, the program and the buttons are inside the window every time, and the box does not grow while it is up 〔未訳〕

## 5. Edits

- [ ] **5.1** Approve a net, then add a comment line to its file by hand → Within seconds it is `▲ 未確認` again 〔未訳〕
- [ ] **5.2** Approve it again → The box (and `approve`) shows the change as `+` / `-` lines against the approved text 〔未訳〕

## 6. Check

- [ ] **6.1** `koyomi check <file>` on a good file and on files with a typo'd field, bad syntax, no/empty `run`, an unknown `after`, a cycle, a duplicate id → 0 for the good one; for the others the file, the line and the cause, and exit 1 〔未訳〕
- [ ] **6.2** Look at a broken file in the tree and try `approve` → `✖ 定義の誤り`; it cannot be approved 〔未訳〕
- [ ] **6.3** A file with `timeout = 60` (no unit) and one with `retry = { count = 2 }` → `timeout は "30m" のように文字列で書く…` for the first, "この版ではまだ使えない: retry" for the second; neither can be approved 〔未訳〕

## 7. Safety of the places

- [ ] **7.1** Let Everyone write a definition, then `check` and `approve` → Both exit 1 and name the file 〔未訳〕
- [ ] **7.2** Same, with the window open → A warning band names the place and who; 確かめる… is disabled for every net 〔未訳〕
- [ ] **7.3** Break `approved.toml` by hand, then `list` and `approve` → Every net shows `▲ 未確認`, a warning names `approved.toml`, `approve` exits 1 and the file is left as it was; once it is mended the approvals come back without a restart 〔未訳〕

## 8. Runs

- [ ] **8.1** Approve a net, select it, press 今すぐ流す → 今日 lists the run as `▶ 実行中` with its start time, and the detail switches to the run's picture 〔未訳〕
- [ ] **8.2** Watch while it runs → Each circle goes `·` → `▶` → `✓`; the line out of a finished job is painted in its colour; the running job's log flows under the picture, newest line at the bottom 〔未訳〕
- [ ] **8.3** A net whose one job exits 1 → That job is `✕ 異常終了` with its exit code, the jobs after it `· 未実行`, and the run `✕ 異常終了` in 今日 and in `koyomi status --today` 〔未訳〕
- [ ] **8.4** Press 強制終了, then Enter; press 強制終了 again, then 強制終了する → The first Enter only closes the box (the keyboard starts on やめる); the second ends the job as `■ 強制終了` and the run as `✕ 異常終了` 〔未訳〕
- [ ] **8.5** Press 縦に流す, close the window and open it again → The picture flows downwards, and still does after the restart (`state.toml`); 横に流す brings it back 〔未訳〕
- [ ] **8.6** Click a circle, and click a job's row in words → The log under the picture is that job's; the circle is outlined 〔未訳〕
- [ ] **8.7** `koyomi run 経理/月次締め --wait` → The run id is printed, then its line and one line per job (symbol, word, name, times, duration); exit 0 only when every job ended ok 〔未訳〕
- [ ] **8.8** `koyomi run` the same net while it runs → `動いている間はもう一度は流せない` and exit 1 〔未訳〕
- [ ] **8.9** `koyomi run` on an unconfirmed net → `未確認のため流さない` with `koyomi approve …` and exit 1 〔未訳〕
- [ ] **8.10** `koyomi status`, `koyomi status --today`, `koyomi log <run> <job>`, `koyomi kill <run>` → status lists the runs still going (`--today` every run of today); log prints the tail; kill ends the run 〔未訳〕
- [ ] **8.11** Stop the engine (`koyomi engine stop`) while a run is going, start it again → The run is `✕ 異常終了` and its running job `エンジンが止まった` 〔未訳〕
- [ ] **8.12** Set `concurrency = 1` in `koyomi.toml` and run a net with two jobs that have no `after` → The second job is `○ 待ち` until the first ends 〔未訳〕
- [ ] **8.13** A job that prints more than `log_max_mb` → Its log ends with `（ログが 10 MB を超えたので、この先は書かない）` and the job still ends with its own exit code 〔未訳〕

## 9. Timeout and kill

- [ ] **9.1** Approve a net with `timeout = "30m"` on a job → The confirm box and `koyomi approve` show `打ち切り: 30m` under that job 〔未訳〕
- [ ] **9.2** A job with `timeout = "5s"` that runs `cmd.exe /c ping -n 60 127.0.0.1` → After about 5 s the job is `⏱ 打ち切り` with `打ち切り（5s を超えた）`, the run `✕ 異常終了`, the jobs after it `· 未実行`; the log ends with `! 打ち切り（5s を超えた）`; `ping.exe` is gone from Task Manager 〔未訳〕
- [ ] **9.3** 強制終了 a job that runs `cmd.exe /c start /wait notepad` → Both `cmd.exe` and `notepad.exe` are gone from Task Manager; the job is `■ 強制終了` and its log ends with `! 強制終了` 〔未訳〕
- [ ] **9.4** 強制終了 a job whose `timeout` is about to pass → The job is `■ 強制終了`, never `⏱ 打ち切り`; the words tell the two apart, the colour is the same 〔未訳〕

## 10. Notifications

- [ ] **10.1** With no `[notify]` in `koyomi.toml`, run a net whose job exits 1, from the window → A system notification `✕ 異常終了  <net>` with the failed job's line and the run id; no sound 〔未訳〕
- [ ] **10.2** The same net started with `koyomi run` and with `koyomi run --wait` → The same notification, worded the same 〔未訳〕
- [ ] **10.3** Run a net that ends `✓ 正常終了` → No system notification, no sound 〔未訳〕
- [ ] **10.4** `[notify]` with `system = ["done", "failed"]`, run a net that ends well → A system notification `✓ 正常終了  <net>` 〔未訳〕
- [ ] **10.5** `[notify]` with `sound = ["failed"]`, a failing net → The alert sound with the notification; without that line, none 〔未訳〕
- [ ] **10.6** A failing net, then the bell → The bell counts 1 unread; the row is `✕ 異常終了  <net>` with the failed job under it; clicking it shows that run in the detail 〔未訳〕
- [ ] **10.7** A net that ends well, then the bell → Nothing new under the bell 〔未訳〕
- [ ] **10.8** `[notify]` with `webhook = "https://ntfy.sh/<topic>"` (plain text), start the engine → `koyomi list` prints `注意: koyomi.toml: [notify] の webhook のアドレスが平文で書かれている…`; the bell has the same line once; a failing net sends nothing to the topic 〔未訳〕
- [ ] **10.9** Add `hide_webhooks = false` to 10.8 → No note; a failing net posts to the topic with the same title and body as the notification 〔未訳〕
- [ ] **10.10** Put the address in Credential Manager as `uchmk/koyomi/ntfy` (generic credential, the URL as the password) and write `webhook = "secret:ntfy"` → No note; a failing net posts to the topic; the address appears nowhere in `koyomi.toml`, the history or the logs 〔未訳〕
- [ ] **10.11** `webhook = "secret:nothing"` (no such credential) → `koyomi list` prints a note that it does not read; nothing is sent; runs still start 〔未訳〕
- [ ] **10.12** `koyomi check koyomi.toml` with `system = ["failed", "exploded"]` → A line naming `exploded`, exit code 1; with a clean file, `koyomi.toml: 問題なし` and exit code 0 〔未訳〕
- [ ] **10.13** Edit `[notify]` while the engine runs → Within about 5 s `koyomi list` reflects the change; no restart 〔未訳〕

## 11. Schedule

- [ ] **11.1** A confirmed net with `[[when]] every = "day"` and `at` two minutes from now; wait → At that minute the net starts by itself: the 今日 list shows it `▶ 実行中` with the planned time, then its result; the bell stays quiet when it ends well 〔未訳〕
- [ ] **11.2** Before the time of 11.1 comes, the 今日 list → A dim row `○ 待ち  <net>  HH:MM:SS` above the runs; it is not a row to open; it is gone once the run starts 〔未訳〕
- [ ] **11.3** The detail of a net with `[[when]]` → Under the jobs, `次の 10 回` with rows like `2026-11-02（月）02:00`; `koyomi next <group/name>` prints the same rows; `--count 3` prints three 〔未訳〕
- [ ] **11.4** `koyomi next` on a net with no `[[when]]` → `時刻の予定はありません（手で流します）`; the detail shows the same line 〔未訳〕
- [ ] **11.5** A net that is `▲ 未確認` with `[[when]]`; let its time come → Nothing runs; the 今日 list shows `🔒 止めた  <net>` with `未確認のため流さなかった（koyomi approve <group/name>）`; the bell has it like an error; the detail's `次の 10 回` carries `未確認のため、時刻が来ても流しません` 〔未訳〕
- [ ] **11.6** `koyomi engine stop`, let two times of a net pass, start the engine again → Each passed time is a `↷ 時刻を過ぎた` row with `H:MM に PC が止まっていました`; nothing starts; one system notification `↷ 時刻を過ぎた  <net>` naming both times (`2:00、6:30 の回は時刻を過ぎました（PC が止まっていました）`); the bell has one warning row 〔未訳〕
- [ ] **11.7** A net with `at` every minute whose job takes three minutes → The second and third times are `⊘ スキップ` with `前の回がまだ動いていました`; no notification for them 〔未訳〕
- [ ] **11.8** Start the engine for the first time on a PC (delete `schedule.toml` in the state folder with the engine stopped) → No `時刻を過ぎた` rows for times before this start 〔未訳〕
- [ ] **11.9** Set the PC clock back an hour while the engine runs → Nothing runs twice; the times between are not reported as passed 〔未訳〕
- [ ] **11.10** A net with `[[when]] every = "month" day = "last"` at 02:00 → `koyomi next` lists the last day of each coming month 〔未訳〕
- [ ] **11.11** A net with `[[when]] every = "week" on = ["mon"]` and `calendar = "jp"` → The rows skip Japanese holidays that fall on a Monday 〔未訳〕
- [ ] **11.12** A confirmed net with `[[when]]` exists and `koyomi` is not set to start at sign-in → A band under the title: `時刻で流す定義がありますが、サインイン時に koyomi は起動しません…`; it goes away once the setting is on, or when no confirmed net has `[[when]]` 〔未訳〕
- [ ] **11.13** While 11.6's engine is stopped, write a `[[when]]` whose `at` is in the past of today into an unconfirmed net, then start the engine → The row is `🔒 止めた`, never `↷ 時刻を過ぎた` for a time the engine never knew of 〔未訳〕

## 12. The week ahead at approval

- [ ] **12.1** A new net with `[[when]] every = "day" at = ["02:00"]`; open the confirm box → Under the jobs: `この先 7 日の予定: 7 回` and seven rows like `2026-10-31（土）02:00`; `koyomi approve` prints the same before asking for `y` 〔未訳〕
- [ ] **12.2** Approve 12.1, then change it to `every = "minutes" n = 15 from = "00:00" to = "23:59"`; open the box → `7 日で 7 回 → 672 回（+665）`, then 20 rows starting `+ ` with the next quarter hours, then `… 他 645 件`; the 2:00 rows are not listed (they stay) 〔未訳〕
- [ ] **12.3** Approve 12.2, then change it back to daily 2:00 → `7 日で 672 回 → 7 回（-665）` and 20 rows starting `- ` 〔未訳〕
- [ ] **12.4** Approve a daily net, then add `except = ["<a day within the week>"]` → `7 日で 7 回 → 6 回（-1）` and one `- ` row for that day 〔未訳〕
- [ ] **12.5** A net with `calendar = "jp"` on weekdays, approved; change it to `calendar = "none"` across a holiday → The holiday's time appears as a `+ ` row 〔未訳〕
- [ ] **12.6** A new net with no `[[when]]`; open the box → No week lines at all; after approving, remove a `[[when]]` from a net that had one → `7 日で N 回 → 0 回（-N）` with the `- ` rows 〔未訳〕
- [ ] **12.7** Open the box, wait, open it again a minute later → The counts are from the moment the box opens: a time that passed meanwhile is gone from the list 〔未訳〕

## 13. `--keys`

- [ ] **13.1** `set KOYOMI_KEYS_DONE=<dir>\done.txt` then `koyomi --keys "<Wait:1500><Shot:start>"` → `start.png` is written beside the done file; the done file ends with `keys: done` 〔未訳〕
- [ ] **13.2** `koyomi --keys "<Wait:1500><State:s><Quit>"` → `s.txt` holds `engine: up`, `nets: N`, `runs: N`, `selected: -`, `asking: -`, `flow: horizontal`; the window closes by itself; the done file has `quit: yes` then `keys: done` 〔未訳〕
- [ ] **13.3** `koyomi --keys "<Wait:30000>"` and close the window with × while it waits → The done file says `keys: closed` and `left: 1 not pressed` 〔未訳〕
- [ ] **13.4** `koyomi --keys "<Nope>"` → Exit code 2, the message starts `koyomi: --keys:`, and the done file says `keys: refused` 〔未訳〕
- [ ] **13.5** `koyomi --keys` (no script) or `koyomi --keys j --more` → Exit code 2 and the usage; nothing opens 〔未訳〕
- [ ] **13.6** Move the window under `--keys "<Wait:30000>"`, close it, then start `koyomi` without `--keys` → The window comes back where it was before the scripted run: a script keeps no place and writes no `state.toml` 〔未訳〕
- [ ] **13.7** `koyomi --keys "<Theme:light><Lang:en><Wait:500><Shot:en>"` → The picture is light and in English; `common.toml` is unchanged afterwards 〔未訳〕
- [ ] **13.8** `koyomi --keys "<Wait:1500><Click:500,500><State:c>"` with a net in the tree at that point → `c.txt` says `selected: <group/name>` 〔未訳〕

## 14. Keys (`keymap.toml`)

- [ ] **14.1** Press Ctrl+Q with the tree focused → The window closes (the engine keeps running) 〔未訳〕
- [ ] **14.2** Press Ctrl+, and then F1 → Ctrl+, opens the settings; Esc closes them; F1 opens the key help over the tree 〔未訳〕
- [ ] **14.3** Open the palette (Ctrl+Shift+P) → "Quit" shows `Ctrl+Q` as its key 〔未訳〕
- [ ] **14.4** Write `<uchmk>\koyomi\keymap.toml` with `[[app.keymap]]` `on = "<C-q>"` `run = "none"` and another binding `on = "<C-w>"` `run = "quit"`, restart → Ctrl+Q does nothing; Ctrl+W quits; the palette shows `Ctrl+W` 〔未訳〕
- [ ] **14.5** Add a line with `run = "explode"` and one with `on = "<F12>"`, restart → The bell lists two warnings naming the file: `explode` is no action, F12 is fixed; the other keys still work 〔未訳〕
- [ ] **14.6** Type Ctrl+Q while the search box of the palette has the focus → The palette takes the key (nothing quits); Esc, then Ctrl+Q quits 〔未訳〕

## 15. Settings (Ctrl+,)

- [ ] **15.1** Press Ctrl+, → The settings cover the tree: General, Theme, Keys, Advanced, About down the left; the foot of the nav opens `common.toml` 〔未訳〕
- [ ] **15.2** Press Esc, or ← at the top → The tree is back as it was (the selected net still selected) 〔未訳〕
- [ ] **15.3** General: pick 日本語 → The window turns Japanese at once; `common.toml` has `language = "ja"` within 2 seconds; minato or kura at the same desk follow 〔未訳〕
- [ ] **15.4** General: pick 125 % → The window scales; `scale = 1.25` is in `common.toml`; Ctrl+0 puts it back and the file follows 〔未訳〕
- [ ] **15.5** General: switch the clock's date on and change its format → The status bar's clock shows the date in that format at once 〔未訳〕
- [ ] **15.6** General: "Start at sign-in" → The switch shows what the OS has; toggled, the registry (Windows) or the autostart folder gets koyomi, and the band about starting at sign-in (TESTING 11) goes away at the next start 〔未訳〕
- [ ] **15.7** General: "Forget the window's place" → `state_dir("koyomi")\window.toml` is gone; the next start opens at the default size in the middle 〔未訳〕
- [ ] **15.8** Theme: pick Dark, then Light, then System → The colours change at once, the sample card with it; `theme = "…"` is in `common.toml`; System follows the OS's light/dark 〔未訳〕
- [ ] **15.9** Theme: edit `common.toml` by hand to `theme = "koyomi Light"` while the window is open → The window turns light within 2 seconds without a restart 〔未訳〕
- [ ] **15.10** Keys: click Ctrl+Q's key, press Ctrl+W → The row shows Ctrl+W; `<uchmk>\koyomi\keymap.toml` holds only that change; Ctrl+W quits from then on and the palette shows it 〔未訳〕
- [ ] **15.11** Keys: while the page waits for a key, press F12 and Ctrl+Shift+P → They are taken as the key (no bug form, no palette); Esc gives the page up 〔未訳〕
- [ ] **15.12** Keys: bind F12 to an action → The row says koyomi reads F12 itself and the binding never runs 〔未訳〕
- [ ] **15.13** Keys: ↺ on a changed row → The built-in key is back and the line is gone from `keymap.toml` (the file is removed when nothing is left in it) 〔未訳〕
- [ ] **15.14** Keys: write `run = "explode"` into `keymap.toml` by hand while the window is open → The bell lists the warning within 2 seconds; the other keys still work 〔未訳〕
- [ ] **15.15** Advanced: "Open the config folder" and "Open the state folder" → The file manager opens `<uchmk>` and `state_dir("koyomi")` 〔未訳〕
- [ ] **15.16** Advanced: "Put the settings back" → `common.toml` and `keymap.toml` are moved aside as `*.bak-<stamp>`; the definitions, `koyomi.toml`, `approved.toml` and the runs are untouched; the window goes back to the defaults at once 〔未訳〕
- [ ] **15.17** About → The version, the commit, the OS line and the licence; "Report a bug" opens the issue form as F12 does 〔未訳〕
- [ ] **15.18** Type `keymap` in the search box → The Keys page is offered; Enter opens it with its rows highlighted 〔未訳〕
- [ ] **15.19** Open the settings while the engine runs a job → The run still ends (the bell still rings for a failed one) and the tree shows it once the settings close 〔未訳〕

## 16. Help (F1, ?)

- [ ] **16.1** Press F1, or `?` → A card over the tree, the tree dimmed behind it: Everywhere (Ctrl+Q, Ctrl+,, F1 / ?), Always (F12, Ctrl+Shift+P, Ctrl++ / Ctrl+-, Ctrl+0), A box over the tree, Keys 〔未訳〕
- [ ] **16.2** Press Esc; open again and press F1; open again and press `?`; open again and click the dimmed tree → Each closes the card; the tree is as it was 〔未訳〕
- [ ] **16.3** Make the window short (about 300 px tall) and open the help → The card is cut to the window; ↓ ↑ PgDn PgUp End Home and the wheel scroll the list; a wide window lays it out in two columns, a narrow one in one 〔未訳〕
- [ ] **16.4** Open the palette (Ctrl+Shift+P) and type `?` in its search box → The search box gets the `?`; the help does not open 〔未訳〕
- [ ] **16.5** In `keymap.toml` bind `<F2>` to `help` (and `<F1>` to `none`), then open the help with F2 → The Everywhere group shows F2 / ? for the help and no F1 〔未訳〕
- [ ] **16.6** With the help open press Ctrl+Q → koyomi quits (the keymap still works under the help) 〔未訳〕
- [ ] **16.7** Select a net, press Approve… so its box shows, then press F1 and Enter → The help closes nothing behind it: Enter does not press the box's button; Esc closes the help first, then the box 〔未訳〕
- [ ] **16.8** Open the palette and look at the Keys entry → It shows F1 (the first key bound to `help`); picking it opens the help 〔未訳〕
- [ ] **16.9** Press Ctrl+, while the help is open → The settings cover the tree and the help is closed; Esc brings back the tree without the help 〔未訳〕

## 17. Settings: Notifications

- [ ] **17.1** With no `koyomi.toml`, open the settings and the Notifications page → A table with the columns Ended ok / Ended abnormally: the system notification is on for Ended abnormally only, no sound, no webhook; "Keep the addresses out of the file" is on; nothing is written yet 〔未訳〕
- [ ] **17.2** Switch the system notification on for Ended ok → `koyomi.toml` gets `[notify]` with `system = ["done", "failed"]` and nothing else; within 5 s `koyomi list` shows no note, and a net that ends well is told of (10.4) 〔未訳〕
- [ ] **17.3** Switch the sound on for Ended abnormally and pick another sound → `sound = ["failed"]` and `sound_failed = "<name>"` are written; a failing net plays it (10.5) 〔未訳〕
- [ ] **17.4** Press ↺ at the top → The switches go back to 17.1; `[notify]` keeps only what still differs from the defaults (the webhooks and the switch stay) 〔未訳〕
- [ ] **17.5** Add a webhook with an `https://ntfy.sh/<topic>` address, ntfy, for Ended abnormally → `koyomi.toml` gets `[[notify.webhooks]]` with `url = "secret:webhook-1"`; the address is in Credential Manager under `uchmk/koyomi/webhook-1`; the page shows the address; a failing net posts to the topic 〔未訳〕
- [ ] **17.6** Press "Send a test" at the top, then the test button on the webhook's row → A system notification `koyomi: 試しの通知` (the first with the sound when one is on); the topic gets the same text; the second sends to that topic only 〔未訳〕
- [ ] **17.7** Switch "Keep the addresses out of the file" off → `hide_webhooks = false` and the address itself are written; the credential is deleted; `koyomi check koyomi.toml` says nothing about it 〔未訳〕
- [ ] **17.8** Write `webhook = "https://ntfy.sh/<topic>"` by hand with `hide_webhooks` unset, then open the page → The page shows the red note about the plain-text address (10.8) with the address in the webhook's row; changing anything on the page moves the address to the credential store and `secret:webhook-1` into the file; the note goes 〔未訳〕
- [ ] **17.9** Edit `[notify]` with a text editor while the page is open → Within 2 s the page shows it; a line it cannot read is shown in red on the page and the switches stay 〔未訳〕
- [ ] **17.10** Open the page with `concurrency = 2` and a comment in `koyomi.toml`, change a switch → Only `[notify]` changed; `concurrency`, the comment and the line order stay 〔未訳〕
- [ ] **17.11** Search the settings for `webhook` or `[notify]` → The results show the Notifications page 〔未訳〕

## 18. Settings: Records

- [ ] **18.1** With no `koyomi.toml`, open the settings' General page → Under "Records": Days of logs and history to keep = 90 days, Jobs at a time = 4, Log size = 10 MB; nothing is written 〔未訳〕
- [ ] **18.2** Pick 180 days → `koyomi.toml` gets `keep_days = 180` at the top (before any table); no box asks (it is longer); `koyomi engine status` shows the sweep line again within 5 s 〔未訳〕
- [ ] **18.3** Put `logs/2026-01-05/` and `history/2026/01/06.toml` in the state folder, then pick 30 days → A box says 30 days are kept, 180 → 30, and that 2 days of logs and history go at the engine's next sweep; Enter writes `keep_days = 30`; within 5 s the two are gone and `engine status` says `ログ 1 日・履歴 1 日を消した` 〔未訳〕
- [ ] **18.4** Pick a shorter number again with nothing old there, and press Esc → The box says no days go right away; Esc keeps the file as it was; the row shows the days as before 〔未訳〕
- [ ] **18.5** Type 2 in Jobs at a time and press Enter → `concurrency = 2` is written at the top; a net with 3 parallel jobs runs 2 at a time within 5 s 〔未訳〕
- [ ] **18.6** Type 0, `abc` or 1.5 in Log size → A red note under "Records" says to enter a whole number of 1 or more; nothing is written; the field goes back to the number in the file 〔未訳〕
- [ ] **18.7** Write `keep_days = 45` by hand with a comment above `[notify]`, open the page, pick 60 → The row showed "45 days" among the choices; only `keep_days` changed; the comment, `[notify]` and the line order stay 〔未訳〕
- [ ] **18.8** Put `logs/notes/` and `logs/2026-01-05-old/` in the state folder and shorten the days → Neither is counted nor removed: only names that are a day (`2026-01-05`, `2026/01/05.toml`) are swept 〔未訳〕
- [ ] **18.9** Search the settings for `keep_days`, `concurrency`, `log_max_mb` or `koyomi.toml` → The results show the General page 〔未訳〕

## 19. Retry

- [ ] **19.1** Write a job with `retry = { count = 2, wait = "5s" }` and run `koyomi check`, then `koyomi approve` → check says OK; the approve text shows `やり直し: 2 回まで、5s おいて` under the job 〔未訳〕
- [ ] **19.2** Make that job `cmd.exe /c exit 3` and run it → The log shows `! やり直し (1/2): 5s 待つ`, then `! やり直し (2/2): 5s 待つ`; the job turns 異常 after about 10 s with the reason `終了コード 3、やり直し 2 回`; the run screen and the history show the same reason 〔未訳〕
- [ ] **19.3** Make the job fail once and succeed the second time (a flag file) → The run ends 正常; the job's reason is `やり直し 1 回`; the log has `(1/2)` and no `(2/2)` 〔未訳〕
- [ ] **19.4** Give the job `timeout = "2s"` and a command that sleeps 10 s → 打ち切り after 2 s, no やり直し line: a timeout is not run again; the same for `koyomi kill` during the job (強制終了) 〔未訳〕
- [ ] **19.5** With `wait = "60s"`, run `koyomi kill` while the log says やり直し → The job ends 強制終了 at once (not after the wait); the log ends with `! 強制終了` 〔未訳〕
- [ ] **19.6** Write `retry = 2`, `retry = { count = 0, wait = "5m" }`, `retry = { count = 2 }` or `retry = { count = 2, wait = 300 }` → `koyomi check` refuses each with the line number and what to write (the shape, 1 to 100, wait missing, a string like "5m") 〔未訳〕

## 20. A passed time run once, a time that waits (`missed`, `overlap`)

- [ ] **20.1** Write `missed = "once"` at the top of a net with `at = ["02:00"]`, approve it, stop the engine, set the clock past 02:00 (or wait), and start the engine → The run starts at once with 遅れて実行（2:00 の予定を 8:12 に流しました） as its reason on the run screen, in `koyomi status` and in the history; the notification title reads `正常（遅れて実行）  <title>`; the approve text says 時刻を過ぎた回: 起動したとき最後の 1 回を流す（遅れて実行） 〔未訳〕
- [ ] **20.2** The same with two passed times (`at = ["02:00", "03:00"]`) → Only the 03:00 run starts late; 02:00 is 時刻を過ぎた with `2:00 に PC が止まっていました` 〔未訳〕
- [ ] **20.3** The same net, but unconfirmed, or with a definition folder writable by others → Nothing starts late: the passed time is 止めた with the same reason an on-time one gets 〔未訳〕
- [ ] **20.4** Write `overlap = "queue"` and `at = ["10:00", "10:01"]` with a job that sleeps 3 minutes → At 10:01 nothing starts; `koyomi engine status` shows `待ち: <title>  10:01 の回は前の回が終わってから流す（10:01 から待っている）`; when the 10:00 run ends, the 10:01 run starts (how 予定, planned 10:01, no 遅れて実行) 〔未訳〕
- [ ] **20.5** `at = ["10:00", "10:01", "10:02"]` with the same job → 10:01 waits; 10:02 is スキップ with `前の回がまだ動いていました（その次の回が待っています）` 〔未訳〕
- [ ] **20.6** Run `koyomi engine stop` while a time waits → The waiting time is recorded スキップ with `待っている間にエンジンが止まった`; nothing starts when the engine comes back 〔未訳〕
- [ ] **20.7** Write `missed = "later"` or `overlap = 1` → `koyomi check` refuses with the line number: `missed は "skip"・"once" のどれか` / `overlap は "skip"・"queue" のどれか`; `missed = "skip"` and `overlap = "skip"` are accepted, as is leaving both out 〔未訳〕

## 21. Exit-code judge and PowerShell script jobs (`judge`, `shell`, `script`)

- [ ] **21.1** Write `judge = { warn = "1", error = "2.." }` on a job and run `koyomi check`, then `koyomi approve` → check says OK; the approve text shows `判定: 警告 1、異常 2..、他は 0 だけ正常` under the job 〔未訳〕
- [ ] **21.2** Make that job `cmd.exe /c exit 1` with a job after it, and run it → The job is 警告 with the reason `終了コード 1（警告）`, the next job runs, and the run ends 警告 (yellow) on the run screen, in `koyomi status`, in the history and in the notification title 〔未訳〕
- [ ] **21.3** The same job with `exit 2`, and with `exit 5` and no `judge` at all → Both are 異常 with `終了コード 2` / `終了コード 5`; the next job is 止めた 〔未訳〕
- [ ] **21.4** Give the warn job `retry = { count = 2, wait = "5s" }` and `exit 1` → No `! やり直し` line in the log: a 警告 end is not run again; `exit 2` is retried as before 〔未訳〕
- [ ] **21.5** Write `shell = "powershell"` and `script = ''' 〔未訳〕
- [ ] **21.6** Put a `"`, a `$`, a `'` and a Japanese word in the script (`Write-Output "a `"b`" $env:USERNAME 'c' 日本語"`) → The log shows the line as PowerShell prints it: quotes and `$` reach the shell as written (the script is passed as `-EncodedCommand`) 〔未訳〕
- [ ] **21.7** Write `judge = { warn = 1 }`, `judge = { warn = "5..2" }`, `judge = { ok = "0" }`, `judge = "1"`, `shell = "cmd"`, `shell = "powershell"` with no `script`, `script` with no `shell`, or both `run` and `shell` → `koyomi check` refuses each with the line number and what to write (strings like "1" or "2..", start not above end, unknown field, the shape, `"powershell" のどれか`, `shell には script が要る`, `script には shell = "powershell" が要る`, `run と shell は両方書けない`) 〔未訳〕

## 22. Rerun and start at a job (`rerun`, `run --from`)

- [ ] **22.1** Run a net whose second job fails (`exit 3`), then `koyomi rerun <run>` → A new run with the reason `再実行（<run>）`; the first job is ✓ with the reason `前の回の結果` and keeps its old times, and no new log for it appears under `logs/<day>/<new run>/`; the failed job runs again and has a log; `koyomi status --today` lists both runs 〔未訳〕
- [ ] **22.2** `koyomi rerun <run> --from <first job>`, then `--from x` → The first job runs again too (a new log, the reason `再実行（<run> を <first job> から）`); `--from x` is refused with `ジョブがない: x` and nothing starts 〔未訳〕
- [ ] **22.3** `koyomi rerun` while the run is still going; then a run where everything passed, without `--from` → `<run>: まだ終わっていない`; `<run>: 全部済んでいる（流し直すなら --from <ジョブ>）`; nothing starts either time 〔未訳〕
- [ ] **22.4** Change the net's file (add a job) and `koyomi approve` it, then `koyomi rerun` the old run → The rerun uses the old copy (`logs/<day>/<run>/definition.toml`): the added job is not in it, and `definition.toml` under the new run is the old text 〔未訳〕
- [ ] **22.5** Change the net's file without approving, then `koyomi rerun` → Refused with `未確認のため流さない: <file>（koyomi approve <group/name>）` 〔未訳〕
- [ ] **22.6** `koyomi run <group/name> --from <second job>` → The jobs before it are ⊘ スキップ with the reason `<second job> から流したので飛ばした`, the run line reads `<second job> から`, and only the later logs exist 〔未訳〕
- [ ] **22.7** In the window, open a run that is over and press 再実行; then pick a job in the flow and look at the second button → The new run opens (selected in 今日); the second button reads 「<job> から再実行」 and starts at that job; both buttons are grey while the run is going or while the top warning is shown 〔未訳〕
- [ ] **22.8** `koyomi rerun` with yesterday's run id (from `koyomi history`), and with an id that never was → The rerun starts for yesterday's business day (`--date` is the old run's date); the unknown id is refused with `実行がない: <id>` 〔未訳〕

## 23. Hold, skip, pause (`hold`, `release`, `skip`, `pause`, `resume`)

- [ ] **23.1** `koyomi hold <group/name>` on a confirmed net with a time a minute away, then `koyomi list`, the window's tree, and wait for the time → `保留した: <name>（koyomi release <group/name> で戻す）`; the list and the tree show `⏸ <name>  確認済み・保留`; when the time comes, 今日 and `koyomi status --today` have a ⏸ 保留 run with the reason `保留中（koyomi release <group/name>）`, no job ran, no log folder appeared, and no notification was sent; `koyomi engine status` has the line `保留中: <group/name>（koyomi release … で戻す）` 〔未訳〕
- [ ] **23.2** While held, `koyomi run <group/name>` and the window's 今すぐ流す; then `koyomi hold` again → `保留中のため流さない: <group/name>（koyomi release <group/name>）`, the button is grey and the detail shows the ⏸ 保留中 line; the second hold is refused with `もう保留している: <group/name>` 〔未訳〕
- [ ] **23.3** `koyomi release <group/name>` (and once more), then wait for the next time → `保留を解いた: <group/name>`, then `保留していない: <group/name>`; the mark goes back to ✔ and the next time runs as usual 〔未訳〕
- [ ] **23.4** Run a net a → b → c where a sleeps 10 seconds; while a runs, `koyomi skip <run> b` (also the window: pick b in the flow and press 「b をスキップ」) → `<run> の b を飛ばした`; b is ⊘ スキップ with the reason `人がスキップした` and no log; c runs after a ends; the run ends ✔ and `koyomi history` shows b's skip 〔未訳〕
- [ ] **23.5** `koyomi skip <run> a` while a runs; `koyomi skip <run> c` after the run is over; `koyomi skip <run> x` → `a: もう動いている`, `<run>: もう終わっている`, `ジョブがない: x`; the skip button is grey when the picked job runs or has ended 〔未訳〕
- [ ] **23.6** `koyomi pause`, then `koyomi list`, `koyomi engine status`, the window, and wait for a time; then `koyomi run <group/name>` → `スケジュールを止めた（koyomi resume で戻す）`; the list starts with `⏸ スケジュールを止めている（koyomi resume で戻す）`, engine status has the same line, and the window shows the band 「⏸ スケジュールを止めています…」 with the スケジュールを戻す button while the left panel's スケジュールを止める button is gone; the time becomes a ⏸ 保留 run with the reason `スケジュールを止めている（koyomi resume）` and nothing runs; `koyomi run` still starts the net 〔未訳〕
- [ ] **23.7** `koyomi pause` again, then `koyomi resume` (CLI or the band's button), then `koyomi resume` again → `もう止めている（koyomi resume で戻す）`; `スケジュールを戻した` and the band goes away; `止めていない` 〔未訳〕
- [ ] **23.8** Hold a net and pause, then `koyomi engine stop` and `koyomi engine start`; look at `state_dir("koyomi")/held.toml` → The file has `paused = true` and `held = ["<group/name>"]`; after the restart the hold and the pause are still on (list shows both) 〔未訳〕
- [ ] **23.9** Write `held = 1` into `held.toml` while the engine runs (or before `engine start`), then wait for a time → The warning band and `koyomi list` say `held.toml が読めない（…）。直すか退けるまで流さない`, nothing runs, `koyomi hold`/`pause` are refused with the same line; fixing the file clears it at the next scan 〔未訳〕
- [ ] **23.10** Queue a time behind a running run (`overlap = "queue"`), then `koyomi hold <group/name>` (or `koyomi pause`) before the run ends → The waiting time turns into a ⏸ 保留 run with the reason `待っている間に保留した（koyomi release …）` (`待っている間にスケジュールを止めた（koyomi resume）`), and `engine status` no longer lists it under 待ち 〔未訳〕

## 24. Calendar (`calendars/<name>.toml`, `calendar`, `business_day`, `on_closed`, `koyomi calendar`)

- [ ] **24.1** Write `calendars/会社.toml` with `format = 1`, `holidays = "japan"`, `weekly_off = ["sat", "sun"]`, `closed = [{ date = 2026-11-20, name = "創立記念日" }]`, `open = [2026-11-28]`; then `koyomi calendar 会社 --month 2026-11` → The heading `会社 2026-11（営業日 19 日）`, a weekday header 月〜日, the weeks with `( 3)`, `( 7)( 8)`, `(20)`, `(23)` and `[28]`, then the lines `3 文化の日`, `20 創立記念日`, `23 勤労感謝の日`, `28 休日出勤`, `祝日の表: 2027-12-31 まで` 〔未訳〕
- [ ] **24.2** `koyomi calendar 工場` (no such file), `koyomi calendar 会社 --month 2026-13` → `カレンダーがない: 工場` (exit 1); `--month takes YYYY-MM` 〔未訳〕
- [ ] **24.3** A net with `calendar = "会社"` and `[[when]] every = "month", business_day = 5, at = ["09:00"]`; `koyomi next 経理/<name>`, the window's detail → The next times are the 5th business day of each month (2026-11: 11-09, since 11-03 is 文化の日 and the weekend is off); the window shows the same rows under 次の 10 回 〔未訳〕
- [ ] **24.4** `business_day = -1`, then `day = 20, shift = -3` (same net), `koyomi next` → The last business day of each month (2026-11-30, 2026-12-30 if 12-31 is not in `closed`); 3 business days before the 20th (2026-11-17 since 11-20 is closed: 19→18→17) 〔未訳〕
- [ ] **24.5** `[[when]] every = "day", at = ["02:00"]` with `calendar = "会社"` and no `on_closed`; then `on_closed = "prev"`, `"next"`, `"run"` → `skip` is the default: Saturday, Sunday and 文化の日 have no time; `prev` runs the weekend's and holiday's times on the Friday before (one each, in order), `next` on the Monday after, `run` on the day itself 〔未訳〕
- [ ] **24.6** `business_day = 5` in a net without `calendar` and no `_group.toml` naming one; `koyomi check <file>` and `koyomi list` → `calendar が要る（business_day・shift・on_closed を使うとき）`; the net is ✕ 無効 and does not run 〔未訳〕
- [ ] **24.7** Write `jobnets/経理/_group.toml` with `format = 1` and `calendar = "会社"`; a net of 経理 without `calendar`; `koyomi check` the net and the window's detail → The net counts by 会社 (its 次の 10 回 skip the weekend); `koyomi check jobnets/経理/_group.toml` says `_group.toml: 問題なし`; write `x = 1` into `_group.toml`: every net of 経理 is ✕ 無効 with `_group.toml: 知らない欄 x` 〔未訳〕
- [ ] **24.8** `calendar = "工場"` (no file), then write `calendars/工場.toml` with `holidays = 1` → `カレンダーがない: calendars/工場.toml`, then `calendars/工場.toml が読めない（…）`; the window's band 「カレンダーの注意」 and `koyomi list`'s `注意:` line show `calendars/工場.toml: …`; nothing else stops 〔未訳〕
- [ ] **24.9** `KOYOMI_DATE=2027-10-15` (fewer than 90 days left), `koyomi list`, the window, `koyomi check calendars/会社.toml` → The band and the `注意:` line read `祝日の表は 2027-12-31 までの分です（あと 77 日）`; `koyomi check` prints the same line and exits 0 〔未訳〕
- [ ] **24.10** `KOYOMI_DATE=2028-01-05`, a net on 会社 with `every = "day"`, `at` a minute away; `koyomi check calendars/会社.toml`, `koyomi next`, wait for the time → The band reads `祝日の表が古い（2027-12-31 まで）。それより後の日は流さない`; `koyomi check` prints it and exits 1; `koyomi next` marks each row `祝日の表が古い`; when the time comes, 今日 has a 🔒 run with the reason `祝日の表が古い（2027-12-31 まで）`, no job ran, and a notification was sent 〔未訳〕
- [ ] **24.11** In the window, pick 会社 under カレンダー; use ◀ ▶; click a Saturday, a holiday, a weekday, and each again → The month table shows the holidays and closed days in the warning colour with `( )`, weekends dim, `open` days in the done colour with `[ ]`, today selected; the names list under it; the clicks write `calendars/会社.toml` (`open = [...]` / `closed = [...]` appear and go), the engine reads it again and the table follows within a few seconds; `<file> に書きました` under the month 〔未訳〕
- [ ] **24.12** Make `calendars/会社.toml` read-only, click a day → `calendars/会社.toml に書けない: …` in the error colour; the table does not change 〔未訳〕
- [ ] **24.13** Pick a net that uses 会社 in the tree, then pick 会社 under カレンダー → The calendar view lists the net under 「このカレンダーで流す定義」 and its next 10 times under it, counted by 会社 〔未訳〕
- [ ] **24.14** `koyomi approve` a net on 会社 with `every = "day"` → The week's count in the approval box and in the CLI skips the weekend (5 times, not 7) 〔未訳〕

## 25. Parts of the flow (lines `on` / `code`, branch, join, time, file, notify, child jobnet)

- [ ] **25.1** A net: `a` (fails, exit 3), `b` with `after = [{ job = "a", on = "error" }]`, `c` with `after = ["a"]`; run it → `b` runs, `c` ends `－ 実行なし`（理由 `a が異常終了`）; the run is ✕ 異常（`a` failed）even though the error line led on; the thick line a→c stays in the border colour, the thin line a→b takes `a`'s colour 〔未訳〕
- [ ] **25.2** `d` with `after = [{ job = "a", on = "any" }]` after a failing and after a passing `a` → `d` runs both times; `koyomi review` of the net lists `線: a (any)` under `d` 〔未訳〕
- [ ] **25.3** `a` ends by `timeout = "1s"`, then by `koyomi kill`; `b` has `on = "error"`, `c` has `on = "ok"` → Both times `b` runs and `c` is 実行なし: 打ち切り and 強制終了 count as abnormal on a line 〔未訳〕
- [ ] **25.4** `p` with `kind = "branch"`, `after = ["a"]`; `x` with `after = [{ job = "p", code = "0" }]`, `y` with `code = "1..3"`, `z` with `code = "else"`; `a` exits 0, 2, 9 in turn → Exactly one of x / y / z runs each time (0 → x, 2 → y, 9 → z); the others are 実行なし; `p` reads `【分岐】p` and ends ○ with `a` 's code as its reason 〔未訳〕
- [ ] **25.5** `j` with `kind = "join"`, `wait = "all"`, `after = ["a", "b"]` (a 1 s, b 5 s), `c` after `j`; then `wait = "any"` → `all`: `c` starts after `b` ends; `any`: `c` starts right after `a`, `b` keeps running and the run ends when `b` does 〔未訳〕
- [ ] **25.6** `k` (a plain job) with `after = ["a", "b"]` → `k` waits for both, like `wait = "all"`: a job with 2 or more lines waits for all of them 〔未訳〕
- [ ] **25.7** `t` with `kind = "time"`, `at` 2 minutes ahead, `after = ["a"]`, `c` after `t`; then `at = "00:00"` → `t` reads 実行中 `【時刻待ち】t` with the reason `HH:MM を待っています` and a dashed line; at the time it ends ○ `HH:MM になった` and `c` runs; `00:00`: it ends at once with `00:00 を過ぎていた` 〔未訳〕
- [ ] **25.8** `t` with `days = 1`, `at = "00:00"`; `koyomi kill <run> t` → The reason reads `YYYY-MM-DD 00:00 を待っています` (tomorrow); kill ends it ✕ 強制終了, the run ends ✕ 異常, `c` is 実行なし 〔未訳〕
- [ ] **25.9** `f` with `kind = "file"`, `file = 'C:\in\*.csv'`, `settle = "3s"`, `timeout = "10m"`, `c` after `f` running `cmd /c echo %KOYOMI_FILE%`; drop `note.txt`, then `Sales.csv` and keep writing it for 5 s → `f` waits (`C:\in\*.csv を待っています`), `note.txt` changes nothing; `Sales.csv` ends `f` 3 s after it stops changing: ○ `C:\in\Sales.csv が来た`; `c`'s log shows `C:\in\Sales.csv`; without `settle` it ends as soon as the file is there 〔未訳〕
- [ ] **25.10** `f` with `timeout = "5s"` and no file → `f` ends ✕ 打ち切り（`打ち切り（5 秒 を超えた）`）after 5 s, the run ends ✕ 異常 〔未訳〕
- [ ] **25.11** `n` with `kind = "notify"`, `after = [{ job = "a", on = "error" }]`, and `to = "x"` on it; then without `to`; `a` fails → `koyomi check`: `to は kind = … にだけ書く` (the part has no `to`); without it the notification arrives as `✕ 異常  <title>: n` with the line of `a` and the run's id and day; `n` ends ○ `通知した` at once; the run is ✕ 異常 (`a` failed) 〔未訳〕
- [ ] **25.12** `n` after a passing `a` → The notification is a `done` one (`○ 正常  <title>: n`) with the run's line 〔未訳〕
- [ ] **25.13** `jobnets/共通/バックアップ.toml` (confirmed, jobs `x`, `y`); a net with `c` `kind = "jobnet"`, `use = "共通/バックアップ"`, `after = ["a"]`, `b` after `c`; run it → The run lists `a`, `c/x`, `c/y`, `【子ジョブネット】c`, `b` in that order; `c/x` and `c/y` run after `a`, `b` after them; `c` ends with the child's result; the child's own `[[when]]` is not used 〔未訳〕
- [ ] **25.14** `use = "共通/無い"`; then make `バックアップ.toml` invalid (`x = 1`); then leave it unconfirmed (edit it) → `koyomi check` and `koyomi list`: `子ジョブネットがない: 共通/無い`, `子ジョブネット 共通/バックアップ に定義の誤りがある`; the parent is ✕ 無効 and does not run; unconfirmed: `koyomi run` says `子ジョブネット 共通/バックアップ が未確認` and nothing starts 〔未訳〕
- [ ] **25.15** `バックアップ.toml` gets a `kind = "jobnet"` part with `use` = the parent → Both are ✕ 無効 with `子ジョブネットの呼び合いが輪になっている: 共通/バックアップ → <parent> → 共通/バックアップ` (the ids in the order they call) 〔未訳〕
- [ ] **25.16** A part with no `after` (`kind = "join"`) and a job nothing follows that has `kind = "time"` → `孤立した部品: j（after が無い）` and `孤立した部品: t（後に続くものが無い）`; `after = ["a", "b"]` with `b` after `a` after `b`: `after が輪になっている: a → b → a` 〔未訳〕
- [ ] **25.17** `koyomi review` / the approval box of a net with every kind of part → Each part is headed by its kind (`分岐 p`, `待ち合わせ j` …) with `線:` per line, `待ち方: 全部が終わってから`, `時刻: 06:00（1 日後）`, `待つファイル:`, `落ち着くまで:`, `使う定義: 共通/バックアップ`; a job's lines read `線: a (error)` 〔未訳〕
- [ ] **25.18** `koyomi run` a net with 3 running jobs and `concurrency = 2`, one of them a `kind = "time"` part → The part waits without taking a slot: the two jobs run and the part is 実行中 at the same time 〔未訳〕

## 26. Starts by events (`[[when]] file = …` + `settle`, `on_start = true`)

- [ ] **26.1** A net with `[[when]] file = 'D:\受信\*.csv'` and a job echoing `%KOYOMI_FILE%`; `koyomi approve` it; copy `sales.csv` into `D:\受信` → A run starts within a few seconds; `koyomi show` says `ファイル`（`D:\受信\sales.csv が来た`）; the job's log is the path 〔未訳〕
- [ ] **26.2** The same with `settle = "10s"`; write the file slowly (append every 3 s for 20 s, then stop) → No run while it is being written; a run starts about 10 s after the last write 〔未訳〕
- [ ] **26.3** Copy `note.txt` into `D:\受信` → Nothing starts: only `*.csv` counts 〔未訳〕
- [ ] **26.4** Two files are already in `D:\受信` when the net is approved (or the engine starts) → No run; the history has a ⊘ スキップ row `見張り始めたときに既に 2 個あった（D:\受信\*.csv）: 流さない`; a file copied in afterwards starts a run 〔未訳〕
- [ ] **26.5** Copy `b.csv` in while the run for `a.csv` is still going → `b.csv` waits; its run starts right after the first run ends (one run per file, in name order) 〔未訳〕
- [ ] **26.6** Overwrite `a.csv` (new size or time) after its run ended → A run starts again; a file left as it is does not start another, and the engine's restart does not run it again 〔未訳〕
- [ ] **26.7** Delete `D:\受信` before the engine starts, then make it and copy a file in → `koyomi check` (or the window) says `待つファイルのフォルダーを見張れない（5 秒ごとに見る）`; the file still starts a run within about 5 s 〔未訳〕
- [ ] **26.8** The net has `calendar = "会社"` and the file comes on a Sunday → No run; a ⊘ スキップ row `… が来たが、休業日なので流さない（on_closed = "run" で流せる）`; with `on_closed = "run"` in the `[[when]]` the run starts 〔未訳〕
- [ ] **26.9** The `[[when]]` has `except = [<today>]` or `start = <tomorrow>` → No run; a ⊘ スキップ row `… が来たが、except か start / end の外なので流さない` 〔未訳〕
- [ ] **26.10** `koyomi hold` the net (or `koyomi pause`), then copy a file in → No run; a ⏸ 保留 row with the reason, as for a time; `koyomi release` / `resume` does not run it afterwards (the file counts as recorded) 〔未訳〕
- [ ] **26.11** A net with `[[when]] on_start = true`; `koyomi approve`; `koyomi engine stop` and start the engine again (or sign in again) → A run starts once; `koyomi show` says `起動`（`起動したとき`）; the engine's running on does not start it again 〔未訳〕
- [ ] **26.12** The same net, unapproved, when the engine starts; and with a calendar on a closed day → 🔒 `未確認のため流さなかった（koyomi approve …）`; ⊘ スキップ `起動したが、休業日なので流さない（on_closed = "run" で流せる）` 〔未訳〕
- [ ] **26.13** `koyomi next` and the net's 予定 in the window for a net with both events and `every = "day"` → The rows `ファイルが来たとき: D:\受信\*.csv（10 秒 変わらなくなってから）` and `PC の起動・サインインのとき（エンジンの起動）` come first, then the times; `koyomi review` of it shows the same two rows under 今週 〔未訳〕
- [ ] **26.14** `koyomi check` a `[[when]]` with `file = 'in\*.csv'`, with `file = 'D:\*\a.csv'`, with `file = …` and `on_closed = "prev"`, and with `on_start = false` → `file は絶対パスで書く（例: …）`; `file の * / ? はファイル名の部分だけ（フォルダーには使えない）`; `file / on_start では on_closed は "skip" か "run"`; `on_start は true だけ（使わないなら [[when]] ごと消す）` 〔未訳〕

## 27. Links between jobnets (`[[when]] after_jobnet = …` + `on`)

- [ ] **27.1** Net `g/a` (one job echoing hi) and net `g/b` with `[[when]] after_jobnet = "g/a"` and a job echoing `%KOYOMI_DATE%`; `koyomi approve` both; `koyomi run g/a` → When `g/a` ends 正常終了, a run of `g/b` starts within a few seconds; its history row's reason is `g/a が終わった（正常終了）` (and `how = "after"` in the history file); its job's log is today's date 〔未訳〕
- [ ] **27.2** `koyomi rerun` a run of `g/a` whose business day was 2 days ago → `g/b` starts again, with that run's business day (`KOYOMI_DATE` is 2 days ago, and the history row shows that date) 〔未訳〕
- [ ] **27.3** `g/a`'s job made to fail (`exit 1`); `koyomi run g/a` with `g/b` on `on = "ok"` (default), then with `on = "error"`, then `on = "any"` → With the default, nothing starts (no row at all for `g/b`); with `"error"` and `"any"`, `g/b` starts with `g/a が終わった（異常終了）` 〔未訳〕
- [ ] **27.4** `g/a`'s job ends 警告終了 (`warn_codes`), `g/b` on `on = "error"` and then on `on = "ok"` → `"error"`: nothing; `"ok"`: `g/b` starts (`g/a が終わった（警告終了）`) 〔未訳〕
- [ ] **27.5** `koyomi run g/a` twice on the same business day (after 27.1) → The second time `g/b` does not start: a ⊘ スキップ row `g/a が終わった（正常終了）が、同じ業務日（<date>）に既に流した（<run id>）: 後続は流していない。要るなら手で流す（koyomi run g/b）`, and a notification says so 〔未訳〕
- [ ] **27.6** After 27.5, `koyomi run g/b` by hand (and from the window) → It runs: only the automatic start is once a day 〔未訳〕
- [ ] **27.7** `g/b` given a job that sleeps 60 s; `koyomi run g/a` twice quickly (two different runs of `g/a` ending while `g/b` is still running, with the second `g/a` run `rerun` from another day) → The second end: ⊘ スキップ `… が、前の回がまだ動いているので流さない`; nothing queues 〔未訳〕
- [ ] **27.8** `g/b` unapproved; `koyomi hold g/b`; `koyomi pause`; each time `koyomi run g/a` → 🔒 `未確認のため流さなかった（koyomi approve …）`; ⏸ 保留 with the reason; ⏸ with the pause's reason — as for a time 〔未訳〕
- [ ] **27.9** `g/b` with `calendar = "会社"`, and `koyomi rerun` a run of `g/a` whose business day is a Sunday → ⊘ スキップ `g/a が終わった（正常終了）が、休業日なので流さない（on_closed = "run" で流せる）`; with `on_closed = "run"` in the `[[when]]`, `g/b` runs with that Sunday as `KOYOMI_DATE` 〔未訳〕
- [ ] **27.10** `koyomi check` with `after_jobnet = "g/none"`; and with `g/a` after `g/b` and `g/b` after `g/a` → `先行のジョブネットがない: g/none`; `after_jobnet が輪になっている: g/a → g/b → g/a` on both (the window shows them as ✗ 誤り and does not run them); a net after `g/a` that is not itself in the loop is fine 〔未訳〕
- [ ] **27.11** `koyomi check` a `[[when]]` with `after_jobnet = "売上取込"`; with `on = "sometimes"`; with `every = "day"` and `on = "ok"`; with `after_jobnet = …` and `on_closed = "prev"` → `after_jobnet は "グループ/名前" で書く（例: …）`; `on は "ok"・"error"・"any" のどれか`; `on は day では使わない`; `after_jobnet では on_closed は "skip" か "run"` 〔未訳〕
- [ ] **27.12** `koyomi next`, the net's 予定 in the window, and `koyomi review` for `g/b` → A row with no time: `g/a が終わったとき（正常終了・警告終了のとき）` (`異常終了のとき` / `どう終わっても` for the other `on`s) 〔未訳〕
- [ ] **27.13** Three nets `g/a` → `g/b` → `g/c` (each after the one before); `koyomi rerun` a run of `g/a` from 3 days ago → `g/b` and then `g/c` run, each with `g/a`'s business day; the history has the three rows with that date 〔未訳〕

## 28. History search and the rest of the CLI (`history`, `show`, `list --group`, `--help`, `--toml`)

- [ ] **28.1** Some runs over the last days (ok, error, skipped); `koyomi history` → The runs of the last 7 days up to today, oldest first, each as `status --today` prints it (the mark, the id, the title, the date, the times, the jobs under it) 〔未訳〕
- [ ] **28.2** `koyomi history --from 2026-10-01 --to 2026-10-15`; `--from` alone; `--to` alone → Those days only; `--from` alone goes up to today; `--to` alone covers the 7 days up to it 〔未訳〕
- [ ] **28.3** `koyomi history --status error`; `--status ok g/a`; `koyomi history g\a` → Only the runs of that state; only that net's; `\` reads as `/` 〔未訳〕
- [ ] **28.4** `koyomi history --status nope`; `--from 2026/10/01`; `--from 2026-10-20 --to 2026-10-01`; `--from 2024-01-01` → `--status takes one of ok, warn, …` and the usage (exit 2); `--from takes YYYY-MM-DD` (exit 2); `koyomi: 範囲が逆: …` (exit 1); `koyomi: 範囲は 1 年まで` (exit 1) 〔未訳〕
- [ ] **28.5** `koyomi history --status held` with no held run in the days → `該当する実行はありません` (exit 0) 〔未訳〕
- [ ] **28.6** `koyomi history --toml`, `koyomi status --toml`, `koyomi list --toml`, `koyomi show g/a --toml` → The runs as `[[runs]]` tables (the same shape as the history files); the snapshot (`[[defs]]`, `warnings`, `held`, `paused`, …); the definition as the engine read it (`id`, `file`, `standing`, `[net]`, `[[problems]]`) 〔未訳〕
- [ ] **28.7** `koyomi next g/a --toml`; `koyomi calendar 会社 --toml`; `koyomi log <run> <job> --toml` → `unconfirmed = …` and `[[next]]` with `at = "2026-11-02 09:00:00"` or `event = "…"`; `calendar`, `month`, `business_days` and a `[[day]]` per date with `kind = "business" / "open" / "closed" / "holiday" / "weekly_off"` and `name` when it has one; `run`, `job`, `text` 〔未訳〕
- [ ] **28.8** `koyomi hold g/a --toml`; `koyomi engine status --toml` with the engine down; `koyomi check bad.toml --toml` → `ok = true` + `lines = ["…"]`; `ok = false` + `lines = ["koyomi: エンジンは動いていない"]` (exit 1); `ok = false` and the problems as `lines` (exit 1) 〔未訳〕
- [ ] **28.9** `koyomi approve g/a --toml` on an unconfirmed net → The preview and `確かめたなら y を入力` are still said as text (a person answers); after `y`, `ok = true` + `lines` 〔未訳〕
- [ ] **28.10** `koyomi list --help`, `koyomi history -h`, `koyomi --toml list --help`, `koyomi pause --help` → Only that command's lines of the usage under `usage:`, then `every command takes --help …` (exit 0); `pause --help` prints the `pause \| resume` line 〔未訳〕
- [ ] **28.11** `koyomi list --group 経理`; `koyomi list --group nothing`; `koyomi list --toml --group 経理` → Only that group's nets in the tree (the warnings and notes still above); `koyomi: グループがない: nothing` (exit 1); the snapshot with only those defs 〔未訳〕
- [ ] **28.12** `koyomi show g/a` (confirmed, unconfirmed, held, and ✗ 誤り) → The file and its standing (`⏸ 保留` added when held), then what `approve` shows (the jobs, the times, the next week) without the question or the diff; a ✗ net prints its problems and nothing under them; exit 0 〔未訳〕
- [ ] **28.13** The window: click 履歴 under 今日 → The right side shows 履歴: 開始日 (7 days ago), 終了日 (today), ジョブネット = すべて, 状態 = すべて, 検索; the rows of those days newest first (`2026-10-29  ✓ 正常終了  売上取込  09:00:00  09:00:12  12 s`, coloured by state); the count above; the tree's and 今日's picks are off 〔未訳〕
- [ ] **28.14** Change 開始日 to a date and press 検索; type `2026/10/1` and press 検索 → The rows of the new days; `日付は YYYY-MM-DD で書く` in red and the rows stay 〔未訳〕
- [ ] **28.15** Pick a net in ジョブネット, then a state in 状態 → The rows narrow at once (no 検索 needed); the count follows; `該当する実行はありません` when nothing matches 〔未訳〕
- [ ] **28.16** Click a row in 履歴 → Under the rows: the run's picture, its jobs' logs on click, 再実行 and the rest as under 今日; the row stays marked 〔未訳〕
- [ ] **28.17** While 履歴 is open: click a net in the tree, a run under 今日, or a calendar; then 履歴 again → Each closes 履歴 and shows its own detail; 履歴 opens again on the last 7 days with nothing narrowed 〔未訳〕
- [ ] **28.18** Narrator / NVDA over 履歴 → The 履歴 row, the 検索 button and each run row are read (row: the date, the state, the title, the times) 〔未訳〕
- [ ] **28.19** `koyomi --keys` report with 履歴 open → A `history: 2026-10-23..2026-10-30 * * 12` line (the days, the net or `*`, the state key or `*`, the rows shown) 〔未訳〕

## 29. Editor (`ui/canvas.rs`, `koyomi new` / `versions` / `restore`)

- [ ] **29.1** Select a jobnet in the tree, click 編集 → The canvas replaces the picture: the parts where the file (or 整列) puts them, the shelf on the left, 保存 / 整列 / 元に戻す / やり直す / ジョブネット / ← at the top; the tree stays usable 〔未訳〕
- [ ] **29.2** A jobnet whose file does not read (a `問題` row) → 編集 is greyed out; nothing opens 〔未訳〕
- [ ] **29.3** Drag a part from the shelf onto the canvas; click a part in the shelf → The part appears where it was dropped with a new id (`job2`, `notify1` …) and is selected; a click puts it after the cursor's part and links it with 成功したら 〔未訳〕
- [ ] **29.4** Drag from the right port of a part to another part → A line (成功したら) from the first to the second; dropping on nothing or on the same part draws no line 〔未訳〕
- [ ] **29.5** Right-click a line → A menu: 成功したら / 失敗したら / どちらでも / 線を消す; the choice changes or removes the line 〔未訳〕
- [ ] **29.6** Double-click a job → The right pane shows its fields (id, 名前, 実行, フォルダー, 制限時間, 再試行, シェル, 警告にする/エラーにする); a bad value (an empty id, `制限時間 = abc`, an id already used) turns red and is not applied; a good one applies at once 〔未訳〕
- [ ] **29.7** Double-click a branch, join, time, file or notify part → The pane shows that part's fields (the condition, the time, the path, the message); the same red rule 〔未訳〕
- [ ] **29.8** Click ジョブネット at the top → The pane shows the net's fields (名前, カレンダー, 逃した回, 重なったら) and its `[[when]]` list: each with a kind chooser and its own fields, 追加 / 削除, and 次の 10 回 below; a bad field keeps the last good list and shows the error in red 〔未訳〕
- [ ] **29.9** Drag on empty canvas; wheel; Ctrl+wheel; Shift+drag; Ctrl+click → Drag pans, the wheel scrolls, Ctrl+wheel zooms around the pointer, Shift+drag selects the parts in the box, Ctrl+click adds one to the selection 〔未訳〕
- [ ] **29.10** Keys: arrows, `a`, `c` + arrow + Enter, Enter, Delete → Arrows move the cursor to the nearest part that way; `a` adds a job after the cursor; `c` starts a line from the cursor, arrows pick the target, Enter draws it (Escape cancels); Enter opens the cursor's pane; Delete removes the selected parts and their lines 〔未訳〕
- [ ] **29.11** Ctrl+Z / Ctrl+Y / Ctrl+C / Ctrl+V / Ctrl+A → Undo and redo every change (a rename keeps the pane on the same part); copy and paste the selection (new ids, offset a little); select all 〔未訳〕
- [ ] **29.12** 整列 → Every part moves to where `layout::align` puts it (layers along the flow, rows across); undo brings the old places back 〔未訳〕
- [ ] **29.13** The flow button (横 / 縦) in the toolbar → The canvas and the pictures elsewhere switch between horizontal and vertical; the choice is kept in `state.toml` (`flow`) 〔未訳〕
- [ ] **29.14** Make a job with no `run` (or two jobs with one id), Ctrl+S → The problems show in red under the toolbar with the part named; 保存 is greyed out and Ctrl+S does nothing until they are fixed 〔未訳〕
- [ ] **29.15** Fix the problems, 保存 (Ctrl+S) → 保存した appears; the file is rewritten from the editor (comments gone), the old text is under `versions/<group>/<name>/<time>.toml`, and the net is confirmed (no 未確認 badge, no `approve` needed) 〔未訳〕
- [ ] **29.16** Edit the file by hand (or `koyomi restore`) while the editor is open, then 保存 → A box: ほかで変わっています, with the differences between what the editor started from and the file now; 読み直す drops the editor's changes and loads the file, やめる keeps editing, 上書きする saves over it (the hand edit is kept as a version); Escape = やめる 〔未訳〕
- [ ] **29.17** With unsaved changes: ← , the tree, a run under 今日, 履歴, or the palette's 閉じる → A box: 変更を捨てて閉じますか; 捨てて閉じる leaves, やめる (the default, Escape) stays 〔未訳〕
- [ ] **29.18** Without changes: ← → The editor closes at once and the picture is back 〔未訳〕
- [ ] **29.19** Double-click a child jobnet part (or select it and Enter) → Its net opens in the editor on top; ← comes back to the parent (asking first when the child has changes) 〔未訳〕
- [ ] **29.20** Ctrl+P while editing → The palette lists 保存 / 整列 / 横と縦 / 元に戻す / やり直す / 閉じる above the usual commands; they are gone when the editor is closed 〔未訳〕
- [ ] **29.21** `koyomi new 経理/新規` → `jobnets/経理/新規.toml` from the template (one job, no `[[when]]`), the net appears unconfirmed; `new` on an existing file refuses 〔未訳〕
- [ ] **29.22** `koyomi versions 経理/新規`; `koyomi restore 経理/新規 <time>` → The versions newest first; restore writes that version back (today's text becomes a version too) and the net is unconfirmed until `approve` 〔未訳〕
- [ ] **29.23** Narrator / NVDA over the editor → The toolbar buttons, the shelf's parts, the pane's fields and the box buttons are read 〔未訳〕
- [ ] **29.24** `koyomi --keys` report with the editor open → An `editor: 経理/新規 dirty=false parts=1 selected=[] cursor=- pane=- problems=0 connect=false` line 〔未訳〕

## 30. Import, export and the daily backup (`koyomi export` / `import`, `backups/`)

自動テスト済みなので下には出していない: 30.4, 30.5, 30.6

- [ ] **30.1** `koyomi export -o out.toml --all` with nets, a group file, a calendar and `koyomi.toml` that has `webhook = "secret:x"` → One TOML with `[[calendar]]`, `[[group]]`, `[[jobnet]]` (each file's text as it is, comments kept) and `[settings]` whose webhook addresses are `***`; the count is printed 〔未訳〕
- [ ] **30.2** `koyomi export -o out.toml` again; `-n 経理/月次締め` alone; `--group 経理`; `--calendars`; `--settings` → The second write is refused (`もうある`); a net comes with its group file, calendar and child nets; a group brings its nets; the other flags pick only those 〔未訳〕
- [ ] **30.3** `koyomi import out.toml --dry-run` on a PC where one net differs → The plan: `追加`/`上書き`/`同じ` per thing with a `+`/`-` diff under what differs; exit 1 and nothing written 〔未訳〕
- [ ] **30.7** `koyomi import out.toml --toml` → The plan as TOML (`lines`, `conflicts`, `problems`, `undecided`, `writes`, `applied`) 〔未訳〕
- [ ] **30.8** Palette → 書き出す → Under the detail: `書いた: <state>/exports/<YYYYMMDD-HHMMSS>.toml（N 件…）` and the file is there 〔未訳〕
- [ ] **30.9** Palette → 取り込む…, type a wrong path, Enter → The box keeps the field with `…を読めない` in red; Esc closes it 〔未訳〕
- [ ] **30.10** Type the path of a bundle whose nets differ, Enter → The plan box (the lines of 30.3, cut after 30 with `あと N 行`) with 上書きする / やめる / 名前を変えて足す / 違うものは飛ばす; Enter on the first frame = やめる, Tab moves, Esc = やめる 〔未訳〕
- [ ] **30.11** Pick 上書きする → `取り込んだ（ジョブネットは未確認…）` under the detail; the tree shows the nets `未確認`; old files are under `versions/` 〔未訳〕
- [ ] **30.12** Type a bundle with nothing different, Enter → 取り込む / やめる; a bundle that is all the same says `書くものはありません` with one 閉じる button 〔未訳〕
- [ ] **30.13** Start the engine, wait, `koyomi engine status` → A `バックアップ: <time>、backups/<YYYY-MM-DD>.toml に N 件（7 日分を残す）` line; the file is a bundle `import` reads; an empty settings folder says `定義がないので書かなかった` and leaves the old day files 〔未訳〕
- [ ] **30.14** Narrator / NVDA over the 取り込む box → The field is read as ファイル, the buttons by their labels, the plan's lines as text 〔未訳〕

## 31. A new jobnet from the window

- [ ] **31.1** Start `koyomi` with no definitions → The tree shows `＋ 新しいジョブネット` above `定義がありません…`; the message names the button 〔未訳〕
- [ ] **31.2** Press `＋ 新しいジョブネット` → The 新しいジョブネット box opens in the middle with グループ and 名前; the keyboard is in 名前; the window behind takes no clicks 〔未訳〕
- [ ] **31.3** Press 作る with a field empty; then type `a/b`, `..`, `a:b` or `締め.` as the name → `グループと名前を入れてください` and `/ \ : * ? " < > \| と、末尾の . は使えません` in red; the box stays open 〔未訳〕
- [ ] **31.4** Type a group and a name a net already has (try other upper/lower case) → `もうあります: <group/name>`; the box stays open 〔未訳〕
- [ ] **31.5** Type `テスト` and `新規1`, Enter → The box closes; the editor opens titled `編集 テスト/新規1（新規・未保存） *` with one job `初のジョブ`; no file is written yet 〔未訳〕
- [ ] **31.6** Edit the job, Ctrl+S → `jobnets/テスト/新規1.toml` is written; the title loses `（新規・未保存）` and `*`; the tree shows the net under テスト as confirmed, not `未確認` 〔未訳〕
- [ ] **31.7** Right-click a group heading in the tree → このグループに新しいジョブネット → The box opens with グループ filled in; the グループ list (今あるグループ) picks another one 〔未訳〕
- [ ] **31.8** Ctrl+N, and palette → 新しいジョブネット… (the hint says Ctrl+N) → Both open the box; Ctrl+N does nothing while the editor is open; Tab moves between the fields, Esc closes the box 〔未訳〕
- [ ] **31.9** Open the box while the editor has unsaved changes, fill it, Enter → `編集中の変更を保存するか閉じてから作ってください`; nothing is lost 〔未訳〕
- [ ] **31.10** Make `テスト/新規2`, then before saving write `jobnets/テスト/新規2.toml` by hand, then Ctrl+S → `もうある（ほかで作られた）: テスト/新規2。閉じて、別の名前で作り直す`; the hand-written file is unchanged; no overwrite button 〔未訳〕
- [ ] **31.11** Make a net and press 閉じる without saving → The discard question comes first, with the keyboard on やめる (Enter keeps the editor); Tab, then Enter on 捨てて閉じる leaves no file 〔未訳〕
- [ ] **31.12** `koyomi --keys "<Wait:2500><C-n><Wait:400>shime<Enter><Wait:300><State:b><S-Tab>keiri<Enter><Wait:500><State:c><Quit>"` → `b.txt` has `make: /shime (グループと名前を入れてください)`; `c.txt` has `editor: keiri/shime dirty=true parts=1 … new=true` (with no nets yet; once a group exists, its list sits between the two fields, so press `<S-Tab>` twice) 〔未訳〕
- [ ] **31.13** Narrator / NVDA over the box → The fields are read as グループ and 名前, the buttons by their labels; the error is read as text 〔未訳〕

## 32. 日付の埋め込み

- [ ] **32.1** A job `run = ["cmd.exe", "/c", "echo", "{date}", "{month-1}"]`; `koyomi run <net> --date 2026-01-15 --wait`, then `koyomi log` → The log has `2026-01-15 202512` 〔未訳〕
- [ ] **32.2** A net with `calendar = "会社"` (土日 off) and `{date-1bd:%m%d}`; run it with `--date 2026-11-02` → The log has `1030` (the Friday) 〔未訳〕
- [ ] **32.3** The same `{date-1bd}` in a net with no calendar; `koyomi run` → Refused: `… 営業日で動かすには calendar が要る`; no run starts 〔未訳〕
- [ ] **32.4** Write `{date-1x}` in `run`; `koyomi check` → A problem with the line number: `run: {date-1x}: 単位が読めない…` 〔未訳〕
- [ ] **32.5** A PowerShell `script` with `@{ a = 1 }` and `{ … }` → Runs as written; the braces are not touched 〔未訳〕
- [ ] **32.6** `koyomi rerun` a run of an earlier day → The tokens are written for that run's business day, not today 〔未訳〕
