# 借りた機械をまたぐ生成と学習の再開

## 目的

[強化学習の実行の設計書](../../plans/alphazero-rl.md)のフェーズ1として、Colab機で生成と学習を途中で止めて機械ごと破棄し、作業機のNASで確定したファイルだけを新しいColab機に置いて、続きが走ることを確かめる。
生成では、確定済みの塊が変わらず、欠けた塊だけが生成し直されることを、学習では、読んだデータと更新回数に重複も欠落もないことを、転送したファイルの検査和の照合まで含めて確かめる。

## コマンドライン

Colab機では`/content/az`を、NASの`/mnt/synology_nas/minase/alphazero/`と同じ相対パスの構成にした。
導入、生成、学習、および転送には、[試走](alphazero-rl-trial.md)と同じスクリプト（`colab-setup.sh`、`gen.sh`、`train.sh`、`pull.sh`、`push.sh`、`pull-chunks.sh`）を使った。
`pull.sh`と`push.sh`は、送り元で求めたSHA-256と送り先で求めたSHA-256が一致したときだけファイルを確定させる（[送ったファイルの検査和](alphazero-rl-resume-check/gen-pushed.txt)、[学習の再開で送ったファイルの検査和](alphazero-rl-resume-check/train-pushed.txt)）。

生成は、機械A（`rl-p1a`）と機械B（`rl-p1b`）で、同じ引数で起動した。

```console
minase data zero-selfplay --output-dir phase1/gen \
  --nn-model cycles/cycle-0000/model.onnx --nn-backend tensorrt \
  --generation-commit d786c5e12aa5134e1cc25353480d28962def5b36 \
  --seed 200000000 --games 3000 --chunk-games 500 --simulations 32 \
  --playthrough-percent 100 --threads 4 --games-per-thread 128
```

学習は、機械Bで[kill-train.sh](alphazero-rl-resume-check/kill-train.sh)から次の引数で起動し、400回目の更新のチェックポイントを確定させたことを示す行が標準エラーに現れた直後に、学習器のプロセス（uvとPython）を`SIGKILL`で止めた。
借りた機械が予告なく止まる場合に合わせて、終了処理の機会を与えないシグナルを選んだ。
機械C（`rl-p1c`）では、[train-c.sh](alphazero-rl-resume-check/train-c.sh)が`train --resume phase1/train-b --device cuda`で再開し、続けて比較の対照として、同じ引数の学習を中断せずに最初から最後まで行った（以下、止めない学習。出力先は`phase1/train-a`）。

```console
python -m minase_zero train --init cycles/cycle-0000/checkpoint.pt \
  --data phase1/gen/chunk-00000.mnsd phase1/gen/chunk-00000.resigned.mnsd ...（6塊の2ファイルずつ、計12ファイル） \
  --out-dir phase1/train-b --cycle 1 --seed 1001 --batch-size 256 --lr 0.01 --momentum 0.9 \
  --weight-decay 0.0001 --clip-grad-norm 10 --value-lambda 0.333 --precision bf16 \
  --checkpoint-steps 200 --device cuda --trainer-commit d786c5e12aa5134e1cc25353480d28962def5b36
```

生成器は1つの塊につき、投了以外で終えた対局の`chunk-N.mnsd`、投了で終えた対局の`chunk-N.resigned.mnsd`（投了を無効にしたので対局を含まない）、および塊の集計`chunk-N.json`の3ファイルを作り、学習器にはこのうち前の2つを与える。
学習器に与える教師データのファイルの列（学習窓）とチェックポイントのパスは、どの機械でも`/content/az`からの相対パスで与えた。

## エンジン

生成器と学習器は、masterのコミットd786c5e1を`git archive`で書庫にし、各Colab機で展開して導入とビルドを行ったものである。
ネットは、学習器の`init`（シード20261029）が作ったランダム初期化の10ブロック128チャネルのネット（SHA-256 `641a311e…06e3`）である。
規則セットは`engine-default`（L0＋P0＋R1＋E0）、探索はGumbel AlphaZeroの1手32回、投了は無効である。
シードの区間[200,000,001, 200,003,000]は、生成の前に作業機のシードの台帳に登録した。

## 環境

3台とも、2026年10月28日から29日に借りたColab機のL4（GPUはNVIDIA L4、CPUはIntel Xeon 2.20 GHzの12スレッド、メモリ52 GB）である。

| 機械 | 借りた時刻（日本時間） | 止めた時刻 | 役割 |
|---|---|---|---|
| A（`rl-p1a`） | 10月28日23時38分 | 10月29日0時35分 | 試走と、生成の途中までの実行 |
| B（`rl-p1b`） | 10月29日0時13分 | 0時41分 | 生成の再開と、学習の途中までの実行 |
| C（`rl-p1c`） | 0時42分 | （最初のサイクルへ続けて使用） | 学習の再開と、止めない学習 |

Colab機は同時に1本の`colab ssh`しか受け付けず、2本目はHTTP 429で拒否されたので、転送とコマンドの実行はOpenSSHのControlMasterで1本の接続を共有した。

## 結果

### 生成

機械Aでは、起動から約4分で塊0と塊2が確定した。
塊は、その塊に属する対局のうち最も遅いものが終局した時点で確定するので、番号の順に確定するとは限らず、この生成では塊2が塊1より先に確定した（[生成の後の一覧](alphazero-rl-resume-check/gen-after-b.txt)の更新時刻）。
作業機は確定した塊を約20秒ごとに受け取り、2つの塊の6ファイルと`config.json`がNASで確定した時点で、機械Aを`colab stop`で破棄した。
破棄の時点で、機械Aでは塊1と塊3〜5が一時ファイル（`*.mnsd.tmp`）のまま残っていた。

機械Bには、NASで確定した`config.json`と塊0・塊2の6ファイルだけを送った（[送る前の一覧](alphazero-rl-resume-check/gen-before-b.txt)）。
同じ引数で起動した生成器は、欠けた塊1、3、4、5の2,000局だけを生成し、237.8秒で終えた（[標準エラー](alphazero-rl-resume-check/gen-b.err)）。
送った塊0と塊2の6ファイルは、生成の後もSHA-256と更新時刻が変わらなかった（[生成の後の一覧](alphazero-rl-resume-check/gen-after-b.txt)）。
6つの塊と[集計](alphazero-rl-resume-check/gen-summary.json)は、3,000局、329,689局面、王駒の捕獲2,561局と詰み439局を記録し、手数の上限による打ち切りはなかった。

### 学習

機械Bの学習は、全1,231回のうち400回目の更新のチェックポイントを確定させた直後に止まった（[1回目のログ](alphazero-rl-resume-check/train-b-run1.log)）。
作業機はこのチェックポイント（SHA-256 `a9a034d6…cfae`）だけをNASへ受け取り、機械Bを破棄した。
機械Cには、学習窓の12ファイルと塊の集計、初期のチェックポイント、および止めた学習のチェックポイントを、機械Bと同じ相対パスに送った。
再開した学習は「resume: completed_steps=400/1231」と出力して401回目の更新から続け、全1,231回の更新を終えた（[2回目のログ](alphazero-rl-resume-check/train-b-run2.log)）。

再開した学習（出力先`train-b`）と止めない学習（出力先`train-a`）の集計（[train-a](alphazero-rl-resume-check/train-a-summary.json)、[train-b](alphazero-rl-resume-check/train-b-summary.json)）は、時間と常駐メモリの欄を除くと、次の項目で一致した。どちらも機械Cで実行した。

| 項目 | 値 |
|---|---|
| 全更新回数と完了した更新回数 | 1,231 |
| 訓練局面数と読んだ訓練局面数 | 314,962 |
| 検証局面数 | 14,727 |
| 読んだ局面の順序のハッシュ（`data_order_sha256`） | `9c708df6d5c6888c02b2e6d8c9a38b4106d029120b9f044a88c6212de81af778` |
| 学習曲線（200更新ごとの7区間の訓練の平均損失） | 全区間で一致 |
| `model.onnx`のSHA-256 | `2bb1c1e6359de033dc012d324e69f75aa6f0e2d0f19f1e9a6a0462362b799851` |

最終の重みでの診断は、2つの値だけが8桁目以下で異なった。
方策が合法手に割り当てた確率の合計（合法手の確率質量）について、訓練標本での対数の平均が`train-a`で−0.07622764259576797、`train-b`で−0.07622763514518738、検証集合での平均が`train-a`で0.9281272292137146、`train-b`で0.9281272888183594である。
重みはバイト単位で一致しているので、この差はGPUの推論での加算の順序による丸めの違いとみられる。

止めない学習の速さは、読み込みを除いて毎秒6,311局面（bf16、バッチ256）であった。
最終の重みでの対局結果に対する価値の損失（二値交差エントロピー）は、訓練標本で0.355、検証集合で0.753であり、検証の値は、勝敗を常に五分と予測したときの損失ln 2（約0.693）を上回った。
方策の損失は、訓練標本で4.497、検証集合で4.516であった。

## 結論

Colab機を途中で破棄しても、NASで確定したファイルだけを新しい機械の同じ相対パスに置けば、生成は欠けた塊だけを生成し直し、学習は読んだ局面の順序、更新回数、学習曲線、および出力したネットのバイト列が、止めない学習と一致した。
