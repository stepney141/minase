# search-revival-ranges

## 目的

[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)の処置群Tで、上限を無効側の端とする係数の範囲の上限（深さ5の`bench`で発動率が1%以下になる最小の値）を求め、厳密な無効値を持つ3項目が無効値で発動しないことを確かめる。

## コマンドライン

計数を加えるパッチは[ranges.patch](search-revival-ranges/ranges.patch)であり、探索の挙動を変えずに計数だけを加える。
係数の値は、`src/search/alphabeta/params.rs`の既定値を1つだけ書き換えて再ビルドして与えた（`bench`は調整用オプションを受け付けない）。

```console
git apply ranges.patch
nice -n 19 cargo run --release --bin bench -- --depth 5 --threads 1
```

各係数について、定義域の下限と暫定の上限から始め、発動率が1%以下になる最小の整数を二分探索した。
試した値と発動率の全件は[ranges.txt](search-revival-ranges/ranges.txt)、無効値での確認は[disabled-counters.txt](search-revival-ranges/disabled-counters.txt)にある。

## エンジン

コミットfe7d874（8項目を移植し、評価関数のPSTを新しい重みG23とした master b96a931 の上へ載せ直した版）に計数のパッチを当てたビルドである。
新しい13係数は、測定中は元の値（`NonImprovingFutility1`〜`3` = 25、50、50、`LmpBase` = 300、`LmpSlope` = 100、`ReverseFutilityMargin` = 50、`RazoringMargin1`〜`2` = 400、`NullMoveEvalScale` = 100、`HistoryDecay` = 75、`CaptureHistoryLimit` = 20,755、`CaptureHistoryScale` = 100、`QsearchMoveLimit` = 1）に置いた。
1つの係数を測るとき、他の係数はこの値のままにした。
ただし`LmpBase`は`LmpSlope`を100に、`LmpSlope`は`LmpBase`を300に固定した。
規則セットは`bench`の既定である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、`Threads`は1である。
計数はノードの数だけに依存し、測定機の負荷に影響されない。
計数を加える前後で、15局面の深さ、ノード数、最善手、および評価値が一致した。

## 結果

条件を満たしたノードの定義は次のとおりである。
reverse futility pruningとrazoringでは、余裕値の比較と王駒への利きの判定を除く適用条件（残り深さ、零窓、詰み帯でない）を満たしたノードであり、razoringは残り深さ1と2を分けて数えた。
late move pruningでは、ノードの条件（残り深さ3以下、零窓、詰み帯でない、王駒に利きがない）を満たしたノードである。
静止探索の手数制限では、捕獲手を1手以上取り出したノードである。
発動は、その改良で打ち切ったノード、または1手以上を読まなかったノードである。

| 係数 | 元の値 | 元の値での発動率 | 範囲の上限 | 上限での発動率 | 上限より1小さい値での発動率 |
|---|---:|---:|---:|---:|---:|
| `ReverseFutilityMargin` | 50 | 56.0% | 1,938 | 0.997% | 1.007% |
| `RazoringMargin1` | 400 | 4.9% | 2,668 | 0.997% | 1.005% |
| `RazoringMargin2` | 400 | 11.3% | 2,885 | 0.955% | 1.016% |
| `LmpBase` | 300 | 4.8% | 8,600 | 0.982% | 1.025% |
| `LmpSlope` | 100 | 4.8% | 3,600 | 0.993% | 1.003% |
| `QsearchMoveLimit` | 1 | 14.2% | 7 | 0.444% | 1.344% |

試したすべての点で、発動率は係数の値に対して増えなかった。

元の値では、null moveの加算が正だった回数は383、improvingで余裕値を縮めた回数は4,292、捕獲履歴の寄与が0でない捕獲手を順序付けた回数は359であった。
`NullMoveEvalScale` = 0、`NonImprovingFutility1`〜`3` = 100、`CaptureHistoryScale` = 0にすると、3つとも0になった。

## 結論

上限を無効側の端とする6係数の範囲の上限を上の表の値に定め、設計書の規則で開始値と摂動幅を決めた。
reverse futility pruningとrazoringは余裕値を歩兵価値の19〜29倍まで広げないと発動率が1%を下回らず、改めたlate move pruningは開始値でも発動率が4.8%と小さい。
厳密な無効値を持つ3項目は、無効値で1回も発動しない。
