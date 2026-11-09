# AlphaZero型探索の深層強化学習を回す手引き

本書は、AlphaZero型探索のネットを育てる深層強化学習（RL）について、標準レシピ、起動から定例の測定までの手順、止めて再開する操作、およびオプションを定める。
費用、進捗の判定、および規模の引き上げといった進め方の規則と、学習の現在の状態は[強化学習の2回目の実行の設計書](../plans/alphazero-rl-2.md)にある。
本書の標準レシピと手順は、サイクル32で中断した1回目の学習のものであり、2回目の学習に合わせた書き直しは、その設計書のフェーズ1で行う。
本書には、実行者（人間およびエージェント）が手順どおりに実行する部分だけを書く。

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
- 学習器のサブコマンドは、`tools/train/zero`を作業ディレクトリにして`uv run --frozen python -m minase_zero <サブコマンド>`で起動する。Pythonの環境がない作業ツリーでは、`bash tools/train/zero/setup.sh`で導入する。
- 機械を借りる操作は[借りる機械の操作の手引き](alphazero-machines.md)に、ネットの出力の照合は[出力の照合の手引き](alphazero-network-check.md)に、対局の測定の規約は[棋力測定の手引き](sprt.md)にある。

## サイクルの全体像

RLは、自己対局の生成、ネットの学習、および出力の照合を1サイクルとして繰り返す。
生成では、前のサイクルのネットで、訓練局面が目標の100万に届くまで対局する。
学習では、直近の数十サイクル分のデータ（リプレイバッファ）から抽出した局面でネットを更新する。
照合では、学習したネットをminaseが正しく読めることを確かめる。
照合に通ったネットが、次のサイクルの生成に使われる。

サイクルはColab機のL4で回し、結果はそのたびにNASへ受け取る。
NASの主な置き場所は次のとおりである。

| 置き場所 | 内容 |
|---|---|
| `cycles/cycle-NNNN/` | サイクルの生成、学習、および照合の出力 |
| `nets/cycle-NNNN.onnx` | 照合に通って確定したネット |
| `cycles.jsonl` | サイクルごとの記録 |
| `sessions.jsonl` | サイクルごとの時間とCU（Colabの計算単位）の区間 |
| `rentals.jsonl` | 機械を借りた時刻と止めた時刻、および残高 |
| `seed-ledger.jsonl` | 生成に割り当てたシードの台帳 |
| `logs/runner-<セッション名>.log` | 起動スクリプトと実行器のログ |
| `progress/`、`external/` | 作業機の進捗測定と、固定の検証用棋譜の診断の出力 |

1サイクルは、リプレイバッファがサイクル3〜32の時点で約1時間（生成33〜37分、学習22〜25分）、約2 CUであった。
L4の消費レートは1.94 CU/時なので、休まず回すと1日に約47 CUを使う。

## 標準レシピ

2026年11月8日時点の標準である。
2026年10月31日に、それまでの学習をリセットしてランダム初期化のネットから学び直しており（以下、再出発）、標準レシピは再出発の後のサイクル26〜32を回した設定と同じである。
起動スクリプト`launch-session.sh`がL4を1台借りて環境を導入し、実行器`minase_zero run-cycles`を次の引数で走らせる。

```sh
# 作業機で起動する（引数はセッション名、最初のサイクル、最後のサイクル、コミット）
A=/mnt/synology_nas/minase/alphazero
nohup setsid bash docs/measurements/alphazero-rl/alphazero-rl-sessions/launch-session.sh \
  rl-r33 33 50 "$(git rev-parse HEAD)" > "$A/logs/runner-rl-r33.log" 2>&1 < /dev/null &

# スクリプトが走らせる実行器（tools/train/zeroで実行される）
uv run --frozen python -m minase_zero run-cycles --root "$A" --session <セッション名> \
  --cycles-dir cycles --base-cycles-dir cycles --branch-from 0 \
  --generation-commit <コミット> --trainer-commit <コミット> \
  --first-cycle <最初のサイクル> --last-cycle <最後のサイクル> \
  --deadline <借りた時刻の8時間後> --cycle-minutes 80 --cycle-cu 2.4 \
  --target-positions 1000000 --chunks 20 --simulations 32 --threads 4 --games-per-thread 128 \
  --replay-buffer-first-cycle 3 --replay-buffer-positions 47802000 \
  --batch-size 256 --lr 0.01 --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 \
  --value-lambda 0.333 --precision bf16 --checkpoint-steps 200 \
  --train-seed-base 20261100 --minase <作業ツリー>/target/release/minase --poll-seconds 60
```

実行器は、各サイクルでColab機の生成器と学習器を次の引数で起動する。
山括弧の値は実行器がサイクルごとに決める（[`minase data zero-selfplay`](#minase-data-zero-selfplay)と[`minase_zero train`](#minase_zero-train)の表）。

```sh
# 生成
minase data zero-selfplay --output-dir cycles/cycle-NNNN/selfplay \
  --nn-model <前のサイクルのネット> --nn-backend tensorrt --generation-commit <コミット> \
  --seed <サイクル番号×10⁹> --games <見積もった対局数> --chunk-games <対局数÷20の切り上げ> \
  --target-positions 1000000 --simulations 32 --playthrough-percent <100か20> \
  --threads 4 --games-per-thread 128

# 学習
python -m minase_zero train --init <前のサイクルのcheckpoint.pt> \
  --data <リプレイバッファのファイル...> --latest-data <最新のサイクルのファイル...> \
  --out-dir cycles/cycle-NNNN/train --cycle NNNN --seed <20261100＋サイクル番号> \
  --batch-size 256 --lr 0.01 --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 \
  --value-lambda 0.333 --precision bf16 --checkpoint-steps 200 \
  --device cuda --trainer-commit <コミット>
```

ネットは10ブロック128チャネル（推論で約478万パラメータ）であり、`--replay-buffer-positions`はその10倍である。
ネットの大きさは学習器の`init`で決まり、以後のサイクルは前のサイクルのチェックポイントから続ける。

**止まる条件と見直す条件**

| 時点 | 条件 | 対応 |
|---|---|---|
| サイクルの開始前 | 終了期限まで80分未満か、残高が2.4 CU未満 | 実行器が次のサイクルを始めずに正常終了する |
| 生成の後 | 投了の誤りが5%を超えた | 実行器が投了の対局を学習から外し、投了なしで生成し直す |
| 生成の後 | 手数の上限で捨てた対局が10%を超えた | 上限または投了の条件を見直す |
| 照合 | `check status=1` | 実行器が止まる。[止まったときに調べる](#止まったときに調べる)に従う |
| 進捗測定 | 10サイクル前のネットに対するEloの点推定が2回続けて0以下 | 設計書の「伸びが止まったらネットと探索回数を段階的に増やす」の節に従う |

## 手順

次の順に進める。

1. [起動の前に確かめる](#起動の前に確かめる)
2. [セッションを起動する](#セッションを起動する)
3. [経過を確かめる](#経過を確かめる)
4. [定例の測定を行う](#定例の測定を行う)

### 起動の前に確かめる

作業機で次の3点を確かめる。
最初の1回だけは、[借りる機械の操作の手引き](alphazero-machines.md)の1節の準備も必要である。

```console
colab usage                                   # 認証が切れていれば、利用者が対話できる端末で実行して認証し直す
git status && git rev-parse HEAD              # 借りた機械へ送るコミットを確かめる
cargo build --release --bin minase            # 実行器が各サイクルの集計に使う
```

借りた機械には指定したコミットの書庫を送るので、コミットしていない変更は使われない。

残高は、次に回すサイクルの数に2 CUを掛けた値より多いことを確かめる。
足りない場合の扱いは、設計書の「費用の規則」の節が定める。

### セッションを起動する

[標準レシピ](#標準レシピ)の起動コマンドで、セッションを1つ起動する。
最初のサイクルには、`cycles.jsonl`の最後のサイクル番号（`tail -1 $A/cycles.jsonl | jq .cycle`で確かめる）に1を加えた番号を指定する。
最後のサイクルには、次の測定を行うサイクルなど、止めたい番号を指定する。
セッション名は、`rentals.jsonl`の過去の名前と重ならないものにする。

スクリプトはL4を1台借り、環境を導入して（3〜17分）実行器を走らせる。
実行器が終わると、終了コードによらず機械を返し、借用の記録を`rentals.jsonl`に追記する。
スクリプトが渡す`--replay-buffer-first-cycle 3`は`--first-cycle`以下でなければならないので、スクリプトは最初のサイクルが3以上のときだけ使える。
サイクル1から始める系列では`run-cycles`を直接起動し、`--first-cycle 1`に`--replay-buffer-first-cycle 1`と`--cycle1-reference-summary`を添える（[オプション一覧](#minase_zero-run-cycles)）。

1つのセッションには、借りた時刻から8時間の終了期限がある。
実行器は、期限までに80分残っていないとき、または残高が2.4 CUを下回るときに次のサイクルを始めず、正常に終了する。
このため、1セッションで進むのは7サイクル前後である。
実行器が走っている作業ツリーは、セッションが終わるまで消さない。

前のセッションが止まったら、次のセッションを同じ手順で起動する。
待たずに続けたい場合は、前のセッションの終了を待って次のセッションを起動するシェルを、作業機で動かしておく。
次の例は、前のセッションが正常に終わったときだけ、続きのサイクルから起動する。

```bash
L=$A/logs/runner-rl-r33.log
until grep -q chain-done "$L"; do sleep 60; done
if grep -q "runner exit 0" "$L"; then
    next=$(( $(tail -1 "$A/cycles.jsonl" | jq .cycle) + 1 ))
    bash docs/measurements/alphazero-rl/alphazero-rl-sessions/launch-session.sh rl-r$next $next 50 "$(git rev-parse HEAD)"
fi
```

この例は、サイクル50に届いて止まった場合も次の番号で起動してしまうので、使うときは最後のサイクルに達したかどうかの判定を加える。

### 経過を確かめる

実行器は、工程ごとの状態を1分ごとにログへ書く。

```console
tail -f $A/logs/runner-rl-r33.log
pgrep -af "[l]aunch-session|[r]un-cycles"
colab sessions; colab usage
```

`pgrep`の形式は、自分のコマンド行に一致しないように角括弧を入れてある。
この形式を使わずに`kill $(pgrep -f ...)`とすると、自分のシェルを止めてしまう。
ログの行の意味は次のとおりである。

| ログの行 | 意味 |
|---|---|
| `cycle=NNNN generation games=… playthrough_percent=…` | 生成の開始。`games`は対局数の上限であり、`playthrough_percent`が100なら投了なし、20なら投了ありである |
| `cycle=NNNN selfplay status=running received=…` | 生成中。`status=0`で生成の完了を示す |
| `cycle=NNNN train`、`cycle=NNNN check` | 学習と出力の照合。`status=0`が完了であり、`check status=1`は照合の不合格である |
| `cycle=NNNN complete balance=…` | サイクルの完了と残高 |
| `deadline does not allow another cycle` | 終了期限により、次のサイクルを始めずに終わる |
| `runner exit <コード>`、`chain-done` | 実行器の終了と、機械の返却を含むスクリプトの終了 |

サイクルが終わったら、`cycles.jsonl`の最後の行で次の項目を見る。
どれも学習が健全に進んでいるかの目安であり、基準を外れたら設計書の該当する節に従う。

| 項目 | `cycles.jsonl`の欄 | 目安 |
|---|---|---|
| 手数の上限で捨てた対局 | `selfplay_summary`の`discarded_max_ply`÷`games` | 10%を超えたら、上限または投了の条件を見直す |
| 1局の手数と引き分け | `selfplay_summary`の`plies`÷`games`と`draws`÷`games` | 伸び続ける場合は、引き延ばしを疑う（[定例の測定](#定例の測定を行う)） |
| 投了の誤り | `selfplay_summary`の`false_resignations`÷`resign_triggered_playthrough_games` | 5%を超えたサイクルは、実行器が投了の対局を除いて自動で生成し直す |
| 価値の損失 | `training_summary.diagnostics`の`validation`の`value_result` | ln 2（約0.693）より小さいのが正常である。`latest_training_sample`との差が広がったら、過学習を疑う |
| 合法手に乗る確率質量 | `training_summary.diagnostics`の`validation`の`legal_probability_mass` | 1に近い（サイクル30前後で約0.996） |

サイクル間で検証集合が変わるので、損失の1サイクルごとの増減には、学習の良し悪しのほかに、生成した局面の分布の変化も影響する。
伸びの判断には、次の節の対局の結果を使う。

### 定例の測定を行う

10サイクルごとに、作業機で進捗測定と診断を行う。
どれもノード数で制限した対局か、時間に依存しない処理なので、学習のセッションを動かしたまま実行してよい。
サイクル40の場合の手順は次のとおりである（全体で約2時間半）。

```console
D=docs/measurements/alphazero-rl/alphazero-rl-restart-progress-c0040
mkdir -p $D
cp docs/measurements/alphazero-rl/alphazero-rl-restart-progress-c0030/{check.sh,progress-workstation.sh} $D/
bash $D/check.sh 0040
bash $D/progress-workstation.sh 0040 0030 16 <シード> <シード+1> $A/progress
```

`check.sh`は作業機でネットの出力を照合する。
`progress-workstation.sh`は、新しいネットと10サイクル前のネットをPUCTの1手800シミュレーションで200ペア対局させ、続けて探索なしの新しいネットを乱択と100ペア対局させる。
第3引数の16は推論のバッチの大きさ`NnBatchSize`であり、[10サイクル目の進捗測定](../measurements/alphazero-rl/alphazero-rl-progress-c0010.md)で決めた値に固定する。
シードは過去の測定で使っていない値にする。

固定の検証用棋譜の診断は、前回の測定より後の各サイクルについて行う。

```console
cd tools/train/zero
uv run --frozen python -m minase_zero external-eval --checkpoint $A/cycles/cycle-NNNN/train/checkpoint.pt \
  --data <リポジトリの場所>/data/nnue-lr-schedule-gen/generated-8200000.bin --games 1000 --seed 20261030 \
  --device cuda --out $A/external/cycle-NNNN.json
```

引き延ばしの兆候は、`alphazero-rl-long-games/`のスクリプトで集計する。
800手以上の対局について、手数ごとの駒数と評価値、および600手目に有利な側が勝った割合を出力する。

```console
cd docs/measurements/alphazero-rl/alphazero-rl-long-games
uv run --frozen --project <リポジトリの場所>/tools/train/zero python long_trend.py $A/cycles/cycle-NNNN
```

結果は、`docs/measurements/alphazero-rl/alphazero-rl-restart-progress-cNNNN.md`に記録する（先例は[30サイクル目の記録](../measurements/alphazero-rl/alphazero-rl-restart-progress-c0030.md)）。
10サイクル前のネットに対するEloの点推定が2回続けて0以下になったら、設計書の「伸びが止まったらネットと探索回数を段階的に増やす」の節に従う。

設計書が定める間隔に達したときや、進め方を決める必要が生じたときは、最新のネットのMCTSとαβ探索を、[棋力測定の手引き](sprt.md)の短時間条件（STC）で対局させる。
先例は[サイクル32の対局](../measurements/alphazero-rl/alphazero-rl-restart-stc-c0032.md)であり、`alphazero-rl-restart-stc-c0032/run-stc.sh`を複製して、ネットとシードだけを変えて実行する。
同時対局数1で1ペア約1分かかる。
時間制御の対局は負荷の影響を受けるので、対局の間は作業機で重い処理を動かさない。
この対局は到達の判定ではなく、判定の扱いは[統括計画](../plans/alphazero.md)の「到達の判定」の節が定める。

## 必要なときの操作

### 止めて再開する

止めるだけなら、次のセッションを起動しなければよい。
セッションは終了期限か最後のサイクルで自動で止まり、機械を返す。
全サイクルの出力はNASにあるので、止めても出力は失われない。

途中で止める場合は、作業機の実行器を止める。
起動スクリプトが続けて機械を返すので、`colab stop`は別に実行しなくてよい。

```console
kill -TERM $(pgrep -f "[.]venv/bin/python -m minase_zero run-cycles")
```

止めたら、理由を`rentals.jsonl`に1行追記する。
途中で止めたサイクルの消費は`sessions.jsonl`に入らないので、合計は`rentals.jsonl`の残高で確かめる。

再開は、[セッションを起動する](#セッションを起動する)の手順で、止まったサイクルの番号から起動する。
同じコミットで起動すれば、受け取り済みの生成の塊と学習のチェックポイントから続きを行い、NASで完了した工程はやり直さない。
実行器のコードを変えて再開するときは、次の制約がある。

- 生成が完了している場合は、`selfplay/summary.json`がNASにあれば、生成時のコミットが異なっていてもそのまま受け入れる。
- 生成が途中の場合は、コミットが異なると実行器はエラーで止まる。
- 学習が途中の場合は、学習器が同じ局面を同じ順に読むことを照合するので、リプレイバッファや学習の設定を変えると再開できない。

コードを切り替えるときは、生成が終わって学習が始まった直後に実行器を止めるのが最も無駄が少ない。
その場合は、そのサイクルの`train.started.json`、`train/`などの学習の出力を、削除せずに`cycles/cycle-NNNN/rejected-<理由>/`へ移してから、新しいコミットで同じサイクルから起動する。

### 止まったときに調べる

実行器が止まったら、ログの末尾のエラーと、該当サイクルの`cycles/cycle-NNNN/`の状態ファイル（`*.started.json`、`*.exit`、`*.log`）を見る。
機械は起動スクリプトが返しているので、調べている間にCUは減らない。

出力の照合で止まった場合（`check status=1`）は、`check/`の`*.compare.exit`と`*.compare.err`で不合格の組を確かめる。
照合は、PyTorchでの計算を基準に、同じネットをONNX RuntimeのCPUとTensorRTのFP16で計算した結果を、288局面で比べるものである。
目的はONNXへの書き出しの誤りや、minaseが別の手の出力を読む誤りを見つけることであり、誤りがあれば差は桁違いに大きくなると見込まれる。
CPUの照合が通っていてFP16の差だけが許容誤差をわずかに超えた場合は、FP16の丸めの累積とみられる。
ただし許容誤差は[照合のADR](../adr/compare-fp16-zero-network-values-within-five-hundredths.md)の決定なので、実行者の判断では変えず、数値を添えて利用者に判断を仰ぐ。
学習をやり直すときは、不合格の出力を[止めて再開する](#止めて再開する)と同じく`rejected-<理由>/`へ移す。

そのほかの止まり方の対応は次のとおりである。

| 止まり方 | 対応 |
|---|---|
| 接続の切断 | 実行器は接続の終了コード255とHTTP 429を30秒間隔で5回まで再試行する。それでも切れた場合や、Colab側に機械を回収された場合は、同じコミットで再開する |
| 認証の切れ | `colab`が401か403を返したら、利用者が端末で`colab usage`を実行して認証し直す |
| 機械を借りられない | `colab new`が失敗してもCUは減らない。同じ日に3回続けて借りられなければ、GPU種別を決め直す |
| 残高の不足 | 実行器は正常に終了する。設計書の「費用の規則」の節に従う |

計算を止めた後に判断を待つときは、機械を返したことを`colab sessions`で確かめる。

### 設定を変える

標準レシピの値は`launch-session.sh`の引数にある。
探索回数、ネットの大きさ、学習率などは、まず設計書の規則に照らして変更を決める。
変更は次の順に進める。

1. 判断の理由をADRに書き、変えた時点を設計書の「状態」に書く。
2. まだ測っていない構成なら、`--cycles-dir`に本線以外の名前を指定した別系列で1サイクルの試走を行い、時間とCUを確かめる（[借りる機械の操作の手引き](alphazero-machines.md)の6節）。
3. コードの変更はブランチで行い、起動はそのブランチの作業ツリーの`launch-session.sh`から行ってよい。スクリプトは自分の置かれた作業ツリーの実行器を使う。

ネットの大きさを変えるときは、`init`で新しいネットを作り、[出力の照合の手引き](alphazero-network-check.md)で照合してから、その系列の`cycle-0000/`に置く。
先例は[学習技法の導入の照合](../measurements/alphazero-restart-techniques/alphazero-restart-techniques-random-net-output-check.md)である。
`--replay-buffer-positions`は新しいネットのパラメータ数の10倍に変える。

NASの記録はコミットのハッシュを参照するので、記録に現れたコミットのブランチはリベースせず、`--no-ff`でマージする。
生成のシードは実行器がサイクル番号から決めるので、実行器は別系列の分も含めてシードの台帳（`seed-ledger.jsonl`）で重なりを検査する。

## オプション一覧

学習器、実行器、および生成器の引数はすべて必須で、既定値を持たない。
表の「標準の値」は、[標準レシピ](#標準レシピ)が渡す値である。
値域は引数を検査するソースから写したものであり、食い違うときはソースと`--help`を正とする。

### `minase_zero run-cycles`

実行器の引数と再開の契約は、`tools/train/zero/minase_zero/runner.py`のdocstringが定める。

**系列と置き場所**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--root <DIR>` | `$A` | NASの置き場所。Colab機の`/content/az`と同じ相対パスを使う。 |
| `--cycles-dir <DIR>` | `cycles` | 出力を置く`--root`からの相対ディレクトリ。`..`と空白を含めない。`cycles`以外なら別系列になり、記録は`<cycles-dir>.jsonl`、ネットは`nets/<cycles-dir>/`に置く。 |
| `--base-cycles-dir <DIR>` | `cycles` | `--branch-from`以下のサイクルを読む相対ディレクトリ。 |
| `--branch-from <N>` | 0 | 分岐するサイクル番号。0以上`--first-cycle`未満。 |
| `--session <NAME>` | 起動の第1引数 | セッション名。英数字、`_`、`-`だけ。 |
| `--minase <FILE>` | 作業ツリーの`target/release/minase` | 作業機で集計に使う実行ファイル。借りた機械で使うものとは別である。 |
| `--poll-seconds <S>` | 60 | 状態を確かめる間隔。正の有限値。 |

**コミット、サイクル、および期限**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--generation-commit <HASH>` | 起動の第4引数 | 生成器のコミット。40桁の16進数字。未完了の生成は、これが一致する場合だけ再開する。 |
| `--trainer-commit <HASH>` | 起動の第4引数 | 学習器のコミット。40桁の16進数字。Colab機にはこのコミットの書庫を展開する。 |
| `--first-cycle <N>` | 起動の第2引数 | 1以上。 |
| `--last-cycle <N>` | 起動の第3引数 | `--first-cycle`以上。 |
| `--cycle1-reference-summary <FILE>` | 指定しない | サイクル1の対局数を見積もる生成の集計。`--first-cycle`が1のときだけ必須で、それ以外では指定できない。 |
| `--deadline <TIME>` | 借りた時刻の8時間後 | 時差を含むISO 8601の時刻。 |
| `--cycle-minutes <M>` | 80 | 期限までの残りがこれより短ければ、次のサイクルを始めない。正の有限値。 |
| `--cycle-cu <CU>` | 2.4 | 残高がこれを下回れば、次のサイクルを始めない。正の有限値。 |

**生成**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--target-positions <N>` | 1000000 | 1サイクルの訓練局面数の目標。1以上。 |
| `--chunks <N>` | 20 | 対局数の上限を分ける塊の数。1以上。 |
| `--simulations <N>` | 32 | 完全探索の回数。速い探索はこの4分の1。4以上の4の倍数（生成器が検査する）。 |
| `--threads <N>` | 4 | 生成器のスレッド数。1以上。 |
| `--games-per-thread <N>` | 128 | スレッドごとの同時対局数。1以上。 |

**リプレイバッファ**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--replay-buffer-first-cycle <N>` | 3 | リプレイバッファがさかのぼる最も古いサイクル。1以上`--first-cycle`以下。別系列でも本線と同じ通し番号を使う。 |
| `--replay-buffer-positions <N>` | 47802000 | リプレイバッファの局面数の目標。最新のサイクルからさかのぼり、生成し直しを含む`training_positions`の合計が初めてこの値以上になるまでをリプレイバッファとする。1以上。 |

**学習**

| 引数 | 標準の値 | 値域と意味 |
|---|---|---|
| `--batch-size <N>` | 256 | 1更新の局面数。1以上。 |
| `--lr <VALUE>` | 0.01 | 基準の学習率。正の有限値。 |
| `--momentum <VALUE>` | 0.9 | ネステロフのモーメンタムの係数。正の有限値。 |
| `--weight-decay <VALUE>` | 0.0001 | 重み減衰。非負の有限値。 |
| `--clip-grad-norm <VALUE>` | 10 | 勾配のノルムの上限。正の有限値。 |
| `--value-lambda <VALUE>` | 0.333 | 価値の教師で根の価値に掛ける割合λ。0以上1以下。 |
| `--precision <NAME>` | `bf16` | `bf16`（順伝播だけをbf16で計算）または`fp32`。 |
| `--checkpoint-steps <N>` | 200 | チェックポイントを保存する更新回数の間隔。1以上。 |
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
| `--seed <N>` | サイクル番号×10⁹ | サイクルの基本シード。別系列では7×10⁸を、生成し直しでは5×10⁸を加える。 |
| `--games <N>` | 前のサイクルの1局あたりの訓練局面数から、`--target-positions`に要る対局数を切り上げた値 | 対局数の上限。1以上。生成し直しでは本体の投了の対局数。 |
| `--target-positions <N>` | `--target-positions` | 塊0からの累計がこれに届いた最初の塊までをサイクルに含める。1以上。生成し直しでは本体の投了の対局の局面数。 |
| `--chunk-games <N>` | `--games`を`--chunks`で割って切り上げた値 | 1つの塊の対局数。1以上。 |
| `--simulations <N>` | `--simulations` | 完全探索の回数。4以上の4の倍数。 |
| `--playthrough-percent <N>` | 投了が無効なら100、有効なら20（生成し直しでは100） | 投了させずに最後まで指させる対局の割合。1以上100以下。 |
| `--threads <N>` | `--threads` | 推論器を1つずつ持つスレッドの数。1以上。 |
| `--games-per-thread <N>` | `--games-per-thread` | スレッドごとの同時対局数であり、推論の最大バッチでもある。1以上。 |

投了の有効と無効は、直前のサイクルの較正（`resign-calibration`）で実行器が決める。

### `minase_zero train`

学習器であり、実行器が借りた機械で起動する。
出力先、抽出、学習率の式、および再開の契約は、`tools/train/zero/minase_zero/train.py`のdocstringが定める。

| 引数 | 実行器が渡す値 | 値域と意味 |
|---|---|---|
| `--init <FILE>` | 前のサイクルの`train/checkpoint.pt`（サイクル1は`cycle-0000/checkpoint.pt`） | すべての更新を終えたチェックポイント。 |
| `--data <FILE...>` | リプレイバッファのファイル | 古いサイクルから、各生成の`training_files`の順に並べ、生成し直しを本体の後に加える。 |
| `--latest-data <FILE...>` | 最新のサイクルのファイル | `--data`の末尾とパスの文字列も順も一致させる。 |
| `--out-dir <DIR>` | `<cycles-dir>/cycle-NNNN/train` | 存在しないか空のディレクトリ。 |
| `--cycle <N>` | サイクル番号 | 0以上。 |
| `--seed <N>` | `--train-seed-base`＋サイクル番号 | 抽出と更新順の乱数シード。uint64。 |
| `--batch-size`、`--lr`、`--momentum`、`--weight-decay`、`--clip-grad-norm`、`--value-lambda`、`--precision`、`--checkpoint-steps` | `run-cycles`の同名の引数 | `run-cycles`と同じ。 |
| `--device <NAME>` | `cuda` | `cuda`または`cpu`。 |
| `--trainer-commit <HASH>` | `--trainer-commit` | 40桁の16進数字。 |

中断した学習は、`train --resume <出力先> --device <cuda|cpu>`で続ける。
ほかの設定はチェックポイントから読み、教師ファイルの大きさ、局面数、および抽出の検査和が一致しなければ再開しない。

### その他のサブコマンド

| コマンド | 引数 | 使うとき |
|---|---|---|
| `minase_zero init` | `--blocks`、`--channels`、`--seed`、`--out-dir`（存在しないか空）、`--trainer-commit` | ランダムに初期化したネットを作るとき |
| `minase_zero fixture-eval` | `--checkpoint`、`--onnx`、`--fixture`、`--out` | 固定の局面集でPyTorchの出力を書き出し、照合の基準にするとき |
| `minase_zero external-eval` | `--checkpoint`、`--data`（αβ探索の自己対局のMNSDの列）、`--games`、`--seed`、`--device`、`--out` | 固定の検証用棋譜で診断するとき |
| `minase_zero seed-ledger` | `--ledger`（既存の台帳）、`--name`、`--seed`、`--games` | 実行器を通さずに生成するシードを台帳に登録するとき |
| `minase_zero resign-calibration` | `--summary`（直前のサイクルの生成器の集計の列） | 投了を有効にできるかを手で確かめるとき |

### 引数で変えられない固定値

変えるにはソースの変更が要る。
生成器の場所は`crates/minase/src/`から、学習器の場所は`tools/train/zero/minase_zero/`からの相対パスである。

| 項目 | 値 | 場所 |
|---|---|---|
| 規則 | `engine-default` | `bin/minase/zero_selfplay/mod.rs` |
| 手数の上限 | 序盤を含めて3,000手。到達した対局は捨てる | `bin/minase/zero_selfplay/mod.rs`の`MAX_PLY` |
| 序盤の乱択 | 初期局面から8〜16手を一様に選ぶ | `bin/minase/zero_selfplay/game.rs` |
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
| ウォームアップ | 初期化から累計2,000,000標本 | `train.py`の`WARMUP_SAMPLES` |
| サイクル内の学習率の予定 | 基準の学習率×(1＋cos(πt/T))/2 | `train.py` |
| 照合の許容誤差 | CPUは方策のlogitと価値が10⁻⁴、TensorRTのFP16は方策の確率が10⁻²、価値が5×10⁻² | `runner.py` |

## 標準レシピの設定と根拠

標準レシピの値は、ADRと利用者の決定で定めたものである。
対局で比べて決めたのは、重みの移動平均を使わないこと（リセット前のサイクル25の比較）だけである。

| 項目 | 値 | 根拠 |
|---|---|---|
| ネット | 10ブロック128チャネル、入力159面、価値ヘッド32チャネル | [ネットの構成のADR](../adr/start-the-zero-network-as-a-10-block-128-channel-resnet-with-global-pooling-and-a-32-channel-value-head.md) |
| 探索 | Gumbel AlphaZero、完全探索32回（確率0.25、それ以外は8回）、候補16、Qの尺度1.0、減衰する指し手の温度 | [Gumbel AlphaZeroのADR](../adr/use-gumbel-alphazero-with-a-unit-q-scale-and-a-decaying-final-move-temperature.md)、[探索回数の乱択のADR](../adr/randomize-zero-self-play-playout-caps-and-learn-the-policy-only-from-full-searches.md) |
| 1サイクルの生成量 | 訓練局面100万。目標に届いた最初の塊で締める | Gumbel dlshogiと同じ量（設計書の「生成と学習を交互に行う」の節）、[塊のADR](../adr/commit-zero-self-play-in-fixed-game-chunks-up-to-the-first-prefix-reaching-the-target-positions.md) |
| 生成の並列 | 4スレッド、スレッドごとに128局 | [試走](../measurements/alphazero-rl/alphazero-rl-trial.md)で比べた組のうち最も速かった（毎秒974局面） |
| リプレイバッファと抽出 | サイクル3以後、パラメータ数の10倍の局面。最新のサイクルの訓練局面数の4倍を一様に抽出する | [リプレイバッファのADR](../adr/sample-four-times-from-a-zero-replay-buffer-of-ten-times-the-parameter-count-from-cycle-3.md) |
| 学習率 | 基準0.01、サイクルごとのcosine annealing、累計200万標本のウォームアップ | 基準はdlshogiの学習器の既定値、予定は[学習率の予定のADR](../adr/warm-up-the-zero-learning-rate-and-anneal-it-within-each-cycle-without-comparing-a-constant-rate.md)。一定の学習率との比較は打ち切った |
| 最適化器 | ネステロフのモーメンタムつきSGD、係数0.9、重み減衰10⁻⁴、勾配のノルムの上限10 | dlshogiの学習器の既定値（[学習器の設計書](../plans/archived/alphazero-trainer.md)の「着手時に補った決定」の節） |
| 価値の教師 | λ=0.333 | [定数マスクと診断のADR](../adr/mask-impossible-policy-labels-and-diagnose-the-zero-trainer-on-final-weights.md) |
| 方策の損失 | 常に不可能なラベルを除く16,108ラベルの定数マスク。合法手だけでは正規化しない | 同じADR。調べた先行実装の多くが全出力のsoftmaxで学習する（設計書の「学習が動いてから決める技法」の節） |
| 補助目標 | ソフト方策、相手の次の手、短期の価値 | [補助目標のADR](../adr/train-the-zero-network-on-katago-auxiliary-targets.md)。重みはKataGoの値 |
| バッチと精度 | 256局面、bf16の順伝播 | [実現性の実測](../measurements/alphazero-environment/alphazero-feasibility.md)でL4が最も速かった設定 |
| 重みの移動平均 | 使わない | リセット前のサイクル25の比較で平均のネットが強いと言えなかった（[ADR](../adr/do-not-keep-a-moving-average-of-the-zero-network-weights.md)） |
| 投了 | 勝率1%未満で投了、較正で有効化、有効なサイクルは20%を最後まで指させる | [投了のADR](../adr/resign-zero-self-play-games-below-one-percent-after-bounding-false-resignations.md) |
| 手数の上限 | 3,000手（再出発の後のサイクル9までは1,000手） | [手数の上限のADR](../adr/discard-zero-self-play-games-that-reach-3000-plies.md)。上限に達した対局の大半は3,000手までに終わる（[長い対局の調査](../measurements/alphazero-rl/alphazero-rl-long-games.md)） |
| 3手以内の勝ちによる打ち切り | 王手だけを読む | [打ち切りのADR](../adr/stop-zero-self-play-at-check-only-wins-within-three-plies-found-under-the-search-module.md)。dlshogiの3手詰めに当たる |
| チェックポイントの間隔 | 200更新 | 監査対応で定めた暫定値 |
| ゲーティング | 行わない | AlphaZeroはゲーティングなしでも学習が進む（設計書の「生成と学習を交互に行う」の節） |

## 詳細の置き場所

| 知りたいこと | 場所 |
|---|---|
| 進め方の規則（費用、進捗の判定、規模の引き上げ）と現在の状態 | [強化学習の2回目の実行の設計書](../plans/alphazero-rl-2.md) |
| 借りる機械の操作、所要時間とCUの見積もり、レンタル機 | [借りる機械の操作の手引き](alphazero-machines.md) |
| ネットの出力の照合 | [出力の照合の手引き](alphazero-network-check.md) |
| 進捗測定の固定局数とEloの読み方 | [棋力測定の手引き](sprt.md) |
| 実行器の引数、記録の欄、機械をまたぐ再開の規則 | `tools/train/zero/minase_zero/runner.py`のdocstring |
| 学習の出力の欄、損失、抽出、学習率の式、再開 | `tools/train/zero/minase_zero/train.py`のdocstring |
| 生成の条件、塊、投了、教師の記録 | `crates/minase/src/bin/minase/zero_selfplay/`の`mod.rs`、`game.rs`、`output.rs` |
| 対局用MCTSのUSIオプション（`NnBackend`、`NnCacheDir`、`NnBatchSize`、`PolicyTemperature`） | `crates/minase/src/search/mcts/engine.rs` |
| ファイル形式の一覧 | [ファイル形式の手引き](file-formats.md) |
