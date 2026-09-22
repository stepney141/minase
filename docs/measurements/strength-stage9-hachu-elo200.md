# 棋力向上段階9の最終構成の固定200ペア（HaChuとの比較）

## 目的

[棋力向上段階9](../plans/strength-stage9.md)のフェーズ8として、段階9の完了処理後の最終構成をHaChuと固定200ペアで比較し、進捗指標として記録する。採否には使わない。

## コマンドライン

対局ハーネスは、masterの `4f946d4` でビルドしたバイナリから起動した。

```console
target/release/match_runner \
  --run-dir data/matches/strength-stage9-hachu-elo200 --seed 69000921 \
  --candidate commit:4f946d4d19e70110796f754a0258f0a778f59cd8 \
  --baseline cecp:/home/stepney141/board-games/hachu-debian/hachu \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 10 \
  --max-ply 4096 --response-timeout 120 \
  elo --pairs 200
target/release/match_report --run-dir data/matches/strength-stage9-hachu-elo200
```

## エンジン

候補は `4f946d4`（[段階開始版との比較](strength-stage9-elo200.md)と同じ構成）である。
HaChuはDebianパッケージ収録のオリジナル版（コミット `df26f4a`）を段階8と同じ手順でビルドしたもので、規則は既定設定に対応する `L1,L3,P0,P5,P6,R2,E1,E2` を審判層とminaseの双方に与えた。
両エンジンの置換表は256 MB、minaseは `Threads=1`、HaChuは探索ワーカー数を報告しない。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数10、他の負荷はない。
2026年9月23日1時27分から5時41分に実施した。

## 結果

| 量 | 値 |
|---|---:|
| 有効ペア | 200（破棄0） |
| ペンタノミアル度数 | [2, 0, 24, 0, 174] |
| 正規化した平均得点 | 0.930 |
| Elo | +449.4、95%区間[+389.6, +534.7] |
| エンジン異常 | 不正着手1、拒否着手4（いずれもHaChu側）、クラッシュ0、応答タイムアウト0、時間切れ0 |
| 終局の理由 | 投了366、王駒の捕獲29、不正着手1、拒否着手4 |
| 総CPU時間 | 148,896秒 |
| 経過時間 | 15,275秒（累計15,269秒） |

異常5件はすべてHaChu側であり、minaseの勝ちとして算入した。
不正着手はペア32の第1局（1,687手目、HaChuが審判層で不合法な手を返した）、拒否着手はペア44の第1局（11手目）と第2局（12手目）、ペア74の第2局（1,126手目）、ペア117の第2局（1,676手目）で、審判層が合法としたminaseの着手をHaChuが `Illegal move` で拒否した。
段階8の比較でもHaChu側の異常が2件あり、同じ扱いで算入している。
ペア44の2局が序盤の同じ開始局面で連続して拒否された点は、規則の解釈の差が疑われるが、本記録では原因を調べていない。

## 結論

最終構成はHaChuに対して+449.4 Elo（95%区間[+389.6, +534.7]）であり、[段階8の最終構成](strength-stage8-hachu-elo200.md)の+541.1 Eloより低い点推定だが、区間は重なる。
HaChu側の拒否着手4件は反則負けとして算入しており、その原因の調査は本段階の範囲外として残す。
