# NNUEの初代（P型、幅128）の段階ゲート　STC

## 目的

[NNUEの共通土台と初代のネットの設計書](../../plans/archived/nnue-replacement.md)のフェーズ6として、フェーズ5で選んだP型の幅128のNNUEを埋め込んだコミットを、基準M（PST＋FM）に対する標準の段階ゲートの短時間条件（STC）で選別する。

## コマンドライン

```console
cargo run --release --bin minase -- match run \
  --run-dir data/matches/nnue-replacement-stc --seed 2300000000 \
  --candidate commit:90280a0 --baseline commit:8d0e382 \
  --each time=10000+100 --concurrency 12 gsprt --max-pairs 3000
```

実行はブランチ`nnue-replacement`のworktreeの`target/release/minase`（runner 2.3.0）で、常駐スクリプト`data/nnue-replacement-train/phase6-stc.sh`から2026年10月12日14時14分に起動した。

## エンジン

- 候補: `90280a0`（P型の学習済みネット（幅128、最良エポック1）を`nets/nnue.bin`に埋め込み、探索が使う評価関数をNNUEにしたコミット。バイナリのSHA-256 `02f25af1…`）。
- 基準: `8d0e382`（基準M。候補のブランチが最後に取り込んだmasterのコミットで、評価関数はPST＋FM。探索は候補と同じ。SHA-256 `2dd1490f…`）。
- 規則セットは`L0,P0,R1,E0`、`USI_Hash`は両者256 MB、`Threads`は1。

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）、メモリ32 GB、同時12ペア。lishogiのBot（Threads 4、同時2局）が同居していた。

## 結果

| 項目 | 値 |
|---|---:|
| 実行ペア数と有効ペア数 | 105、104（手数上限による破棄1） |
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2） | [0, 0, 24, 0, 80] |
| LLR | +2.950 |
| 判定 | `H1` |
| 異常件数 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 終局理由 | 詰み205局、反復2局、王駒の捕獲2局 |
| 経過時間 | 980秒 |

候補は両局とも負けたペアがなく、80ペアを両局とも勝った。
得点率は約0.885であり、Eloに換算するとおよそ+350に相当する大差である。

## 結論

判定は`H1`であり、振分け規則に従って候補をLTC（[記録](nnue-replacement-ltc.md)）へ進めた。
