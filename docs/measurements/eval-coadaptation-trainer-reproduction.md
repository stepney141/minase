# 現行の学習器によるG23の再現

## 目的

[共適応の検証](../plans/eval-search-coadaptation.md)のフェーズ1として、候補Pλの学習に使う現行の学習器が、採用PSTであるG23を学習した条件で、G23の重みをバイト単位で再現するかを確かめる。
G23は、学習器を `tools/train/pst/` に持つブランチ`pst-gen3`のコミット`bbb3b3c`で学習したものであり、その後のmasterでは学習器がパッケージ`tools/train/src/minase_train/`へ再編された。

## コマンドライン

G23の実行ディレクトリ`data/pst-gen3-g23/commands.jsonl`が記録した学習の引数のうち、初期値と出力先だけを置き換え、現行の学習コマンド`train-pst`へ渡した。
置き換えた後の引数の全体は`data/eval-coadaptation-reproduction/argv.json`に保存した。

```console
tools/train/.venv/bin/train-pst train \
  --data <世代2の5ファイル> <世代3の5ファイル> \
  --init data/pst-gen3-g23/pst-base.bin \
  --output data/eval-coadaptation-reproduction/training/pst.bin \
  --k 1072.6529541015625 --model mirrored --lr 0.03 --epochs 10 --batch 16384 \
  --seed 1 --removal-penalty 1000.0 --device cuda --validation-sample 10000 \
  --rescore - - - - - - - - - - --lookahead-gamma 0.9 --lookahead-plies 40
```

入力は、`data/strength-stage7/gen2/generated-600000.bin`から`generated-1000000.bin`までの5ファイルと、`data/gen3/generated-1100000.bin`から`generated-1500000.bin`までの5ファイルであり、[G23の学習](pst-gen3-training.md)と同じである。
初期値`data/pst-gen3-g23/pst-base.bin`のSHA-256は`72d7aee949e7672a2b4fb2e7b98ddaed76b352112187bfbcc7e2d772224fb67b`であり、`44ae7cb`の`nets/pst.bin`と一致する。

## エンジン

学習器は、master `4fb1582`（本計画の基点M）の`tools/train/`である。
比較対象は、Mの`crates/minase/nets/pst.bin`（SHA-256 `e5ba50220c7bb484256e798b85f446cb568d81ed3b1925b5afb6c55ae72a4ba2`）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB、GeForce RTX 3060 Ti）で、2026年10月4日20時52分から21時14分に実施した。
同じ時間帯に他の測定は走っていなかった。

## 結果

出力の重みはG23とバイト単位で一致した（SHA-256 `e5ba5022…`）。
最良エポックは10、検証損失は0.436731092であり、[G23の学習](pst-gen3-training.md)の記録と一致する。
学習ログの初期局面の評価値は27 cpであり、Mの`crates/minase/src/eval/pst/tests.rs`の期待値と一致する。

## 結論

現行の学習器はG23の学習条件でG23を再現するので、Pλの学習にこの学習器を使ってよい。
