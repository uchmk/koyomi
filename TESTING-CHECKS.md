# Checklist

21 checks in TESTING.md, 20 left for a person.

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
- [ ] **6.3** A file with `timeout = 60` → "この版ではまだ使えない: timeout"; it cannot be approved 〔未訳〕

## 7. Safety of the places

- [ ] **7.1** Let Everyone write a definition, then `check` and `approve` → Both exit 1 and name the file 〔未訳〕
- [ ] **7.2** Same, with the window open → A warning band names the place and who; 確かめる… is disabled for every net 〔未訳〕
- [ ] **7.3** Break `approved.toml` by hand, then `list` and `approve` → Every net shows `▲ 未確認`, a warning names `approved.toml`, `approve` exits 1 and the file is left as it was; once it is mended the approvals come back without a restart 〔未訳〕
