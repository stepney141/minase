# リリース設定は使い捨てのクローンで本番を実行して確かめる

## 症状

1つのcrateだったminaseをワークスペースの2つのcrateへ分けたとき、release.toml のコミットの件名を`chore(release): {{tag_name}}`とし、ライブラリの件名をパッケージごとの設定で上書きした。
`cargo release`の予行は、版数とリリースノートを正しく表示し、件名については「Unrendered {{tag_name}}」という警告を1行出すだけだった。
使い捨てのクローンで`--execute`を付けて実行すると、リリースのコミットの件名は`chore(release): v{{version}}`のまま展開されず、パッケージごとの件名の設定も使われていなかった。

## 原因

cargo-release は、複数のcrateを持つワークスペースでは既定でリリースのコミットを1つにまとめる（`consolidate-commits = true`）。
まとめたコミットではワークスペースの件名だけが使われ、`{{version}}`や`{{crate_name}}`のようにcrateごとに決まる変数は展開されない。
予行はコミットもタグも作らないので、件名やタグがどうなるかを結果として見せず、警告を見落とせば設定の誤りは本番のリリースで初めて表に出る。

## 以後の規則

release.toml または`[package.metadata.release]`を変更したら、リポジトリを一時ディレクトリへクローンしてリモートを外し、`cargo release ... --execute --no-confirm`を実際に走らせて、コミットの件名、タグの名前と本文、および変更履歴の差分を確かめてから採用する。

## 出典

- [plans/crate-split.md](../plans/crate-split.md) の「リリース手順と変更履歴」
