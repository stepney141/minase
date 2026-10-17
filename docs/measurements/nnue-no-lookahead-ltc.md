# 先読みなしの教師で学んだNNUEのLTC

## 目的

[NNUEの検証損失の頭打ちの切り分け](../plans/archived/nnue-early-best-epoch.md)のフェーズ3として、[STC](nnue-no-lookahead-stc.md)で`H1`となった先読みなしのネット（候補）を、長時間条件（LTC）のGSPRTで現行のmasterと比べ、採否を決める。

## コマンドライン

STCと同じく、ryzen0の`/var/tmp/minase-early-best`から、同時対局数を`--concurrency 30`で明示して実行した。
シードは、STCの基本シード3400000000からペア数より十分に離した値である。

```console
target/release/minase match run --run-dir data/matches/nnue-no-lookahead-ltc --seed 3410000000 \
  --candidate commit:872f7d6bb4b9a59cbe275632451824c809904792 \
  --baseline commit:b7d91bc24e1d69a68c6d4d522c9c11490538ad56 \
  --concurrency 30 --each time=60000+200 gsprt
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
| 有効ペア数 | 237（破棄4） |
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2） | [31, 3, 116, 1, 86] |
| 候補の得点率 | 61.4%（ロジスティックEloで約+81） |
| LLR | +2.956 |
| 判定 | `H1`（候補が有意に強い） |
| 異常件数 | 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手のいずれも0 |
| 累計実行時間 | 3,284秒（約55分、`summary.json`の`active_wall_time_ns`） |

破棄した4ペア（ペア番号6、20、82、154）は、いずれも片方の局が手数上限4,096手に達して打ち切られたものであり、もう片方の局はすべて詰みで終局した。
得点率とEloは参考であり、手引きの規定により採否には使わない。

## 結論

候補はLTCで`H1`となり、異常件数、時間切れ、および拒否着手がいずれも0件なので、[SPRTの手引き](../guides/sprt.md)の規定により採用する。
