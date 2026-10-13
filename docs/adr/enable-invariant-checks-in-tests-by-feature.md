# 重い整合検査をフィーチャ`invariants`に置き、`cargo test`では常に有効にする

## ステータス

承認済み。決定日は2026年9月27日。

## コンテキスト

`Position::validate`は、盤の全升を走査して局面の不変条件を検査する。
[合法手生成の設計書](../plans/movegen.md)の5.1節は、着手の適用と取り消しの末尾にあった`validate`の`debug_assert!`を外した。
`opt-level = 3`でも`debug_assertions`が有効な`[profile.dev]`で動く`cargo test`と、デバッグビルドの深いperftが実用にならなかったからである。
[デバッグ機能の整備](../plans/debugging-tools.md)は、着手のたびの局面の全検査と、探索中の評価のたびに差分更新した中間値を全計算と照合する検査を加えることにした。
起案時は検査を既定で無効にする予定だったが、実測では検査を有効にした全試験の所要時間の増加は約1割にとどまった（同設計書の「利用者の決定」の2）。

## 決定

重い整合検査は、既定のフィーチャに含めないCargoフィーチャ`invariants`の下に置く。
各crateは自分自身を`invariants`付きで開発依存に加え、`cargo test`では検査を常に有効にする。
検査の失敗は実装の誤りなので、診断を含むメッセージでパニックし、内容は`Position::assert_invariants`などのdocコメントにある。

## 検討した代案

着手の適用と取り消しの`debug_assert!`へ戻す案は、`cargo test`とデバッグビルドのperftを再び遅くするので採らなかった。
`debug_assertions`で検査を有効にする案は、`cargo test`だけでなく`cargo run`などの開発プロファイルのビルドにも検査が入るので採らなかった。
`cargo test`でも検査を既定で無効にする起案時の案は、上記の実測を受けてエンジンの開発者が退けた。

## 結果

Cargoは開発依存のフィーチャを、試験のように開発依存を使うターゲットのビルドにだけ統合するので、`cargo build`と`cargo run`のバイナリには検査が入らず、出荷するビルドの速度は変わらない。
この契約は、ワークスペースが`resolver = "3"`であることに依存する（[ルートを仮想マニフェストにする](arrange-crates-under-a-virtual-workspace.md)）。
エンジンの`invariants`はライブラリの`invariants`も有効にする。
crates.ioへの公開時には開発依存が除かれるので、公開されたcrateで`cargo test`を実行しても検査は有効にならない。
パニックで止めることは、外部入力の誤りをエラーで返す規則（[教訓](../lessons/no-panic-on-external-input.md)）に反しない。
この規則は外部入力の経路に関するものであり、実装の不変条件の検査には当たらないからである。
