# crateごとに版数を付けてリリースし、crates.ioへの公開は手で行う

## ステータス

承認済み。決定日は2026年10月4日。

## コンテキスト

[ライブラリとエンジンのcrate分割](../plans/archived/crate-split.md)の後、リポジトリはライブラリ`minase-core`とエンジン`minase`の2つのcrateを持つ。
分割の前のエンジンは`vX.Y.Z`のタグの系列でリリースしてきた。
ライブラリの公開パスは、2026年9月27日の[中核モジュールの再編](../plans/archived/core-layout.md)で変わったばかりだった。
リリースに使うcargo-releaseは、ルート以外に置いたcrateのタグに`<crate名>-`の接頭辞を既定で付け、複数のcrateのリリースを既定で1つのコミットにまとめる。
crates.ioへの公開は取り消せない。

## 決定

2つのcrateは別々の版数を持ち、cargo-releaseを1つのcrateずつ実行してリリースする。
タグはライブラリが`minase-core-vX.Y.Z`、エンジンが従来の系列を引き継ぐ`vX.Y.Z`とし、各crateの変更履歴にはそのcrateのディレクトリ以下を変更したコミットだけを載せる。
cargo-releaseはpushとcrates.ioへの公開を行わず、公開はタグとpushを確かめた後に`cargo publish`で手動で行う（手順はCONTRIBUTING.mdの「リリースの手順」にある）。

## 検討した代案

cargo-releaseに公開まで任せる案は、取り消せない操作を自動化することになるので採らなかった。
これは、cargo-releaseにpushをさせず手作業に残している従来の方針と同じ理由である。
複数のcrateのリリースを1つのコミットにまとめるcargo-releaseの既定は、コミットの件名に版数を使えず、crateごとの件名の設定も効かないので採らなかった。
rshogiも、エンジン全体の`vX.Y.Z`のタグとは別に、ライブラリを0.x系の別系列で公開する（[crate分割の参考資料](../plans/archived/crate-split.md#参考資料)）。

## 結果

エンジンの分割後の最初のリリースは2.0.0とした。
ライブラリの型の公開パスがすべて`minase::`から`minase_core::`へ移り、エンジンの公開APIとの互換性が失われたからである。
ライブラリは、APIが安定していないことを示すために0.1.0から始めた。
エンジンは公開時に`path`が除かれてcrates.io上のライブラリの版数で依存を解決するので、ライブラリを先に公開する必要がある。
両方のcrateに触れたコミットは、両方の変更履歴に載る。
後に加わった学習器のcrate`minase-trainer`は`publish = false`であり、タグを付けたリリースの対象にしていない。
