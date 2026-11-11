# koyomi

Small job scheduler and monitor (a light JP1)

uchmk のアプリ。共通の部品は [ito](https://github.com/uchmk/ito) を使う。

エラーや警告が出たときは [こんな時はどうする](docs/troubleshooting.md) を、メッセージの先頭の ID（`KOYOMI-E2101` など）で検索してください。

    cargo run

## Debian でエンジンを動かす

エンジンを Debian 12 以降で systemd のサービスとして動かし、Windows の画面から HTTPS で接続できます。

1. [Releases](https://github.com/uchmk/koyomi/releases) から `koyomi_<版>_amd64.deb` (または `koyomi-<版>-linux-x86_64.tar.gz`) を Debian に置きます。
2. `sudo apt install ./koyomi_<版>_amd64.deb` を実行します (tar.gz は展開して `sudo ./install.sh`)。最後に、接続する URL、証明書の指紋、token の読み込み方法が表示されます。
3. Windows の画面で、状態バーの接続先 →「接続先を管理…」→「接続先を追加する…」から、URL と token を入力します。

利用者ごとに役割（`view`・`run`・`edit`・`admin`）を決めて、できる操作を分けられます。利用者の名前は koyomi の中だけの名前で、Linux の利用者とは関係ありません。

詳しい手順（利用者と役割、ファイルの場所、更新、削除、制約）は [koyomi エンジンを Debian にインストールする](packaging/linux/README.md) にあります。
