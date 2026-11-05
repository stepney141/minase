# NNUEの学習と診断の手順

本書は、NNUE（差分更新可能なニューラルネットワーク）を`minase-trainer`で学習、検査、診断するための標準レシピ、手順、オプションを定める。

- 学習を実行するときは「標準レシピ」と2〜6節、8〜10節を読む。
- 学習器のコードを変更したときは1節と7節を実行する。
- 引数を調べるときは[11節](#11-設定できるオプション)を読む。

**前提**

- 学習を始める前に、[設計書の型](../plans/README.md)に従って目的と基準コミットを定め、[ROADMAP](../ROADMAP.md)に状態を登録する。
- コマンドは、明記した箇所を除いてリポジトリのルートで実行する。
- `minase-trainer`は、`cargo build --release -p minase-trainer`でビルドした`target/release/minase-trainer`を指す。

**詳細の置き場所**

| 知りたいこと | 場所 |
|---|---|
| 計算式と目的関数 | [NNUEによる評価関数と学習の目的関数](evaluation-nnue.md) |
| 採否の判定 | [棋力測定の手引き](sprt.md) |
| 引数の既定値と値域の定義 | `crates/minase-trainer/src/nnue/`の`options.rs`、`train/args.rs`、`schedule.rs` |
| 保存物の各項目 | `crates/minase-trainer/src/nnue/train/record.rs` |
| チェックポイントの形式 | `crates/minase-trainer/src/nnue/model/checkpoint.rs`の`Checkpoint` |
| 学習データの形式、教師値、検証集合の分け方 | `crates/minase/src/training/`と`crates/minase-trainer/src/data/`（PSTとFMの学習器と共通） |
| 方式を選んだ理由 | ADRの[カーネルの書き方](../adr/write-trainer-kernels-in-cuda-cpp-via-nvrtc.md)、[第1層の勾配](../adr/backpropagate-the-first-layer-through-a-reverse-index.md)、[AdamWの式](../adr/use-pytorch-adamw-with-bias-correction.md)、[教師と損失](../adr/train-the-nnue-on-search-value-teachers-without-lookahead.md) |

## 標準レシピ

2026年11月6日時点の標準であり、[世代反復](../plans/nnue-generation-reset.md)の第2世代の学習と同じ条件である。
世代反復では、`scripts/vast/generation_chain.py`がこの手順を続けて実行する。

**コマンド**

```sh
# 学習
minase-trainer nnue train \
  --data <教師...> --base <前の世代のネット> --output-dir <空のパス> \
  --lambda 1.0 --teacher-cp-per-logit calibrate \
  --feature-set P --width 256 --head pairwise-l16-l32 \
  --batch 16384 --lr-schedule cosine:3:0.1 --epochs 50 --patience 5 \
  --seed 1 --device cuda --validation-interval 1000 --threads <スレッド数>

# 診断
minase-trainer diag nnue-removal \
  --float <学習の出力>/best.npz --net <学習の出力>/nnue.bin \
  --data <教師...> --sample 10000 --seed 1 --output <新規の報告.json>

# 補正
minase-trainer nnue fit-linear --base <学習の出力>/nnue.bin \
  --data <教師...> --tau 10 20 40 80 --seed 20261105 --threads 12 --output-dir <空のパス>
```

- `--threads`の実績は、作業機（RTX 3060 Ti）で4、生成機（RTX 5090）で32である。
- エンジンには、補正が出力した`nnue.bin`を埋め込む。

**止める条件**

| 段階 | 次のときに中止する |
|---|---|
| 学習 | `summary.json`の`status`が`candidate`でない。 |
| 診断 | 終了コードが0以外である。 |
| 補正 | `decision.d`が負でない。 |
| 埋め込みの後 | 深さ5の`minase dev bench`が完走しない。 |

**設定と根拠**

対局で測ってから決める残りの項目は、[調査メモ](../research/nnue-training-recipes.md#対局で測ってから決める項目)にある。

| 項目 | 値 | 根拠 |
|---|---|---|
| 教師の量 | 1世代約1億局面、1手2万ノード | 世代反復の設計判断（量を対局で比べてはいない） |
| λ | 1.0 | 第1世代で、第0世代に対してλ=0.75は`H0`、λ=1.0は`H1`（[記録](../measurements/nnue-generation-reset/nnue-generation-reset-gen1-stc.md)） |
| 対局結果の反映 | 学習の後に`fit-linear`で線形の経路だけを補正する | 補正後のネットがλ=1.0のネットに`H1`（[記録](../measurements/nnue-generation-reset/nnue-generation-reset-gen1-review.md)） |
| 構成 | P型、幅256、`pairwise-l16-l32` | [ADR](../adr/compose-the-nnue-of-a-piece-value-initialized-linear-path-and-pairwise-products-with-two-hidden-layers.md)で確定（`H1`なしで確定し、世代の判定で検証中） |
| 最適化手法 | AdamW（β1=0.9、β2=0.999、ε=1e-8） | Stockfishの本番と同じ値（[調査メモ](../research/nnue-training-recipes.md)） |
| 基準の学習率 | 第1層と出力部0.001、線形の経路0.03 | 0.001は先行の初期値の範囲内（線形の経路は単位が違うので比べられない） |
| 学習率の予定 | `cosine:3:0.1` | 一定の1倍にSTCで`H1`（[ADR](../adr/anneal-the-nnue-learning-rate-with-a-cosine-schedule.md)）。総長は未決であり、`cosine:10:0.1`との比較（測定名`nnue-generation-reset-gen2-cosine10-stc`）の結果で見直す |
| 重み減衰 | 第1層と出力部1e-4、線形の経路0 | 値を選んだ根拠の記録なし（先行の多くは0） |
| バッチ | 16,384局面 | 先行の多くと同じ |
| 停止とネットの選択 | 改善なし5エポックで止め、検証損失が最良のネットを書き出す | 慣習（検証損失で選ぶのを標準とする学習器は、調べた範囲ではminaseだけ） |

## 1. 学習器を検査する

学習器を変更したら、学習の前に次の検査を通す。

**GPUなしの検査**

```sh
cargo fmt --all --check
nice -n 19 cargo clippy -j 2 --workspace --all-targets -- -D warnings
nice -n 19 cargo test -j 2 -p minase-trainer
```

**GPUありの検査**（GPUを載せた機械で実行）

```sh
nice -n 19 cargo test -j 2 -p minase-trainer -- --ignored gpu_ --test-threads=1
for FEATURE_SET in P HalfKP-hm-b8; do
  nice -n 19 cargo run -j 2 -p minase-trainer -- nnue check-equivalence \
    --feature-set "$FEATURE_SET" --width 8 --batch 100 --seed 71 --lambda 1.0
  nice -n 19 cargo run -j 2 -p minase-trainer -- nnue check-equivalence \
    --feature-set "$FEATURE_SET" --width 256 --batch 4096 --seed 71 --lambda 1.0
done
```

- `check-equivalence`は、GPUのカーネルがCPUの参照実装と一致することを確かめる。
- `gpu_`のテストは、これに加えて次の2つを確かめる。
  - CUDAで再開した学習が連続した学習と一致する。
  - 途中検証の間隔によらず学習の軌跡が変わらない。
- 許容誤差は`crates/minase-trainer/src/nnue/gpu/equivalence.rs`にある。

## 2. 学習データと初期値を用意する

**学習データ**

- [PSTとFMの学習手順](pst-training.md)の2節と3節に従ってMNSDファイルを生成し、`--data`に入力順で指定する。教師の分類の番号は初出順に付く。
- 学習器は開始時に全ファイルの手順を再生して局面を復元する。この時間は`config.json`の`preprocessing_seconds`に残る。

**初期値（`--base`）**

- `--base`のMNUEファイルから、勝率尺度（1ロジットあたりのcp）、探索用駒価値、規則セット名を引き継ぐ。規則セットは学習データと一致していなければならない。
- `--base`の線形の経路、第1層、出力部の重みは使わない。線形の経路は探索用駒価値から次のように作る。
  - 自駒の行に駒価値を、相手駒の行に符号を反転した値を置く。144升すべてと序中盤用・終盤用の2列に同じ値を置く。
  - 王駒と先獅子の対象升の行は0とする。
- 手書きの評価関数v0から始めるときは、`nnue export-handcrafted`で書き出したネットを`--base`に与える。このネットを埋め込むと、探索はv0の評価で動く。

**教師値**

- `--lambda`は探索値と対局結果を混ぜる比であり、必ず指定する。NNUEの学習では1.0とする。
- λは自己対局のデータにだけ適用され、実戦対局のデータでは常に0である。
- 標準の学習は先読み教師を使わない。比較で使うときは`--lookahead-gamma`と`--lookahead-plies`を同時に指定する。

**教師の勝率尺度（`--teacher-cp-per-logit`）**

- 通常は`calibrate`を指定し、訓練集合から教師の分類ごとに推定する。
- 入力を変えても教師値を固定したい比較では、間引く前の全入力から較正ファイルを作って各学習に渡す。先読み教師の引数も学習と揃える。

```sh
minase-trainer data calibrate --data <教師...> --output <新規の較正.json>
```

- 較正は、学習と同じ検証分割で訓練集合を選ぶ。
- 較正ファイルは、分類の属性（生成コミット、重みの検査和、探索の規模、規則セット、由来、先読み条件）で突き合わせる。λが0でない分類に項目がなければ、学習を始めずに止まる。
- 使った値は、`config.json`の`preprocessing`（`teacher_cp_per_logit`、`teacher_score_multiplier`、`teacher_cp_per_logit_source`）に残る。

## 3. 試走する

本番の前に1ファイルと1エポックで試走し、前処理、更新、検証、保存が最後まで通ることを確かめる。

```sh
for FEATURE_SET in P HalfKP-hm-b8; do
  minase-trainer nnue train \
    --feature-set "$FEATURE_SET" --width 256 \
    --data <1ファイル> --base crates/minase/nets/nnue.bin \
    --lambda 1.0 --teacher-cp-per-logit calibrate \
    --batch 16384 --epochs 1 --seed 1 \
    --output-dir "/tmp/nnue-trial-${FEATURE_SET}"
done
```

- `--output-dir`には、存在しないパスか空のディレクトリを指定する。
- GPUのない機械やCPUの参照経路を試すときは、`--device cpu --threads 2`を加える。既定の`--device cuda`は、GPUが見つからなければエラーで終わる。

## 4. 学習する

- 試走のコマンドの`--data`に全入力を並べ、`--epochs`、`--patience`、`--time-limit-hours`で上限を決める。
- 測定機を他の作業と共用するときは、`--threads`を明示して他の作業の分のコアを残す。

**停止と最良の重み**

- 検証損失が改善しないエポックが`--patience`回続くか、`--epochs`または`--time-limit-hours`の上限に達すると止まる。
- 最良の重みは、学習したエポック（1以上）の中から検証損失で選ぶ。

**主な調整用オプション**（値域と式は[11節](#11-設定できるオプション)）

- `--lr-schedule`: 総長は`--epochs`と独立である。`cosine:<総長>:<下限>`で総長まで学ぶときは、`--patience`で先に止まらないように大きな値にする。
- `--validation-interval`: 1回に数十秒かかるので、1エポックの更新回数（`ceil(訓練局面数 / バッチ)`）の数分の1程度を指定する。診断用であり、最良の重みの選択、早期終了、チェックポイントはエポック境界のまま変わらない。
- `--train-subsample`: 同じ局面数で「少数の対局から全局面」（`games`）と「多数の対局から疎に」（`positions`）を比べるための引数である。

### メモリに収まらない教師データで学習する

- 既定の`--buckets 1`は、全局面を圧縮表現でメモリに持ち、毎エポック全体を並べ替える。
- 収まらないときは、`--buckets <B>`（2以上）、`--bucket-positions <上限>`、`--scratch-dir <ディレクトリ>`を指定する。
- Bが2以上なら、局面を一時ファイルへ書き出し、エポックごとにB個のbucketへ無作為に振り分け、1つずつメモリへ読んで並べ替えて流す。結果は全局面の一様な並べ替えになる（[ADR](../adr/shuffle-the-whole-training-set-per-epoch-with-rao-sandelius-buckets.md)）。

**値の選び方**（Nは訓練局面の数）

- `--buckets <B>`: 次の3つの合計が使える実メモリに収まるように選ぶ。
  - 学習プロセスの固定のメモリ（ネット、最適化器、GPUのドライバなど）。
  - bucket1つ分（局面1つにつき一時表現の138バイトと順列の8バイト）。
  - 先行して組み立てるバッチ（`--threads × --batch`局面）。
- `--bucket-positions <上限>`: `N/B + 6√(N/B)`以上を指定する。超える確率は約10⁻⁹であり、超えたら学習器が局面数を示して止まるので、上限を大きくしてやり直す。
- `--scratch-dir`: 訓練局面1つにつき約300バイトの空きを用意する。一時ファイルは測定機の空き容量の4分の1までとする（2026年10月17日の利用者の決定）。
- 1エポックの読み書きは一時表現の総量の約3倍である。時間への影響は[B=16の測定記録](../measurements/out-of-core-shuffle/out-of-core-shuffle-time-memory.md)にある。

**一時ファイルの後始末**

- 一時ファイルは`<scratch-dir>/nnue-<UNIX秒>-<プロセスID>-<連番>/`に作られ、正常終了と捕捉した異常終了で消える。
- 強制終了や電源断で残ったものは、再開にも別の学習にも使えない。`lock`のプロセスIDが`ps -p <ID>`で実行中でないことを確かめてから、ディレクトリごと`rm -r`で消す。

## 5. 結果を確認する

**保存物**

| ファイル | 内容 |
|---|---|
| `nnue.bin` | エンジンが読む重み |
| `best.npz` | 量子化前の最良モデル（再開用のチェックポイントを兼ねる） |
| `latest.npz` | 最後に検証を終えたモデルの再開用のチェックポイント |
| `summary.json` | 停止理由、最良エポック、量子化検査の結果、`removal_sign` |
| `training.jsonl` | エポックごとの学習曲線（エポック0は学習前の検証） |
| `validation-interval.jsonl` | `--validation-interval`による途中の検証 |
| `config.json` | 引数、入力、前処理の結果、再開の履歴 |

**確かめること**

- `summary.json`の停止理由、最良エポック、量子化検査の結果を確かめる。
- `removal_sign`は、駒を1枚除いたときの評価差の符号が手書きの評価関数v0の駒価値の差と逆になる割合である。閾値はなく学習の成否に影響しない。採否の測定で負けたときの手がかりにする。
- `training.jsonl`の訓練損失、検証損失、符号一致率、非線形出力の標準偏差、飽和率、線形重みの移動量、学習率の倍率を読み、更新の不足や過学習がないかを点検する。
- GPUメモリの欄はGPU全体の使用量であり、他のプロセスの確保も含む。

**失敗の扱い**

- 引数、入力、または再開の条件が満たされないと、学習を始める前に終了コード1で終わる。
- 学習が始まった後に終了コード1で終わるのは次の2つである。
  - 時間の上限に達し、1エポックも検証を終えていない。
  - 量子化した重みとCPUの浮動小数点参照の平均絶対誤差が上限を超え、`summary.json`に`quantization_error`が記録された。この重みは採否の測定にかけず、整数化の段を1つずつ適用して原因を調べる（[教訓](../lessons/check-quantization-scale-against-the-win-rate-scale.md)）。
- 時間の上限で止まった学習は収束を確認していないので、測定記録にもそう書く。

## 6. 中断した学習を再開する

- 最初と同じコマンドに、`--output-dir`と同じパスで`--resume <出力ディレクトリ>`を加える。
- 再開時に変えられるのは、`--epochs`、`--patience`、`--time-limit-hours`、`--threads`、`--validation-interval`、`--bucket-positions`、`--scratch-dir`だけである。
- `--epochs`と`--time-limit-hours`は、再開前を含む総量の上限である。
- それ以外の設定、入力ファイルの識別（パス、バイト長、ヘッダ）、前処理の結果、またはデバイスが違うと、再開を拒否する。
- 最後に確定した`latest.npz`から復元して未確定のエポックをやり直し、`validation-interval.jsonl`のそれより後の行を捨てる。
- 再開の履歴は`config.json`の`resumes`に残るので、測定記録に再開の回数と変更した上限を書く。

## 7. GPUでの1更新の時間を測る

学習器の速度に関わる変更は、`nnue bench-step`で、バッチの供給、入力の転送、GPUの計算に分けて測る。

```sh
for FEATURE_SET in P HalfKP-hm-b8; do
  CARGO_NET_OFFLINE=true nice -n 19 cargo run --release -j 2 -p minase-trainer -- nnue bench-step \
    --feature-set "$FEATURE_SET" --width 256 --batch 16384 --steps 20 \
    --threads 6 --profile --lambda 1.0 --data <MNSDファイル>
done
```

- この例には、訓練集合が344,064局面以上要る。検証集合は使わない。
- `--lambda`は学習と揃える。
- `--profile`はカーネルごとの時間を表示する。`--device cpu`とは併用できない。
- GPUメモリの出力はデバイス全体の使用量と確保の前後の差であり、他のプロセスの変動も含む。
- 測る区間は`crates/minase-trainer/src/nnue/gpu/bench.rs`にある。
- 性能の採否は実GPUの測定で決める。CUDAのコンパイルやGPUなしのテストの結果から速度を推定しない。

## 8. 学習した重みを診断する

採否の測定の前に、次の3つの診断を行う。

- いずれもCPUで動き、`--output`に指定した新規のJSONへ報告を書く。`--output`が既にあれば、何も計算せずに終わる。
- 入力や統計が条件を満たさなければ、報告を書かずに終了コード1で終わる。
- 報告の各項目は`crates/minase-trainer/src/diagnostics/`にある。診断を設けた経緯は[NNUEの完全代替](../plans/archived/nnue-replacement.md)にある。

### 評価値の分布

基準と候補を同じ標本で評価し、散らばりと相関を比べる。

```sh
minase-trainer diag nnue-distribution \
  --pair <比較名> <基準のPST＋FM（FMを持つMNPT）> <候補のNNUE（nnue.bin）> \
  --data <データ...> --sample <標本数> --seed <シード> \
  --output <報告.json>
```

- 標本は全局面から抽出する。`--sample`は2以上で全局面数以下とする。
- `--pair`は複数回指定でき、すべての組を同じ標本で比べる。
- 報告の`pairs`の`verdict`で、標準偏差の比が0.9以上1.1以下に収まるかをまず確かめる。
- 収まらないときは、`by_phase_q`と`by_baseline_absolute_cp`で、どの帯で散らばりが変わったかを確かめる。

### 駒除去差分

検証集合の局面から王駒以外の駒を1枚ずつ除いたときの評価の変化を、量子化の前後で比べる。

```sh
minase-trainer diag nnue-removal \
  --float <出力ディレクトリ>/best.npz --net <出力ディレクトリ>/nnue.bin \
  --data <データ...> --sample <標本数> --seed <シード> \
  --output <報告.json>
```

- `--sample`は1以上で検証集合の局面数以下とする。
- `--float`と`--net`には同じ学習が書き出した組を与える。
- 報告の`reversals`は、手書きの評価関数v0の駒価値による参照と整数のネットで除去差分の符号が反転した件数と割合である。駒損を有利と評価する傾向の目安になる。
- 量子化前の2つの計算経路の差が上限を超えると、`error`を含む報告を保存して終了コード1で終わる。学習器の不具合として調べる。

### 合法手の評価差

指定した局面のすべての合法手について、着手による評価の変化をNNUEとPST＋FMで比べる。

```sh
minase-trainer diag nnue-moves \
  --net <出力ディレクトリ>/nnue.bin --base <PST＋FMの重み> \
  --positions <局面のMNSD> --output <報告.json>
```

- `--base`には、PSTとFMの両方を持つMNPTを与える。
- `--positions`には、ヘッダの規則セットがエンジンの既定の`engine-default`であるMNSDを指定する。由来が合成のMNSDも指定できる。
- 片方のモデルにしかない合法手があると、`error`を含む報告を保存して終了コード1で終わる。規則か局面の復元の不一致として調べる。

## 9. 線形の経路を対局結果で補正する

学習したネットは教師の探索値を写すので、対局結果に現れる配置や駒の損得の情報を持たない。
`nnue fit-linear`は、第1層と出力部を固定し、線形の経路の序中盤の列を対局結果へのロジスティック回帰（Texel tuningと同種）で補正する。

```sh
minase-trainer nnue fit-linear --base <出力ディレクトリ>/nnue.bin \
  --data <学習に使ったデータ...> --tau 10 20 40 80 --seed <シード> \
  --output-dir <新規のディレクトリ>
```

- CPUだけで動き、`--output-dir`に補正後の`nnue.bin`と`summary.json`を書く。教師30ファイルで約2分（幅256、`--threads 8`）、最大常駐メモリは約3.3 GBである。
- `decision.d`は、検証分割の対局で補正ありの損失から補正なしの損失を引いた値Dである。
- Dの95%区間は、`decision.games`から対局単位のブートストラップで求める（例は[`fit_report.py`](../measurements/nnue-outcome-regression/nnue-outcome-regression-fit/fit_report.py)）。上限が0未満なら補正は予測を改善している。0以上なら改善を確認できない。
- 補正は`--base`のネットに対して求めたものなので、ネットを学び直したら求め直す。
- Dと補正の大きさは、教師や`--lambda`を変えたときに残る偏りの目安になる。
- 式と局面の選び方は`crates/minase-trainer/src/nnue/fit_linear/mod.rs`に、方式の理由は[ADR](../adr/correct-the-nnue-linear-path-by-regressing-on-game-outcomes.md)にある。

## 10. 採否を測定して記録する

- 基準コミットから候補ブランチを作り、`nnue.bin`を候補の`crates/minase/nets/nnue.bin`へコピーする。
- 候補で`cargo test --locked`を実行してから、[段階ゲート](sprt.md#機能採否の段階ゲートstcとltc)で採否を決める。
- 検証損失、符号一致率、診断の値の改善だけを、棋力が向上した根拠にしない。
- 結果は`docs/measurements/<設計書名>/<実験名>-training.md`にまとめる。どの設計書も計画していなければ`adhoc/`に置く。
- 記録には、学習器のコミット、コマンドライン全体、入力のパスと記録数、重みのSHA-256、停止理由、診断の要点を書く。

## 11. 設定できるオプション

値はソースの既定値を写したものである。食い違うときはソースと`--help`を正とする。
「再開」の列の○は、`--resume`のときに変えられる引数を示す。

### `nnue train`

**入力と出力**

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--data <FILE...>` | 必須 | MNSDファイルを全体のレコード順に指定する。 |
| `--base <FILE>` | 必須 | 探索用駒価値、勝率尺度、規則セット名を持つMNUEファイル。 |
| `--output-dir <DIR>` | 必須 | 存在しないパスか空のディレクトリ。 |
| `--resume <DIR>` | なし | `--output-dir`と同じ場所から再開する。 |
| `--train-subsample <SPEC>` | 全件 | `games:<k>:<r>`または`positions:<k>:<r>`。k≥2、0≤r<k。 |

`--train-subsample`の補足は次のとおりである。

- 式は`crates/minase-trainer/src/data/subsample.rs`の`TrainSubsample`にある。
- 教師の勝率尺度の推定、先読み教師、観測マスク、初期値は間引く前の訓練集合全体から作る。検証集合は間引かない。
- 1エポックの更新回数、学習率の倍率、`--validation-interval`は抽出後の局面数で決まる。

**教師**

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--lambda <VALUE>` | 必須 | 探索値の割合。0以上1以下。 |
| `--teacher-cp-per-logit <calibrate\|FILE>` | 必須 | 教師の勝率尺度を推定するか、較正JSONで与える。 |
| `--lookahead-gamma <VALUE>` | なし | 先読みの割引率。0より大きく1より小さい。`--lookahead-plies`と同時に指定する。 |
| `--lookahead-plies <N>` | なし | 先読みの手数。1以上4096以下。`--lookahead-gamma`と同時に指定する。 |

**ネットの構成**

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--feature-set <NAME>` | 必須 | `P`（王の位置で条件付けない）または`HalfKP-hm-b8`（自玉の位置を8区分）。 |
| `--width <N>` | 必須 | 第1層の幅。4以上512以下の4の倍数。探索が使うネットは256である。 |
| `--head <NAME>` | `pairwise-l16-l32` | 出力部。この値だけである。式と初期値は`nnue/model/mod.rs`の`HeadKind`と`nnue/reference/head.rs`にある。 |

**最適化**

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--lr-ft <VALUE>` | 0.001 | 第1層と仮想行の学習率。正の有限値。 |
| `--lr-head <VALUE>` | 0.001 | 出力部の学習率。正の有限値。 |
| `--lr-linear <VALUE>` | 0.03 | 線形の経路の学習率。正の有限値。 |
| `--weight-decay-ft <VALUE>` | 0.0001 | 第1層と仮想行の重み減衰。非負の有限値。 |
| `--weight-decay-head <VALUE>` | 0.0001 | 出力部の重み減衰。非負の有限値。 |
| `--weight-decay-linear <VALUE>` | 0 | 線形の経路の重み減衰。非負の有限値。 |
| `--lr-schedule <SPEC>` | `cosine:3:0.1` | 下表の3つの形。 |
| `--batch <N>` | 16384 | 1更新の最大局面数。末尾のバッチは少なくてもよい。 |
| `--seed <N>` | 1 | 初期化、順列作成、検証標本抽出の乱数シード。 |

| `--lr-schedule`の形 | 倍率m(t) | 条件 |
|---|---|---|
| `constant:<c>` | c | c>0 |
| `exponential:<h>:<f>` | max(f, 2^(−t/(h·U))) | h>0、0<f≤1 |
| `cosine:<T>:<f>` | f + (1−f)·(1+cos(π·min(t/(T·U), 1)))/2 | T>0、0<f≤1 |

- tは完了した更新回数、Uは1エポックの更新回数である。
- 倍率は3つのパラメータ群の学習率に同じ値で掛かり、重み減衰にも学習率を通じて掛かる。

**学習量と停止**

| 引数 | 既定値 | 値域と意味 | 再開 |
|---|---|---|---|
| `--epochs <N>` | 50 | 再開前を含むエポック数の上限。1以上50以下。 | ○ |
| `--patience <N>` | 5 | 改善しないエポックがこの数だけ続くと止まる。1以上。 | ○ |
| `--time-limit-hours <VALUE>` | 24 | 前処理、検証、再開前を含む累積時間の上限。24以下。 | ○ |

**検証**

| 引数 | 既定値 | 値域と意味 | 再開 |
|---|---|---|---|
| `--validation-interval <N>` | 0 | この更新回数ごとに検証集合全体を評価して記録する。0は無効。 | ○ |
| `--validation-sample <N>` | 10000 | 量子化誤差の検査に使う検証局面数の上限。1以上。 | |

**実行環境**

| 引数 | 既定値 | 値域と意味 | 再開 |
|---|---|---|---|
| `--device <NAME>` | `cuda` | `cuda`または`cpu`。CUDAが使えなければエラーにする。 | |
| `--threads <N>` | 物理コア数−2（最小1） | バッチ供給とCPU更新のスレッド数。 | ○ |
| `--buckets <N>` | 1 | bucket数。1以上4096以下。1は全局面をメモリに持つ。 | |
| `--bucket-positions <N>` | なし | 1つのbucketの局面数の上限。Bが2以上のとき必須。 | ○ |
| `--scratch-dir <DIR>` | なし | 一時ファイルを置く既存のディレクトリ。Bが2以上のとき必須。 | ○ |

### `nnue fit-linear`

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--base <FILE>` | 必須 | 補正の基点となるMNUEファイル。 |
| `--data <FILE...>` | 必須 | 乱数開始の自己対局を記録したMNSDファイル。入力順を保つ。 |
| `--tau <VALUE...>` | 必須 | 事前分布の標準偏差（cp）。正の有限値で重複なし。 |
| `--seed <N>` | 必須 | 局面の選択と、訓練とτの選択の分割に使う。 |
| `--output-dir <DIR>` | 必須 | 新規または空のディレクトリ。 |
| `--threads <N>` | 物理コア数−2（最小1） | ファイル読み込みの並列度。 |

### その他のサブコマンド

同名の引数の値域と既定値は`nnue train`と同じである。括弧内はそれと異なる値を示す。

| コマンド | 必須の引数 | 任意の引数 |
|---|---|---|
| `nnue export-handcrafted` | `--cp-per-logit <VALUE>`（正の有限値、100,000以下）、`--output <FILE>`（新規のパス） | なし |
| `data calibrate` | `--data`、`--output`（新規のパス） | `--lookahead-gamma`、`--lookahead-plies`、`--threads` |
| `nnue check-equivalence` | `--feature-set`、`--width`、`--batch`、`--seed`、`--lambda` | `--head`、「最適化」の学習率と重み減衰 |
| `nnue bench-step` | `--feature-set`、`--width`、`--batch`、`--steps`（2以上）、`--data`、`--lambda` | `--head`、`--profile`、`--seed`（既定71）、`--threads`、`--device`、`--cp-per-logit`（既定1000）、「最適化」の学習率と重み減衰 |
| `diag nnue-distribution` | `--pair <名前> <MNPT> <MNUE>`（複数可）、`--data`、`--sample`、`--seed`、`--output` | なし |
| `diag nnue-removal` | `--float`、`--net`、`--data`、`--sample`、`--seed`、`--output` | なし |
| `diag nnue-moves` | `--net`、`--base`、`--positions`、`--output` | なし |

### 引数で変えられない固定値

変えるにはソースの変更が要る。場所は`crates/minase-trainer/src/`からの相対パスである。

| 項目 | 値 | 場所 |
|---|---|---|
| AdamWの係数 | β1=0.9、β2=0.999、ε=1e-8 | `nn/adam.rs` |
| 第1層の初期値 | 正規分布、標準偏差√(1/(3D))、Dは基底特徴数 | `nnue/train/mod.rs` |
| 重みの切り詰め（線形の経路） | −4096以上4095.875以下 | `nnue/model/mod.rs` |
| 重みの切り詰め（第1層） | ±220/255、バイアスは±2 | `nnue/model/mod.rs` |
| 重みの切り詰め（隠れ層と出力） | W1とW2は±127/64、バイアスは±8、oとvは±1.98 | `nnue/model/mod.rs` |
| 量子化誤差の上限 | 平均絶対誤差がK/40以下（Kは勝率尺度） | `nnue/train/mod.rs` |
