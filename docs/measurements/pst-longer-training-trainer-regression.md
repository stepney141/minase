# 早期終了を加えた学習器によるG23の再現

## 目的

[採用PSTの追加学習のエポック数の延長](../plans/pst-longer-training.md)のフェーズ1で、学習器に早期終了の打ち切りを加え、最良エポックの選択から学習前の重みを除いた。
この変更が、打ち切りを指定せず最良エポックが1以上となる学習の出力を変えないことを、回帰テストとして確かめる。
この比較は決定性を保証するためのものではなく、変更していない部分を壊していないかの検査である。

## コマンドライン

[前回の再現](eval-coadaptation-trainer-reproduction.md)と同じ引数（G23の学習の引数のうち、初期値を`data/pst-gen3-g23/pst-base.bin`、出力先を新しいディレクトリへ置き換えたもの）を、変更後の学習器へ渡した。
`--patience`は指定していない。
引数の全体は`data/pst-longer-training-reproduction/argv.json`に保存した。

```console
PYTHONPATH=data/worktrees/pst-longer-training/tools/train/src \
  tools/train/.venv/bin/python -m minase_train.pst.train train <argv.jsonの引数>
```

## エンジン

学習器は、ブランチ`pst-longer-training`の作業ツリーにある変更後の`tools/train/src/`である（コミットは本記録と同じ）。
比較対象は、G23（`crates/minase/nets/pst.bin`の変更前の重み、SHA-256 `e5ba50220c7bb484256e798b85f446cb568d81ed3b1925b5afb6c55ae72a4ba2`）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB、GeForce RTX 3060 Ti）で、2026年10月5日19時3分に学習を終えた。
同じ時間帯に対局は走っていなかった。

## 結果

出力の重みはG23とバイト単位で一致した（SHA-256 `e5ba5022…`）。
最良エポックは10、検証損失は0.436731092、初期局面の評価値は27 cp、総更新回数は16,940で、いずれもG23の記録と一致する。
学習前の検証損失0.439711409は、エポック0として`validation`の先頭に記録された。
打ち切りを指定していないので、`training.json`に`patience`と`last_epoch`は書かれていない。

## 結論

変更後の学習器は、最良エポックが1以上で打ち切りを指定しない学習で、従来と同じ重みを出す。
フェーズ2の学習へ進む。
