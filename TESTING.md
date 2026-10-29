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
| 6.3 | A file with `timeout = 60` | "この版ではまだ使えない: timeout"; it cannot be approved |

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
