# nnue-outcome-regression-reference-stc

## 目的

[設計書](../../plans/archived/nnue-outcome-regression.md)の配置の距離の基準とするため、master同士の短時間の時間制御（STC）の対局を200ペア行い、`--candidate`側の駒の配置と、教師より新しい対局での予測の損失の差Dを求める。

## コマンドライン

対局は、`da460a24`の`crates/minase`からビルドしたrunner（`data/nnue-outcome-regression/minase-runner`、SHA-256 `3a5dfa9ee988afea4f3c2bcbdc3e6533b6f0f6863c9a9d70f6899783878eeb06`）で実行した。
シードは、既存の測定の最大のシード（20,261,104,000,000）から上限ペア数より離れた未使用の値を選んだ。

```console
minase-runner match run --run-dir data/matches/nnue-outcome-regression-reference-stc --seed 20261105000000 \
  --candidate commit:dab974640967939cc3ef73fcfec4dba6dcce0775 --baseline commit:dab974640967939cc3ef73fcfec4dba6dcce0775 \
  --each time=10000+100 --concurrency 16 elo --pairs 200
```

Dの診断は次のコマンドで行った。
スクリプトは`nnue-outcome-regression-fit/saved_games_d.py`、標準出力は`nnue-outcome-regression-reference-stc/saved_games_d.out`である。

```console
python3 -I -u saved_games_d.py --data <リポジトリのdata> \
  --base-bin target/match-cache/dab974640967939cc3ef73fcfec4dba6dcce0775/minase \
  --corrected-bin data/nnue-outcome-regression/minase-corrected \
  --runs nnue-outcome-regression-reference-stc
```

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
| ペア数 | 200（有効198、破棄2） |
| ペンタノミアル度数 | [34, 17, 98, 9, 40] |
| Elo | +3.51（95%区間 −27.40〜+34.48） |
| 異常 | 不合法手0、クラッシュ0、タイムアウト0、`time_forfeits` 0、`rejected_moves` 0 |
| 経過時間 | 1,762.7秒 |

配置の距離Mは0.4602である。

Dの診断では、1,817局面（368局）を取った。
ペア番号の奇数側の177局のDは−0.004252、95%区間は−0.013020〜+0.005061である。
区間の下限が0より大きくないので、教師の対局との補正の向きの食い違いは見えない。
局数が少ないので、区間は0をまたぐ。

## 結論

M = 0.4602を配置の距離の基準とする。
Dの診断は、先へ進むことを妨げない。
