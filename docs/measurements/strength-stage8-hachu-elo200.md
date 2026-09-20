# 段階8の最終構成とHaChuの固定200ペア測定

## 目的

[棋力向上段階8](../plans/strength-stage8.md)の最終構成をHaChuと固定200ペアで比較し、外部エンジンに対する棋力を進捗指標として記録する。採否の根拠には使わない。

## コマンドライン

対局ハーネスは、コミットを固定したworktreeでビルドしたバイナリから起動した。

```console
target/release/match_runner \
  --run-dir /home/stepney141/board-games/minase/data/matches/strength-stage8-hachu-elo200 --seed 36000919 \
  --candidate commit:0c1bd76 \
  --baseline cecp:/home/stepney141/board-games/hachu-debian/hachu \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 10 \
  --max-ply 4096 --response-timeout 120 \
  elo --pairs 200
target/release/match_report --run-dir /home/stepney141/board-games/minase/data/matches/strength-stage8-hachu-elo200
```

## エンジン

候補は段階8の最終構成であるコミット`0c1bd76722a2a372bb5dff9af03e5ad7b50a0650`（バイナリのSHA-256 `87a4832600df6bdb0960768f5d45790ca2e6738857641d462c204b64333e2418`）である。
基準はH. G. Muller作のHaChuであり、[段階7の測定](strength-stage7-hachu-elo200.md)と同じ実行ファイル（`/home/stepney141/board-games/hachu-debian/hachu`、SHA-256 `10db1299f95626fe58875b1c24a1cee6acce0eee5fb42ed8b17ce1373a9dcbdf`、Debian版`0.21-29-gdf26f4a-4`）を既定の規則オプションで使った。
規則セットはHaChuの既定設定に対応する`L1,L3,P0,P5,P6,R2,E1,E2`であり、審判層とminaseの双方へ同じ規則を与えた。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、minaseは`Threads=1`、置換表は双方256 MB、同時対局数は10である。HaChuは探索ワーカー数を報告しないので、`match_report`の`maximum_engine_threads`は`null`である。2026年9月21日に実施した。
同じ機械でlishogi Botのエンジンが対局していた。

## 結果

値は`match_report`が保存記録から再計算したものである。

| 項目 | 値 |
|---|---|
| ペア数 | 200（有効200、破棄0） |
| ペンタノミアル度数 | [0, 0, 17, 0, 183] |
| Elo | +541.10、95%信頼区間[+472.45, +649.92] |
| 正規化ペア得点の平均 | 0.9575（標準誤差0.0099） |
| 局の勝敗 | minaseの383勝17敗、引き分け0 |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 局時間 | 中央値353.6秒、95%点614.5秒 |
| 最大常駐メモリ | 1局あたり最大569 MB、同時対局の保守的な合計5.7 GB |
| 経過時間 | 15,427.4秒（`summary.json`の`active_wall_time_ns = 15427379614194`） |

minaseの383勝はすべてHaChuの投了で終わり、HaChuの17勝はすべてminaseの最後の王駒の捕獲で終わった。
HaChuが勝った17局は、HaChuが後手の局が11、先手の局が6で、手数は252手から809手であった。
得点1.0の17ペアはいずれも1勝1敗である。

## 結論

段階8の最終構成は、HaChuに対して固定200ペアで+541.10 Elo、95%信頼区間[+472.45, +649.92]であり、異常は0件であった。
段階7の最終構成の+391.57 Elo、95%信頼区間[+339.80, +460.03]より点推定は高いが、得点率が0.96に達しており、この帯ではEloの点推定は少数の敗局に敏感なので、差の大きさは参考値にとどまる。
