# pst-fm-joint-training-j100-stc

## 目的

[PSTとFMの同時学習](../plans/pst-fm-joint-training.md)のフェーズ3として、λ=1.0で同時学習した候補J100の採否を、段階ゲートのSTCで測る。

## コマンドライン

```console
data/pst-fm-joint-training/bin/minase-m match run \
  --run-dir data/matches/pst-fm-joint-training-j100-stc --seed 28000000 \
  --candidate commit:b7420e41639058e577611c9817c1dd14e2b373c8 \
  --baseline commit:21d6cd0eb0c3cee465254825af3c0a7c34aa3b14 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

runnerは、[J75のSTC](pst-fm-joint-training-j75-stc.md)と同じ`minase`（SHA-256 `1d86c156…`）である。

## エンジン

候補はブランチ`pst-fm-joint-training-j100`の`b7420e4`（バイナリのSHA-256 `06833944…`）であり、PSTとFMをλ=1.0の教師で同時学習した重みを持つ（[学習の記録](pst-fm-joint-training-training.md)）。
基準はM（master `21d6cd0`、バイナリのSHA-256 `412b4f98…`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MBである。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月8日5時39分に開始し、経過時間は2,786秒（約46分）だった。
他の作業の対局は重なっていない。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 330（完走328、手数上限による破棄2） |
| ペンタノミアル度数 | [98, 3, 169, 4, 54] |
| 正規化得点 | 0.4337 |
| LLR | −2.951 |
| 判定 | `H0` |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

終局理由は、詰み630局、反復による勝敗20局と引き分け7局、駒枯れ1局、手数上限2局である。

## 結論

STCは`H0`なので、振分け規則に従ってJ100を採用しない。
正規化得点は、ロジスティックのEloに換算して約−46である。
