# strength-stage5-piece-history-stc

## 目的

棋力向上段階5の駒種と到達升で引くhistory（手番側・着手前の駒種・到達升のpiece-to表を加え、butterfly表との合計を順序付けに使う変更）を、直前の採用構成と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage5-piece-history-stc --seed 20480903 \
  --candidate commit:6084078 --baseline commit:7877531 \
  --each time=10000+100 --concurrency 4 gsprt --max-pairs 3000
```

## エンジン

候補はコミット6084078、基準は順序付けキーの重複計算の除去を採用したコミット7877531、規則セットは`L0,P0,R1,E0`である。
benchの発動率（静かな手の順序付けでpiece-to表の値が0でない手の割合）は深さ5で6.7%であった。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した4である。
測定開始時に別の時間制御測定（`Threads=4`対2のLTC、同時対局数4）が同じ機械で並行していたため同時対局数を4に抑え、再開時の条件一致の制約からその後も変えなかった。
その測定は約4時間後に停止し、以後は本測定の他にmalusとLMRのSTC・LTC（同時対局数14）が並行した。

## 結果

1,777ペアを実行し、有効ペア1,750、破棄ペア27（手数上限）であった。
ペンタノミアル度数は[387, 26, 942, 31, 364]、LLRは−3.004で`decision: H0`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は59,161秒である。

## 結論

短時間GSPRTは`H0`であり、駒種と到達升で引くhistoryは不採用とする。
