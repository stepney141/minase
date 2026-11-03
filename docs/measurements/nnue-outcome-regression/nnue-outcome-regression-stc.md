# nnue-outcome-regression-stc

## 目的

線形の経路を対局結果への回帰で補正したネット（補正ネット）を候補、masterを基準とする短時間の時間制御（STC）のGSPRTで、候補を長時間の時間制御（LTC）へ進めるかを[設計書](../../plans/archived/nnue-outcome-regression.md)の規則で判定する。

## コマンドライン

対局は、`da460a24`の`crates/minase`からビルドしたrunner（`data/nnue-outcome-regression/minase-runner`、SHA-256 `3a5dfa9ee988afea4f3c2bcbdc3e6533b6f0f6863c9a9d70f6899783878eeb06`）で実行した。

```console
minase-runner match run --run-dir data/matches/nnue-outcome-regression-stc --seed 20261106000000 \
  --candidate commit:da460a24865724dab055aed94e5c5aadae0e2929 --baseline commit:dab974640967939cc3ef73fcfec4dba6dcce0775 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

仮説は既定のとおり、H0をelo=0、H1をelo=10とし、誤り率はα=β=0.05である。
シードは、[配置の距離の比較](nnue-outcome-regression-placement-stc.md)のシード20,261,105,000,000から上限ペア数より離れた未使用の値を選んだ。

## エンジン

| 役 | コミット | 実行ファイルのSHA-256 | 埋め込みのネット |
|---|---|---|---|
| 候補 | `da460a24865724dab055aed94e5c5aadae0e2929` | `d2aa658f70d839366111dcc0c6701d683ee4ea7cd6f57c0e830d1d9608cb57a0` | 補正ネット（`84529f38…`） |
| 基準 | `dab974640967939cc3ef73fcfec4dba6dcce0775`（master） | `5eb0887840816265eec94f3a5007ecde3d8a5b48bba79a84925e8fe830ef7be2` | 現行のネット（`915aef3b…`） |

規則セットは`engine-default`（L0,P0,R1,E0）である。

## 環境

2026年11月3日に測定機（Intel Core Ultra 7 265KF、物理20コア、論理20コア）で行った。
候補と基準のワーカー数（`Threads`）はともに1、`USI_Hash`はともに256 MB、同時対局数は16、時間制御は持ち時間10秒、1手0.1秒加算である。
同じ機械でlishogiのBotが動いている。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 117（有効116、破棄1） |
| ペンタノミアル度数 | [3, 0, 53, 2, 58] |
| LLR | 2.9613 |
| 判定 | `H1` |
| 異常 | 不合法手0、クラッシュ0、タイムアウト0、`time_forfeits` 0、`rejected_moves` 0 |
| 経過時間 | 1,218.8秒 |

## 結論

判定が`H1`なので、候補をLTCへ進める。
