# koyomi

Small job scheduler and monitor (a light JP1)

One of the uchmk apps. Its shared parts come from [ito](https://github.com/uchmk/ito).

When koyomi shows an error or a warning, search [こんな時はどうする](docs/troubleshooting.md)
(what to do when…, in Japanese) for the ID at the start of the message, such as `KOYOMI-E2101`.

    cargo run

## What it looks like

![The Today tab: today's runs and the ones still to come (ended, running, waiting, and the sales report that failed), the gantt chart below them, and on the right the selected job net's next runs and its jobs](docs/images/today.png)

![Going round the window: picking the failed run in today's table, the flow of the month-end closing job net, the working days and holidays of the company calendar, the failed job's log in History, and marking the failure as handled](docs/images/koyomi.gif)

![The History tab: the runs of the past 7 days, and for the selected failed run the flow of its jobs, their states and exit codes, and the failed job's log](docs/images/history.png)

How the images were taken is in [docs/images/README.md](docs/images/README.md) (in Japanese).

## Running the engine on Debian

The engine can run on Debian 12 or later as a systemd service, and the window on Windows
connects to it over HTTPS.

1. From [Releases](https://github.com/uchmk/koyomi/releases), put `koyomi_<version>_amd64.deb`
   (or `koyomi-<version>-linux-x86_64.tar.gz`) on the Debian machine.
2. Run `sudo apt install ./koyomi_<version>_amd64.deb` (for the tar.gz, unpack it and run
   `sudo ./install.sh`). At the end it prints the URL to connect to, the certificate's
   fingerprint and how to load the token.
3. In the window on Windows, click the connection in the status bar, then
   **Manage connections...** and **Add a connection...**, and enter the URL and the token.

Each user is given a role (`view`, `run`, `edit` or `admin`) that decides what they can do.
The user names exist only inside koyomi; they have nothing to do with the Linux users.

The full steps (users and roles, where the files are, updating, removing, and the limits)
are in [koyomi エンジンを Debian にインストールする](packaging/linux/README.md)
(installing the koyomi engine on Debian, in Japanese).
