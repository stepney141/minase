# nnue-architecture-revision-init-fixed-nodes

## 目的

[第1層の初期値を小さくした候補のSTC](nnue-architecture-revision-init-stc.md)が`H0`となったので、探索速度を除いた評価の質を基準と比べ、あわせて教師の分布の外の誤差、隠れ層の寄与、および同時実行時の速度を調べて、[設計書](../../plans/nnue-architecture-revision.md)の切り分けの判断材料を得る。
固定ノードの測定は診断であり、採否の根拠には使わない。

## コマンドライン

対局は候補のコミットからビルドしたrunnerで、本体のリポジトリのルートから実行した。

```console
minase-runner-init match run --run-dir data/matches/nnue-architecture-revision-init-fixed-nodes --seed 20261114000000 \
  --candidate commit:9af68a00dfe397ecfc680e74972c58db20d452e5 \
  --baseline commit:ee1d6e08f294a5bfdc41c6737d445e0944928f77 \
  --each nodes=100000 --concurrency 16 elo --pairs 200
python3 -I ood_compare.py --run data/matches/nnue-architecture-revision-init-stc --teacher <31c2a465のminase> \
  --engine candidate1=<77e11950のminase> --engine candidate2=<9af68a00のminase> --engine baseline=<ee1d6e08のminase> \
  --step 20 --max-positions 2000 --nodes 20000 --seed 1 --output ood_compare.json
python3 -I eval_steps.py --run data/matches/nnue-architecture-revision-init-stc \
  --engine candidate1=<77e11950のminase> --engine candidate2=<9af68a00のminase> --engine baseline=<ee1d6e08のminase> \
  --pairs 1500 --seed 1 --output eval_steps.json
python3 -I load_nps.py --engine candidate2=<9af68a00のminase> --engine baseline=<ee1d6e08のminase> \
  --engine candidate1=<77e11950のminase> --copies 16 --depth 13 --rounds 3 --output load_nps.json
```

スクリプトと出力は[同じ名前のディレクトリ](nnue-architecture-revision-init-fixed-nodes/)にある。
隠れ層の寄与は、量子化誤差の切り分けに使った[集計](nnue-architecture-revision-training/quant_decomposition.rs)を拡張して求めた。

## エンジン

候補1は`77e11950`（標準偏差0.05の学習）、候補2は`9af68a00`（小さい初期値の学習）、基準は`ee1d6e08`である。
教師の探索値は、教師を生成したmasterの時点に近い`31c2a465`のエンジンで求めた。
規則セットは`engine-default`（L0,P0,R1,E0）である。

## 環境

2026年11月4日に測定機（Intel Core Ultra 7 265KF、物理20コア、論理20コア）で行った。
候補と基準のワーカー数（`Threads`）はともに1、`USI_Hash`はともに256 MB、同時対局数は16、思考制限は1手10万ノードである。
同じ機械でlishogiのBotと、別の作業の探索が動いていた時間がある。

## 結果

### 固定ノードの対局

| 項目 | 候補2 | 候補1（[記録](nnue-architecture-revision-fixed-nodes.md)） |
|---|---:|---:|
| ペンタノミアル度数（ペア得点0、0.5、1、1.5、2） | 44、9、96、4、45 | 49、6、109、5、30 |
| 有効ペア数 | 198 | 199 |
| Elo（95%信頼区間） | −2.6（−35.8〜+30.5） | −34.2（−65.4〜−3.5） |
| 異常 | いずれも0件 | いずれも0件 |
| 平均の到達深さ（候補、基準） | 17.93、17.96 | 17.31、17.44 |

### 時間制御との比較

| 項目 | 候補2 | 候補1 |
|---|---:|---:|
| STCの判定 | `H0`（306ペア） | `H0`（241ペア） |
| STCの得点率から求めたElo（95%信頼区間） | −44.5（−69.9〜−19.6） | −58.9（−87.7〜−30.9） |
| STCの平均の到達深さ（候補、基準） | 17.06、17.34 | 17.63、17.88 |

### 同時実行時の速度

深さ13のbenchを同時に実行したときの、1プロセスのNPSの中央値（3回の中央値）は次のとおりである。
探索木が異なるので目安である。

| 同時実行数 | 基準 | 候補2 | 比 | 候補1 | 比 |
|---:|---:|---:|---:|---:|---:|
| 1 | 561,160 | 514,846 | 0.917 | 529,372 | 0.943 |
| 16 | 359,234 | 327,810 | 0.913 | 336,702 | 0.937 |

同時実行で比が悪化することはなかった。
[教訓](../../lessons/alternate-speed-rounds-against-parent.md)の換算（2%で約6 Elo）では、約8.7%の差は約26 Eloにあたる。

### 教師の分布の外の誤差

STCの対局から20手ごとに抽出した局面（1,983局面、637局）で、`31c2a465`の1局面2万ノードの探索値を教師とした誤差を求めた。
差は基準との差であり、区間は対局単位の復元抽出による95%区間である。

| 指標 | 基準 | 候補1 | 候補2 |
|---|---:|---:|---:|
| 平均絶対誤差（センチポーン） | 261.8 | 246.6 | 253.0 |
| 基準との差 | | −15.3（−24.6〜−6.2） | −8.8（−18.6〜+0.7） |
| BCE | 0.54102 | 0.54044 | 0.54087 |
| 基準との差 | | −0.00058（−0.00141〜+0.00022） | −0.00014（−0.00111〜+0.00068） |

対局で現れた局面でも、候補の誤差は基準と同等か小さく、教師の分布の外で汎化が悪いという形ではない。

### 評価の手ごとの変動と隠れ層の寄与

対局の連続する局面の静的評価の変化`E(t)+E(t+1)`（1,500組）の絶対値の平均は、基準が191.7、候補1が198.1、候補2が191.7センチポーンであり、候補の評価が手ごとに大きく揺れてはいない。

検証局面での非線形の出力の内訳は次のとおりである。

| 項目 | 候補1 | 候補2 |
|---|---:|---:|
| skip接続の標準偏差（センチポーン） | 1,222 | 1,201 |
| 隠れ層の出力の標準偏差（センチポーン） | 592 | 558 |
| 2つの相関 | 0.845 | 0.901 |

隠れ層は働いているが、出力はskip接続と強く相関する。

## 結論

候補2は固定ノードでは基準と区別できず、候補1より改善した。
時間制御では負けるが、速度の差（約9%）で説明できる分（約26 Elo）を除くと、残りは信頼区間の幅の中にある。
評価は基準と同等以上の精度で教師の探索値を予測するが、同じノード数で強くはなっていない。
この教師では、幅と隠れ層を増やしても棋力の利得が見えず、速度の費用だけが残っている。
