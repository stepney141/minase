# 教師局面を静止探索の末端へ置き換えたNNUEの学習

## 目的

[NNUEの教師局面を静止探索の末端へ置き換える比較](../../plans/archived/nnue-qsearch-leaf.md)のフェーズ2として、教師局面を静止探索の主要変化の末端へ置き換えて候補のネットを学び（qsearch leaf replacement）、標準の3診断で対局へ進めるかを確かめる。
棋力の測定ではなく、対局の勝敗を使わない。

## コマンドライン

`$F`は`data/nnue-lr-schedule-gen/generated-*.bin`の30ファイルを名前順に並べた配列であり、基準の学習と同じ順である。
末端を選ぶネットには、基準の補正前のネット`data/nnue-qsearch-leaf/baseline-nnue.bin`（`git show 6d47da2a:crates/minase/nets/nnue.bin`、SHA-256 `5257f374…`）を使った。
引数は、`--qsearch-leaf-net`を除いて[補正前のネットの学習](../nnue-material-linear-init/nnue-material-linear-init-training.md)と同じである。

```console
D=data/nnue-qsearch-leaf
systemd-run --user --scope --quiet --unit=nql-train -p MemoryMax=16G -p MemorySwapMax=0 \
  nice -n 19 /usr/bin/time -v -o $D/train.time \
  $D/minase-trainer-leaf nnue train --feature-set P --width 128 --data $F \
  --qsearch-leaf-net $D/baseline-nnue.bin \
  --base crates/minase/nets/pst.bin --lambda 1.0 \
  --batch 16384 --epochs 50 --patience 5 --seed 1 --threads 4 --device cuda \
  --lr-schedule cosine:3:0.1 --teacher-cp-per-logit calibrate --validation-interval 1000 \
  --output-dir $D/train
```

学習は、エポック6の完了後に、機械全体のメモリ不足によってClaude Codeの機構に止められた。
機械の空きを確かめてから、同じ引数に`--resume $D/train`を加え、`systemd-run --user`の独立したサービスとして再開した。

診断は[学習の手引き](../../guides/nnue-training.md)の8節の3つのコマンドで、基準の学習の記録と同じ標本の条件で行った。

```console
$D/minase-trainer-leaf diag nnue-distribution \
  --pair baseline crates/minase/nets/pst.bin $D/baseline-nnue.bin \
  --pair candidate crates/minase/nets/pst.bin $D/train/nnue.bin \
  --data $F --sample 20000 --seed 1 --output $D/diag/distribution.json
$D/minase-trainer-leaf diag nnue-removal --float $D/train/best.npz --net $D/train/nnue.bin \
  --data $F --sample 10000 --seed 1 --output $D/diag/removal.json
$D/minase-trainer-leaf diag nnue-moves --net $D/train/nnue.bin --base crates/minase/nets/pst.bin \
  --positions data/out-of-core-shuffle/teacher/lishogi.bin --output $D/diag/moves.json
```

## エンジン

学習器は、ブランチ`nnue-qsearch-leaf`のコミット`036e587e`を`cargo build --release --locked -p minase-trainer --features qsearch-leaf`でビルドしたもの（SHA-256 `43316099…`）である。
起点のmaster `48fa73da`と基準の学習器`7e720567`の間で、`crates/minase-trainer/`の`src/nnue/train`、`src/data`、`src/nnue/gpu`、`src/nnue/model`、および`kernels`に差分はない。
機能フラグなしのビルドの深さ5のbenchは、経過時間と速度を除いて`48fa73da`と全行一致した。

## 環境

測定機（Intel Core Ultra 7 265KF、20コア、メモリ30 GB、RTX 3060 Ti）で、2026年11月3日から4日に行った。
同じ機械では、lishogiのBotと別の計画の実装および学習が動いていた。

## 結果

### 末端の取り出し

全30ファイルの根のすべてで、末端の静的評価に$`(-1)^k`$を掛けた値が静止探索の値と整数で一致した。

| 項目 | 件数 |
|---|---:|
| 取り出しを実行した根 | 99,714,695 |
| 1手以上進んだ根 | 16,672,853（16.7%） |
| 教師へ戻した戦術的な局面 | 16,103,310 |
| 戦術的かつ反復で除いた局面 | 1,629 |
| 詰みで除いた根 | 0 |
| 根から末端までの最大の手数 | 18 |

訓練は94,758,533局面、検証は4,956,162局面である。
教師の勝率尺度は1,047.7 cpであり、基準の1,039.6 cpとほぼ同じである。

### 学習の経過

最良はエポック15（検証損失0.455919）で、エポック20で早期終了した。
再開後の経過時間は1時間35分26秒、再開後の最大常駐メモリは12.7 GBだった。
検証の局面も末端へ置き換えているので、基準の検証損失とは比べない。

### 診断

| 項目 | 基準 | 候補 |
|---|---:|---:|
| PST＋FMに対する標準偏差の比（判定の範囲0.9〜1.1） | 1.027 | 1.018 |
| PST＋FMとのピアソン相関 | 0.9938 | 0.9938 |
| 駒除去差分の符号反転の割合 | 0.43%（2,444件） | 0.30%（1,702件） |

基準の符号反転は[補正前のネットの学習の記録](../nnue-material-linear-init/nnue-material-linear-init-training.md)の値である。
駒除去差分の量子化前の重みの2つの計算経路の差の最大は3.6×10⁻¹²センチポーン（上限10⁻⁶）だった。
符号反転の1,702件のうち1,699件は駒状態29に集中しており、最大は519 cpである。
合法手の評価差では、47局面すべてで候補とPST＋FMの合法手の集合が一致した。

### 保存物

生成物は`data/nnue-qsearch-leaf/`にあり、Gitの管理外である。
候補のネットは`train/nnue.bin`（SHA-256 `bf2dfe32…`）である。

## 結論

候補は量子化の検査と3つの診断を通り、駒価値の歪みも基準より増えていないので、STCの比較へ進める。
