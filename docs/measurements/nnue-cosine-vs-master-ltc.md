# 学習率スケジュールcosineのネット対現行masterのネットのLTC

## 目的

[STC](nnue-cosine-vs-master-stc.md)で`H1`となった、cosine annealing（総長3エポック、下限0.1）で学んだNNUEを埋め込んだ候補を、現行masterのネットを埋め込んだ基準に対して長時間の時間制御のGSPRTで測り、採否を決める。

## コマンドライン

```console
target/release/minase match run \
  --run-dir data/matches/nnue-cosine-vs-master-ltc --seed 2700100000 \
  --candidate commit:a8a7b7fd --baseline commit:446c6e19 \
  --concurrency 16 --each time=60000+200 gsprt
```

## エンジン

STCと同じである。
候補は`a8a7b7fd`（master `446c6e19`に対し、`crates/minase/nets/nnue.bin`をcosineのネット（SHA-256 `311be801…`）へ差し替えただけのコミット）、基準は`446c6e19`（master、埋め込みのネットのSHA-256は`3679bebf…`）、規則セットは`L0,P0,R1,E0`、`USI_Hash`は両者256 MB、`Threads`は両者1である。

## 環境

作業機（Core Ultra 7 265KF、物理20コア、論理20コア、メモリ30 GB）、同時16ペア、runnerはminase 2.4.0。
測定の間、lishogiのBotが同じ機械で動いていた。

## 結果

| 項目 | 値 |
|---|---|
| 有効ペア数 | 143 |
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2） | [13, 0, 43, 1, 86] |
| LLR | +2.971（境界+2.944） |
| 判定 | `H1` |
| 破棄ペア数 | 0 |
| 異常件数 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 3,447秒（2026年10月15日23時9分から2026年10月16日0時9分） |

## 結論

候補はLTCで`H1`であり、異常、時間切れ、拒否着手が0件なので、cosineのネットをエンジンの埋め込みネットとして採用する。
