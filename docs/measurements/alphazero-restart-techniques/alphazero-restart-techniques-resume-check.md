# 作業機での新しい構成の生成と学習の強制終了からの再開

## 目的

[学習技法の導入の設計書](../../plans/alphazero-restart-techniques.md)の「検証」の節として、新しい構成（PCR、3手以内の勝ちによる打ち切り、159面の入力、補助目標、新規の訓練局面の4倍の標本の抽出、およびウォームアップ）の生成と学習を作業機で短く走らせ、途中で強制終了して再開した生成と学習が、止めない場合と同じ塊とネットのバイト列になるかを確かめる。

## コマンドライン

測定日は2026年10月31日である。
生成は[gen-resume.sh](alphazero-restart-techniques-resume-check/gen-resume.sh)、学習は[train-resume.sh](alphazero-restart-techniques-resume-check/train-resume.sh)で行った。

生成は、次の引数で起動し、塊の集計`chunk-N.json`が2つ以上確定した時点でプロセスグループに`SIGKILL`を送り、同じ引数で起動し直した。

```console
minase data zero-selfplay --output-dir resume-check/gen \
  --nn-model net-check/cycle-0000/model.onnx --nn-backend tensorrt \
  --generation-commit 8c6b72421c496f12f4c95fe0731c3c2ebf4fcbf7 \
  --seed 300000000 --games 1024 --chunk-games 128 --simulations 32 \
  --playthrough-percent 100 --threads 2 --games-per-thread 128
```

学習は、生成の8塊の16ファイルを学習窓かつ最新のサイクルとして、次の引数で起動した。
1回目は100回目の更新のチェックポイントを確定させたことを示す行が標準エラーに現れた直後に`SIGKILL`で止め、`train --resume`で再開した（出力先`train-b`）。
続けて、同じ引数の学習を止めずに最初から最後まで行い（`train-a`）、その最終のチェックポイントから`--init`で2サイクル目を学んだ（`train-c2`、シード20261102）。

```console
python -m minase_zero train --device cuda --init net-check/cycle-0000/checkpoint.pt \
  --data <16ファイル> --latest-data <16ファイル> --out-dir resume-check/train-b --cycle 1 --seed 20261101 \
  --batch-size 256 --checkpoint-steps 50 --ema-period-samples 0 --ema-scale 0 --lr 0.01 --momentum 0.9 \
  --weight-decay 0.0001 --clip-grad-norm 10 --value-lambda 0.333 --precision bf16 --lr-schedule cosine \
  --trainer-commit 5f3100063882d18aa17777995a801169a69ef1f9
```

## エンジン

生成器はコミット8c6b7242、学習器はコミット5f310006でビルドした。
2つのコミットの間の差分は学習器のソフト方策の目標の計算だけであり、生成器は変わらない。
ネットは、[ランダム初期化のネットの出力の照合](alphazero-restart-techniques-random-net-output-check.md)で照合したシード20261031のネット（SHA-256 `b2de222c…7c48`）である。
規則セットは`engine-default`（RULES.md第33条のL0＋P0＋R1＋E0）、探索はGumbel AlphaZeroのQの尺度1.0、完全探索32回、速い探索8回であり、投了は無効である。

## 環境

作業機（Intel Core Ultra 7 265KF、NVIDIA GeForce RTX 3060 Ti）で、ONNX Runtime 1.28.3、TensorRT 10.14.1、およびPyTorch 2.11.0（CUDA 13.0）を使った。

## 結果

### 生成

生成器は1つの塊につき、投了以外で終えた対局の`chunk-N.mnsd`、投了で終えた対局の`chunk-N.resigned.mnsd`、および塊の集計`chunk-N.json`の3ファイルを作る。
1回目の生成は、塊0、3、および4が確定した時点で止まった（[止めた後の一覧](alphazero-restart-techniques-resume-check/gen-after-run1.txt)）。
同じ引数で起動し直した生成器は、欠けた塊1、2、5、6、7の640局だけを生成し、28.7秒で終えた（[標準エラー](alphazero-restart-techniques-resume-check/gen-run2.err)）。
止める前に確定した9ファイルと`config.json`は、再開の後もSHA-256が変わらなかった（[再開の後の一覧](alphazero-restart-techniques-resume-check/gen-after-run2.txt)）。
TensorRTの推論の数値はバッチの構成で変わるので、生成し直した塊が止まる前に生成しかけた内容と同じになることは求めない（[塊のADR](../../adr/commit-zero-self-play-cycles-in-fixed-game-chunks.md)）。

8塊の[集計](alphazero-restart-techniques-resume-check/gen-summary.json)は、1,024局、89,441手を記録した。
探索した手番は、完全探索が19,103、速い探索が58,050であり、完全探索の割合は0.248であった。
訓練局面は方策を記録した完全探索の手番なので、訓練局面の数`training_positions`は完全探索の手番の数と同じ19,103であった。
1,020局が王手限定の3手以内の勝ちによる打ち切り（終局理由12）で終わり、残る4局は王駒の捕獲で終わった。
ランダム初期化のネットは王駒を守らないので、ほとんどの対局が打ち切りで終わる。

### 学習

再開した学習は「resume: completed_steps=100/285」と出力して101回目の更新から続け、全285回の更新を終えた（[1回目のログ](alphazero-restart-techniques-resume-check/train-b-run1.log)、[2回目のログ](alphazero-restart-techniques-resume-check/train-b-run2.log)）。
再開した学習と止めない学習の集計（[train-a](alphazero-restart-techniques-resume-check/train-a-summary.json)、[train-b](alphazero-restart-techniques-resume-check/train-b-summary.json)）は、読み込みの最大常駐メモリと速さの欄を除いてすべて一致した。

| 項目 | 値 |
|---|---|
| 全更新回数と完了した更新回数 | 285 |
| 抽出の数（重複を含む列の長さ） | 72,808（保持した訓練局面18,202の4倍。18,202は訓練局面19,103から検証用に分けた対局の901局面を除いた数） |
| 検証局面数 | 901 |
| 読んだ順序のハッシュ（`data_order_sha256`） | `ace40e079206f5bd0f6600e1a77c27ba38ca9b6d641e0427f1e99fa96714c0b1` |
| 学習曲線（50更新ごとの6区間の各損失と学習率） | 全区間で一致 |
| 最後の学習率 | 1.108×10⁻⁸ |
| `model.onnx`のSHA-256 | `bc742d489cee94aec89a44adde4544083d8aba9052141da5667d61c7707bdd22` |

チェックポイントのファイルは、経過秒数と読み込みの最大常駐メモリの欄だけが異なり、重み、最適化器の状態、更新回数、累計標本数、および学習曲線は一致した。
窓は1サイクルだけなので、窓の訓練局面数Tは最新のサイクルの訓練局面数Nに等しく、抽出は全局面の順列を4回つないだ列になった。

2サイクル目の学習は、開始時の累計標本数`samples_seen_start`が1サイクル目の終了時の72,808を引き継ぎ、終了時は145,616であった（[train-c2](alphazero-restart-techniques-resume-check/train-c2-summary.json)）。
最後の学習率は2.214×10⁻⁸で、ウォームアップの係数が累計標本数に比例して上がっている。

学習の速さは、読み込みを除いて毎秒約3,500標本（bf16、バッチ256）であった。

この確認の最初の実行では、強制終了より前の最初の50更新の区間から、再開した学習と止めない学習の損失が一致しなかった。
ソフト方策の目標の正規化に使っていたGPUの`scatter_add_`は加算の順序が実行ごとに変わり、その丸めの差が目標の値を通じて勾配に入っていた。
`torch.use_deterministic_algorithms(True)`で2回の学習がバイト列まで一致することを確かめたうえで、目標をバッチの準備のときにCPUのnumpyで計算するように直した（コミット5f310006）。
上の結果は、この修正の後の実行である。

## 結論

新しい構成でも、生成は強制終了の後に確定済みの塊を変えずに欠けた塊だけを生成し直す。
学習は、再開しても止めない学習とネットのバイト列、学習曲線、および読んだ順序が一致し、ウォームアップの累計標本数はサイクルをまたいで引き継がれる。
