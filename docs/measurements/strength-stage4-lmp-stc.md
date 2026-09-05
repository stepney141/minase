# strength-stage4-lmp-stc

## 目的

棋力向上段階4のlate move pruning（残り深さ3以下の非PVノードで、手番号が深さ別の上限以上の静かな手を展開しない）を、採用済みのfutility pruningと標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage4-lmp-stc --seed 20380903 \
  --candidate commit:2643294 --baseline commit:a641083 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット2643294、基準はfutility pruningを採用したコミットa641083、規則セットは`L0,P0,R1,E0`である。
手数の上限は残り深さ1が12、深さ2が23、深さ3が13である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

114ペアを実行し、有効ペア114、破棄ペア0であった。
ペンタノミアル度数は[49, 2, 58, 2, 3]、LLRは−2.947で`decision: H0`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は764秒である。

参考として、候補のbenchは深さ5で総ノード数が1,553,119から1,279,645へ18%減っていた。

## 結論

短時間GSPRTは`H0`であり、late move pruningは採用しない。
実装はコードから外し、手数上限違いの再測定は行わない。
