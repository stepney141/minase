# 棋力向上段階9の最終構成の固定200ペア（段階開始版との比較）

## 目的

[棋力向上段階9](../plans/strength-stage9.md)のフェーズ8として、段階9の完了処理後の最終構成を段階開始版と固定200ペアで比較し、進捗指標として記録する。採否には使わない。

## コマンドライン

対局ハーネスは、masterの `4f946d4` でビルドしたバイナリから起動した。

```console
target/release/match_runner \
  --run-dir data/matches/strength-stage9-elo200 --seed 66000921 \
  --candidate commit:4f946d4d19e70110796f754a0258f0a778f59cd8 \
  --baseline commit:6c5c559a084704d66ff9b5366efa22eb946601fb \
  --rules engine-default --each time=10000+100 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --max-ply 4096 --response-timeout 120 \
  elo --pairs 200
target/release/match_report --run-dir data/matches/strength-stage9-elo200
```

## エンジン

候補は `4f946d4`（masterの先頭 `a620286` とエンジンの内容が同じで、差は文書だけ）であり、段階開始版に対して、SPSAの調整結果の適用（[spsa-apply](../plans/spsa-apply.md)）、[先読み教師](../plans/lookahead-teacher.md)で再学習した学習PST、および評価の償却を含む。
基準は段階開始版 `6c5c559` である。規則は `engine-default`、両エンジンとも `Threads=1`、`USI_Hash` 256 MB。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数16、他の負荷はない。
2026年9月23日0時48分から1時26分に実施した。

## 結果

| 量 | 値 |
|---|---:|
| 有効ペア | 187（破棄13、手数上限） |
| ペンタノミアル度数 | [36, 8, 91, 6, 46] |
| 正規化した平均得点 | 0.524 |
| Elo | +16.7、95%区間[−16.9, +50.7] |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 総CPU時間 | 28,448秒 |
| 経過時間 | 2,291秒（累計2,275秒） |

## 結論

最終構成は段階開始版に対して+16.7 Elo（95%区間[−16.9, +50.7]）であり、固定200ペアの精度では有意ではない。
段階9の項目は採用されておらず、この差は先読み教師の重みとSPSAの係数を含む工程全体の差である。
