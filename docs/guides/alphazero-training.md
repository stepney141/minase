# AlphaZero型探索の深層強化学習を回す手引き

本書は、AlphaZero型探索のネットを育てる深層強化学習（RL）について、標準レシピ、起動から定例の測定までの手順、止めて再開する操作、およびオプションを定める。
費用、進捗の判定、期間の切り替え、および探索回数の引き上げといった進め方の規則と、学習の現在の状態は[強化学習の2回目の実行の設計書](../plans/alphazero-rl-2.md)（以下、設計書）にある。
本書の標準レシピと手順は、20ブロック256チャネルのネットで始める2回目の学習のものである。
2回目の学習に使う機械とその起動スクリプトは、設計書のフェーズ2で決めて用意するので、本書は実行器`minase_zero run-cycles`に渡す引数までを定める。
標準レシピの学習器と実行器は、[複数GPU化の設計書](../plans/alphazero-multi-gpu.md)の作業で改めたものである。学習は、借りた機械のGPUの枚数によらず全体のバッチ1,024で行い、実行器はColabの機械とvast.aiの機械を引数で選ぶ。
サイクル32で中断した1回目の学習（10ブロック128チャネル）の起動スクリプトと測定のスクリプトは、測定記録の`docs/measurements/alphazero-rl/`に残っている。
本書には、実行者（人間およびエージェント）が手順どおりに実行する部分だけを書く。
実行者が判断を仰ぐ相手と、対話できる端末で認証を行う人を、本書では利用者と呼ぶ。

| したいこと | 読む場所 |
|---|---|
| サイクルを回す | [標準レシピ](#標準レシピ)と[手順](#手順) |
| 止めて再開する | [止めて再開する](#止めて再開する) |
| 止まった原因を調べる | [止まったときに調べる](#止まったときに調べる) |
| 探索回数、ネットの大きさ、学習率などを変える | [設定を変える](#設定を変える) |
| 引数を調べる | [オプション一覧](#オプション一覧) |
| 標準レシピの根拠を調べる | [標準レシピの設定と根拠](#標準レシピの設定と根拠) |
| 記録の欄、式、方式の理由を調べる | [詳細の置き場所](#詳細の置き場所) |

**前提**

- 学習を再開する前と設定を変える前に、設計書の「状態」の節を読む。再開の時期と、次に回すサイクルの設定はそこで決まる。
- コマンドは、明記した箇所を除いて作業機のリポジトリのルートで実行する。本書の`$A`は、作業機にNFSでマウントしたNASのパス`/mnt/synology_nas/minase/alphazero`であり、シェルで`A=/mnt/synology_nas/minase/alphazero`と設定してから使う。
- `minase_zero`のサブコマンド（実行器の`run-cycles`など）は、`tools/train/zero`を作業ディレクトリにして`uv run --frozen python -m minase_zero <サブコマンド>`で起動する。学習器の`train`だけは、`uv run --frozen torchrun --standalone --nproc-per-node <GPUの枚数> -m minase_zero train ...`で起動する。Pythonの環境がない作業ツリーでは、`bash tools/train/zero/setup.sh`で導入する。
- 機械を借りる操作は[借りる機械の操作の手引き](alphazero-machines.md)に、ネットの出力の照合は[出力の照合の手引き](alphazero-network-check.md)に、対局の測定の規約は[棋力測定の手引き](sprt.md)にある。

## サイクルの全体像

RLは、自己対局の生成、ネットの学習、および出力の照合を1サイクルとして繰り返す。
生成では、前のサイクルのネットで、訓練局面が目標の100万に届くまで対局する。
学習では、直近のサイクルのデータ（リプレイバッファ）から抽出した局面でネットを更新する。
照合では、学習したネットをminaseが正しく読めることを確かめる。
照合に通ったネットが、次のサイクルの生成に使われる。

作業機で動く実行器が、借りたGPUの機械でサイクルを回し、出力をそのつどNASへ受け取る。
実行器は、出力の置き場所と記録を分けた学習の単位を系列と呼び、1回目の学習の`cycles/`、`cycles.jsonl`、および`nets/cycle-NNNN.onnx`を本線、それ以外を別系列とする。
2回目の学習の出力は別系列`cycles-rl2`として置き、本線は変更しない。
NASの`$A`の下の主な置き場所は次のとおりである。

| 置き場所 | 内容 |
|---|---|
| `cycles-rl2/cycle-NNNN/` | サイクルの生成、学習、および照合の出力。`cycle-0000/`には初期ネットとそのチェックポイントを置く |
| `nets/cycles-rl2/cycle-NNNN.onnx` | 照合に通って確定したネット |
| `cycles-rl2.jsonl` | サイクルごとの記録 |
| `cycles-rl2-sessions.jsonl` | セッションで回したサイクルごとの開始と終了の時刻、および開始と終了の残高 |
| `rentals.jsonl` | 機械を借りた時刻と止めた時刻、および残高 |
| `seed-ledger.jsonl` | 生成に割り当てたシードの台帳。1回目の学習と共有する |
| `logs/runner-<セッション名>.log` | 起動スクリプトと実行器のログ |
| `progress/cycles-rl2/`、`external/cycles-rl2/` | 作業機の進捗測定と、固定の検証用棋譜の診断の出力 |

1サイクルの時間と費用は、ColabのG4の機械で約1時間、約9 CU（Colabの計算単位）と見込む（設計書の「費用」の節）。
vast.aiのRTX 4090が4枚の機械では、サイクル1を約36分、約1.23米ドルで回した（[vast.aiの測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)）。
1回目の学習の10ブロック128チャネルのネットでは、ColabのL4の機械で約1時間、約2 CUであった。

## 標準レシピ

2026年11月10日時点の、2回目の学習の標準である。
1回目の学習から変えたのは、ネットの大きさ、リプレイバッファの上限と最古のサイクル、開始局面、系列の置き場所、および系列の基本シードの加算値であり、ほかの値は1回目のサイクル26〜32を回した設定と同じである（設計書の「やり直しの範囲」の節）。
起動スクリプトは、実行器`minase_zero run-cycles`を次の引数で走らせる。

```sh
# tools/train/zeroで実行する
uv run --frozen python -m minase_zero run-cycles --machine colab --session <セッション名> \
  --cycle-cu <1サイクルの見込みのCU> --root "$A" \
  --cycles-dir cycles-rl2 --base-cycles-dir cycles-rl2 --branch-from 0 \
  --generation-commit <コミット> --trainer-commit <コミット> \
  --first-cycle <最初のサイクル> --last-cycle <最後のサイクル> \
  --deadline <終了期限> --cycle-minutes <1サイクルの見込みの分> \
  --start-position <shuffled-setupかrandom-opening> --generation-seed-offset 300000000 \
  --target-positions 1000000 --chunks 20 --simulations 32 --threads-per-gpu 4 --games-per-thread 128 \
  --replay-buffer-first-cycle <1か3> --replay-buffer-positions 280520080 \
  --batch-size 1024 --lr 0.04 --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 \
  --value-lambda 0.333 --precision bf16 --checkpoint-steps 50 \
  --train-seed-base 20261100 --minase <作業ツリー>/target/release/minase --poll-seconds 60
# 最初のサイクルが1のセッションでは --cycle1-reference-summary <生成器の集計summary.json> を加える
# vast.aiの機械では、--machine colab --session ... --cycle-cu ... の代わりに --machine vast --vast-instance <インスタンスの番号> を渡す
```

セッションごとに値の変わる引数は、次の規則で決める。

| 引数 | 値 |
|---|---|
| `--start-position` | 事前学習の期間は組み替えた初期配置の`shuffled-setup`、微調整の期間は初期局面から8〜16手を乱択で指す`random-opening`とする。期間を切り替えるサイクルでセッションを区切る（設計書の「期間の切り替え」の節） |
| `--replay-buffer-first-cycle` | 最後のサイクルが13以下のセッションは1、最初のサイクルが14以上のセッションは3とし、サイクル13でセッションを区切る（[リプレイバッファのADR](../adr/sample-four-times-from-a-zero-replay-buffer-of-ten-times-the-parameter-count-from-cycle-3-after-cycle-13.md)） |
| `--cycle1-reference-summary` | 最初のサイクルが1のセッションだけに与える。サイクル1の対局数の上限を見積もるのに使う、生成器の集計`summary.json` |
| `--machine`と機械ごとの引数 | Colabの機械では`--machine colab`、セッション名の`--session`、および1サイクルの見込みのCUの`--cycle-cu`、vast.aiの機械では`--machine vast`とインスタンスの番号の`--vast-instance` |
| `--deadline`、`--cycle-minutes` | 借りた機械の終了期限と、その機械での1サイクルの見込みの時間（設計書の「系列の置き場所と起動」の節）。vast.aiの機械では、終了期限は起動スクリプトが決める |

実行器は、各サイクルで借りた機械の生成器と学習器を次の引数で自動で起動するので、実行者がこれらを入力する必要はない。
山括弧の値は実行器がサイクルごとに決める（[`minase data zero-selfplay`](#minase-data-zero-selfplay)と[`minase_zero train`](#minase_zero-train)の表）。

```sh
# 生成
minase data zero-selfplay --output-dir cycles-rl2/cycle-NNNN/selfplay \
  --nn-model <前のサイクルのネット> --nn-backend tensorrt --generation-commit <コミット> \
  --start-position <run-cyclesと同じ値> \
  --seed <サイクル番号×10⁹＋3×10⁸> --games <見積もった対局数> --chunk-games <対局数÷20の切り上げ> \
  --target-positions 1000000 --simulations 32 --playthrough-percent <100か20> \
  --threads <4×GPUの枚数> --devices <GPUの枚数> --games-per-thread 128

# 学習
torchrun --standalone --nproc-per-node <GPUの枚数> -m minase_zero train --init <前のサイクルのcheckpoint.pt> \
  --data-list cycles-rl2/cycle-NNNN/train-data.json --latest-data <最新のサイクルのファイル...> \
  --out-dir cycles-rl2/cycle-NNNN/train --cycle NNNN --seed <20261100＋サイクル番号> \
  --batch-size 1024 --micro-batch-size 256 --lr 0.04 --momentum 0.9 --weight-decay 0.0001 \
  --clip-grad-norm 10 --value-lambda 0.333 --precision bf16 --checkpoint-steps 50 \
  --device cuda --replay-threads <使えるCPUの数÷GPUの枚数の切り捨て> --trainer-commit <コミット>
```

ネットは20ブロック256チャネル（パラメータ数は補助ヘッドを含めて28,052,008）であり、`--replay-buffer-positions`はその10倍である。
ネットの大きさは、`minase_zero init`で初期ネットを作るときに決まる。
サイクル1は`cycles-rl2/cycle-0000/`の初期ネットから、以後のサイクルは前のサイクルのチェックポイントから学習を続ける。
初期ネットは`minase_zero init --blocks 20 --channels 256 --seed 20261109`で作った（[準備の設計書](../plans/archived/alphazero-rl-2-prep.md)）。

**止まる条件と見直す条件**

| 時点 | 条件 | 対応 |
|---|---|---|
| サイクルの開始前 | 終了期限までの残り時間が`--cycle-minutes`の分数に満たないか、Colabの機械で残高が`--cycle-cu`未満 | 実行器が次のサイクルを始めずに正常終了する |
| 生成の後 | 投了の誤り（投了の条件を満たした後も最後まで指させた対局で、条件を満たした側が負けなかった割合）が5%を超えた | 実行器が投了の対局を学習から外し、投了なしで生成し直す |
| 生成の後 | 手数の上限で捨てた対局が10%を超えた | 上限または投了の条件を見直す |
| 照合 | `check status=1` | 実行器が止まる。[止まったときに調べる](#止まったときに調べる)に従う |
| 進捗測定 | 10サイクル前のネットに対するEloの点推定が2回続けて0以下 | 設計書の「伸びが止まったとき」の節に従う |

## 手順

次の順に進める。

1. [起動の前に確かめる](#起動の前に確かめる)
2. [セッションを起動する](#セッションを起動する)
3. [経過を確かめる](#経過を確かめる)
4. [定例の測定を行う](#定例の測定を行う)

### 起動の前に確かめる

作業機で次の点を確かめる。
最初の1回だけは、[借りる機械の操作の手引き](alphazero-machines.md)の1節の準備も必要である。

```console
git status && git rev-parse HEAD              # 借りた機械へ送るコミットを確かめる
cargo build --release --bin minase            # 実行器が各サイクルの集計に使う
```

借りた機械には指定したコミットの書庫を送るので、コミットしていない変更は使われない。
Colabの機械を使う場合は、`colab usage`で認証と残高も確かめる。
認証が切れていれば、利用者が対話できる端末で実行して認証し直す。
残高と料金が次に回すサイクルの見込みに足りない場合の扱いは、設計書の「費用」の節が定める。

最初のセッションを起動する前に、シードの台帳`seed-ledger.jsonl`で、サイクル1〜50の通常の生成の区間（サイクル番号×10⁹＋3×10⁸の次の値から、対局数の上限の分の区間）と生成し直しの区間（その始点にさらに5×10⁸を足した区間）が、既存のどの区間とも重ならないことを確かめる（設計書の「系列の置き場所と起動」の節）。

### セッションを起動する

起動スクリプトで、セッションを1つ起動する。
起動スクリプトは、設計書のフェーズ2で機械を選んだ後に`docs/measurements/alphazero-rl-2/alphazero-rl-2-sessions/`に置き、使い方はそのスクリプトの冒頭に書く。
最初のサイクルには、`cycles-rl2.jsonl`の最後のサイクル番号（`tail -1 $A/cycles-rl2.jsonl | jq .cycle`で確かめる）に1を加えた番号を指定する。
最後のサイクルには、次の測定を行うサイクルなど、止めたい番号を指定する。
セッションは、次の3つの時点で必ず区切る。

- サイクル13の後で区切る。サイクル14からはリプレイバッファの最古のサイクルが1から3に変わる。
- 10サイクルごとの進捗測定を行うサイクルの後で区切る。測定の結果で次の設定が決まるので、結果を待つ間は借りた機械を止める。
- 事前学習の期間から微調整の期間へ切り替える前で区切る。`--start-position`の値が変わる。

セッション名は、`rentals.jsonl`の過去の名前と重ならないものにする。
実行器は、終了期限までの残り時間が`--cycle-minutes`の分数に満たないとき、またはColabの機械で残高が`--cycle-cu`を下回るときに次のサイクルを始めず、正常に終了する。
vast.aiの機械では残高を照会しない。
実行器が走っている作業ツリーは、セッションが終わるまで消さない。
前のセッションが止まったら、次のセッションを同じ手順で起動する。

### 経過を確かめる

実行器は、工程ごとの状態を1分ごとにログへ書く。

```console
tail -f $A/logs/runner-<セッション名>.log
pgrep -af "[r]un-cycles"
```

`pgrep`の形式は、自分のコマンド行に一致しないように角括弧を入れてある。
この形式を使わずに`kill $(pgrep -f ...)`とすると、自分のシェルを止めてしまう。
ログの行の意味は次のとおりである。

| ログの行 | 意味 |
|---|---|
| `cycle=NNNN generation games=… playthrough_percent=…` | 生成の開始。`games`は対局数の上限である。`playthrough_percent`は投了させずに最後まで指させる対局の割合で、100なら投了なし、20なら投了ありである |
| `cycle=NNNN selfplay status=<状態> received=…` | 生成の状態。`status=running`が生成中、`status=0`が生成の完了を示す |
| `cycle=NNNN train status=<状態>`、`cycle=NNNN check status=<状態>` | 学習と出力の照合の状態。`status=0`が完了であり、`check status=1`は照合の不合格である |
| `cycle=NNNN complete balance=…` | サイクルの完了と残高 |
| `deadline does not allow another cycle` | 終了期限により、次のサイクルを始めずに終わる |
| `runner exit <コード>` | 実行器の終了 |

サイクルが終わったら、`cycles-rl2.jsonl`の最後の行で次の項目を見る。
どれも学習が健全に進んでいるかの目安であり、基準を外れたら設計書の該当する節に従う。

| 項目 | `cycles-rl2.jsonl`の欄 | 目安 |
|---|---|---|
| 開始局面の種類 | `generation.start_position` | そのサイクルの期間の種類と一致する |
| 手数の上限で捨てた対局 | `selfplay_summary.discarded_max_ply`÷`selfplay_summary.games` | 10%を超えたら、上限または投了の条件を見直す |
| 1局の手数と引き分け | `selfplay_summary.plies`÷`selfplay_summary.games`と`selfplay_summary.draws`÷`selfplay_summary.games` | 伸び続ける場合は、引き延ばしを疑う（[定例の測定](#定例の測定を行う)） |
| 投了の誤り | `selfplay_summary.false_resignations`÷`selfplay_summary.resign_triggered_playthrough_games` | 5%を超えたサイクルは、実行器が投了の対局を除いて自動で生成し直す |
| 価値の損失 | `training_summary.diagnostics.validation.value_result` | ln 2（約0.693）より小さいのが正常である。`latest_training_sample`との差が広がったら、過学習を疑う |
| 合法手に乗る確率質量 | `training_summary.diagnostics.validation.legal_probability_mass` | 1に近い（1回目の学習のサイクル30前後で約0.996） |

サイクル間で検証集合が変わるので、損失の1サイクルごとの増減には、学習の良し悪しのほかに、生成した局面の分布の変化も影響する。
期間を切り替えたサイクルでは、開始局面が変わるので、この影響が特に大きい。
伸びの判断には、次の節の対局の結果を使う。

### 定例の測定を行う

10サイクルごとに、作業機で進捗測定と診断を行う。
どれもノード数で制限した対局か、時間に依存しない処理なので、作業機でほかの処理が動いていても結果は変わらない。
測定の結果で次のサイクルの設定が決まるので、測定を終えるまで次のセッションを起動しない。
対局に使うバイナリは、サイクル1を始めたときのmasterのコミットに固定する（設計書の「進捗とαβ探索との差の記録」の節）。
そのコミットの作業ツリーを作って`cargo build --release --bin minase`でビルドし、この節のコマンドはその作業ツリーのルートで実行する。
スクリプトは、置かれた作業ツリーの`target/release/minase`と`tools/train/zero`を使う。
サイクル20の場合の手順は次のとおりである。

```console
D=docs/measurements/alphazero-rl-2/alphazero-rl-2-progress-c0020
mkdir -p $D
cp docs/measurements/alphazero-rl-2-prep/alphazero-rl-2-prep-random-net-output-check/check.sh $D/
cp docs/measurements/alphazero-rl-2-prep/alphazero-rl-2-prep-match-timing/progress-workstation.sh $D/
C=$A/cycles-rl2/cycle-0020
bash $D/check.sh $C/train/model.onnx $C/check/pytorch.json $C/check-workstation
bash $D/progress-workstation.sh $A/nets/cycles-rl2/cycle-0020.onnx $A/nets/cycles-rl2/cycle-0010.onnx \
  16 200 <シード> 0 <シード+1> $A/progress/cycles-rl2
```

`check.sh`は、学習した機械で`fixture-eval`が書いたPyTorchの出力を基準に、作業機のONNX RuntimeのCPUとTensorRTのFP16の出力を照合する。
`progress-workstation.sh`は、新しいネットと10サイクル前のネットを、対局用のMCTS（PUCT）の1手800シミュレーションで対局させる。
乱択のエンジンとのペア数が0でなければ、続けて探索なしの新しいネットを乱択のエンジンと対局させる。
引数は次の順に渡す。

| 順 | 意味 | 値 |
|---|---|---|
| 1 | 新しいネット | `$A/nets/cycles-rl2/cycle-NNNN.onnx` |
| 2 | 10サイクル前のネット | 同じ形式のパス |
| 3 | 推論のバッチの大きさ`NnBatchSize` | 16 |
| 4 | MCTSどうしの対局のペア数 | 200 |
| 5 | その対局のシード | 過去の測定で使っていない値 |
| 6 | 乱択のエンジンとの対局のペア数 | サイクル10では100、それ以外は0 |
| 7 | その対局のシード | 過去の測定で使っていない値 |
| 8 | 出力先 | `$A/progress/cycles-rl2` |

サイクル10の10サイクル前のネットは、`$A/cycles-rl2/cycle-0000/model.onnx`の初期ネットである。
`NnBatchSize`の16は、1回目の学習の[10サイクル目の進捗測定](../measurements/alphazero-rl/alphazero-rl-progress-c0010.md)で決めた値である。
20ブロック256チャネルのネットでは、作業機の対局の時間が1回目より延びる。
初期ネットで測った所要時間は[準備の設計書](../plans/archived/alphazero-rl-2-prep.md)のフェーズ2の記録にあり、サイクル10の測定を始める前に、サイクル10のネットで見積もり直して利用者へ示す。

αβ探索との差は、同じ10サイクルごとに、1手800シミュレーションのMCTSと、コミット45d5db54のαβ探索の1手500ノードを100ペア対局させて記録する（設計書の「進捗とαβ探索との差の記録」の節）。
MCTSの側は上と同じ固定したコミットのバイナリと最新のネット、αβ探索の側はコミット45d5db54のバイナリとそれに埋め込まれたNNUEである。
同時対局数4などのほかの条件は、1回目の学習の[固定ノードの対局の記録](../measurements/adhoc/alphazero-rl-c0032-ab-ladder.md)と同じである。
その記録の`run-cell.sh`はサイクル32のネットと出力先を固定しているので使わず、bashで次のように実行する。

```bash
source tools/train/zero/env.sh
export XDG_CACHE_HOME=$HOME/.cache
M=$PWD/target/release/minase
N=$A/nets/cycles-rl2/cycle-0020.onnx
"$M" match run --run-dir $A/progress/cycles-rl2/cycle-0020-vs-ab500 --seed <シード> \
  --candidate "$M --protocol usi --rules engine-default --search mcts --nn-model $N" \
  --baseline commit:45d5db5498e1739b2307b3719891a1eec24f7fbd \
  --candidate-option NnBatchSize=16 --each nodes=800 --baseline-limit nodes=500 --concurrency 4 elo --pairs 100
```

固定の検証用棋譜の診断は、前回の測定より後の各サイクルについて行う。

```console
(cd tools/train/zero && uv run --frozen python -m minase_zero external-eval \
  --checkpoint $A/cycles-rl2/cycle-NNNN/train/checkpoint.pt \
  --data <リポジトリの場所>/data/nnue-lr-schedule-gen/generated-8200000.bin --games 1000 --seed 20261030 \
  --device cuda --out $A/external/cycles-rl2/cycle-NNNN.json)
```

引き延ばしの兆候は、`alphazero-rl-long-games/`のスクリプトで集計する。
800手以上の対局について、手数ごとの駒数と評価値、および600手目に有利な側が勝った割合を出力する。

```console
(cd docs/measurements/alphazero-rl/alphazero-rl-long-games && \
  uv run --frozen --project <リポジトリの場所>/tools/train/zero python long_trend.py $A/cycles-rl2/cycle-NNNN)
```

結果は、`docs/measurements/alphazero-rl-2/alphazero-rl-2-progress-cNNNN.md`に記録する（1回目の先例は[30サイクル目の記録](../measurements/alphazero-rl/alphazero-rl-restart-progress-c0030.md)）。
10サイクル前のネットに対するEloの点推定が2回続けて0以下になったら、設計書の「伸びが止まったとき」の節に従う。

設計書の「伸びが止まったとき」の節の表の段階を進める前と、到達の判定を行う前には、最新のネットのMCTSとαβ探索を、[棋力測定の手引き](sprt.md)の短時間条件（STC）で対局させる。
先例は1回目の学習の[サイクル32の対局](../measurements/alphazero-rl/alphazero-rl-restart-stc-c0032.md)であり、`alphazero-rl-restart-stc-c0032/run-stc.sh`を複製して、ネットとシードだけを変えて実行する。
時間制御の対局は負荷の影響を受けるので、対局の間は作業機で重い処理を動かさない。
このSTCの対局は到達の判定そのものではなく、判定の手順は[統括計画](../plans/alphazero.md)の「到達の判定」の節が定める。

## 必要なときの操作

### 止めて再開する

止めるだけなら、次のセッションを起動しなければよい。
セッションは終了期限か最後のサイクルで自動で止まり、機械を返す。
全サイクルの出力はNASにあるので、止めても出力は失われない。

途中で止める場合は、起動スクリプトは止めずに、作業機の実行器だけを止める。
実行器が終了すると、起動スクリプトが続けて借りた機械を返す。

```console
kill -TERM $(pgrep -f "[.]venv/bin/python -m minase_zero run-cycles")
```

止めたら、理由を`rentals.jsonl`に1行追記する。
途中で止めたサイクルの消費は`cycles-rl2-sessions.jsonl`に入らないので、合計は`rentals.jsonl`の残高で確かめる。

再開は、[セッションを起動する](#セッションを起動する)の手順で、止まったサイクルの番号から起動する。
同じコミットで起動すれば、受け取り済みの生成の塊と学習のチェックポイントから続きを行い、NASで完了した工程はやり直さない。
実行器のコードを変えて再開するときは、次の制約がある。

- 生成が完了している場合は、`selfplay/summary.json`がNASにあれば、生成時のコミットが異なっていてもそのまま受け入れる。
- 生成が途中の場合は、コミットが異なると実行器がエラーで止まり、開始局面の種類が保存済みの生成条件`selfplay/config.json`と異なると生成器がエラーで止まる。
- 学習が途中の場合は、学習器が同じ局面を同じ順に読むことを照合するので、リプレイバッファや学習の設定を変えると再開できない。

コードを切り替えるときは、生成が終わって学習が始まった直後に実行器を止めるのが最も無駄が少ない。
この方法で切り替えるときは、該当するサイクルの`train.started.json`、`train/`などの学習の出力を、削除せずに`cycles-rl2/cycle-NNNN/rejected-<理由>/`へ移してから、新しいコミットで同じサイクルから起動する。

### 止まったときに調べる

実行器が止まったら、ログの末尾のエラーと、該当サイクルの`cycles-rl2/cycle-NNNN/`の状態ファイル（`*.started.json`、`*.exit`、`*.log`）を見る。
機械は起動スクリプトが返しているので、調べている間に費用はかからない。

出力の照合で止まった場合（`check status=1`）は、`check/`の`*.compare.exit`と`*.compare.err`で不合格の組を確かめる。
照合は、PyTorchでの計算を基準に、同じネットをONNX RuntimeのCPUとTensorRTのFP16で計算した結果を、288局面で比べるものである。
目的はONNXへの書き出しの誤りや、minaseが別の手の出力を読む誤りを見つけることであり、誤りがあれば差は桁違いに大きくなると見込まれる。
CPUの照合が通っていてFP16の差だけが許容誤差をわずかに超えた場合は、FP16の丸めの累積とみられる。
ただし許容誤差は[照合のADR](../adr/compare-fp16-zero-network-values-within-five-hundredths.md)の決定なので、実行者の判断では変えず、数値を添えて利用者に判断を仰ぐ。
学習をやり直すときは、不合格の出力を[止めて再開する](#止めて再開する)と同じく`rejected-<理由>/`へ移す。

そのほかの止まり方の対応は次のとおりである。
Colabの機械を使う場合の表であり、ほかの機械の場合は[借りる機械の操作の手引き](alphazero-machines.md)に従う。

| 止まり方 | 対応 |
|---|---|
| 接続の切断 | 実行器は接続の終了コード255とHTTP 429を30秒間隔で5回まで再試行する。それでも切れた場合や、Colab側に機械を回収された場合は、同じコミットで再開する |
| 認証の切れ | `colab`が401か403を返したら、利用者が端末で`colab usage`を実行して認証し直す |
| 機械を借りられない | `colab new`が失敗してもCUは減らない。同じ日に3回続けて借りられなければ、利用者へ報告してGPU種別を決め直す |
| 残高の不足 | 実行器は正常に終了する。設計書の「費用」の節に従う |

計算を止めた後に判断を待つときは、借りた機械を返したことを確かめる。
Colabの機械では`colab sessions`で確かめる。

### 設定を変える

標準レシピの値は起動スクリプトの引数にある。
探索回数、ネットの大きさ、学習率などは、まず設計書の規則に照らして変更を決める。
変更は次の順に進める。

1. 判断の理由をADRに書き、変えた時点を設計書の「状態」に書く。
2. まだ測っていない構成なら、`--cycles-dir`に本線以外の名前を指定した別系列で1サイクルの試走を行い、時間とCUを確かめる（[借りる機械の操作の手引き](alphazero-machines.md)の6節）。
3. コードの変更はブランチで行い、起動はそのブランチの作業ツリーの起動スクリプトから行ってよい。スクリプトは自分の置かれた作業ツリーの実行器を使う。

ネットの大きさを変えるときは、`init`で新しいネットを作り、[出力の照合の手引き](alphazero-network-check.md)で照合してから、その系列の`cycle-0000/`に置く。
2回目の学習の初期ネットの先例は、[準備の設計書](../plans/archived/alphazero-rl-2-prep.md)のフェーズ2の記録である。
`--replay-buffer-positions`は新しいネットのパラメータ数の10倍に変える。

NASの記録はコミットのハッシュを参照するので、記録に現れたコミットのブランチはリベースせず、`--no-ff`でマージする。
生成のシードは、実行器がサイクル番号と`--generation-seed-offset`から決める。
系列ごとに加算値を変えて区間を分け、実行器は別系列の分も含めてシードの台帳（`seed-ledger.jsonl`）で重なりを検査する。
1回目の学習の本線は加算値0、2回目の学習は3×10⁸である。

## オプション一覧

学習器、実行器、および生成器の引数はすべて必須で、既定値を持たない。
表の「標準の値」は、[標準レシピ](#標準レシピ)が渡す値である。
値域は引数を検査するソースから写したものであり、食い違うときはソースと`--help`を正とする。

### `minase_zero run-cycles`

実行器の引数と再開の契約は、`tools/train/zero/minase_zero/runner.py`のdocstringが定める。

**系列と置き場所**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--machine <KIND>` | 借りた機械の種類 | `colab`または`vast`。接続、作業場所（Colabは`/content`、vast.aiは`/workspace`の下）、および次のサイクルを始める判定を分ける。 |
| `--vast-instance <ID>` | vast.aiの機械だけ | vast.aiのインスタンスの番号。正の整数。起動時に1回だけ接続先を照会する。Colabでは指定できない。 |
| `--root <DIR>` | `$A` | NASの基準のディレクトリ。この下の相対パスの構成は、借りた機械の作業場所（Colabは`/content/az`、vast.aiは`/workspace/az`）の下と同じにする。 |
| `--cycles-dir <DIR>` | `cycles-rl2` | 出力を置く`--root`からの相対ディレクトリ。`..`と空白を含めない。`cycles`以外なら別系列になり、記録は`<cycles-dir>.jsonl`、ネットは`nets/<cycles-dir>/`に置く。 |
| `--base-cycles-dir <DIR>` | `cycles-rl2` | `--branch-from`以下のサイクルを読む相対ディレクトリ。 |
| `--branch-from <N>` | 0 | 分岐するサイクル番号。0以上`--first-cycle`未満。 |
| `--session <NAME>` | Colabの機械だけ、起動スクリプトが渡す | セッション名。英数字、`_`、`-`だけ。vast.aiでは指定できない。 |
| `--minase <FILE>` | 作業ツリーの`target/release/minase` | 作業機で集計に使う実行ファイル。借りた機械で使うものとは別である。 |
| `--poll-seconds <S>` | 60 | 状態を確かめる間隔。正の有限値。 |

**コミット、サイクル、および期限**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--generation-commit <HASH>` | 起動スクリプトが渡す | 生成器のコミット。40桁の16進数字。未完了の生成は、これが一致する場合だけ再開する。 |
| `--trainer-commit <HASH>` | 起動スクリプトが渡す | 学習器のコミット。40桁の16進数字。借りた機械にはこのコミットの書庫を展開する。 |
| `--first-cycle <N>` | 起動スクリプトが渡す | 1以上。 |
| `--last-cycle <N>` | 起動スクリプトが渡す | `--first-cycle`以上。 |
| `--cycle1-reference-summary <FILE>` | 最初のサイクルが1のセッションだけ | サイクル1の対局数を見積もる生成の集計。`--first-cycle`が1のときだけ必須で、それ以外では指定できない。 |
| `--deadline <TIME>` | 借りた機械の終了期限 | 時差を含むISO 8601の時刻。 |
| `--cycle-minutes <M>` | 機械ごとの見込み | 期限までの残りがこれより短ければ、次のサイクルを始めない。正の有限値。 |
| `--cycle-cu <CU>` | Colabの機械だけ、機械ごとの見込み | 残高がこれを下回れば、次のサイクルを始めない。正の有限値。vast.aiでは指定できない。 |

**生成**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--start-position <KIND>` | 事前学習の期間は`shuffled-setup`、微調整の期間は`random-opening` | 開始局面の種類。生成器の同名の引数へそのまま渡し、記録の`generation`の`start_position`に残す。 |
| `--generation-seed-offset <N>` | 300000000 | 系列の基本シードの加算値。0以上5×10⁸未満の整数。通常の生成の基本シードをサイクル番号×10⁹＋この値とし、生成し直しではさらに5×10⁸を足す。この値＋5×10⁸＋対局数の上限が10⁹を超えるときは、台帳に登録する前に止まる。記録の行の`generation_seed_offset`に残す。 |
| `--target-positions <N>` | 1000000 | 1サイクルの訓練局面数の目標。1以上。 |
| `--chunks <N>` | 20 | 対局数の上限を分ける塊の数。1以上。 |
| `--simulations <N>` | 32 | 完全探索（手番の25%で行い、方策を記録する探索）の回数。残りの手番で行う速い探索はこの4分の1。4以上の4の倍数（生成器が検査する）。 |
| `--threads-per-gpu <N>` | 4 | GPUあたりの生成器のスレッド数。1以上。生成器には、これにGPUの枚数を掛けた`--threads`を渡し、使えるCPUの数（cpusetとcgroupの割当ての小さいほう）を超えれば生成を起動しない。 |
| `--games-per-thread <N>` | 128 | スレッドごとの同時対局数。1以上。 |

**リプレイバッファ**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--replay-buffer-first-cycle <N>` | 1（サイクル13まで）か3（サイクル14から） | リプレイバッファがさかのぼる最も古いサイクル。1以上`--first-cycle`以下。別系列でも本線と同じ通し番号を使う。 |
| `--replay-buffer-positions <N>` | 280520080 | リプレイバッファの局面数の目標。最新のサイクルからさかのぼり、生成し直しを含む`training_positions`の合計が初めてこの値以上になるまでをリプレイバッファとする。1以上。 |

**学習**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--batch-size <N>` | 1024 | 全体のバッチ（1更新の局面数）。1以上。学習器が、ミクロバッチ256の倍数であることと、ミクロバッチの数がGPUの枚数の倍数であることを検査する。 |
| `--lr <VALUE>` | 0.04 | 基準の学習率。正の有限値。 |
| `--momentum <VALUE>` | 0.9 | ネステロフのモーメンタムの係数。正の有限値。 |
| `--weight-decay <VALUE>` | 0.0001 | 重み減衰。非負の有限値。 |
| `--clip-grad-norm <VALUE>` | 10 | 勾配のノルムの上限。正の有限値。 |
| `--value-lambda <VALUE>` | 0.333 | 価値の教師で根の価値に掛ける割合λ。0以上1以下。 |
| `--precision <NAME>` | `bf16` | `bf16`（順伝播だけをbf16で計算）または`fp32`。 |
| `--checkpoint-steps <N>` | 50 | チェックポイントを保存する更新回数の間隔。1以上。 |
| `--train-seed-base <N>` | 20261100 | サイクルcの学習のシードを、この値＋cとする。 |

### `minase data zero-selfplay`

自己対局の生成器であり、実行器が借りた機械で起動する。
生成の契約は`crates/minase/src/bin/minase/zero_selfplay/mod.rs`のモジュールのdocコメントが定める。

| 引数 | 実行器が渡す値 | 値域と意味 |
|---|---|---|
| `--output-dir <DIR>` | `<cycles-dir>/cycle-NNNN/selfplay`（生成し直しでは`selfplay-regen`） | 生成条件、塊、および集計を置くディレクトリ。 |
| `--nn-model <FILE>` | 前のサイクルの`train/model.onnx`（サイクル1は`cycle-0000/model.onnx`） | ONNX形式のネット。 |
| `--nn-backend <NAME>` | `tensorrt` | `tensorrt`、`cuda`、または`cpu`。`tensorrt`は`XDG_CACHE_HOME`が空でないことを要する。 |
| `--generation-commit <HASH>` | `--generation-commit` | 40桁の16進数字。 |
| `--start-position <KIND>` | `--start-position` | 開始局面の種類。`shuffled-setup`は組み替えた初期配置から乱択の手順なしで、`random-opening`は初期局面から8〜16手を乱択で指してから対局を始める。生成条件`config.json`と集計`summary.json`の`start_position`に残し、再開のときに異なれば起動を拒む。 |
| `--seed <N>` | サイクル番号×10⁹＋`--generation-seed-offset`（生成し直しではさらに5×10⁸） | サイクルの基本シード。対局のシードは、これと対局番号から導く。 |
| `--games <N>` | 前のサイクルの1局あたりの訓練局面数から、`--target-positions`に要る対局数を切り上げた値 | 対局数の上限。1以上。生成し直しでは、通常の生成で投了した対局の数。 |
| `--target-positions <N>` | `--target-positions` | 塊0からの累計がこれに届いた最初の塊までをサイクルに含める。1以上。生成し直しでは、通常の生成で投了した対局の訓練局面数。 |
| `--chunk-games <N>` | `--games`を`--chunks`で割って切り上げた値 | 1つの塊の対局数。1以上。 |
| `--simulations <N>` | `--simulations` | 完全探索の回数。4以上の4の倍数。 |
| `--playthrough-percent <N>` | 投了が無効なら100、有効なら20（生成し直しでは100） | 投了させずに最後まで指させる対局の割合。1以上100以下。 |
| `--threads <N>` | `--threads-per-gpu`×GPUの枚数 | 推論器を1つずつ持つスレッドの数。`--devices`の倍数。 |
| `--devices <N>` | GPUの枚数 | 使うGPUの枚数。0から数えたスレッドtの推論器をGPU t mod Nに置く。`tensorrt`と`cuda`では、`nvidia-smi`の全GPUが同じ機種と容量で、枚数がNと等しいことを確かめ、`CUDA_VISIBLE_DEVICES`が設定されていれば起動を拒む。`cpu`では1に限る。 |
| `--games-per-thread <N>` | `--games-per-thread` | スレッドごとの同時対局数であり、推論の最大バッチでもある。1以上。 |

投了の有効と無効は、直前のサイクルの較正（`resign-calibration`）で実行器が決める。

### `minase_zero train`

学習器であり、実行器が借りた機械で`torchrun --standalone --nproc-per-node <GPUの枚数>`の下で起動する。
`torchrun`の外で起動すると、引数を検査する前に止まる。
出力先、抽出、全体のバッチとミクロバッチ、学習率の式、および再開の契約は、`tools/train/zero/minase_zero/train.py`のdocstringが定める。

| 引数 | 実行器が渡す値 | 値域と意味 |
|---|---|---|
| `--init <FILE>` | 前のサイクルの`train/checkpoint.pt`（サイクル1は`cycle-0000/checkpoint.pt`） | すべての更新を終えたチェックポイント。サイクル1では初期ネットのチェックポイント。 |
| `--data-list <FILE>` | `<cycles-dir>/cycle-NNNN/train-data.json` | リプレイバッファのファイルのパスを並べたUTF-8のJSONの配列。パスは古いサイクルから、各生成の`training_files`の順に並べ、生成し直しを通常の生成の後に加える。実行器が作ってNASに置き、借りた機械へ送る。 |
| `--latest-data <FILE...>` | 最新のサイクルのファイル | `--data-list`の配列の末尾とパスの文字列も順も一致させる。 |
| `--out-dir <DIR>` | `<cycles-dir>/cycle-NNNN/train` | 存在しないか空のディレクトリ。 |
| `--cycle <N>` | サイクル番号 | 0以上。 |
| `--seed <N>` | `--train-seed-base`＋サイクル番号 | 抽出と更新順の乱数シード。uint64。 |
| `--batch-size`、`--lr`、`--momentum`、`--weight-decay`、`--clip-grad-norm`、`--value-lambda`、`--precision`、`--checkpoint-steps` | `run-cycles`の同名の引数 | `run-cycles`と同じ。 |
| `--micro-batch-size <N>` | 256 | 1回の順伝播の局面数。正の整数。実行器の定数`MICRO_BATCH_SIZE`で、チェックポイントに保存し、再開ではそこから読む。 |
| `--device <NAME>` | `cuda` | `cuda`または`cpu`。 |
| `--replay-threads <N>` | 使えるCPUの数÷GPUの枚数の切り捨て | 各GPUのプロセスのリプレイバッファの再生のスレッド数。1以上。再開の照合には入れない。 |
| `--trainer-commit <HASH>` | `--trainer-commit` | 40桁の16進数字。 |

中断した学習は、`torchrun`の下の`train --resume <出力先> --device <cuda|cpu> --replay-threads <N>`で続ける。
ほかの設定はチェックポイントから読み、教師ファイルの大きさ、局面数、および抽出の検査和が一致しなければ再開しない。
GPUの枚数の違う機械でも再開できるが、加算の順序が変わるので、途中で止めない学習とバイト単位では一致しない。

### その他のサブコマンド

| コマンド | 引数 | 使うとき |
|---|---|---|
| `minase_zero init` | `--blocks`、`--channels`、`--seed`、`--out-dir`（存在しないか空）、`--trainer-commit` | ランダムに初期化したネットを作るとき |
| `minase_zero fixture-eval` | `--checkpoint`、`--onnx`、`--fixture`、`--out` | 固定の局面集でPyTorchの出力を書き出し、照合の基準にするとき |
| `minase_zero external-eval` | `--checkpoint`、`--data`（αβ探索の自己対局の教師データのファイル。形式は[ファイル形式の手引き](file-formats.md)のMNSD。複数指定可）、`--games`、`--seed`、`--device`、`--out` | 固定の検証用棋譜で診断するとき |
| `minase_zero seed-ledger` | `--ledger`（既存の台帳）、`--name`、`--seed`、`--games` | 実行器を通さずに生成するシードを台帳に登録するとき |
| `minase_zero resign-calibration` | `--summary`（直前のサイクルの生成器の集計`summary.json`。複数指定可） | 投了を有効にできるかを手で確かめるとき |

### 引数で変えられない固定値

変えるにはソースの変更が要る。
生成器の場所は`crates/minase/src/`から、学習器の場所は`tools/train/zero/minase_zero/`からの相対パスである。

| 項目 | 値 | 場所 |
|---|---|---|
| 規則 | `engine-default` | `bin/minase/zero_selfplay/mod.rs` |
| 手数の上限 | 序盤を含めて3,000手。到達した対局は捨てる | `bin/minase/zero_selfplay/mod.rs`の`MAX_PLY` |
| 開始局面の作り方 | `random-opening`は初期局面から8〜16手を一様に選んで指す。`shuffled-setup`は歩兵と仲人を動かさず、1〜3段目の駒を段の中で並べ替え、2枚の角行を筋の偶奇の異なるマスに置き、後手を先手と点対称に置く | `bin/minase/zero_selfplay/game.rs`、`crates/minase-core/src/position/setup.rs`の`Position::shuffled_initial` |
| 完全探索の確率 | 0.25。それ以外の手番は回数n/4の速い探索 | `bin/minase/zero_selfplay/game.rs`の`choose_full_search` |
| Gumbelの候補数の上限 | 16 | `search/mcts/gumbel.rs` |
| Qの変換の係数 | c_visit=50、c_scale=1.0 | `search/mcts/gumbel.rs`、`bin/minase/zero_selfplay/mod.rs`の`GUMBEL_VALUE_SCALE` |
| 指す手の温度 | 0.15＋0.85×0.5^(手数/12) | `bin/minase/zero_selfplay/game.rs`の`move_temperature` |
| 3手以内の勝ちによる打ち切り | 王手だけを読む | `search/mate_in_three.rs` |
| 探索木の容量 | 1局あたり16 MiB | `bin/minase/zero_selfplay/mod.rs`の`TREE_CAPACITY` |
| 投了の閾値 | 選んだ手の勝率が1%未満 | `bin/minase/zero_selfplay/mod.rs`の`RESIGN_THRESHOLD` |
| 投了の誤りの上限 | 5%を超えたサイクルの投了の対局を学習から外す | `bin/minase/zero_selfplay/mod.rs`の`FALSE_RESIGNATION_LIMIT` |
| 投了を有効にする条件 | 標本100局以上、誤りの割合のClopper–Pearson法による片側95%上限が5%以下 | `rl.py` |
| 抽出の倍率 | 最新のサイクルの訓練局面数の4倍 | `data.py`の`SAMPLE_MULTIPLIER` |
| 検証の分割 | 対局のハッシュを20で割った余りが0の対局（約5%） | `data.py` |
| 損失の重み | 方策1、価値1、ソフト方策8、相手の方策0.15、相手のソフト方策1.2、短期の価値3項は各1 | `train.py`の`LOSS_WEIGHTS` |
| ソフト方策の目標 | (p＋10⁻⁷)の1/4乗を正規化する | `train.py` |
| ミクロバッチ | 256局面 | `runner.py`の`MICRO_BATCH_SIZE` |
| 学習の順伝播のコンパイル | `torch.compile`の既定のモード、`fullgraph=True`、DDPの内側。Inductorの`emulate_precision_casts`と`deterministic`を有効にする | `train.py`の`compile_training` |
| サイクルの端数 | 抽出数を全体のバッチで割った余りの局面は、そのサイクルでは使わない | `train.py` |
| ウォームアップ | 初期化から累計2,000,000標本 | `train.py`の`WARMUP_SAMPLES` |
| サイクル内の学習率の予定 | 基準の学習率×(1＋cos(πt/T))/2 | `train.py` |
| 照合の許容誤差 | CPUは方策のlogitと価値が10⁻⁴、TensorRTのFP16は方策の確率が10⁻²、価値が5×10⁻² | `runner.py` |

## 標準レシピの設定と根拠

標準レシピの値は、ADRと利用者の決定で定めたものである。
対局の勝敗で比べて決めたのは、重みの移動平均を使わないこと（1回目の学習で2026年10月31日に学び直す前の、サイクル25の比較）だけである。

| 項目 | 値 | 根拠 |
|---|---|---|
| ネット | 20ブロック256チャネル、入力159面、価値ヘッド32チャネル | [ネットの構成のADR](../adr/start-the-zero-network-as-a-20-block-256-channel-resnet-with-global-pooling-and-a-32-channel-value-head.md) |
| 開始局面 | 事前学習の期間は組み替えた初期配置、微調整の期間は初期局面から8〜16手の乱択 | [組み替えた初期配置のADR](../adr/shuffle-the-first-three-ranks-within-each-rank-for-zero-self-play-start-positions.md)、設計書の「期間の切り替え」の節 |
| 探索 | Gumbel AlphaZero、完全探索32回（確率0.25、それ以外は8回）、候補16、Qの尺度1.0、減衰する指し手の温度 | [Gumbel AlphaZeroのADR](../adr/use-gumbel-alphazero-with-a-unit-q-scale-and-a-decaying-final-move-temperature.md)、[探索回数の乱択のADR](../adr/randomize-zero-self-play-playout-caps-and-learn-the-policy-only-from-full-searches.md) |
| 1サイクルの生成量 | 訓練局面100万。目標に届いた最初の塊で締める | Gumbel dlshogiと同じ量（[1回目の実行の設計書](../plans/archived/alphazero-rl.md)の「生成と学習を交互に行う」の節）、[塊のADR](../adr/commit-zero-self-play-in-fixed-game-chunks-up-to-the-first-prefix-reaching-the-target-positions.md) |
| 生成の並列 | GPUあたり4スレッド、スレッドごとに128局 | 1回目の学習の[試走](../measurements/alphazero-rl/alphazero-rl-trial.md)で比べた組のうち最も速かった（毎秒974局面）。4枚の4090ではGPUあたり8スレッドより速かった（[vast.aiの測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)） |
| 系列の基本シード | サイクル番号×10⁹＋3×10⁸ | [シードの加算値のADR](../adr/take-the-zero-series-seed-offset-as-a-required-runner-argument.md) |
| リプレイバッファと抽出 | パラメータ数の10倍の局面。最古のサイクルは、サイクル13までの学習で1、サイクル14からの学習で3。最新のサイクルの訓練局面数の4倍を一様に抽出する | [リプレイバッファのADR](../adr/sample-four-times-from-a-zero-replay-buffer-of-ten-times-the-parameter-count-from-cycle-3-after-cycle-13.md) |
| 学習率 | 基準0.04、サイクルごとのcosine annealing、累計200万標本のウォームアップ | 全体のバッチ256のときの基準0.01（dlshogiの学習器の既定値）を、バッチに比例させて4倍にした。予定は[学習率のADR](../adr/warm-up-the-zero-learning-rate-from-a-base-of-0-04-for-a-global-batch-of-1024.md)。一定の学習率との比較は打ち切った |
| 最適化器 | ネステロフのモーメンタムつきSGD、係数0.9、重み減衰10⁻⁴、勾配のノルムの上限10 | dlshogiの学習器の既定値（[学習器の設計書](../plans/archived/alphazero-trainer.md)の「着手時に補った決定」の節） |
| 価値の教師 | λ=0.333 | [定数マスクと診断のADR](../adr/mask-impossible-policy-labels-and-diagnose-the-zero-trainer-on-final-weights.md) |
| 方策の損失 | 常に不可能なラベルを除く16,108ラベルの定数マスク。合法手だけでは正規化しない | 同じADR。調べた先行実装の多くが全出力のsoftmaxで学習する（1回目の実行の設計書の「学習が動いてから決める技法」の節） |
| 補助目標 | ソフト方策、相手の次の手、短期の価値 | [補助目標のADR](../adr/train-the-zero-network-on-katago-auxiliary-targets.md)。重みはKataGoの値 |
| バッチと精度 | 全体のバッチ1,024局面をミクロバッチ256局面の勾配累積で学ぶ。bf16の順伝播 | [学習器のDDPのADR](../adr/train-zero-cycles-with-fabric-ddp-over-a-fixed-global-batch-of-1024.md)。ミクロバッチの256は、[実現性の実測](../measurements/alphazero-environment/alphazero-feasibility.md)でL4が最も速かったバッチ |
| 重みの移動平均 | 使わない | 1回目の学習で学び直す前のサイクル25の比較で平均のネットが強いと言えなかった（[ADR](../adr/do-not-keep-a-moving-average-of-the-zero-network-weights.md)） |
| 投了 | 勝率1%未満で投了、較正で有効化、有効なサイクルは20%を最後まで指させる | [投了のADR](../adr/resign-zero-self-play-games-below-one-percent-after-bounding-false-resignations.md) |
| 手数の上限 | 3,000手 | [手数の上限のADR](../adr/discard-zero-self-play-games-that-reach-3000-plies.md)。上限に達した対局の大半は3,000手までに終わる（[長い対局の調査](../measurements/alphazero-rl/alphazero-rl-long-games.md)） |
| 3手以内の勝ちによる打ち切り | 王手だけを読む | [打ち切りのADR](../adr/stop-zero-self-play-at-check-only-wins-within-three-plies-found-under-the-search-module.md)。dlshogiの3手詰めに当たる |
| チェックポイントの間隔 | 50更新 | 監査対応で定めた暫定値の200更新（256局面ずつ）と同じ標本数ごとに保存する |
| ゲーティング | 行わない | AlphaZeroはゲーティングなしでも学習が進む（1回目の実行の設計書の「生成と学習を交互に行う」の節） |

## 詳細の置き場所

| 知りたいこと | 場所 |
|---|---|
| 進め方の規則（費用、進捗の判定、期間の切り替え、探索回数の引き上げ）と現在の状態 | [強化学習の2回目の実行の設計書](../plans/alphazero-rl-2.md) |
| 借りる機械の操作、所要時間とCUの見積もり、レンタル機 | [借りる機械の操作の手引き](alphazero-machines.md) |
| ネットの出力の照合 | [出力の照合の手引き](alphazero-network-check.md) |
| 進捗測定の固定局数とEloの読み方 | [棋力測定の手引き](sprt.md) |
| 実行器の引数、記録の欄、機械をまたぐ再開の規則 | `tools/train/zero/minase_zero/runner.py`のdocstring |
| 学習の出力の欄、損失、抽出、学習率の式、再開 | `tools/train/zero/minase_zero/train.py`のdocstring |
| 生成の条件、開始局面、塊、投了、教師の記録 | `crates/minase/src/bin/minase/zero_selfplay/`の`mod.rs`、`game.rs`、`output.rs` |
| 対局用MCTSのUSIオプション（`NnBackend`、`NnCacheDir`、`NnBatchSize`、`PolicyTemperature`） | `crates/minase/src/search/mcts/engine.rs` |
| ファイル形式の一覧 | [ファイル形式の手引き](file-formats.md) |
