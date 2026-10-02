# 現行の学習器による採用PSTの再現

## 目的

[世代3の計画](../plans/pst-gen3.md)の候補の学習に使う現行の学習器が、採用PSTを学習したときの条件（採用工程）で、採用PSTの重みをバイト単位で再現するかを確かめる。
採用PSTは、学習器を `tools/train/pst/` に持つコミット `e783dc8` で学習したものであり、その後のmasterでは学習器のソースが変わっている。

## コマンドライン

学習は、[学習シードの散らばりの測定](pst-seed-spread-nodes.md)と同じ `data/pst-seed-spread/train_seeds.sh` を、学習器の場所と出力先だけを変えて実行した。

```console
TRAINER_DIR=$PWD/data/worktrees/trainer-44ae7cb OUT=$PWD/data/trainer-repro \
  OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 \
  data/pst-seed-spread/train_seeds.sh 1
cmp data/trainer-repro/seed1/pst.bin nets/pst.bin
```

学習の引数は、入力の11ファイル、初期値（段階開始版の重み）、出力K 1072.6529541015625、鏡映共有モデル、学習率0.03、10エポック、バッチ16,384、乱数シード1、駒除去差分の追加損失の係数1000、先読み教師（γ=0.9、n=40）であり、[先読み教師での再学習](lookahead-teacher-pst-training.md)の記録と同じである。

## エンジン

学習器は、本計画の起案時のmaster `44ae7cb` に固定したworktree `data/worktrees/trainer-44ae7cb` のものである。
比較対象は、同じmasterの `nets/pst.bin`（SHA-256 `72d7aee949e7672a2b4fb2e7b98ddaed76b352112187bfbcc7e2d772224fb67b`）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB、GeForce RTX 3060 Ti）で、2026年10月2日11時42分から12時01分に実施した。
同じ時間帯に世代3の生成（同時16対局）が走っていた。

## 結果

出力の重みは採用PSTとバイト単位で一致した（SHA-256 `72d7aee9…`）。
最良エポックは10、検証損失は0.473404901であり、採用時の記録と一致する。

## 結論

現行の学習器は採用工程で採用PSTを再現するので、世代3の候補と半分割の学習にこの学習器（と、省略時の出力が変わらないことをテストで固定した半分割オプション）を使ってよい。
