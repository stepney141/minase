# move-ordering-baseline-bench

## 目的

[手の順序付けの改善](../plans/move-ordering.md)の起案時点の探索統計を、現行masterの`bench`で深さ5と8について記録し、通常探索のβカットのうち最初に探索した手によるものの割合（初手βカット率）と、全手を走査したノードでの最善手の探索順位の分布を得る。

## コマンドライン

探索統計のフィーチャを有効にしたバイナリを別の`target`ディレクトリへビルドし、CPU 3へ固定して各深さを1回ずつ実行した。

```console
CARGO_TARGET_DIR=target/diag cargo build --release --features search-stats --bin minase
taskset -c 3 target/diag/release/minase dev bench --depth 5
taskset -c 3 target/diag/release/minase dev bench --depth 8
```

出力の全文は[bench-depth5.txt](move-ordering-baseline-bench/bench-depth5.txt)と[bench-depth8.txt](move-ordering-baseline-bench/bench-depth8.txt)にある。

## エンジン

masterのコミット`6b97cc26`であり、未コミットの変更は含まない。
`bench`は規則セット`engine-default`（`L0,P0,R1,E0`）、埋め込みの評価重みを使う。
対局ではないため、候補と基準の区別はない。

## 環境

2026年10月9日、Intel Core Ultra 7 265KF（物理20コア、論理20コア）で測定した。
Rustは`rustc 1.98.0 (88d9e12ae 2026-08-18)`、releaseの設定でビルドした。
探索ワーカーは1、置換表は`bench`の既定の256 MB（`DEFAULT_TT_SIZE_MB`）であり、同時対局数は該当しない。
開始前に`match run`と`spsa`のプロセスがないことを確認した。

## 結果

15局面の合計は次のとおりである。
`bench`は局面ごとに置換表とhistory表を消すので、対局内の表の持ち越しの効果は含まない。

| 深さ | 通常探索ノード | 静止探索ノード | βカット | 初手βカット | 初手βカット率 | 最善手順位 1位 / 2位 / 3位 / 4位以降 |
|---:|---:|---:|---:|---:|---:|---|
| 5 | 61,629 | 229,846 | 31,195 | 20,726 | 66.4% | 177 / 10 / 8 / 153 |
| 8 | 1,304,771 | 2,263,361 | 488,713 | 342,486 | 70.1% | 966 / 56 / 70 / 792 |

最善手順位は、βカットせずに全手を走査し、かつ少なくとも1手がαを上げたノードについて、最後にαを上げた手の実探索順位（枝刈りした手を除く）を数えたものである。
ペンタノミアル度数、LLR、破棄ペア数、異常件数、`time_forfeits`、および対局の経過時間は該当しない。
`bench`の総ノード数は深さ5で282,387、深さ8で3,423,994である。

## 結論

初手βカット率は深さ5で66.4%、深さ8で70.1%であり、Chess Programming Wikiが順序付けの目安とする90%超を下回る。
[2026年9月27日の監査](../audits/beta-cutoff-first-move-2026-09-27.md)（コミットdb716df、59.5%と76.7%）とは静止探索の手数制限、捕獲履歴、およびFMの有無が異なり、時系列の比較には使えない。
