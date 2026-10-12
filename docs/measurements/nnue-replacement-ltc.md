# NNUEの初代（P型、幅128）の段階ゲート　LTC

## 目的

[NNUEの共通土台と初代のネットの設計書](../plans/nnue-replacement.md)のフェーズ6として、[STC](nnue-replacement-stc.md)で`H1`となったP型の幅128のNNUEを埋め込んだコミットの採否を、基準M（PST＋FM）に対する長時間条件（LTC）で判定する。

## コマンドライン

```console
cargo run --release --bin minase -- match run \
  --run-dir data/matches/nnue-replacement-ltc --seed 2400000000 \
  --candidate commit:90280a0 --baseline commit:8d0e382 \
  --each time=60000+200 --concurrency 12 gsprt
```

実行はブランチ`nnue-replacement`のworktreeの`target/release/minase`（runner 2.3.0）で、STCの判定を待つ常駐スクリプト`data/nnue-replacement-train/phase6-ltc.sh`から2026年10月12日15時09分に起動した。シードはSTCの基本シード（2,300,000,000）から1億離した未使用の値である。

## エンジン

- 候補: `90280a0`（P型の学習済みネット（幅128、最良エポック1）を`nets/nnue.bin`に埋め込み、探索が使う評価関数をNNUEにしたコミット。バイナリのSHA-256 `02f25af1…`）。
- 基準: `8d0e382`（基準M。評価関数はPST＋FM、探索は候補と同じ。SHA-256 `2dd1490f…`）。
- 規則セットは`L0,P0,R1,E0`、`USI_Hash`は両者256 MB、`Threads`は1。

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）、メモリ32 GB、同時12ペア。lishogiのBot（Threads 4、同時2局）が同居していた。

## 結果

| 項目 | 値 |
|---|---:|
| 実行ペア数と有効ペア数 | 105、104（手数上限による破棄1） |
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2） | [0, 0, 17, 0, 87] |
| LLR | +2.950 |
| 判定 | `H1` |
| 異常件数 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 終局理由 | 詰み208局、反復1局 |
| 経過時間 | 3,138秒 |

候補は両局とも負けたペアがなく、87ペアを両局とも勝った。
得点率は約0.918であり、Eloに換算するとおよそ+420に相当する。STC（得点率約0.885）より差が広がった。

## 結論

判定は`H1`で異常は0件であり、[測定の手引き](../guides/sprt.md)の採用条件を満たす。
P型の幅128のNNUEを埋め込んだ`90280a0`を採用し、探索が使う評価関数をPST＋FMからNNUEへ切り替える。
