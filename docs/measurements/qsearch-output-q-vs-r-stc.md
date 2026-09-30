# 候補Q対対照RのSTC

## 目的

[静止探索の出力を学ぶPSTの学習](../plans/qsearch-output-training.md)のフェーズ4として、同じ静かな局面で学習目標だけを静的値から静止探索の出力へ変えた候補Qが、対照Rより強いかを短時間条件STCの逐次確率比検定GSPRTで判定する。

## コマンドライン

```console
data/qsearch-output-training/phase4/match_runner \
  --run-dir data/matches/qsearch-output-q-vs-r-stc --seed 93000000 \
  --candidate commit:29d44be5e39a0ef4e25641a202391135d9290552 \
  --baseline commit:e98fcca10c049395d655c4621151fc6b52297c35 \
  --concurrency 19 --each time=10000+100 gsprt --max-pairs 3000
```

`match_runner` はブランチ `qsearch-output-training` のコミット `bcc32fa` からビルドした複製である（SHA-256 `b35cd27b…`）。

## エンジン

候補Qは `29d44be`（ブランチ `qsearch-output-q`）、基準Rは `e98fcca`（ブランチ `qsearch-output-r`）であり、どちらも採用版S0のmaster `b59e616` に[学習した重み](qsearch-output-training.md)の `nets/pst.bin` を置いただけのコミットである。
規則は `engine-default`（`L0,P0,R1,E0`）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）である。
両エンジンとも `Threads=1`、`USI_Hash` は既定の256 MB、同時対局数は19、時間制御は `time=10000+100`（秒読みなし）である。
lishogi Botも同じ測定機で動いていた。
2026年9月30日7時04分から8時40分に実行した。

## 結果

| 項目 | 値 |
|---|---|
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2の順） | [150, 26, 326, 22, 116] |
| 有効ペア | 640 |
| 手数上限による破棄ペア | 38 |
| LLR | −2.986 |
| 判定 | `H0` |
| 候補の得点率 | 47.2% |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 5,798秒 |

## 結論

QはRに対してSTCで`H0`となり、比較用のSTCを通過しなかったので、設計書の経路によりQはS0との採否測定へ進まない。
