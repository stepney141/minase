# xboardの`-autoflag`は値を取らない

## 症状

xboardの自動対局で時間切れ検出を有効にしようと`-autoflag true`と書いたところ、xboardはエンジンを起動せず、エンジンなしのモードで開いた。

## 原因

xboard 4.9.1の`-autoflag`は値を取らないArgTrue型のオプションであり、続く`true`は位置引数（ICSホスト名）として解釈される（xboardのargs.hで確認した）。
ホスト名を受け取ったxboardは、指定したエンジンではなくICS接続の構成で起動する。

## 以後の規則

xboardで時間切れ検出を有効にするときは、ArgBoolean型の`-autoCallFlag true`を書き、`-autoflag`に値を続けない。

## 出典

- [plans/engine-connectivity.md](../plans/engine-connectivity.md) の「検証」節にある端到端検証の手順。
