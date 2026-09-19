# USI先読み（ponder）の起案時の的中率の診断

## 目的

[USI先読み](../plans/ponder.md)を実装する前に、minaseが返す主要変化（PV）の2手目が相手の次の実着手と一致する割合を自己対局で数え、先読みの的中率の目安と予想手の出力率を見積もる。

## コマンドライン

先読みは行わず、minaseを2プロセス起動して通常の`go`だけで対局させ、各着手の最後の`info`行のPVの2手目と、相手が次に返した`bestmove`とを文字列で比べた。
開始局面は、対局ごとのシードから初期局面を8手から12手ランダムに進めて作った。
時計は[スクリプト](ponder-hit-rate-diag/hit_rate.py)が管理し、`go btime … wtime … binc … winc …`で残り時間を送った。

```console
cargo build --release --bin minase
python3 docs/measurements/ponder-hit-rate-diag/hit_rate.py --binary target/release/minase \
  --base 10000 --inc 100 --games 24 --seed 20260919 --concurrency 8 --out stc.json
python3 docs/measurements/ponder-hit-rate-diag/hit_rate.py --binary target/release/minase \
  --base 60000 --inc 200 --games 8 --seed 30260919 --concurrency 8 --out ltc.json
python3 docs/measurements/ponder-hit-rate-diag/summarize.py <出力>
```

対局ごとの全着手の記録は[stc.json.gz](ponder-hit-rate-diag/stc.json.gz)と[ltc.json.gz](ponder-hit-rate-diag/ltc.json.gz)に、集計の出力は[summary-stc.txt](ponder-hit-rate-diag/summary-stc.txt)と[summary-ltc.txt](ponder-hit-rate-diag/summary-ltc.txt)にある。

## エンジン

両者ともコミット`216b020`（起案時のmasterの先頭）のminaseであり、規則セットは`engine-default`（L0＋P0＋R1＋E0）である。
対局者ごとに別のプロセスを使うので、置換表は共有しない。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、rustc 1.98.0であり、2026年9月19日に測った。
両エンジンとも`Threads=1`、`USI_Hash`は既定の256 MB、同時対局数は8である。
時間制御の対局なので、結果は完全には再現しない。

## 結果

分母は、対局の最後の着手を除く全着手である。
「出力率」はPVが2手以上あった着手の割合、「的中率」はPVの2手目が相手の次の着手と一致した着手の割合である。

| 条件 | 局数 | 着手数 | 出力率 | 的中率 | 対局ごとの的中率（最小、中央値、最大） | 思考時間の中央値 | 到達深さの中央値 |
|---|---|---|---|---|---|---|---|
| STC（10秒＋0.1秒） | 24 | 16,742 | 98.1% | 47.4% | 36.3%、55.5%、66.4% | 91 ms | 9 |
| LTC（60秒＋0.2秒） | 8 | 3,981 | 98.5% | 55.9% | 50.5%、56.8%、61.1% | 325 ms | 9 |

手数帯ごとの的中率は次のとおりである。

| 手数帯 | STC | LTC |
|---|---|---|
| 0から99手 | 63.9% | 63.1% |
| 100から199手 | 58.8% | 59.8% |
| 200から399手 | 53.3% | 58.4% |
| 400手以降 | 37.0% | 44.8% |

STCの24局の手数は205手から4,096手であり、手数上限の4,096手に達した1局と2,642手の1局が400手以降の着手の多くを占める。
STCの全体の的中率が対局ごとの中央値より低いのは、この2局の重みによる。
時間切れは0件であり、最後の`info`行のPVの1手目が`bestmove`と異なる着手も0件だった。

## 結論

自己対局での的中率の目安はSTCで47%、LTCで56%であり、[段階計画](../plans/strength-stages.md)が発動の事前確認に使う基準の5%を大きく超えるので、予想手を指した後の局面を先読みする方式で着手してよい。
出力率は98%なので、PVの2手目だけを予想手の出所とする設計で足り、補完の追加は要らない見込みである。
この値は、同じエンジン同士の対局でPVの2手目を数えたものであり、実際の先読みの的中率の上側の目安である。
反復規則による予想手の抑止と、異なるエンジンや人間を相手にした場合の低下は含まない。
外部エンジンHaChu同士の同じ数え方の値は、60秒＋1秒の4局、278手で40.6%だった（スクリプトが着手の表記の不一致で64手から82手で打ち切られたため、序盤から中盤の入口までの値である）。
