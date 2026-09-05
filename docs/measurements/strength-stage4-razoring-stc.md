# strength-stage4-razoring-stc

## 目的

棋力向上段階4のrazoring（残り深さ2以下の非PVノードで、静的評価が歩兵価値の4倍以上αを下回るとき静止探索を呼び、その値がα以下ならその値を返す）を、採用済みのfutility pruningと標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage4-razoring-stc --seed 20440903 \
  --candidate commit:3820f5f --baseline commit:a641083 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット3820f5f、基準はfutility pruningを採用したコミットa641083、規則セットは`L0,P0,R1,E0`である。
余裕値は残り深さ1と2とも歩兵価値の4倍である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

157ペアを実行し、有効ペア157、破棄ペア0であった。
ペンタノミアル度数は[64, 3, 73, 3, 14]、LLRは−2.963で`decision: H0`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は1,188秒である。

参考として、候補のbenchは深さ5で総ノード数が1,553,119から1,283,260へ17%減っていた。

## 結論

短時間GSPRTは`H0`であり、razoringは採用しない。
実装はコードから外し、余裕値違いの再測定は行わない。
