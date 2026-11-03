# nnue-outcome-regression-control-stc

## 目的

配置の距離の判定が差のないネットを誤って「偏りが減った」としないことを確かめるため、[基準の対局](nnue-outcome-regression-reference-stc.md)と同じシードとペア数でmaster同士の短時間の時間制御（STC）の対局をもう1回行い、基準の対局とのMの差を求める。

## コマンドライン

runnerは基準の対局と同じである。
比較のスクリプトは`nnue-outcome-regression-fit/placement_distance.py`、標準出力は`nnue-outcome-regression-control-stc/compare.out`である。

```console
minase-runner match run --run-dir data/matches/nnue-outcome-regression-control-stc --seed 20261105000000 \
  --candidate commit:dab974640967939cc3ef73fcfec4dba6dcce0775 --baseline commit:dab974640967939cc3ef73fcfec4dba6dcce0775 \
  --each time=10000+100 --concurrency 16 elo --pairs 200
python3 -I -u placement_distance.py compare --data <リポジトリのdata> \
  --reference nnue-outcome-regression-reference-stc --placement nnue-outcome-regression-control-stc
```

出力の「補正ネット」の欄は、この記録では対照の対局を指す。

## エンジン

| 役 | コミット | 実行ファイルのSHA-256 |
|---|---|---|
| 候補、基準 | `dab974640967939cc3ef73fcfec4dba6dcce0775`（master） | `5eb0887840816265eec94f3a5007ecde3d8a5b48bba79a84925e8fe830ef7be2` |

規則セットは`engine-default`（L0,P0,R1,E0）である。

## 環境

2026年11月3日に測定機（Intel Core Ultra 7 265KF、物理20コア、論理20コア）で行った。
候補と基準のワーカー数（`Threads`）はともに1、`USI_Hash`はともに256 MB、同時対局数は16、時間制御は持ち時間10秒、1手0.1秒加算である。
同じ機械でlishogiのBotが動いている。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 200（有効195、破棄5） |
| ペンタノミアル度数 | [33, 13, 100, 16, 33] |
| Elo | +2.67（95%区間 −27.27〜+32.65） |
| 異常 | 不合法手0、クラッシュ0、タイムアウト0、`time_forfeits` 0、`rejected_moves` 0 |
| 経過時間 | 2,046.6秒 |

Mは基準の対局で0.4602、この対局で0.4622である。
Mの差は+0.0021、95%区間は−0.0025〜+0.0064、半幅は0.0044である。
2回の対局は時間制御なので、同じシードでも探索の列は再現されず、棋譜は一致しない。

## 結論

同じネット同士の比較で区間は0をまたぎ、判定は「偏りが減ったとは言えない」となったので、判定が差のないネットで誤って「偏りが減った」とならないことを、この1回について確かめた。
