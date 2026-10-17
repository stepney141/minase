# 先読みなしの教師で学んだNNUEのSTC

## 目的

[NNUEの検証損失の頭打ちの切り分け](../plans/archived/nnue-early-best-epoch.md)のフェーズ3として、現行の埋め込みネットD0bの学習コマンドから先読み教師の引数だけを外して学んだネット（候補）を、短時間条件（STC）のGSPRTで現行のmasterと比べ、長時間条件（LTC）へ進めるかを決める。

## コマンドライン

候補のネットの学習は[先読みなしの学習の記録](nnue-no-lookahead-training.md)にある。
STCは、ryzen0の`/var/tmp/minase-early-best`（masterと候補のブランチをgit bundleで移したクローン）から実行した。
同時対局数は、runnerがこのCPUの物理コア数を8と誤って判定する（[HalfKPの再学習の記録](halfkp-retrain-stc.md)）ので、`--concurrency 30`で明示した。

```console
target/release/minase match run --run-dir data/matches/nnue-no-lookahead-stc --seed 3400000000 \
  --candidate commit:872f7d6bb4b9a59cbe275632451824c809904792 \
  --baseline commit:b7d91bc24e1d69a68c6d4d522c9c11490538ad56 \
  --concurrency 30 --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

- 候補: `872f7d6b`（ブランチ`nnue-no-lookahead`）。master `b7d91bc2`に対し、`crates/minase/nets/nnue.bin`を先読みなしで学んだネット（SHA-256 `faa5fbe2…`）へ差し替えただけのコミットである。バイナリのSHA-256は`bbb3b076…`。
- 基準: master `b7d91bc2`。埋め込みのネットはD0b（SHA-256 `046be0e6…`）である。バイナリのSHA-256は`3c5a56c7…`。
- 規則セットは`L0,P0,R1,E0`、`USI_Hash`は両者256 MB、`Threads`は1。

## 環境

ryzen0（AMD Ryzen Threadripper 2990WX、物理32コア、論理64コア、メモリ125 GB）で、2026年10月17日に行った。ほかの負荷はなかった。
runnerはmaster `b7d91bc2`のビルド（SHA-256 `1b53c4d2…`）であり、同時対局数は30である。

## 結果

| 項目 | 値 |
|---|---:|
| 有効ペア数 | 210（破棄4） |
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2） | [24, 1, 108, 3, 74] |
| 候補の得点率 | 62.1%（ロジスティックEloで約+86） |
| LLR | +2.947 |
| 判定 | `H1`（候補が有意に強い） |
| 異常件数 | 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手のいずれも0 |
| 累計実行時間 | 958秒（約16分、`summary.json`の`active_wall_time_ns`） |

破棄した4ペア（ペア番号14、32、36、94）は、いずれも2局目が手数上限4,096手に達して打ち切られたものであり、1局目はすべて詰みで終局した。手数上限での打ち切りは、手引きの規定により実力と無関係として当該ペアごと破棄される。
得点率とEloは参考であり、手引きの規定により採否には使わない。

## 結論

候補はSTCで`H1`となったので、[SPRTの手引き](../guides/sprt.md)の振分け規則に従ってLTCへ進める。
