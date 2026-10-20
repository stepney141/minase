# correction historyの表の変更の時間切れの事前確認

## 目的

[correction historyの表の追加と持ち越しの設計書](../../plans/correction-history-tables.md)のフェーズ2として、SPSAの本番の前に、時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても、換算した持ち時間で時間切れとエンジン異常が起きないことを確かめる。
棋力測定ではない。

## コマンドライン

パラメーターファイル`correction-history-tables-safety.params`は、本番の89係数のファイルの写しで、開始値を`HardRemainingShare` = 50、`HardSoftRatio` = 800、`MinMoves` = 40、および`IterationRatio` = 150へ書き換えたものである。
反復数156は、[統括計画](../../plans/search-forms.md)が定める`⌈13 × 同時対局数 ÷ 16⌉`（同時対局数191）である。
持ち時間は、測定機のSTC`time=10000+100`に、この調整機で測った換算係数1.29を掛けた値である。
換算係数は、測定機の負荷時の速度476,382 NPSを、調整機で191本を同時に走らせた負荷時の速度367,951 NPSで割った比を小数第2位に丸めた値である。
測定機の値は、調整用ビルドを16本同時に`minase dev bench --depth 9 --repetitions 3`で走らせた平均NPSを5回測った中央値であり、5回の値は469,986、472,515、476,382、489,743、および504,806 NPSだった。
測定の間、lishogiのBotは通信対局を指し続けていたので、各回の前にBotのエンジンが3秒間CPUを使っていないことを確かめ、各回の間にBotのエンジンが使ったCPU時間が0.5秒未満の回だけを採った。5回はいずれも0秒であり、Botの思考と重なって0.5秒以上となった1回（444,267 NPS、21.8秒）は捨てた。
コマンドは、調整機の`scripts/vast-spsa/remote.py`が起動した。

```console
minase spsa \
  --run-dir /workspace/job/correction-history-tables-safety --seed 4100000000 \
  --engine commit:e695176a --params /workspace/job/safety.params \
  --rules engine-default --each time=12900+129 --concurrency 191 \
  --iterations 156 --pairs-per-iteration 8
```

## エンジン

調整対象は、形の変更の最後のコミット`e695176a58cd33c1c01a054dfc3899820803601a`の調整用ビルドであり、調整機でビルドした実行ファイルのSHA-256は`2a500f11…`である。
runnerは同じコミットの通常ビルド（版2.4.0、SHA-256`d3e7ea8c…`）である。
調整機では、測定機で作った`git bundle`からブランチを取り込み、`cargo build --release --locked --bin minase`でrunnerを、`minase spsa params --engine commit:e695176a --rules engine-default`で調整用ビルドを作った。
このコマンドが出力した既定のパラメーターファイルから`FmScale`の行を除いたものは、測定機で作った89係数のファイルと一致した。
規則は`engine-default`（`L0,P0,R1,E0`）、置換表は既定の256 MB、探索のワーカーは1である。

## 環境

調整機は、vast.aiのインスタンス56856484（出品56499914、機械のID 51088、AMD EPYC 9K84の2基構成、物理192コア、論理384スレッド（runnerが記録した論理CPUは368）、`cpu_ram`は773,514 MB、ベトナム、1時間約1.87米ドル）である。[サブタスク3](../history-forms/history-forms-safety.md)と同じ機械である。
同時対局数は、物理コア数から1を引いた191であり、メモリによる制限（調整機の実メモリを、1局あたりの最大常駐メモリ約1,002 MBの1.25倍で割った数）は受けなかった。
1局あたりの最大常駐メモリは、測定機で調整用ビルド同士の1ペアをSTCで指し、2局のそれぞれで2つのエンジンの最大常駐メモリを足し、その大きい方の値（1,002,328,064バイト、[benchの記録](correction-history-tables-bench.md)）とした。
調整機は、照会した出品から条件に合う最安のものを`launch.py`がその場で作成する方式で、2026年10月20日16時33分（日本時間）に作った。1時間2.5米ドル以下の96コアの型の出品が現れるまで、10時19分から約6.2時間待った。

## 結果

156反復をすべて適用し、1,248ペアのうち有効ペアは1,229、手数上限で破棄したペアは19だった。
異常件数は、不正着手、クラッシュ、応答タイムアウト、時間切れ（`time_forfeits`）、および拒否着手のいずれも0だった。
所要時間は1,568秒だった。

## 結論

時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても、換算した持ち時間で時間切れとエンジン異常は起きなかったので、本番のSPSAへ進む。
