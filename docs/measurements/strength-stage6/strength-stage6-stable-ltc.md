# strength-stage6-stable-ltc

## 目的

棋力向上段階6の最善手安定時の早期終了（[STC](strength-stage6-stable-stc.md)で`H1`）を、直前の構成（internal iterative reductionまで）と標準LTCのGSPRTで比較し、採否を確定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-stable-ltc --seed 20670903 \
  --candidate commit:8f4e41c --baseline commit:9d37246 \
  --each time=60000+200 --concurrency 16 gsprt
```

## エンジン

候補はコミット8f4e41c、基準はinternal iterative reductionを含むコミット9d37246（暫定の基準）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した16である。
他の対局や診断は走らせていない。

## 結果

585ペアを実行し、有効ペア574、破棄ペア11（手数上限）であった。
ペンタノミアル度数は[92, 19, 300, 19, 144]、LLRは+2.957で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は16,058秒である。

## 結論

長時間GSPRTは`H1`かつ異常0件であり、最善手安定時の早期終了を採用する。
