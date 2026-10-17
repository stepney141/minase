# 学習率スケジュールcosineのネット対一定1倍のネットのSTC

## 目的

[NNUE学習器の学習率スケジュール](../../plans/archived/nnue-lr-schedule.md)のフェーズ3として、cosine annealing（総長3エポック、下限0.1）で学んだNNUEを埋め込んだ候補が、同じデータ・初期値・シードで学習率を一定（1倍）にして学んだNNUEを埋め込んだ基準より強いかを、短時間の時間制御のGSPRTで選別する。

## コマンドライン

```console
target/release/minase match run \
  --run-dir data/matches/nnue-lr-schedule-cosine-stc --seed 2600000000 \
  --candidate commit:853d252 --baseline commit:7ad1ee0 \
  --concurrency 16 --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

- 候補: `853d252`（ブランチ`nnue-lr-schedule-cosine`。基準のコミットに対し、`crates/minase/nets/nnue.bin`をcosineのネット（SHA-256 `311be801…`）へ差し替えただけのコミット）。
- 基準: `7ad1ee0`（ブランチ`nnue-lr-schedule`。master `31c2a46`に学習器の変更と文書を加え、`crates/minase/nets/nnue.bin`を一定1倍のネット（SHA-256 `ab8eb0a1…`）へ差し替えたコミット）。
- 規則セットは`L0,P0,R1,E0`、`USI_Hash`は両者256 MB、`Threads`は両者1（既定値）。
- 2つのネットの学習は[学習の記録](nnue-lr-schedule-training.md)にある。

## 環境

作業機（Core Ultra 7 265KF、物理20コア、論理20コア、メモリ30 GB）、同時16ペア、runnerはminase 2.4.0。
測定の間、lishogiのBotが同じ機械で動き、途中からcodexによる探索テストの書き換え（`nice -n 19`、並列度2）とそのテストの実行も重なった。

## 結果

| 項目 | 値 |
|---|---|
| 有効ペア数 | 281 |
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2） | [42, 2, 138, 3, 96] |
| LLR | +2.946（境界+2.944） |
| 判定 | `H1` |
| 破棄ペア数 | 2（手数上限4,096への到達。283ペア中） |
| 異常件数 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 2,417秒（2026年10月15日22時5分から22時45分） |

## 結論

候補はSTCで`H1`であり、振分け規則によりLTCへ進める。
ただし利用者の判断（2026年10月15日）により、このゲートのLTCは省略し、採否は現行masterのネットとの段階ゲート（[STC](nnue-cosine-vs-master-stc.md)）で決める。
