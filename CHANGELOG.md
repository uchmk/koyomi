# 変更ログ

形式は [Keep a Changelog](https://keepachangelog.com/ja/1.1.0/)、日付は JST。

## [Unreleased]

### 追加

- 定義を読んで確かめる: `jobnets/<グループ>/<名前>.toml` を読み、誤りは行番号つきで示す。確かめた定義は中身全体の hash で
  `approved.toml` に覚え、1 字でも変わればまた未確認になる。窓の確かめの箱と `koyomi check` / `list` / `approve` / `engine`。
  他のユーザーが書ける場所があれば帯で知らせ、直るまで確かめない。

## [0.1.0] - 2026-10-29

### 追加

- ito の `templates/app` から作った（窓・テーマ・言語・倍率・F12 の不具合報告・`koyomi.com`）。
