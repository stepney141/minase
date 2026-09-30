# 候補Qc対候補QのSTC

## 目的

[静止探索の出力を学ぶPSTの学習](../plans/qsearch-output-training.md)のフェーズ4として、同じ学習目標（静止探索の出力の回帰）に捕獲局面を追加した候補Qcが、静かな局面だけで学んだ候補Qより強いかを短時間条件STCの逐次確率比検定GSPRTで判定する。

## コマンドライン

```console
data/qsearch-output-training/phase4/match_runner \
  --run-dir data/matches/qsearch-output-qc-vs-q-stc --seed 93010000 \
  --candidate commit:3338e6f66a63679c77cd579c29cf89147d609ac1 \
  --baseline commit:29d44be5e39a0ef4e25641a202391135d9290552 \
  --concurrency 19 --each time=10000+100 gsprt --max-pairs 3000
```

`match_runner` は[Q対RのSTC](qsearch-output-q-vs-r-stc.md)と同じ複製（SHA-256 `b35cd27b…`）である。
基本シード93,010,000は、Q対RのSTCの93,000,000から上限の3,000ペア以上離れている。

## エンジン

候補Qcは `3338e6f`（ブランチ `qsearch-output-qc`）、基準Qは `29d44be`（ブランチ `qsearch-output-q`）であり、どちらも採用版S0のmaster `b59e616` に[学習した重み](qsearch-output-training.md)の `nets/pst.bin` を置いただけのコミットである。
規則は `engine-default`（`L0,P0,R1,E0`）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）である。
両エンジンとも `Threads=1`、`USI_Hash` は既定の256 MB、同時対局数は19、時間制御は `time=10000+100`（秒読みなし）である。
lishogi Botも同じ測定機で動いていた。
2026年9月30日8時40分から15時52分に実行した。

## 結果

| 項目 | 値 |
|---|---|
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2の順） | [568, 110, 1444, 96, 598] |
| 有効ペア | 2,816 |
| 手数上限による破棄ペア | 184 |
| LLR | −1.166 |
| 判定 | `pending`（上限3,000ペアに到達） |
| 候補の得点率 | 50.4% |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 25,907秒 |

## 結論

上限3,000ペアに達した時点のLLRが負なので、設計書の振分け規則によりQcはQc対Qの比較用STCを通過せず、S0との採否測定へ進まない。
