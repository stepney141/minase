# 第3世代のネットに合わせた探索係数の再調整の時間切れの事前確認

## 目的

[第3世代のネットに合わせた探索係数の再調整のSPSA](nnue-gen3-search-retune.md)の本番の前に、時間管理の係数を思考時間が長くなる側の値に置いても、調整機の持ち時間で時間切れとエンジン異常が起きないことを確かめる。
棋力測定ではない。

## コマンドライン

パラメーターファイル`data/vast-spsa/nnue-gen3-search-retune/safety.params`は、本番の103係数のファイルの写しであり、次の16係数の開始値を書き換えた。
[SPSAの手引き](../../guides/spsa.md)の第3節が定める4係数に、[持ち時間の伸縮の形の変更の事前確認](../time-instability-scaling/time-instability-scaling-safety.md)で加えた12係数を足し、値もその記録と同じにした。

| 係数 | 本番の開始値 | 事前確認の値 |
|---|---:|---:|
| `HardSoftRatio` | 755 | 800 |
| `HardRemainingShare` | 50 | 50 |
| `MinMoves` | 42 | 40 |
| `IterationRatio` | 152 | 150 |
| `StableIterations` | 5 | 8 |
| `TimeFallingBase` | 33 | 1500 |
| `TimeFallingPrevious` | 2141 | 8000 |
| `TimeFallingIteration` | 1351 | 5000 |
| `TimeReductionIntercept` | 462 | 0 |
| `TimeReductionSlope` | 84 | 0 |
| `TimeReductionScale` | 2651 | 1000 |
| `TimeInstabilityBase` | 1255 | 1500 |
| `TimeInstabilitySlope` | 803 | 4000 |
| `TimeEffortIntercept` | 1752 | 2500 |
| `TimeEffortSlope` | −788 | 500 |
| `TimeAdvantage` | 897 | 0 |

`HardRemainingShare`は、本番の開始値がすでに範囲の上端の50なので、事前確認でも50のままとした。

反復数78は、同時対局数95に対する`⌈13 × 95 ÷ 16⌉`である。
持ち時間`time=8700+87`（基本時間8,700ミリ秒、1手ごとの加算87ミリ秒）は、STCの`time=10000+100`の基本時間と加算に換算係数`f` = 0.87を掛けた値である。
換算係数は、手元の測定機の負荷時の速度440,384 NPS（1秒あたりの探索ノード数）を、調整機で95本の`bench depth 9 repetitions 3`を同時に走らせた負荷時の速度506,619 NPSで割った比を、小数第2位に丸めた値である。
測定機の値は、調整用ビルドを16本同時に`scripts/vast/spsa/bench.py`で走らせた平均NPSを5回測った中央値であり、5回の値は429,562、440,384、433,857、471,152、および454,788 NPSだった。
測定機ではlishogiのBotが長い持ち時間の対局を並行して指していたので、各回の前にBotのエンジンが3秒間CPUを使わない状態になるのを待ち、各回の間にBotのエンジンが使ったCPU時間が0.5秒未満の回だけを採用する条件とした。5回とも0秒であり、除外した回はない。

コマンドは、調整機の`scripts/vast/spsa/remote.py`が起動した。

```console
minase spsa \
  --run-dir /workspace/job/nnue-gen3-search-retune-safety --seed 7200000000 \
  --engine commit:f9b56d03904716b22b3e1f88734a76ea5167c590 --params /workspace/job/safety.params \
  --rules lishogi --each time=8700+87 --concurrency 95 \
  --iterations 78 --pairs-per-iteration 8 \
  --option EvalFile=/workspace/job/network.bin
```

## エンジン

調整対象は、v3.0.0のリリースの後のコミット`f9b56d03904716b22b3e1f88734a76ea5167c590`の調整用ビルドであり、調整機でビルドした実行ファイルのSHA-256は`8f758ac2fa48953c343f2c570230bcd16089606745d79075c1ba61be3c96bbe1`である。
runnerは同じコミットの通常ビルド（版3.0.0、SHA-256は`64d6b625c7f3791bbbf39a8340444d32f2222f92ff6a4d7cbdfe4f518bf23226`）である。
ネットは、当時の標準だった第3世代のネット（SHA-256は`cbbccf6f5ee7097fd890086d2cf4bca72ed969da28353cb0c4a2f4d781acc244`、入力特徴集合の名前が`P`の版）である。
規則セットは`lishogi`（`L1,L2,P0,P3,R1,E1,E3`）、置換表は既定の256 MB、探索のワーカーは1である。
規則セットに`lishogi`を使ったのは、このネットの教師の生成と学習に使った規則であり、lishogiのBotが対局する規則でもあるからである。2026年11月24日に利用者が決めた。

## 環境

調整機は、vast.aiの出品61894939で借りたAMD Ryzen Threadripper PRO 9995WX（ギリシャ、機械のID 154306、インスタンス62363214）である。
物理96コア、論理192スレッドで、`cpu_ram`は192,513 MB、時間単価は約1.34米ドルである。
同時対局数95は、物理コア数から1を引いた95と、実メモリを`game_rss_bytes`（1,017,712,640バイト）の1.25倍で割った152の小さい方である。
`game_rss_bytes`は、測定機で調整用ビルド同士の1ペア（2局）を規則セット`lishogi`のSTCで指させ（実行ディレクトリ`data/vast-spsa/nnue-gen3-search-retune/rss-pair`、シード7300000000）、局ごとに両者の最大常駐メモリの和を求め、2局のうち大きい方を採った値である。
インスタンスは2026年11月24日23時18分（日本時間）に作成し、`remote.py`が開始時に見込んだ本番の所要時間は11.05時間だった。

## 結果

78反復をすべて適用し、有効ペア616、手数上限による破棄ペア8であった。
不正着手、クラッシュ、応答タイムアウト、時間切れ（`time_forfeits`）、および拒否着手はすべて0件、経過時間は1,048秒である。

## 結論

時間管理の係数を思考時間が長くなる側に置いても、調整機の持ち時間で時間切れとエンジン異常は起きなかったので、`remote.py`はそのまま本番を始めた。
