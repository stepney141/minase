# strength-stage6-elo200

## 目的

棋力向上段階6の最終構成（aspiration windows、internal iterative reduction、最善手安定時の早期終了を採用）が段階開始版に対してどれだけ強くなったかを、固定200ペアのEloで進捗指標として記録する。
採否の判定には使わない。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-elo200 --seed 20690903 \
  --candidate commit:8f4e41c --baseline commit:75a08de \
  --each time=10000+100 --concurrency 12 elo --pairs 200
```

## エンジン

候補は段階6の最終構成のコミット8f4e41c、基準は段階開始版のコミット75a08de（探索コードは段階5完了後の156816dと同一）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した12（外部のメモリ圧迫による時間切れを避けるため下げた）である。
開始から約7分は秒読みの煙試験（同時対局数4）が並行していた。

## 結果

200ペアを実行し、有効ペア194、破棄ペア6（手数上限）であった。
ペンタノミアル度数は[16, 3, 84, 9, 82]、Eloは+129.2、95%信頼区間は[+95.2, +165.8]である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は2,404秒である。

## 結論

段階6の最終構成は段階開始版に対してSTCで+129.2 Eloであり、進捗指標として記録する。
