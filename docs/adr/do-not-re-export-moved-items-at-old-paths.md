# 移動した項目を旧パスで再公開しない

## ステータス

承認済み。決定日は2026年9月26日。

## コンテキスト

2026年9月26日から10月4日にかけての4つの作業は、多くの型と関数を別のモジュールやcrateへ移した。
[探索部と評価関数のモジュール再編](../plans/search-eval-layout.md)、[中核モジュールの再編](../plans/core-layout.md)、[実行ファイルと対局ハーネスのモジュール再編](../plans/bin-harness-layout.md)、および[ライブラリとエンジンのcrate分割](../plans/crate-split.md)である。
移した項目の利用者は、このリポジトリの中の実行ファイルと試験だけだった。
兄弟リポジトリの開発用GUI`minase-gui`とlishogiのBotの配備`minase-lishogi-bot`は、minaseのライブラリを参照せず、実行ファイルだけを使う。

## 決定

項目を別のモジュールやcrateへ移すときは、呼び出し側の`use`をすべて新しいパスへ書き換え、旧パスに`pub use`の別名を残さない。
エンジンはライブラリの型を`minase_core::`のパスで参照し、エンジンのルートから再公開しない。
ディレクトリの`mod.rs`が自分の子の項目を`pub use`で公開すること（`minase::harness::X`や`minase_core::rules::parse_rule_set`など）は、旧パスの互換ではなくモジュールの公開面そのものであり、この決定の対象外とする。

## 検討した代案

旧パスを`pub use`で残す案は、書き換えで足りる利用者のために互換の層を残し、利用者がどちらのパスやcrateの型を使うべきかを曖昧にするので採らなかった。

## 結果

公開パスが変わる移動は、互換性を壊す変更としてtypeの直後に`!`を付け、本文に`BREAKING CHANGE:`フッターを書いてコミットする（CONTRIBUTING.mdの「コミットとリリース」）。
crate分割では、エンジンの公開APIから`minase::Position`などが消えたので、エンジンの主版数を上げた（[crateごとに版数を付けてリリースする](release-each-crate-separately-and-publish-by-hand.md)）。
外部の利用者が旧パスを使っていた場合は、移動のたびに書き換えが要る。
