# strength-stage4-elo200

## 目的

棋力向上段階4の採用構成（futility pruning）が段階開始版に対してどれだけ強くなったかを、固定200ペアのEloで進捗指標として記録する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage4-elo200 --seed 20460903 \
  --candidate commit:a641083 --baseline commit:021fbb3 \
  --each time=10000+100 elo --pairs 200
```

## エンジン

候補はfutility pruningを採用したコミットa641083、基準は段階開始版のコミット021fbb3（探索コードは段階3完了時の20a7f48と同一）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

200ペアを実行し、有効ペア195、破棄ペア5（手数上限）であった。
ペンタノミアル度数は[23, 3, 95, 4, 70]、Eloの点推定は+86.4（95%信頼区間+53.2〜+121.2）であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は1,678秒である。

## 結論

段階4の採用構成は段階開始版に対してSTCで+86.4 Eloである。この測定は進捗指標であり、変更の採否には用いない。
