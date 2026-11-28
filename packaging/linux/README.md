# koyomi エンジンを Debian にインストールする

koyomi のエンジンを Debian で systemd のサービスとして実行し、Windows の画面から HTTPS で接続する手順です。

## 必要なもの

- systemd で動いている Debian 12 以降。GUI のライブラリは必要ありません。`LoadCredential` にフォルダーを渡す使い方が必要です (Debian 11 の systemd 247 のマニュアルにはありません)。secret の暗号化が有効になる前は、`install.sh` が実際に試して確認し、読み込めない場合はそこで停止します。
- root の権限 (`sudo`) が必要です。
- token などの secret は、可能なら systemd の `systemd-creds` で、このマシンの鍵を使って暗号化して保存します。暗号化を有効にできないとき (systemd が古いなど) は、インストールがそう表示し、これまでどおり root だけが読めるファイルに平文で保存します。詳しくは「[secret の暗号化](#secret-の暗号化)」にあります。
- Windows の PC からエンジンのポートに TCP で届く必要があります。

## 入手

[Releases](https://github.com/uchmk/koyomi/releases) から、次のどちらかをダウンロードします。中身は同じです。

- `koyomi_<版>_amd64.deb` (ARM の機械は `koyomi_<版>_arm64.deb`): `apt` で入れるパッケージです。更新と削除も `apt` で行います。
- `koyomi-debian-<版>-amd64.tar.gz` (ARM の機械は `koyomi-debian-<版>-arm64.tar.gz`): 展開して `install.sh` で入れます。

Windows でダウンロードした場合は、PowerShell から Debian に送れます。

```powershell
scp .\koyomi_<版>_amd64.deb <Debian の利用者>@<Debian のアドレス>:
```

## インストール

### .deb で入れる

```sh
sudo apt install ./koyomi_<版>_amd64.deb                  # ポートは 8790
sudo KOYOMI_PORT=9000 apt install ./koyomi_<版>_amd64.deb  # 初回にポートを変えるとき
```

`./` を付けて、ファイルを指定します。実行ファイルを `/usr/bin`、`koyomi-secret` と `koyomi-user` を `/usr/sbin` に置き、利用者 `koyomi`、`admin` の token、最初の `koyomi.toml` を用意して、サービスを有効にして起動します。

### tar.gz で入れる

```sh
tar xzf koyomi-debian-<版>-amd64.tar.gz     # ARM の機械は amd64 を arm64 に
cd koyomi-debian-<版>-amd64                 # ここも同じ
sudo ./install.sh            # ポートは 8790
sudo ./install.sh --port 9000   # 初回にポートを変えるとき
```

`install.sh` は、利用者 `koyomi` を作成し、実行ファイルとサービスを配置し、`admin` の token を作成して、エンジンを起動します。何度実行しても問題ありません。

### インストールの最後に表示されるもの

どちらの方法でも、最後に、接続する URL (`https://<アドレス>:<ポート>`)、証明書の指紋、token の読み込み方法が表示されます。ポートの指定 (`KOYOMI_PORT`、`--port`) は最初に `koyomi.toml` を作成するときだけ有効です。

.deb と tar.gz は同じ機械に同時には入れられません。もう一方で入れた koyomi が残っていると、インストールは止まります。切り替える手順は「[.deb と tar.gz を切り替える](#deb-と-targz-を切り替える)」にあります。

## Windows の画面から接続する

1. 画面の状態バーの接続先を押し、一番下の「接続先を管理…」から「接続先を追加する…」を開きます (コマンドパレットの「別の PC のエンジンに接続する」でも開けます)。名前と URL (インストールの最後に表示されたもの) を入力します。
2. 画面に表示された指紋が、インストールの最後に表示された指紋と同じか確認し、「同じでした」を押します。異なる場合は接続しないでください。
3. `admin` の token を入力して「接続する」を押します。token は Debian で `sudo koyomi-secret --show http-admin` を実行すると読み込めます。

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
sudo runuser -u koyomi -- env -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u XDG_DATA_HOME -u XDG_RUNTIME_DIR HOME=/var/lib/koyomi koyomi http fingerprint
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

```sh
sudo koyomi-user add taro --role run --nets '経理/*'
```

- 新しい token を作り、1 回だけ表示します。その場で控えて、その人に渡してください。`koyomi.toml` には token の SHA-256 (`token = "sha256:…"`) だけが書かれ、token の値はどこにも残りません。
- 引数を付けずに `sudo koyomi-user add` を実行すると、名前、役割 (番号で選びます)、見せるジョブネットを絞るかを順に尋ねます。最後に `koyomi.toml` に追加する内容を表示し、確認してから追加します。
- 数秒以内に使えるようになります。再起動は要りません。
- 渡された人は、Windows の画面の「接続先を追加する…」から接続します。
- `koyomi.toml` に `[http]` がないと追加できません (`install.sh` が作る `koyomi.toml` には `[http]` があります)。

### 確認・変更・削除する

```sh
sudo koyomi-user list                                   # 名前、役割、ジョブネット、token の状態 (token の値は表示しません)
sudo koyomi-user role taro view --roles '経理/*=run'     # 役割を変える
sudo koyomi-user reset taro                             # token を作り直す (古い token は数秒以内に使えなくなります)
sudo koyomi-user remove taro                            # 削除する
```

- `koyomi-user` は koyomi のサービスに頼んで `koyomi.toml` を書き換えるので、サービスが動いている必要があります。ほかの行とコメントはそのまま残ります。
- 最後の `admin` を `admin` から外すことと、削除することはできません。
- token をなくしたときは `reset` で作り直します。その人は、表示された token を Windows で `koyomi secret set <名前>` を実行して保存し直します (`<名前>` は、Windows の `koyomi.toml` の `[[remote]]` にある `token = "secret:<名前>"` の名前です)。
- Windows の画面を `admin` の token でこのエンジンに接続しているときは、設定 → 利用者 のページでも同じこと (一覧・追加・役割の変更・token の作り直し・削除) ができます。作った token は画面に 1 回だけ表示されます。画面が接続に使っている利用者は、このページでは変更できません。

### koyomi.toml を直接書き換える

`koyomi.toml` を直接書き換えることもできます。その場合は token を `koyomi-secret` で登録し、`token = "secret:<名前>"` と書きます。

1. token を登録します。値は長い乱数にします。次の例は乱数を作って、そのまま登録します。

   ```sh
   od -An -tx1 -N32 /dev/urandom | tr -d ' \n' | sudo koyomi-secret http-taro
   ```

   `koyomi-secret` は値を引数で受け取らず、標準入力から読み込みます (端末から入力したときは画面に表示されません)。koyomi が起動していれば再起動します (実行中のジョブは停止します)。暗号化が有効なときは、値を暗号化して保存します。

2. `koyomi.toml` に `[[http.user]]` を追加します。`token` の `secret:` の後ろには、手順 1 で登録した名前を書きます。保存すると、エンジンが数秒のうちに読み直して使い始めます (再起動は要りません)。

   ```sh
   sudo -H -u koyomi nano /var/lib/koyomi/.config/uchmk/koyomi/koyomi.toml
   ```

   `/var/lib/koyomi` の中は利用者 `koyomi` が書き換えられる場所なので、`install.sh` と同じく root ではなく `koyomi` として書き換えます。
3. 登録した token を `sudo koyomi-secret --show http-taro` で読み込み、その人に渡します。

先に `koyomi.toml` に書いて token を登録していないと、エンジンは HTTPS の受け付けを始められず、ほかの人も接続できなくなります。token を先に登録してください。

## ファイルの場所

| 内容 | 場所 |
| --- | --- |
| 設定 (`koyomi.toml`、`jobnets/`、`calendars/`、`tls/`) | `/var/lib/koyomi/.config/uchmk/koyomi/` |
| 状態 | `/var/lib/koyomi/.local/state/uchmk/koyomi/` |
| シークレット (token など) | `/etc/koyomi/credstore/` (暗号化。root だけが読める)。暗号化が有効になる前は `/etc/koyomi/credentials/` (平文。root だけが読める) |
| 暗号化の状態 | `/etc/koyomi/credstore.state` |
| 暗号化の前に使っていた平文のファイルのうち、使われていないもの | `/etc/koyomi/credentials.unused/` |
| 暗号化した secret を使うための設定 | `/etc/systemd/system/koyomi.service.d/50-koyomi-credstore.conf` |
| 暗号化の鍵 | `/var/lib/systemd/credential.secret` |
| 実行ファイル (.deb) | `/usr/bin/koyomi`、`/usr/sbin/koyomi-secret`、`/usr/sbin/koyomi-user` |
| 実行ファイル (tar.gz) | `/usr/local/bin/koyomi`、`/usr/local/sbin/koyomi-secret`、`/usr/local/sbin/koyomi-user` |
| サービス (.deb) | `/usr/lib/systemd/system/koyomi.service` |
| サービス (tar.gz) | `/etc/systemd/system/koyomi.service` |

## secret の暗号化

Debian 13 (systemd 257) など、`systemd-creds` が使えるマシンでは、secret (token など) を、このマシンの鍵で暗号化して保存します。インストールが自動で有効にします。

### インストールと更新で起きること

.deb でも tar.gz でも、インストールや更新のたびに、次のことが起きます。

- `/etc/koyomi/credentials` の平文のファイルを暗号化して、`/etc/koyomi/credstore` に置きます。
- 暗号化した secret で koyomi を起動します。
- koyomi が暗号化した secret で動いているのを確認してから、はじめて平文のファイルを削除します。削除した名前と、使われていないため `/etc/koyomi/credentials.unused/` に移した名前が表示されます。これらの値は平文で保存されていたので、値を変更することをお勧めします。
- 確認できなかったときは、平文のファイルに戻し、そう表示します。もう一度インストールを実行すると、やり直します。
- koyomi が起動しなかったとき (サービスを無効にしている、など) は、切り替えは途中 (`--state` が `pending`) のままで、平文のファイルも残ります。koyomi を起動したあとで `sudo koyomi-secret --finish` を実行すると、確認して切り替えを終えます。
- 暗号化を有効にできないとき (systemd が古い、systemd が動いていないなど) は、`注意:` と理由が表示され、これまでどおり平文のファイルで動きます。

### 守られるもの

- 守られるもの: `/etc/koyomi/credstore` のコピー (バックアップやフォルダーのコピー) は、鍵 `/var/lib/systemd/credential.secret` がなければ読めません。
- 守られないもの: 平文のファイル (暗号化が有効になる前と切り替えの途中の `/etc/koyomi/credentials`、`/etc/koyomi/credentials.unused`)、このマシンの root、ディスク全体のコピー (鍵が同じディスクにあるため)、ジョブ (これまでどおり `koyomi` の権限で動きます)。

### バックアップ

`/etc/koyomi` のバックアップとは別の場所に、`/var/lib/systemd/credential.secret` のバックアップを取ってください。secret を読み直すには、両方が必要です (別のマシンでも同じです)。

### コマンド

```sh
sudo koyomi-secret --check          # 暗号化したフォルダーを確認する (値は表示しません)
sudo koyomi-secret --show <名前>     # 値を表示する
sudo koyomi-secret --state          # off / pending / on のどれかを表示する
```

### 使われていない平文のファイル

`/etc/koyomi/credentials.unused/` には、暗号化が有効になったあとで見つかった平文のファイル (古い版が書いたもの、手で置いたもの) があります。これらは使われません。値が必要なときは `koyomi-secret` で登録し直してから、ファイルを削除してください。

### 設定ファイル (drop-in)

`/etc/systemd/system/koyomi.service.d/50-koyomi-credstore.conf` が、サービスを暗号化したフォルダーに切り替えます。削除しないでください。`systemctl revert koyomi` で削除されたときは、`sudo koyomi-secret --prepare` で戻します (戻したあとは `sudo systemctl restart koyomi` が必要です)。

### 復旧

- koyomi が起動せず、`journalctl -u koyomi` に `status=243/CREDENTIALS` と出るとき: `sudo koyomi-secret --check` が、問題のファイルの名前を表示します。バックアップから鍵を戻すか、そのファイルを `/etc/koyomi/credstore` から別の場所に移して、値を `koyomi-secret` で登録し直します。そのあと `sudo systemctl restart koyomi` を実行します。
- 鍵のバックアップがないとき: `/etc/koyomi/credstore` を別の場所に移し、drop-in と `/etc/koyomi/credstore.state` を削除して、`sudo systemctl daemon-reload` を実行します。そのあとインストールをもう一度実行します (新しい admin の token が作られます)。ほかの secret は登録し直します。
- 暗号化が有効になったあとで、古い版に戻すことはできません。古い版の `koyomi-secret` と `install.sh` は、使われない平文のファイルを書いてしまいます。

## 状態を確認する

```sh
systemctl status koyomi
journalctl -u koyomi
sudo -H -u koyomi koyomi status
```

## 更新

- .deb: 新しい .deb を `sudo apt install ./koyomi_<新しい版>_amd64.deb` で入れます。
- tar.gz: 新しい tar.gz を展開して、もう一度 `sudo ./install.sh` を実行します。

どちらも、サービスを再起動するので、実行中のジョブは停止します。設定と token は変更されません。この版に最初に更新したときに、secret の暗号化が有効になります (「[secret の暗号化](#secret-の暗号化)」を見てください)。`systemctl disable` で無効にしたサービスは、無効のまま起動しません。そのときは、暗号化への切り替えは koyomi を起動したあとの `sudo koyomi-secret --finish` で終わります。

## 停止する

```sh
sudo systemctl stop koyomi
```

`koyomi engine stop` で停止すると、`Restart=on-failure` でも自動では再起動しないので、`sudo systemctl start koyomi` で起動してください。

## 削除する

```sh
sudo apt purge koyomi          # .deb で入れたとき
sudo ./install.sh --uninstall  # tar.gz で入れたとき (展開したフォルダーで)
```

サービスを止め、実行ファイルとサービスを削除します。`/var/lib/koyomi` (設定と状態)、`/etc/koyomi` (シークレット)、利用者 `koyomi`、暗号化のための設定 (`/etc/systemd/system/koyomi.service.d/50-koyomi-credstore.conf`) は、`apt purge` でも残ります。これらを削除するコマンドは最後に表示されます (設定のファイルも削除するコマンドです。自動では実行しません)。

`sudo apt remove koyomi` でも削除できます。その場合は、サービスが止まったまま無効になります (`systemctl` では masked と表示されます)。もう一度 .deb を入れると元に戻ります。

## .deb と tar.gz を切り替える

設定、状態、token はそのまま引き継がれます。

- tar.gz から .deb へ: 展開したフォルダーで `sudo ./install.sh --uninstall` を実行してから、`sudo apt install ./koyomi_<版>_amd64.deb` を実行します。
- .deb から tar.gz へ: `sudo apt purge koyomi` を実行してから、`sudo ./install.sh` を実行します。`apt remove` だけで止めていると、`install.sh` は止まり、`sudo dpkg -P koyomi` を案内します (後から purge すると、`install.sh` が有効にしたサービスが無効に戻ってしまうためです)。

## 第 1 段階の制約

- `koyomi.toml` は、Debian 側で修正するか、admin の token でつないだ画面の「設定 → 一般 → 接続先の koyomi.toml を直す…」で修正します。画面からは、`[http]` の port・lan・ipv6・tailnet と、自分自身の `[[http.user]]` は変更できません (つながらなくなるため)。これらは Debian 側で修正してください。
- 画面の「今日」は、エンジンの日付で決めます。時刻帯が画面を開く PC と違えば、画面の帯で知らせます。状態バーの時計と書き出すファイルの名前は、画面を開く PC の時刻です。
- エンジンの通知 (`[notify]` の `system` か `sound` に挙げた出来事) は、画面を開いているあいだ、`[[remote]]` のすべての PC について、edit 以上でつないでいれば、画面がどの PC を映していても、画面を開く PC のシステム通知として出します (題の先頭に PC の名前が付きます。押すとその PC に切り替えて開きます)。画面を閉じているあいだの通知は、あとから出しません (Webhook とメールはエンジンが送ります)。PC のスリープや接続の切れ目のあとで、一度に 3 件以上届いたときは、件数と最後の通知を書いた 1 件にまとめて出します (「開く」で今日の画面を開きます)。数えるのは `system` か `sound` に挙げた出来事の通知だけです。画面は、PC ごとに見張りを止めること（「接続先を管理」の `見張る`、または koyomi.toml の `[[remote]]` の `watch = false`）も、すべて止めること（設定 → 通知、`watch_remotes = false`）もできます。
- Linux で動作しない機能があります: `pwsh` が必要な後始末、`run_as`、イベントログ。画面に注意として表示されます。
