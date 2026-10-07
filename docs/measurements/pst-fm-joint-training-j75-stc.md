# pst-fm-joint-training-j75-stc

## 目的

[PSTとFMの同時学習](../plans/pst-fm-joint-training.md)のフェーズ3として、λ=0.75で同時学習した候補J75の採否を、段階ゲートのSTCで測る。

## コマンドライン

```console
data/pst-fm-joint-training/bin/minase-m match run \
  --run-dir data/matches/pst-fm-joint-training-j75-stc --seed 26000000 \
  --candidate commit:4aad6dfdde31425847d1ecf3468002b88f9e43f1 \
  --baseline commit:21d6cd0eb0c3cee465254825af3c0a7c34aa3b14 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

runnerは、ブランチ`pst-fm-joint-training`の`crates/`をビルドして専用の場所へ複製した`minase`（SHA-256 `1d86c156…`）である。
このブランチの`crates/`はM（`21d6cd0`）と同じであり、対局ハーネスのコードも同じである。

## エンジン

候補はブランチ`pst-fm-joint-training-j75`の`4aad6df`（バイナリのSHA-256 `6545bcff…`）であり、PSTとFMをλ=0.75の教師で同時学習した重みを持つ（[学習の記録](pst-fm-joint-training-training.md)）。
基準はM（master `21d6cd0`、バイナリのSHA-256 `412b4f98…`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MBである。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月8日2時43分に開始し、経過時間は10,511秒（約2時間55分）だった。
他の作業の対局は重なっていない。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 1,265（完走1,245、手数上限による破棄20） |
| ペンタノミアル度数 | [298, 6, 666, 6, 269] |
| 正規化得点 | 0.4884 |
| LLR | −2.945 |
| 判定 | `H0` |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

終局理由は、詰み2,450局、反復による勝敗42局と引き分け12局、王駒の捕獲4局、駒枯れ2局、手数上限20局である。

## 結論

STCは`H0`なので、振分け規則に従ってJ75を採用しない。
正規化得点は、ロジスティックのEloに換算して約−8である。
