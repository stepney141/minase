# strength-stage3-see-ltc

## 目的

棋力向上段階3の`attackers_to`とSEE（静止探索でSEEが負の捕獲手を展開しない）を、採用済みの駒価値の再調整と標準LTCのGSPRTで比較し、採否を確定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage3-see-ltc --seed 20310903 \
  --candidate commit:1894d14 --baseline commit:6b4c460 \
  --each time=60000+200 gsprt
```

## エンジン

候補はコミット1894d14、基準は駒価値の再調整を採用したコミット6b4c460、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

986ペアを実行し、有効ペア975、破棄ペア11（手数上限）であった。
ペンタノミアル度数は[185, 20, 502, 24, 244]、LLRは+3.001で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は22,732秒である。

## 結論

長時間GSPRTは`H1`かつ異常0件であり、`attackers_to`とSEEを採用する。
