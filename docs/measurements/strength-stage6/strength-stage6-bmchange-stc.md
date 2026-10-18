# strength-stage6-bmchange-stc

## 目的

棋力向上段階6の最善手交替時の延長（完了した反復の最善手が直前の完了反復と異なる場合、直後の反復継続の判断1回だけsoftの条件を外す変更）を、直前の構成（internal iterative reductionまで）と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-bmchange-stc --seed 20650903 \
  --candidate commit:ad0d9fe --baseline commit:9d37246 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

候補はコミットad0d9fe（fail-lowによる延長を外し、最善手交替時の延長だけを含む構成）、基準はinternal iterative reductionを含むコミット9d37246（暫定の基準）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した16である。
他の対局や診断は走らせていない。

## 結果

718ペアを実行し、有効ペア700、破棄ペア18（手数上限）であった。
ペンタノミアル度数は[170, 14, 371, 8, 137]、LLRは−2.960で`decision: H0`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は6,571秒である。

## 結論

短時間GSPRTは`H0`であり、最善手交替時の延長は不採用とする。
実装はコミットc8c38bdで外し、反復継続の判断を段階2の2引数の規則へ戻した。
最善手安定時の早期終了は、延長の信号を持たない形（コミット8f4e41c）で改めて実装して測る。
