# koyomi エンジンを Debian にインストールする

koyomi のエンジンを Debian で systemd のサービスとして実行し、Windows の画面から HTTPS で接続する手順です。

## 必要なもの

- systemd で動いている Debian 12 以降。GUI のライブラリは必要ありません。`LoadCredential` にフォルダーを渡す使い方が必要です (Debian 11 の systemd 247 のマニュアルにはありません)。`install.sh` が実際に試して確認し、読み込めない場合はそこで停止します。
- root の権限 (`sudo`) が必要です。
- Windows の PC からエンジンのポートに TCP で届く必要があります。

## インストール

```sh
tar xzf koyomi-<版>-linux-<arch>.tar.gz
cd koyomi-<版>-linux-<arch>
sudo ./install.sh            # ポートは 8790
sudo ./install.sh --port 9000   # 初回にポートを変えるとき
```

`install.sh` は、利用者 `koyomi` を作成し、実行ファイルとサービスを配置し、`admin` の token を作成して、エンジンを起動します。何度実行しても問題ありません。

最後に、接続する URL (`https://<アドレス>:<ポート>`)、証明書の指紋、token の読み込み方法が表示されます。`--port` は最初に `koyomi.toml` を作成するときだけ有効です。

## Windows の画面から接続する

1. 画面の状態バーの接続先を押し、一番下の「接続先を管理…」から「接続先を追加する…」を開きます (コマンドパレットの「別の PC のエンジンに接続する」でも開けます)。名前と URL (`install.sh` が表示したもの) を入力します。
2. 画面に表示された指紋が、`install.sh` が表示した指紋と同じか確認し、「同じでした」を押します。異なる場合は接続しないでください。
3. `admin` の token を入力して「接続する」を押します。token は Debian で `sudo cat /etc/koyomi/credentials/http-admin` を実行すると読み込めます。

## 利用者を追加する

`/var/lib/koyomi/.config/uchmk/koyomi/koyomi.toml` に `[[http.user]]` を追加し、token を `koyomi-secret` で登録します。

```toml
[[http.user]]
name = "taro"
role = "run"
token = "secret:http-taro"
```

```sh
sudo koyomi-secret http-taro      # 値は標準入力から読み込みます (端末では入力が画面に表示されません)
```

`koyomi-secret` は値を引数で受け取りません。koyomi が起動していれば再起動します（実行中のジョブは停止します）。停止していれば次の起動から使われます。

## 権限

`role` は `view`・`run`・`edit`・`admin` のどれかです。`admin` は確定もできるので、token を渡す相手と token の扱いに注意してください。

## ファイルの場所

| 内容 | 場所 |
| --- | --- |
| 設定 (`koyomi.toml`、`jobnets/`、`calendars/`、`tls/`) | `/var/lib/koyomi/.config/uchmk/koyomi/` |
| 状態 | `/var/lib/koyomi/.local/state/uchmk/koyomi/` |
| シークレット (token など) | `/etc/koyomi/credentials/` (root だけが読める) |
| 実行ファイル | `/usr/local/bin/koyomi`、`/usr/local/sbin/koyomi-secret` |
| サービス | `/etc/systemd/system/koyomi.service` |

## 状態を確認する

```sh
systemctl status koyomi
journalctl -u koyomi
sudo -H -u koyomi koyomi status
```

## 更新

新しい tar.gz を展開して、もう一度 `sudo ./install.sh` を実行します。実行中のジョブは停止します。設定と token は変更されません。

## 停止する

```sh
sudo systemctl stop koyomi
```

`koyomi engine stop` で停止すると、`Restart=on-failure` でも自動では再起動しないので、`sudo systemctl start koyomi` で起動してください。

## 削除する

```sh
sudo ./install.sh --uninstall
```

サービスと実行ファイルを削除します。`/var/lib/koyomi` (設定と状態)、`/etc/koyomi` (シークレット)、利用者 `koyomi` は残ります。これらを削除するコマンドは `--uninstall` の最後に表示されます (自動では実行しません)。

## 第 1 段階の制約

- `koyomi.toml` は Debian 側で修正します。画面からは修正できません。
- 時刻帯は変更しません。画面を開く PC と同じ時刻帯で使用してください。
- Linux で動作しない機能があります: `pwsh` が必要な後始末、`run_as`、イベントログ。画面に注意として表示されます。
