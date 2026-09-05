# strength-stage4-rfp-stc

## 目的

棋力向上段階4のreverse futility pruning（残り深さ3以下の非PVノードで、静的評価がβを半歩兵以上上回り、王駒に相手の利きが届かなければ静的評価を返す）を、段階開始版と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage4-rfp-stc --seed 20340903 \
  --candidate commit:495c7f4 --baseline commit:021fbb3 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット495c7f4、基準は段階開始版のコミット021fbb3（探索コードは段階3完了時の20a7f48と同一）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

315ペアを実行し、有効ペア311、破棄ペア4（手数上限）であった。
ペンタノミアル度数は[102, 9, 142, 2, 56]、LLRは−2.980で`decision: H0`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は2,290秒である。

参考として、候補のbenchは深さ5で総ノード数が3,942,412から2,285,612へ42%減り、実行時間は35%短くなったが、NPSは2,701,424から2,413,089へ11%下がっていた。

## 結論

短時間GSPRTは`H0`であり、reverse futility pruningは採用しない。
実装は王駒への利きの判定とともにコードから外し、余裕値違いの再測定は行わない。
