# Checklist

148 checks in TESTING.md, 147 left for a person.

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
