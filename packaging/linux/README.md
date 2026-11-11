# koyomi エンジンを Debian にインストールする

koyomi のエンジンを Debian で systemd のサービスとして実行し、Windows の画面から HTTPS で接続する手順です。

## 必要なもの

- systemd で動いている Debian 12 以降。GUI のライブラリは必要ありません。`LoadCredential` にフォルダーを渡す使い方が必要です (Debian 11 の systemd 247 のマニュアルにはありません)。`install.sh` が実際に試して確認し、読み込めない場合はそこで停止します。
- root の権限 (`sudo`) が必要です。
- Windows の PC からエンジンのポートに TCP で届く必要があります。

## 入手

[Releases](https://github.com/uchmk/koyomi/releases) から `koyomi-<版>-linux-x86_64.tar.gz` をダウンロードします。同じページの `koyomi-v<版>-linux-x64.tar.gz` (`v` のつくもの) は最新の Ubuntu でビルドしたもので、Debian では確かめていません。インストールの道具も入っていません。

Windows でダウンロードした場合は、PowerShell から Debian に送れます。

```powershell
scp .\koyomi-<版>-linux-x86_64.tar.gz <Debian の利用者>@<Debian のアドレス>:
```

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

### アドレス

- URL には IP アドレスもホスト名も使えます (`https://192.168.0.10:8790`)。IPv6 は `https://[fd00::10]:8790` のように角かっこで囲みます。
- 接続は証明書の指紋だけで確かめるので、証明書の名前とアドレスが一致していなくても接続できます。
- アドレスが変わると接続できなくなります。DHCP を使っている場合は、ルーターでアドレスを予約するか、固定のアドレスにしてください。変わった場合は、接続先を削除して追加し直すか、Windows の `koyomi.toml` の `[[remote]]` の `url` を書き換えます。

### ファイアウォール

Debian は標準ではポートを閉じません。`ufw` を使っている場合は、ポートを開けます。

```sh
sudo ufw allow 8790/tcp
```

### 指紋をもう一度確認する

Debian では次のコマンドで表示できます。

```sh
sudo runuser -u koyomi -- env -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u XDG_RUNTIME_DIR HOME=/var/lib/koyomi /usr/local/bin/koyomi http fingerprint
```

Windows では `koyomi remote peek https://<アドレス>:<ポート>` で、相手が見せている指紋を表示できます。

## 利用者と役割

Windows の画面から接続する人ごとに、`[[http.user]]` で名前・役割・token を決めます。役割によって、その人ができる操作が分かれます。

### 利用者の名前

- `name` は koyomi の中だけで使う名前です。Linux の利用者とは関係がなく、Debian に同じ名前の利用者を作る必要はありません。
- 名前は自由に決められます。ただし、空白を含む名前と `default` は使えません。ほかの `[[http.user]]` と同じ名前も使えません。
- 名前は、その人が行った操作と一緒に操作の記録 (`koyomi ops`) に残ります。
- ジョブは、誰が実行しても Debian の利用者 `koyomi` の権限で動きます (Linux では `run_as` を使えません)。

### 役割 (`role`)

上の役割は、下の役割ができることを全部できます。

- `view`: 見るだけです。状態、実行の一覧、履歴、ジョブのログ、操作の記録を見られます。定義の本文は見られません。
- `run`: `view` に加えて、ジョブネットの実行・停止・再実行・飛ばし、保留と解除、次回だけ飛ばす、などを操作できます。
- `edit`: `run` に加えて、定義の本文を読めます。定義を送ることもできますが、送った定義は `admin` が確定するまで動きません。
- `admin`: `edit` に加えて、定義の確定と保存、予定の一時停止と再開、カレンダーと祝日の変更、名前の変更、削除、全体の書き出しができます。確定もできるので、token を渡す相手と token の扱いに注意してください。

役割が分かれるのは、HTTPS で接続してきた人だけです。Debian の上で `koyomi` として行う操作 (`sudo -H -u koyomi koyomi …`) は、役割に関係なく全部できます。

### ジョブネットごとに絞る

- `nets`: 書いたジョブネットだけが見え、ほかは無いものとして扱います。
- `roles`: ジョブネットごとに役割を変えます。書いていないジョブネットは `role` の役割になります。
- どちらも `<グループ>/<名前>` の形で書き、`*` は任意の文字の並びです (`経理/*` は経理のすべて、`*/日次` はすべての日次)。
- `admin` には `nets` も `roles` も付けられません。`roles` の値にも `admin` は書けません。

```toml
[[http.user]]
name = "taro"
role = "view"                    # 下の roles に書いていないジョブネットは見るだけ
roles = { "経理/*" = "run" }      # 経理のジョブネットは実行もできる
token = "secret:http-taro"

[[http.user]]
name = "hanako"
role = "run"
nets = ["人事/*"]                 # 人事のジョブネットだけが見える
token = "secret:http-hanako"
```

### 追加する

1. token を登録します。値は長い乱数にします。次の例は乱数を作って、そのまま登録します。

   ```sh
   od -An -tx1 -N32 /dev/urandom | tr -d ' \n' | sudo koyomi-secret http-taro
   ```

   `koyomi-secret` は値を引数で受け取らず、標準入力から読み込みます (端末から入力したときは画面に表示されません)。koyomi が起動していれば再起動します (実行中のジョブは停止します)。

2. `koyomi.toml` に `[[http.user]]` を追加します。`token` の `secret:` の後ろには、手順 1 で登録した名前を書きます。保存すると、エンジンが数秒のうちに読み直して使い始めます (再起動は要りません)。

   ```sh
   sudo -H -u koyomi nano /var/lib/koyomi/.config/uchmk/koyomi/koyomi.toml
   ```

   `/var/lib/koyomi` の中は利用者 `koyomi` が書き換えられる場所なので、`install.sh` と同じく root ではなく `koyomi` として書き換えます。
3. 登録した token を `sudo cat /etc/koyomi/credentials/http-taro` で読み込み、その人に渡します。その人は Windows の画面で「接続先を追加する…」から接続します。

先に `koyomi.toml` に書いて token を登録していないと、エンジンは HTTPS の受け付けを始められず、ほかの人も接続できなくなります。token を先に登録してください。

役割や `nets`・`roles` だけを変えるときは、`koyomi.toml` を書き換えて保存するだけで反映されます。

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
