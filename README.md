# koyomi

Small job scheduler and monitor (a light JP1)

uchmk のアプリ。共通の部品は [ito](https://github.com/uchmk/ito) を使う。

エラーや警告が出たときは [こんな時はどうする](docs/troubleshooting.md) を、メッセージの先頭の ID（`KOYOMI-E2101` など）で検索してください。

    cargo run

## 画面

![今日のタブ: 今日の実行と予定の表（正常終了・実行中・待ちと、異常終了した売上レポート）、その下のガントチャート、右に選んだジョブネットの次の予定と構成](docs/images/today.png)

![画面を回る動き: 今日の表で失敗した実行を選び、月次締めのジョブネットの流れ、会社のカレンダーの営業日と休み、履歴で失敗したジョブのログを見て、対応済みにする](docs/images/koyomi.gif)

![履歴のタブ: 過去 7 日の実行の一覧と、選んだ失敗した実行のジョブの流れ・状態・終了コード、失敗したジョブのログ](docs/images/history.png)

画像の撮り方は [docs/images/README.md](docs/images/README.md) にあります。

## Debian でエンジンを動かす

エンジンを Debian 12 以降で systemd のサービスとして動かし、Windows の画面から HTTPS で接続できます。

1. [Releases](https://github.com/uchmk/koyomi/releases) から `koyomi_<版>_amd64.deb` (または `koyomi-<版>-linux-x86_64.tar.gz`) を Debian に置きます。
2. `sudo apt install ./koyomi_<版>_amd64.deb` を実行します (tar.gz は展開して `sudo ./install.sh`)。最後に、接続する URL、証明書の指紋、token の読み込み方法が表示されます。
3. Windows の画面で、状態バーの接続先 →「接続先を管理…」→「接続先を追加する…」から、URL と token を入力します。

利用者ごとに役割（`view`・`run`・`edit`・`admin`）を決めて、できる操作を分けられます。利用者の名前は koyomi の中だけの名前で、Linux の利用者とは関係ありません。

詳しい手順（利用者と役割、ファイルの場所、更新、削除、制約）は [koyomi エンジンを Debian にインストールする](packaging/linux/README.md) にあります。
