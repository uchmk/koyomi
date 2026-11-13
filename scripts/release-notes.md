### The engine on Debian

`koyomi-debian-<version>-amd64.tar.gz` (and `-arm64`) is the engine for Debian 12 or later,
packed by `scripts/package-linux.sh` and added to the release by hand after the build. It holds the binary
(built for glibc 2.31), `install.sh`, `koyomi-secret`, `koyomi-user`, the systemd unit and a README (Japanese):

```sh
tar xzf koyomi-debian-<version>-amd64.tar.gz
cd koyomi-debian-<version>-amd64
sudo ./install.sh
```

`install.sh` starts the engine as the `koyomi` service, makes an `admin` token and prints the fingerprint
and where to read the token; the window on Windows connects with them (the status bar's connection list).

`koyomi_<version>_amd64.deb` and `koyomi_<version>_arm64.deb` hold the same files as a package, made by
`scripts/package-linux.sh --deb` and added by hand too. `sudo apt install ./koyomi_<version>_amd64.deb` does what
`install.sh` does; updates and removal go through `apt`. The .deb and the tar.gz do not go on one machine together.

