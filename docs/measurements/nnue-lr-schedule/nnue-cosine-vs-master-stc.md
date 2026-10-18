# 学習率スケジュールcosineのネット対現行masterのネットのSTC

## 目的

[NNUE学習器の学習率スケジュール](../../plans/archived/nnue-lr-schedule.md)で選んだcosine annealing（総長3エポック、下限0.1）のNNUEを、現行masterのネット（以前の教師データで学習率を一定にして学んだネット）の代わりにエンジンへ埋め込んでよいかを、短時間の時間制御のGSPRTで選別する。
学習率スケジュール計画の段階ゲート（cosineのネット対一定1倍のネット、[STC](nnue-lr-schedule-cosine-stc.md)）は、学習率の効果だけを測るものであり、現行のネットとの比較を含まなかった。

## コマンドライン

```console
target/release/minase match run \
  --run-dir data/matches/nnue-cosine-vs-master-stc --seed 2700000000 \
  --candidate commit:a8a7b7fd --baseline commit:446c6e19 \
  --concurrency 16 --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

- 候補: `a8a7b7fd`（ブランチ`nnue-cosine-vs-master`。master `446c6e19`に対し、`crates/minase/nets/nnue.bin`をcosineのネット（SHA-256 `311be801…`）へ差し替えただけのコミット）。
- 基準: `446c6e19`（master。埋め込みのネットは[NNUEの完全代替](../../plans/archived/nnue-replacement.md)で採用したP型の幅128、SHA-256 `3679bebf…`）。
- 規則セットは`L0,P0,R1,E0`、`USI_Hash`は両者256 MB、`Threads`は両者1（既定値）。
- 候補のネットの学習は[学習の記録](nnue-lr-schedule-training.md)にある。基準のネットは、評価関数がPST＋FMだった基準M `8d0e382`で生成した約8,000万局面から学んだものであり、候補のネットは評価関数がNNUEのmaster `31c2a46`で生成し直した約8,000万局面から学んだものなので、この測定は学習率の違いと教師データの世代の違いを合わせて測る。

## 環境

作業機（Core Ultra 7 265KF、物理20コア、論理20コア、メモリ30 GB）、同時16ペア、runnerはminase 2.4.0。
測定の間、lishogiのBotが同じ機械で動いていた。

## 結果

| 項目 | 値 |
|---|---|
| 有効ペア数 | 123 |
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2） | [5, 0, 56, 1, 61] |
| LLR | +2.946（境界+2.944） |
| 判定 | `H1` |
| 破棄ペア数 | 0 |
| 異常件数 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 1,111秒（2026年10月15日22時48分から23時8分） |

## 結論

候補はSTCで`H1`であり、振分け規則によりLTC（[nnue-cosine-vs-master-ltc](nnue-cosine-vs-master-ltc.md)）へ進める。
