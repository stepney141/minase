# AlphaZero型探索の深層強化学習を回す手引き

本書は、AlphaZero型探索のネットを育てる深層強化学習（RL）のサイクルを起動し、経過を確かめ、止めて再開する手順を定める。
本書は実行者（人間およびエージェント）向けの手順だけを書く。
方式を選んだ理由と進め方の規則は[強化学習の実行の設計書](../plans/alphazero-rl.md)に、借りる機械の操作の詳細は[借りる機械の操作の手引き](alphazero-machines.md)に、ネットの出力の照合の詳細は[出力の照合の手引き](alphazero-network-check.md)に、対局の測定の規約は[棋力測定の手引き](sprt.md)にある。
実行器の引数と記録の欄は、`tools/train/zero/minase_zero/runner.py`のdocstringが定める。

## 1. サイクルの全体像

RLは、自己対局の生成、ネットの学習、および出力の照合を1サイクルとして繰り返す。
生成は前のサイクルのネットで訓練局面が目標の100万に届くまで対局し、学習は学習窓から抽出した局面でネットを更新し、照合は学習したネットをminaseが正しく読めることを確かめる。
照合に通ったネットが、次のサイクルの生成に使われる。

サイクルはColab機のL4で回し、結果はそのたびに作業機のNAS（`/mnt/synology_nas/minase/alphazero/`、以下`$A`）へ受け取る。
NASの主な置き場所は次のとおりである。

- **`cycles/cycle-NNNN/`**：サイクルの生成、学習、および照合の出力。
- **`nets/cycle-NNNN.onnx`**：確定したネット。
- **`cycles.jsonl`**：サイクルごとの記録。
- **`sessions.jsonl`**：サイクルごとの時間とCU（Colabの計算単位）の区間。
- **`rentals.jsonl`**：機械を借りた時刻と止めた時刻、および残高。
- **`seed-ledger.jsonl`**：生成に割り当てたシードの台帳。
- **`logs/runner-<セッション名>.log`**：起動スクリプトと実行器のログ。
- **`progress/`、`external/`**：作業機の進捗測定と固定の検証用棋譜の診断の出力。

1サイクルは、学習窓がサイクル3〜32の時点で約1時間（生成33〜37分、学習22〜25分）、約2 CUであった。
L4の消費レートは1.94 CU/時なので、休まず回すと1日に約47 CUを使う。

## 2. 起動の前に確かめること

作業機で次の3点を確かめる。
最初の1回だけは、[借りる機械の操作の手引き](alphazero-machines.md)の1節の準備も要る。

```console
colab usage                                   # 認証が切れていれば、端末で実行し直して認証する
git status && git rev-parse HEAD              # 借りた機械へ送るコミットを確かめる
cargo build --release --bin minase            # 実行器が各サイクルの集計に使う
```

借りた機械には指定したコミットの書庫を送るので、コミットしていない変更は使われない。
Pythonの環境がない作業ツリーでは、`bash tools/train/zero/setup.sh`で導入する。

残高は、次に回すサイクルの数に2 CUを掛けた値より多いことを確かめる。
足りない場合の扱いは、設計書の「費用の規則」の節が定める。

## 3. セッションの起動

サイクルは、次のコマンドで起動する。
最初のサイクルには、`cycles.jsonl`の最後のサイクルの次の番号を指定する（`tail -1 $A/cycles.jsonl | jq .cycle`で確かめる）。
最後のサイクルには、次の測定を行うサイクルなど、止めたい番号を指定する。

```console
A=/mnt/synology_nas/minase/alphazero
C=$(git rev-parse HEAD)
nohup setsid bash docs/measurements/alphazero-rl/alphazero-rl-sessions/launch-session.sh \
  rl-r33 33 50 "$C" > "$A/logs/runner-rl-r33.log" 2>&1 < /dev/null &
```

第1引数のセッション名は、`rentals.jsonl`の過去の名前と重ならないものにする。
スクリプトはL4を1台借り、環境を導入して（3〜17分）実行器を走らせ、実行器の終了コードによらず機械を返し、借用の記録を`rentals.jsonl`に追記する。
学習器の設定はスクリプトの引数にまとめてあり、設定を変えるときの扱いは8節に書く。

1つのセッションには、借りた時刻から8時間の終了期限がある。
実行器は、期限までに80分残っていないとき、または残高が2.4 CUを下回るときに次のサイクルを始めず、正常に終了する。
このため、1セッションで進むのは7サイクル前後である。
実行器が走っている作業ツリーは、セッションが終わるまで消さない。

前のセッションが止まったら、次のセッションを同じ手順で起動する。
待たずに続けたい場合は、前のセッションの終了を待って次を起動する処理を仕掛ける。
次の例は、前のセッションが正常に終わったときだけ、続きのサイクルから起動する。

```bash
L=$A/logs/runner-rl-r33.log
until grep -q chain-done "$L"; do sleep 60; done
if grep -q "runner exit 0" "$L"; then
    next=$(( $(tail -1 "$A/cycles.jsonl" | jq .cycle) + 1 ))
    bash docs/measurements/alphazero-rl/alphazero-rl-sessions/launch-session.sh rl-r$next $next 50 "$C"
fi
```

サイクル50に届いて止まった場合は、この例では次の番号で起動してしまうので、最後のサイクルに達したかどうかの確認を足す。

## 4. 経過の確かめ方

実行器は、工程ごとの状態を1分ごとにログへ書く。

```console
tail -f $A/logs/runner-rl-r33.log
pgrep -af "[l]aunch-session|[r]un-cycles"
colab sessions; colab usage
```

ログの行の意味は次のとおりである。
`pgrep`の形式は、自分のコマンド行に一致しないように角括弧を入れてある（この形式を使わずに`kill $(pgrep -f ...)`とすると、自分のシェルを止めてしまう）。

- **`cycle=NNNN generation games=… playthrough_percent=…`**：生成の開始。`games`は対局数の上限であり、`playthrough_percent`が100なら投了なし、20なら投了ありである。
- **`cycle=NNNN selfplay status=running received=…`**：生成中。`status=0`で生成の完了を示す。
- **`cycle=NNNN train`、`cycle=NNNN check`**：学習と出力の照合。`status=0`が完了であり、`check status=1`は照合の不合格である（7節）。
- **`cycle=NNNN complete balance=…`**：サイクルの完了と残高。
- **`deadline does not allow another cycle`**：終了期限により、次のサイクルを始めずに終わる。
- **`runner exit <コード>`、`chain-done`**：実行器の終了と、機械の返却を含むスクリプトの終了。

サイクルが終わったら、`cycles.jsonl`の最後の行で次の項目を見る。
どれも学習が健全に進んでいるかの目安であり、基準を外れたら設計書の該当する節に従う。

| 項目 | `cycles.jsonl`の欄 | 目安 |
|---|---|---|
| 手数の上限で捨てた対局 | `selfplay_summary`の`discarded_max_ply`÷`games` | 10%を超えたら、上限または投了の条件を見直す |
| 1局の手数と引き分け | `selfplay_summary`の`plies`÷`games`と`draws`÷`games` | 伸び続ける場合は、引き延ばしを疑う（5節） |
| 投了の誤り | `selfplay_summary`の`false_resignations`÷`resign_triggered_playthrough_games` | 5%を超えたサイクルは、実行器が投了の対局を除いて自動で生成し直す |
| 価値の損失 | `training_summary.diagnostics`の`validation`の`value_result` | ln 2（約0.693）を下回る。`latest_training_sample`との差が広がったら、過学習を疑う |
| 合法手に乗る確率 | `training_summary.diagnostics`の`validation`の`legal_probability_mass` | 1に近い（今は約0.996） |

サイクル間で検証集合が変わるので、損失の1サイクルごとの増減には、学習の良し悪しのほかに生成の分布の変化が入る。
伸びの判断には、次節の対局の結果を使う。

## 5. 定例の測定

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

`check.sh`は作業機でネットの出力を照合し、`progress-workstation.sh`は新しいネットと10サイクル前のネットをPUCTの1手800シミュレーションで200ペア対局させ、続けて探索なしの新しいネットを乱択と100ペア対局させる。
シードは過去の測定で使っていない値にする。

固定の検証用棋譜の診断は、前回の測定の後のサイクルごとに行う。

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

設計書が定める間隔に達したときや、進め方を決める必要が生じたときに、最新のネットのMCTSとαβ探索をSTCで対局させる。
先例は[サイクル32の対局](../measurements/alphazero-rl/alphazero-rl-restart-stc-c0032.md)であり、`alphazero-rl-restart-stc-c0032/run-stc.sh`を複製して、ネットとシードだけを変えて実行する。
同時対局数1で1ペア約1分かかる。
時間制御の対局は負荷の影響を受けるので、対局の間は作業機で重い処理を動かさない。
この対局は到達の判定ではなく、判定の扱いは[統括計画](../plans/alphazero.md)の「到達の判定」の節が定める。

## 6. 止め方と再開

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

再開は、3節の手順で、止まったサイクルの番号から起動する。
同じコミットで起動すれば、受け取り済みの生成の塊と学習のチェックポイントから続きを行い、NASで完了した工程はやり直さない。
実行器のコードを変えて再開するときは、次の制約がある。

- **生成が完了している場合**：`selfplay/summary.json`がNASにあれば、生成時のコミットが異なっていてもそのまま受け入れる。
- **生成が途中の場合**：コミットが異なると実行器はエラーで止まる。
- **学習が途中の場合**：学習は同じ局面を同じ順に読むことを照合するので、窓や学習の設定を変えると再開できない。

コードを切り替えるときは、生成が終わって学習が始まった直後に実行器を止めるのが最も無駄が少ない。
その場合は、そのサイクルの`train.started.json`、`train/`などの学習の出力を、削除せずに`cycles/cycle-NNNN/rejected-<理由>/`へ移してから、新しいコミットで同じサイクルから起動する。

## 7. 止まったときの調べ方

実行器が止まったら、ログの末尾のエラーと、該当サイクルの`cycles/cycle-NNNN/`の状態ファイル（`*.started.json`、`*.exit`、`*.log`）を見る。
機械は起動スクリプトが返しているので、調べている間にCUは減らない。

出力の照合で止まった場合（`check status=1`）は、`check/`の`*.compare.exit`と`*.compare.err`で不合格の組を確かめる。
照合は、PyTorchでの計算を基準に、同じネットをONNX RuntimeのCPUとTensorRTのFP16で計算した結果を、288局面で比べるものである。
目的はONNXへの書き出しの誤りや、minaseが別の手の出力を読む誤りを見つけることであり、誤りがあれば差は桁違いに大きくなると見込まれる。
CPUの照合が通っていてFP16の差だけが許容誤差をわずかに超えた場合は、FP16の丸めの累積とみられるが、許容誤差は[照合のADR](../adr/compare-fp16-zero-network-values-within-five-hundredths.md)の決定なので、実行者の判断では変えず、数値を添えて判断を仰ぐ。
学習をやり直すときは、不合格の出力を6節と同じく`rejected-<理由>/`へ移す。

そのほかの止まり方の対応は次のとおりである。

- **接続の切断**：実行器は接続の終了コード255とHTTP 429を30秒間隔で5回まで再試行する。それでも切れた場合や、Colab側に機械を回収された場合は、同じコミットで再開する。
- **認証の切れ**：`colab`が401か403を返したら、利用者が端末で`colab usage`を実行して認証し直す。
- **機械を借りられない**：`colab new`が失敗してもCUは減らない。同じ日に3回続けて借りられなければ、種別を決め直す。
- **残高の不足**：実行器は正常に終了する。設計書の「費用の規則」の節に従う。

計算を止めた後に判断を待つときは、機械を返したことを`colab sessions`で確かめる。

## 8. 設定を変えるとき

学習の設定は`launch-session.sh`の引数にある。
探索回数、ネットの大きさ、学習率などを変える場合は、設計書の規則に従ってから変える。
変更は次の順に進める。

1. 判断の理由をADRに書き、変えた時点を設計書の「状態」に書く。
2. まだ測っていない構成なら、`--cycles-dir`に本線以外の名前を指定した別系列で1サイクルの試走を行い、時間とCUを確かめる（[借りる機械の操作の手引き](alphazero-machines.md)の6節）。
3. コードの変更はブランチで行い、起動はそのブランチの作業ツリーの`launch-session.sh`から行ってよい。スクリプトは自分の置かれた作業ツリーの実行器を使う。

NASの記録はコミットのハッシュを参照するので、記録に現れたコミットのブランチはリベースせず、`--no-ff`でマージする。
生成のシードは実行器がサイクル番号から決めるので、実行器は別系列の分も含めて台帳で重なりを検査する。
