# 段階Aの煙試験（秒読みつき時計の締切と途中結果の採用）

## 目的

[秒読みつき時計での持ち時間と秒読みの活用](../plans/byoyomi-time-usage.md)の段階A（秒読み期の締切、途中結果の採用）を着手前のコミットと秒読みつきの2条件で対局させ、フェーズ2の判定条件（時間切れ0件、秒読み期の思考時間/Bの中央値0.80以上、手数帯ごとの完了深さ）を確かめる。

## コマンドライン

```console
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-a-smoke-short --seed 2028000000 \
  --candidate commit:ea0133d --baseline commit:33e7269 --each time=6000+0,byoyomi=300 --concurrency 16 elo --pairs 10
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-a-smoke-long --seed 2028100000 \
  --candidate commit:ea0133d --baseline commit:33e7269 --each time=45000+0,byoyomi=300 --concurrency 16 elo --pairs 10
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-a-smoke-ponder-short --seed 2028200000 \
  --candidate commit:ea0133d --baseline commit:33e7269 --each time=6000+0,byoyomi=300 --ponder --concurrency 2 elo --pairs 10
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-a-smoke-ponder-long --seed 2028300000 \
  --candidate commit:ea0133d --baseline commit:33e7269 --each time=45000+0,byoyomi=300 --ponder --concurrency 2 elo --pairs 10
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-a-smoke-ponder-idle-short --seed 2028400000 \
  --candidate commit:ea0133d --baseline commit:33e7269 --each time=6000+0,byoyomi=300 --ponder elo --pairs 10
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-a-smoke-ponder-idle-long --seed 2028500000 \
  --candidate commit:ea0133d --baseline commit:33e7269 --each time=45000+0,byoyomi=300 --ponder elo --pairs 10
```

同一局面の比較は次のとおり実行した。局面は`byoyomi-a-smoke-long`の対局記録から手数0〜200を一様に200局面選んだ。

```console
python3 scripts/lishogi_byoyomi_usage/paired_depth_probe.py data/matches/byoyomi-a-smoke-long <基準> <候補> "btime 40000 wtime 40000 byoyomi 300" 200
python3 scripts/lishogi_byoyomi_usage/paired_depth_probe.py data/matches/byoyomi-a-smoke-long <基準> <候補> "btime 0 wtime 0 byoyomi 1000" 200
```

## エンジン

候補はea0133d（段階A）、基準は33e7269（master、着手前のエンジンと同一）である。規則セットは`engine-default`、両者とも`Threads=1`、`USI_Hash`は256 MB、`ByoyomiMargin`は既定の30 msである。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）である。
先読みなしの2条件は同時16対局で、他の測定はなかった。
先読みつきの最初の2条件は同時2対局で、別の測定（`pst-longer-training-ltc`、同時16対局）と重なって実行した。
先読みつきのやり直しの2条件（`ponder-idle`）は、他の測定がない状態で、ハーネスが自動で決めた同時9対局で実行した。

## 結果

| 条件 | ペンタノミアル度数 | Eloの点推定（95%信頼区間） | 破棄 | 時間切れ | その他の異常 | 経過時間 |
|---|---|---|---:|---:|---:|---:|
| 短い条件 | [0, 0, 4, 0, 6] | ＋240.8（＋106.1〜＋518.3） | 0 | 0 | 0 | 270秒 |
| 長い条件 | [0, 0, 5, 2, 2] | ＋120.4（＋23.2〜＋240.9） | 1 | 0 | 0 | 1,018秒 |
| 短い条件・先読み | [1, 0, 4, 1, 4] | ＋127.0（−15.0〜＋332.7） | 0 | 1（候補） | 0 | 827秒 |
| 長い条件・先読み | [2, 1, 5, 1, 1] | −34.9（−173.4〜＋93.0） | 0 | 0 | 0 | 1,186秒 |
| 短い条件・先読み（やり直し） | [1, 1, 3, 2, 2] | ＋58.5（−85.3〜＋227.1） | 1 | 0 | 0 | 1,036秒 |
| 長い条件・先読み（やり直し） | [2, 0, 6, 0, 2] | 0.0（−143.9〜＋143.9） | 0 | 0 | 0 | 420秒 |

不合法な予想手はすべての条件で0件だった。
1ペアの実時間の平均は、短い条件で167秒、長い条件で445秒（最大1,018秒）だった。

時計を再現した秒読み期の手の集計（`scripts/lishogi_byoyomi_usage/smoke_profile.py`）は次のとおりである。

| 条件 | 候補の思考時間/Bの中央値 | 基準 | 候補の途中結果の採用 | 候補の思考時間−Bの最大 | 基準 |
|---|---:|---:|---:|---:|---:|
| 短い条件 | 0.912 | 0.472 | 3,336手中1,186手 | −13.4 ms | −45.1 ms |
| 長い条件 | 0.914 | 0.408 | 6,345手中1,860手 | −16.2 ms | −46.5 ms |
| 短い条件・先読み | 記録なし | 記録なし | 記録なし | ＋8.6 ms | −41.0 ms |
| 長い条件・先読み | 記録なし | 記録なし | 記録なし | −1.8 ms | −44.8 ms |
| 短い条件・先読み（やり直し） | 記録なし | 記録なし | 記録なし | −12.9 ms | −46.7 ms |
| 長い条件・先読み（やり直し） | 記録なし | 記録なし | 記録なし | −16.2 ms | −46.7 ms |

手数帯ごとの完了深さの平均（候補／基準）は、短い条件で0〜50手10.05／9.97、50〜100手9.81／8.98、100〜200手9.72／8.41、200手以上12.07／10.70であった。
長い条件では、0〜50手11.28／11.46、50〜100手10.74／10.80、100〜200手9.92／9.97、200手以上18.17／16.21であった。
長い条件の0〜200手は、持ち時間期の手である。

長い条件の0〜200手で候補が基準をわずかに下回った原因を、同じ局面で比べて切り分けた。
持ち時間期の局面（`btime 40000 byoyomi 300`）では、完了深さの差（候補−基準）は−0.015、標準誤差0.034（200局面）であり、差は検出されなかった。
秒読み期の局面（`btime 0 byoyomi 1000`）では、差は＋0.965、標準誤差0.053であり、候補は200局面中95局面で途中結果を採用し、そのうち18局面で直前の完了反復の主変化の先頭と異なる手を返した。

先読みつきの短い条件の時間切れは、候補の秒読み期の手で、hard停止の思考時間が308.6 msだった。
締切は秒読み300 msから余裕30 msを引いた270 msであり、38.6 ms超過した。

## 結論

先読みなしの2条件は、時間切れ0件、秒読み期の思考時間/Bの中央値0.80以上を満たした。長い条件の持ち時間期の深さの不足は、同じ局面の比較で差が検出されず、対局の分岐による変動と判断した。
先読みつきの短い条件で候補に時間切れが1件出た。余裕30 msは、先読みありで測定機のコアがすべて埋まった状態では足りなかった。
利用者の決定により既定値30 msを保ち、他の測定がない状態で先読みつきの煙試験をやり直したところ、時間切れと不合法な予想手は0件で、候補の秒読み期の手は締切の手前に12.9 ms以上の余裕を残した。これにより判定条件を満たしたので、採否測定へ進む。
