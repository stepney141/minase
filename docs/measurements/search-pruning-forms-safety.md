# 枝刈りとLMRの関数形の候補の時間切れの事前確認

## 目的

[探索の枝刈りとLMRの関数形の見直し](../plans/search-pruning-forms.md)のフェーズ2として、SPSAの本番の前に、時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても、換算した持ち時間で時間切れとエンジン異常が起きないことを確かめる。
棋力測定ではない。

## コマンドライン

パラメーターファイル`search-pruning-forms-safety.params`は、本番の68係数のファイルの写しで、開始値を`HardRemainingShare` = 50、`HardSoftRatio` = 800、`MinMoves` = 40、および`IterationRatio` = 150へ書き換えたものである。
反復数78は、[統括計画](../plans/search-forms.md)が定める`⌈13 × 同時対局数 ÷ 16⌉`（同時対局数95）である。
持ち時間は、測定機のSTC`time=10000+100`に[試走](../measurements/vast-spsa-trial.md)で求めた換算係数1.46を掛けた値である。

```console
minase spsa \
  --run-dir /workspace/job/search-pruning-forms-safety --seed 3200000000 \
  --engine commit:388c5244 --params /workspace/job/search-pruning-forms-safety.params \
  --rules engine-default --each time=14600+146 --concurrency 95 \
  --iterations 78 --pairs-per-iteration 8
```

## エンジン

調整対象は、形の変更の最後のコミット`388c5244950a8fec1dcc7738b1c6b48e1426f532`の調整用ビルドであり、実行ファイルのSHA-256は`c4a1b7b5…`である。
runnerは同じコミットの通常ビルド（版2.4.0、SHA-256`029a8715…`）である。
調整機では、測定機で作った`git bundle`からブランチを取り込み、`cargo build --release --locked --bin minase`でrunnerを、`minase spsa params --engine commit:388c5244 --rules engine-default`で調整用ビルドを作った。
このコマンドが出力した既定のパラメーターファイルから`FmScale`の行を除いたものは、測定機で作った68係数のファイルと一致した。
規則は`engine-default`（`L0,P0,R1,E0`）、置換表は既定の256 MB、探索のワーカーは1である。

## 環境

調整機は、vast.aiのインスタンス56409205（AMD Ryzen Threadripper PRO 9995WX、物理96コア、論理192スレッド、メモリ約241 GiB、ノルウェー）である。
[試走](../measurements/vast-spsa-trial.md)に使った機械をそのまま使った。
同時対局数は95である。

## 結果

78反復をすべて適用し、1,248局のうち有効ペアは621、手数上限で破棄したペアは3だった。
異常件数は、不正着手、クラッシュ、応答期限超過、時間切れ（`time_forfeits`）、および拒否着手のいずれも0だった。
所要時間は1,524秒だった。

## 結論

時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても、換算した持ち時間で時間切れとエンジン異常は起きなかったので、本番のSPSAへ進む。
