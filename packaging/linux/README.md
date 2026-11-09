# koyomi エンジンを Debian に入れる

koyomi のエンジンを Debian で systemd のサービスとして動かし、Windows の窓から HTTPS でつなぐ手順です。

## 必要なもの

- systemd で動いている Debian 12 以降。GUI のライブラリは要りません。`LoadCredential` にフォルダーを渡す使い方が要ります (Debian 11 の systemd 247 の説明書にはありません)。`install.sh` が実際に試して確かめ、読めなければそこで止まります。
- root の権限 (`sudo`)。
- Windows の PC からエンジンのポートに TCP で届くこと。

## 入れ方

```sh
tar xzf koyomi-<版>-linux-<arch>.tar.gz
cd koyomi-<版>-linux-<arch>
sudo ./install.sh            # ポートは 8790
sudo ./install.sh --port 9000   # 初回にポートを変えるとき
```

`install.sh` は、利用者 `koyomi` を作り、実行ファイルとサービスを置き、`admin` の token を作って、エンジンを起動します。何度実行しても構いません。

最後に、つなぐ URL (`https://<アドレス>:<ポート>`)、証明書の指紋、token の読み方が出ます。`--port` は最初に `koyomi.toml` を書くときだけ効きます。

## Windows の窓からつなぐ

1. 窓の「別の PC」のページを開き、名前と URL (`install.sh` が出したもの) を入れます。
2. 窓に出た指紋が、`install.sh` が出した指紋と同じか確かめます。違うときはつながないでください。
3. `admin` の token を入れます。token は Debian で `sudo cat /etc/koyomi/credentials/http-admin` と打つと読めます。

## 人を足す

`/var/lib/koyomi/.config/uchmk/koyomi/koyomi.toml` に `[[http.user]]` を足し、token を `koyomi-secret` で登録します。

```toml
[[http.user]]
name = "taro"
role = "run"
token = "secret:http-taro"
```

```sh
sudo koyomi-secret http-taro      # 値は標準入力から (端末なら画面に出ません)
```

`koyomi-secret` は値を引数で受け取りません。koyomi が動いていれば再起動します（動いているジョブは止まります）。止まっていれば次の起動から使われます。

## 権限

`role` は `view`・`run`・`edit`・`admin` のどれかです。`admin` は確定もできるので、渡す相手と token の扱いに注意してください。

## ファイルの場所

| 内容 | 場所 |
| --- | --- |
| 設定 (`koyomi.toml`、`jobnets/`、`calendars/`、`tls/`) | `/var/lib/koyomi/.config/uchmk/koyomi/` |
| 状態 | `/var/lib/koyomi/.local/state/uchmk/koyomi/` |
| 秘密 (token など) | `/etc/koyomi/credentials/` (root だけが読める) |
| 実行ファイル | `/usr/local/bin/koyomi`、`/usr/local/sbin/koyomi-secret` |
| サービス | `/etc/systemd/system/koyomi.service` |

## 状態を見る

```sh
systemctl status koyomi
journalctl -u koyomi
sudo -H -u koyomi koyomi status
```

## 更新

新しい tar.gz を展開して、もう一度 `sudo ./install.sh` を実行します。動いているジョブは止まります。設定と token は変わりません。

## 止める

```sh
sudo systemctl stop koyomi
```

`koyomi engine stop` で止めると、`Restart=on-failure` でも自動では戻らないので、`sudo systemctl start koyomi` が要ります。

## 消す

```sh
sudo ./install.sh --uninstall
```

サービスと実行ファイルを消します。`/var/lib/koyomi` (設定と状態)、`/etc/koyomi` (秘密)、利用者 `koyomi` は残ります。消すコマンドは `--uninstall` の最後に出ます (自動では実行しません)。

## 第 1 段階の制約

- `koyomi.toml` は Debian 側で直します。窓からは直せません。
- 時刻帯は変えません。窓の PC と同じ時刻帯で使ってください。
- Linux で動かない機能があります: `pwsh` が要る片付け、`run_as`、イベントログ。窓に注意として出ます。
