# SPSAで調整した係数をソースへ反映する先行例

SPSA（同時摂動確率近似法）の調整結果を、通常ビルドで使うソースの定数へ自動転記する先行例はある。
やねうら王の`tune.py apply`、Stockfish向け外部ツールのfishutils、およびRust製エンジンapeironの`spsa apply`で、結果の読み込みからソースファイルへの書き込みまでを確認した。
minaseが採用した「通常ビルドでは定数、調整用ビルドでは実行時に変更可能」という方式と、調整結果の自動転記は両立する。
以下は2026年9月22日に一次資料を確認した結果であり、外部実装はリンク先のコミットに固定している。

## ソースを書き換える3つの実例

### やねうら王

やねうら王のスクリプト集は、調整用ソースを作る`tune`と、調整結果を通常ソースへ適用する`apply`を別コマンドとして提供する。
手引きは、SPSAが更新した`.params`を使って`python tune.py apply param/suisho10.tune YaneuraOu/source`を実行すると、本番ソースが書き換わることを明記している。
調整用ソースの作成と最終値の反映を、別の操作として扱う実例である。[手引き](https://github.com/yaneurao/YaneuraOu-ScriptCollection/blob/a52094eb0c1f9a4932f825a67bbb6c1d2f13d173/SPSA/readme.md#L12-L32)、[applyの使用例](https://github.com/yaneurao/YaneuraOu-ScriptCollection/blob/a52094eb0c1f9a4932f825a67bbb6c1d2f13d173/SPSA/readme.md#L139-L149)。

実装では、`apply_parameters`が`.params`から値を読み、`.tune`で指定された周辺コードへ数値を埋め込む。
続いて`replace_context`が置換箇所の一致数を1件と確認し、対象ファイルへ書き戻す。
画面に最終値を表示するだけの機能ではなく、実際のソース更新である。[値の読み込みと置換](https://github.com/yaneurao/YaneuraOu-ScriptCollection/blob/a52094eb0c1f9a4932f825a67bbb6c1d2f13d173/SPSA/tune.py#L379-L446)、[ファイルへの書き込み](https://github.com/yaneurao/YaneuraOu-ScriptCollection/blob/a52094eb0c1f9a4932f825a67bbb6c1d2f13d173/SPSA/tune.py#L321-L337)。

### fishutils

fishutilsは、fishtestのSPSA結果を入力してStockfishのC++ソースを更新する外部ツールである。
手引きには、結果ファイルを指定する`python3 fishutils.py -s /path/to/stockfish/src/ -i /path/to/tuning_results.txt`と、変更を適用せず差分を見る`--dry-run`が記載されている。
リポジトリは2025年3月19日にアーカイブされているため、現在のStockfishへの互換性を主張する資料としてではなく、自動転記の先行例として参照する。[手引き](https://github.com/ianfab/fishutils/blob/8630f8b5fac19eb7fa9bd0656789a35a0fc64f71/README.md#L1-L25)、[リポジトリの状態](https://github.com/ianfab/fishutils)。

実装では、`process_spsa_match`が定義を探して調整値を丸め、ソース中の数値を置き換える。
`InputParser.process`が差分表示とファイル保存を切り替えるため、書き換え内容の確認と反映を分けられる。[数値の置換](https://github.com/ianfab/fishutils/blob/8630f8b5fac19eb7fa9bd0656789a35a0fc64f71/fishutils.py#L187-L255)、[差分表示と保存](https://github.com/ianfab/fishutils/blob/8630f8b5fac19eb7fa9bd0656789a35a0fc64f71/fishutils.py#L359-L377)。

### apeiron

Rust製のapeironは、SPSAの結果またはチェックポイントをJSONで読み、探索と評価の定数を更新する`apply --input <path>`を持つ。
`apply_selected_values`が係数名を定数名に対応させ、`apply_constants`が該当する`pub const`の初期値を置換して`fs::write`で保存する。
Rustのソースに定数を残したまま、調整後の転記を自動化する実例である。[コマンド定義](https://github.com/Naviary2/apeiron/blob/ee77baa6bd7e478a8c6d8c8501b65eccc3180a54/src/bin/spsa.rs#L139-L143)、[定数の書き換え](https://github.com/Naviary2/apeiron/blob/ee77baa6bd7e478a8c6d8c8501b65eccc3180a54/src/bin/spsa.rs#L1022-L1085)、[コマンドからの呼び出し](https://github.com/Naviary2/apeiron/blob/ee77baa6bd7e478a8c6d8c8501b65eccc3180a54/src/bin/spsa.rs#L1386-L1390)。

これら3例は、調整結果からソースを更新する操作を明示的なコマンドとして提供している。
ソースを更新する機能の存在を確認したものであり、調整終了時に無条件で採用する運用や、各実装の検証の十分性までを推奨するものではない。

## 既存計画書で参照したRust実装との違い

minaseの[SPSA計画書](../plans/spsa.md)は、調整中の係数の公開方式としてHobbes、akimbo、Reckless、およびViridithasを比較している。
その参照箇所で確認できる機能は次のとおりであり、ソースへの最終値の書き込みとは区別する必要がある。

| 実装 | 参照箇所で確認できる機能 |
|---|---|
| [Hobbes](https://github.com/kelseyde/hobbes-chess-engine/blob/1f3e4668c7fef92069ee916f9ed34e1e1d5c968c/src/tools/utils.rs#L3-L70) | マクロが通常版の定数読み出しと調整版の可変値を生成し、`print_params_ob`がOpenBenchへ渡す係数定義を表示する。 |
| [akimbo](https://github.com/jw1912/akimbo/blob/f7dd7677d6ab9a4a76e3769731084894a7f7ae09/src/util.rs#L37-L103) | Hobbesと同型のマクロを持ち、`print_params_ob`が係数定義を表示する。 |
| [Reckless](https://github.com/codedeliveryservice/Reckless/blob/31d9cd6fd2bea6d9f72eeb35e0bac70daa295fb1/src/parameters.rs#L1-L34) | マクロが通常版の定数関数と調整版の可変値を生成し、オプションを公開する。 |
| [Viridithas](https://github.com/cosmobobak/viridithas/blob/13a3fe18fbfde3856262850d599b6b8465ee7329/src/search/parameters.rs#L545-L575) | `emit_json_for_spsa`と`emit_csv_for_spsa`が係数定義を文字列として生成する。 |

この表は、参照したマクロや出力関数の機能を分類したものである。
各開発者が別途持つスクリプトの有無や、リポジトリ外の作業手順までは判定していない。

## 係数の定義形式と反映の手段

係数をJSON、YAML、TOMLなどのデータファイルで定義し、既存のパーサで読む方式を採る実装があるかを、2026年9月22日に同じ固定コミットで確認した。
結果は次のとおりであり、5つのRust製エンジンはいずれも係数の値をRustソースに置く。
「反映」の欄で「コミットを確認した」とあるものは、SPSAの結果をソースの数値の変更として取り込んだコミットの差分を確認したという意味であり、その差分が手作業で作られたか道具で作られたかは資料から判定できない。

| 実装 | 係数の値の定義場所 | 反映 |
|---|---|---|
| Hobbes | [`src/search/parameters.rs`](https://github.com/kelseyde/hobbes-chess-engine/blob/1f3e4668c7fef92069ee916f9ed34e1e1d5c968c/src/search/parameters.rs)の`tunable_params!`の呼び出し（`名前 = 値, 最小..=最大, 調整可否;`）。マクロの定義は`src/tools/utils.rs`。 | SPSA結果の[コミット9a6bb2a](https://github.com/kelseyde/hobbes-chess-engine/commit/9a6bb2a)が同ファイルの値だけを変えていることを確認した。固定コミットのファイル一覧に、名前にspsa、tune、applyを含むスクリプトはない。 |
| akimbo | [`src/search.rs`](https://github.com/jw1912/akimbo/blob/f7dd7677d6ab9a4a76e3769731084894a7f7ae09/src/search.rs)の`tunable_params!`の呼び出し。マクロの定義は[`src/util.rs`](https://github.com/jw1912/akimbo/blob/f7dd7677d6ab9a4a76e3769731084894a7f7ae09/src/util.rs)。 | 参照箇所では、`print_params_ob`がOpenBenchへ渡す係数定義を出力することだけを確認した。反映の手段は未確認。 |
| Reckless | [`src/parameters.rs`](https://github.com/codedeliveryservice/Reckless/blob/31d9cd6fd2bea6d9f72eeb35e0bac70daa295fb1/src/parameters.rs)は`define!`マクロを`#[allow(unused_macros)]`つきで定義するが、固定コミットの全`.rs`ファイルにその呼び出しはない。係数の値は`search.rs`、`movepick.rs`、`time.rs`、`evaluation.rs`の式の中の数値リテラルである。 | SPSA結果の[コミットf9390b4](https://github.com/codedeliveryservice/Reckless/commit/f9390b4)が上記4ファイルのリテラルを変えていることを確認した。固定コミットのファイル一覧に、名前にspsa、tune、applyを含むスクリプトはない。 |
| Viridithas | 既定値は`search.rs`などの定数（例 `RFP_MARGIN`）であり、[`src/search/parameters.rs`](https://github.com/cosmobobak/viridithas/blob/13a3fe18fbfde3856262850d599b6b8465ee7329/src/search/parameters.rs)の`Config`構造体がそれらを取り込み、2つのマクロでオプション名と対応づける。 | `emit_json_for_spsa`と`emit_csv_for_spsa`はOpenBenchへ渡す出力であり、参照箇所にJSONを読んで反映する機能はない。SPSA結果の[コミット59c995a](https://github.com/cosmobobak/viridithas/commit/59c995a)が`search.rs`などの定数を変えていることを確認した。`scripts/tune.sh`は自前の調整器の起動であり、反映の道具ではない。 |
| apeiron | [`src/search/params.rs`](https://github.com/Naviary2/apeiron/blob/ee77baa6bd7e478a8c6d8c8501b65eccc3180a54/src/search/params.rs)の`pub const DEFAULT_*`と、その定数を参照する仕様配列`TUNABLE_PARAM_SPECS`。 | 自動。`apply_constants`が`pub const 名前: 型 = 値;`の行を文字列で探して差し替える。JSONは調整結果の保存に加え、`set_search_params_from_json`による実行時の係数の読込みにも使う。 |

自動反映を持つやねうら王、fishutils、およびapeironの3例は、いずれもソース中の定数の行を独自の規則で探して数値を置き換える。
データファイルを係数の正にする実装は、参照した範囲では見つからなかった。
この結果は、[SPSAの調整結果をソースへ反映するコマンドの設計書](../plans/spsa-apply.md)がRustの表を正のまま保つ判断の根拠である。

## rustfmtの扱い

設計書の表の解析規則が`cargo fmt`と両立するかを、2026年9月22日にrustfmt 1.9.0-stable（88d9e12ae1）で確認した。
rustfmtは、`{}`で区切られたマクロ呼び出しの中身を`trim_left_preserve_layout`に通すだけで、宣言としては整形しない（[`src/macros.rs`](https://github.com/rust-lang/rustfmt/blob/master/src/macros.rs)の`Delimiter::Brace`の分岐、[`src/utils.rs`](https://github.com/rust-lang/rustfmt/blob/master/src/utils.rs)）。
この関数は各行を`trim`して字下げを作り直すので、バイト列は保たれない。

確認した挙動は次のとおりである。

- 現行の`src/search/alphabeta/params.rs`は`cargo fmt --check`を通る。既定値を6桁へ変えた写しと、宣言の行の空白と字下げを崩した写しに`rustfmt --edition 2024`をかけても変化しなかった。
- 標準入力から与えた最小の表では、先頭のタブが空白に、行末の空白が除去され、空白だけの行が空行になった。宣言の途中のタブは残った。`hard_tabs=true`では字下げがタブで作り直された。
- `format_macro_bodies=false`はこの挙動を変えない。同オプションの対象は`macro_rules!`の定義本体である。
- 開始行が本体より深く字下げされた表では、開始行だけが列0へ移り、終了行の字下げは残った。開始行と終了行の字下げの一致を規則にすると、この場合に整形後の表が規則から外れる。
- 外側の`#[rustfmt::skip]`は空白を保つが、`newline_style=Unix`の指定はCRLFをLFへ変換する。改行の変換は出力全体に対して行われる。

したがって設計書の規則は、字下げの量と一致を要求せず、行末の`\r`を無視し、宣言の行の内容だけを見る形にした。

## minaseへの示唆

minaseの現行手引きは、最終値を四捨五入して`src/search/alphabeta/params.rs`の既定値へ書き込み、コミットしてから棋力を検定する手順を定めている。
調査時点の`spsa_runner`は、終了時に`final`と`final_integer`を表示するが、この段階でソースは更新しない。
したがって、通常ビルドへ取り込むまでの操作が残っているという指摘には根拠がある。[現行手順](../guides/spsa.md#7-値の採否を判定する)、[終了時の出力](../../src/bin/spsa_runner.rs)。

既存計画書が棄却したのは、調整用ビルドを作るために探索ソースへ一時パッチを当てる方式である。
マクロとfeatureで調整用ビルドを作る判断は、保存された結果から通常ビルドの既定値を更新するコマンドを設けるかどうかとは別の判断になる。
実際、同計画書が参照したやねうら王のツール自体が、最終値を反映する`apply`を提供している。[計画書の方式比較](../plans/spsa.md)、[やねうら王のコマンド分岐](https://github.com/yaneurao/YaneuraOu-ScriptCollection/blob/a52094eb0c1f9a4932f825a67bbb6c1d2f13d173/SPSA/tune.py#L635-L639)。
現行計画は、最終値を定数へ反映して検定する手順を定めているが、その転記を自動化する手段までは設計していない。

以上から、minaseでは保存記録を検証して最終値を取り出し、`src/search/alphabeta/params.rs`の既定値だけを更新する独立した`apply`コマンドを追加する方針が考えられる。
これは先行例と現行構成から導く提案であり、まだ実装された機能ではない。
候補のコミットを作ってから[既存の棋力検定](../guides/sprt.md)にかける手順は、そのまま保てる。
