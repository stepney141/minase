# strength-stage4-nmp-r-stc

## 目的

棋力向上段階4のnull move pruningの減深量の変更（`R = 2 + depth / 6`に、静的評価とβの差を歩兵価値の2倍で割って0以上3以下に切り詰めた値を加える）を、採用済みのfutility pruningと標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage4-nmp-r-stc --seed 20400903 \
  --candidate commit:3c7ee60 --baseline commit:a641083 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット3c7ee60、基準はfutility pruningを採用したコミットa641083、規則セットは`L0,P0,R1,E0`である。
benchの深さ5では、null moveの試行4,165回のうち29%で加算が正であり、深さ7では116,310回のうち49%であった。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

2,314ペアを実行し、有効ペア2,280、破棄ペア34（手数上限）であった。
ペンタノミアル度数は[491, 47, 1219, 41, 482]、LLRは−2.952で`decision: H0`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は16,446秒である。

## 結論

短時間GSPRTは`H0`であり、減深量の変更は採用しない。
候補と基準の得点はほぼ等しく、変更の効果は検出できる大きさになかった。
実装はコードから外す。
