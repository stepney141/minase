# 作業機での小さな1サイクルと統合の確認

## 目的

[学習レシピを先行実装に揃える設計書](../../plans/archived/zero-recipe-precedents.md)の「検証」の節の4点のうち、固定の検証用データの診断、作業機での小さな1サイクル（生成、投了の較正、学習、および出力の照合）、およびαβ探索の`bench`の一致を、フェーズ1〜3の変更を統合したブランチで確かめる。
残る1点の全試験は、Rustのワークスペースの1,948件（ほかに無視44件）がコミット`f607df17`で、学習器と実行器の315件（スキップ1件）と拡張モジュールの22件がコミット`0402a6db`で通った。
`f607df17`より後のコミットは、ワークスペースのRustのコードを変えていない。

## コマンドライン

作業ディレクトリは`tools/train/zero`とし、bashで`env.sh`を読み込んだ。
`<W>`はworktreeのルート、`<S>`は作業用のディレクトリ、`$A`はNASの`/mnt/synology_nas/minase/alphazero`である。
作業機では持ち時間つきの対局の測定が並行して走っていたので、すべて`nice -n 19`で実行し、学習器には`OMP_NUM_THREADS=4`を与え、生成器は`taskset -c 16-19`で4コアに限った。

固定の検証用データの診断は、NASの新しい初期ネットのチェックポイントで行った。

```console
uv run --frozen python -m minase_zero external-eval --checkpoint $A/cycles-rl2-az/cycle-0000/checkpoint.pt \
  --data $A/validation/zero-validation-games.mnsd --device cuda --out <S>/external-init.json
```

小さな1サイクルの確認では、実行器がサイクルごとに行う工程を、作業機で同じ順に手で実行した。
サイクル1の投了の閾値は、過去の集計なしの較正（`rl.resign_calibration([])`）の値0.0である。

```console
<W>/target/release/minase data zero-selfplay --output-dir <S>/selfplay \
  --nn-model $A/cycles-rl2-az/cycle-0000/model.onnx --nn-backend tensorrt --generation-commit <コミット> \
  --start-position initial --seed 1300000000 --games 400 --target-positions 8192 --chunk-games 20 \
  --simulations 32 --resign-threshold 0.0 --threads 2 --devices 1 --games-per-thread 100
uv run --frozen python -m minase_zero resign-calibration --summary <S>/selfplay/summary.json
uv run --frozen torchrun --standalone --nproc-per-node 1 -m minase_zero train \
  --init $A/cycles-rl2-az/cycle-0000/checkpoint.pt --data-list <S>/train-data.json --latest-data <塊4つ> \
  --out-dir <S>/train --cycle 1 --seed 20261101 --batch-size 1024 --micro-batch-size 256 --lr 0.04 --momentum 0.9 \
  --weight-decay 0.0001 --clip-grad-norm 10 --value-lambda 0.333 --precision bf16 --checkpoint-steps 50 \
  --device cuda --replay-threads 4 --trainer-commit <コミット>
uv run --frozen python -m minase_zero fixture-eval --checkpoint <S>/train/checkpoint.pt --onnx <S>/train/model.onnx \
  --fixture <W>/crates/minase/tests/fixtures/nn-positions.jsonl --out <S>/train/check/pytorch.json
bash <W>/docs/measurements/zero-recipe-precedents/zero-recipe-precedents-initial-net-check/check.sh \
  <S>/train/model.onnx <S>/train/check/pytorch.json <S>/train/check-workstation
```

`bench`は、masterのコミット`0c0e3647`を`git archive`で展開してビルドしたminaseと、ブランチのminaseで、同じ1つのコアに固定して実行し、経過時間と毎秒ノード数を除いた全行を比べた。

```console
printf 'bench depth 6\nquit\n' | taskset -c 19 <minase> --protocol usi
```

## エンジン

生成器、学習器、および実行器は、ブランチ`zero-recipe-precedents`のコミット`94aa98a1`（生成）と`0402a6db`（学習と照合）である。
2つのコミットの違いは実行器と抽出の関数の整理であり、生成器のRustのコードは同じである。
初期ネットは`$A/cycles-rl2-az/cycle-0000/model.onnx`（SHA-256 `4b430a4414d250f96094c475dc489efbc865afa20e1c9bfc5b2a958cae4abf07`、[初期ネットの照合](zero-recipe-precedents-initial-net-check.md)）であり、規則セットは`JCSA`である。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、NVIDIA GeForce RTX 3060 Ti、ドライバ615.78.08）で、ONNX Runtime 1.31.1とTensorRT 10.14.1を使った。

## 結果

固定の検証用データの診断は、811局、164,504局面で動き、手数の帯ごとの局面数（1〜40手目が32,440、41〜100手目が44,978、101〜200手目が54,073、201手目以降が33,013）は[基準の記録](../zero-validation-games/zero-validation-games-baseline.md)と一致した。
初期ネットの価値の損失は0.6934、一致率は0.0162であった。

生成では、20局ずつの塊のうち塊0〜3の80局で訓練局面が8,584となって目標の8,192に届き、生成器がサイクルを締めた。
探索した手番は8,584で、すべてが訓練局面であった。
投了の閾値が0なので投了は起きず、80局すべてが王手限定の3手以内の勝ちによる打ち切りで終わった。
投了禁止の対局は対局番号1、11、…、71の8局で、集計の`resign_calibration_games`もこの8局を持っていた。
この集計から求めた次のサイクルの閾値は0.0、取り込んだ対局は8局、最も小さい候補の推定は0.0998（0.10×(4,999/5,000)⁸）であった。

学習では、8,584局面のうち検証用の対局の497局面を除く8,087局面から8,087局面を抽出し、全体のバッチ1,024で7回更新した。
累計の更新数は0から7になり、最後の学習率は0.00112（0.04×7/250）であった。
学習の時間は15.9秒（コンパイルを含む）であった。
検証集合497局面の損失は、方策9.310、価値0.6938であった。

学習したネット（SHA-256 `61fd8b5cee042d673dc78ea73baa22e63c76f2fd0b9184e0f019e9adf64536b0`）は、CPUとTensorRTのバッチ1、7、256の6通りの照合にすべて通った。
TensorRTのFP16の差の最大は、合法手の確率で5.46×10⁻⁴、価値で8.26×10⁻⁴であった。

`bench`の深さ6の15局面は、ノード数、最善手、評価値、および合計のノード数（56,937）が、masterとブランチで一致した。

## 結論

統合したブランチは、固定の検証用データの診断、作業機での小さな1サイクル、およびαβ探索の`bench`の一致を満たした。
