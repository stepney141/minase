# リポジトリのルートを仮想マニフェストにし、crateを`crates/`の下に並べる

## ステータス

承認済み。決定日は2026年10月4日。

## コンテキスト

[ライブラリとエンジンのcrate分割](../plans/crate-split.md)は、1つだったcrateをライブラリとエンジンに分け、それぞれに別のライセンスを与えた（[ライブラリをMIT、エンジンをGPL-3.0-or-laterで配布する](license-library-under-mit-and-engine-under-gpl.md)）。
crateの配布物には、そのcrateのディレクトリより外のファイルを含められない。
対局ハーネスの`commit:`指定と`scripts/bench_compare.py`は他のコミットを`target/`の下へ展開してビルドし、他のブランチのworktreeは`data/`と`.claude/`の下に置かれる。
分割前のコミットのCargo.tomlは`[workspace]`を持たないので、Cargoは親ディレクトリのワークスペースを見つけると、それらのビルドを拒否する。

## 決定

リポジトリのルートのCargo.tomlは`[workspace]`だけを持つ仮想マニフェストとし、各crateを`crates/<名前>/`に置く。
ワークスペースには`resolver = "3"`を明記し、`target`、`data`、および`.claude`をメンバーの探索から除く。

## 検討した代案

エンジンをルートのパッケージとし、ライブラリだけを`crates/minase-core/`に置く案は採らなかった。
この案では、エンジンのパッケージのディレクトリがライブラリと`docs/`を内側に含むので、MITのディレクトリがGPLのパッケージの中に入れ子になり、配布物の範囲も除外の列挙に頼ることになる。
rshogiも、crateを`crates/`の下に並べる仮想マニフェストをとる（[crate分割の参考資料](../plans/crate-split.md#参考資料)）。

## 結果

crateのディレクトリがそのままライセンスの境界になり、各crateの配布物はそのディレクトリの中で完結する。
このため、エンジンが埋め込む重みの`nets/`はエンジンのcrateの中に置く。
仮想マニフェストではメンバーのeditionからresolverが推定されず、省略すると旧resolverが開発依存のフィーチャを通常のビルドへ統合する。
その場合、`cargo build`のバイナリには整合検査を入れないという契約が崩れる（[重い整合検査をフィーチャ`invariants`に置く](enable-invariant-checks-in-tests-by-feature.md)）。
`target/`はルートに作られるので、`target/release/minase`のパスと`cargo build --release --bin minase`の標準コマンドは分割の前後で変わらない。
Cargoはメンバーの`[profile]`を無視するので、プロファイルはルートに置き、`cargo install`で使われるエンジンのマニフェストにも同じ`[profile.release]`を写している。
