# NNUEの学習と診断の手順

本書は、NNUE（差分更新可能なニューラルネットワーク）の重みを`minase-trainer nnue train`で学習し、GPUの実装を検査し、学習した重みを診断する手順を定める。
学習器は、クレート`crates/minase-trainer`の実行ファイル`minase-trainer`のサブコマンドとして動く。
NNUEの計算式と学習の目的関数は[NNUEによる評価関数と学習の目的関数](evaluation-nnue.md)が説明する。

本書は、実行者が従う手順、判断規則、および標準コマンドだけを記す。
引数の既定値と値域、保存物の各欄、チェックポイントの形式、および検査の許容誤差は、`crates/minase-trainer/src/nnue/`と`crates/minase-trainer/src/diagnostics/`の各項目のdocコメントにあり、本書の各節で該当する項目を示す。
学習データの形式と対局の由来、教師の分類、教師値、および検証集合の分け方は、駒の種類と位置に応じた評価表PSTと、2駒の組の関係を補う補正FMの学習器と共通であり、`crates/minase/src/training/`と`crates/minase-trainer/src/data/`のdocコメントにある。
方式を選んだ理由は、ADRの[NNUEの学習器をRustのホストとNVRTCで実行時にコンパイルするCUDA C++のカーネルで書く](../adr/write-trainer-kernels-in-cuda-cpp-via-nvrtc.md)、[NNUEの学習器は第1層の勾配をホストで作る逆索引から固定の順で足し、アトミック加算を使わない](../adr/backpropagate-the-first-layer-through-a-reverse-index.md)、[学習器のAdamWはPyTorchと同じバイアス補正つきの式にする](../adr/use-pytorch-adamw-with-bias-correction.md)、および[NNUEは探索値だけの先読み教師と二値交差エントロピーで学ぶ](../adr/train-the-nnue-on-search-value-lookahead-teachers.md)にある。

学習を始める際は、[設計書の型](../plans/README.md)に従って目的と比較する基準コミットを定め、[ROADMAP.md](../ROADMAP.md)に状態を登録する。
学習した重みの採否は[棋力測定の手引き](sprt.md)に従う。
以下のコマンドは、明記した箇所を除いてリポジトリのルートで実行する。
`cargo run`を使わない例では、`cargo build --release -p minase-trainer`でビルドした`target/release/minase-trainer`を`minase-trainer`と書く。

## 1. 学習器を検査する

学習器の変更を含む作業では、学習を始める前に、GPUを使わない検査とGPUを使う検査を通す。
GPUを使わない検査は次のコマンドで実行する。

```sh
cargo fmt --all --check
nice -n 19 cargo clippy -j 2 --workspace --all-targets -- -D warnings
nice -n 19 cargo test -j 2 -p minase-trainer
```

GPUのカーネルがCPUの参照実装と一致することは、名前が`gpu_`で始まる`#[ignore]`のテストと`minase-trainer nnue check-equivalence`で確かめる。
`gpu_`のテストは、CUDAで再開した学習が連続した学習と一致することと、途中検証の間隔によらず学習の軌跡が変わらないことも検査する。
GPUを載せた機械で次を実行し、すべて成功することを確認する。

```sh
nice -n 19 cargo test -j 2 -p minase-trainer -- --ignored gpu_ --test-threads=1
for FEATURE_SET in P HalfKP-hm-b8; do
  nice -n 19 cargo run -j 2 -p minase-trainer -- nnue check-equivalence \
    --feature-set "$FEATURE_SET" --width 6 --batch 100 --seed 71 --lambda 1.0
  nice -n 19 cargo run -j 2 -p minase-trainer -- nnue check-equivalence \
    --feature-set "$FEATURE_SET" --width 128 --batch 4096 --seed 71 --lambda 1.0
done
```

一致の判定に使う許容誤差は、`crates/minase-trainer/src/nnue/gpu/equivalence.rs`のdocコメントにある。

## 2. 学習データと初期値を用意する

学習データは、MNSD形式のファイルである。
自己対局データの生成は、[PSTとFMの学習手順](pst-training.md)の2節と3節に従う。
`--data`には複数のファイルを入力順に指定でき、教師の分類の番号は入力順に初めて現れた順で付く。
学習器は開始時に全ファイルを読み、各対局の手順を再生して局面を復元する。
読み込みはファイル単位で`--threads`のスレッドに分けて行い、その時間は`config.json`の`preprocessing_seconds`に含まれる。

線形の経路の初期値には、PSTとFMの同時学習が書き出した圧縮なしのNPZ（`pst-float.npz`）を`--init-pst`に与える。
評価値を勝率へ変換する尺度である出力用のK、探索用駒価値、および規則セット名は、`--base`に与えた重みファイル（MNPT形式）から引き継ぐ。
学習器は開始時に、`--init-pst`の左右の鏡映対が一致することと、`--init-pst`を量子化すると`--base`のPSTの部分に一致することを検査し、一致しなければ停止する。
したがって、`--init-pst`と`--base`には同じ学習から得た組を与える。
学習データの規則セットも、`--base`の規則セットと一致していなければならない。

教師値で探索値と対局結果を混ぜる比λは`--lambda`で必ず指定し、NNUEの学習では探索値だけを教師とする1.0を指定する。
λは自己対局のデータにだけ適用され、実戦対局のデータでは常に0である。
先読み教師を使う場合は、`--lookahead-gamma`と`--lookahead-plies`を同時に指定する。

## 3. 試走する

本番の学習の前に、1ファイルと1エポックで試走し、前処理、更新、検証、および保存が最後まで通ることを確かめる。
本体のデータを使って世代3の1ファイルを両方の入力特徴集合、幅128、1エポックで試走する場合は、リポジトリのルートから次を実行する。
`TRAIN_DATA_ROOT`には、学習データを置いた本体のリポジトリの`data`ディレクトリの絶対パスを指定する。
出力先が既にある場合は、新規実行用の空のパスに変える。

```sh
TRAIN_DATA_ROOT=/absolute/path/to/minase/data
for FEATURE_SET in P HalfKP-hm-b8; do
  cargo run --release --offline -j 2 -p minase-trainer -- nnue train \
    --feature-set "$FEATURE_SET" --width 128 \
    --data "$TRAIN_DATA_ROOT/gen3/generated-1500000.bin" \
    --init-pst "$TRAIN_DATA_ROOT/pst-fm-joint-training/j75/pst-float.npz" \
    --base crates/minase/nets/pst.bin \
    --lambda 1.0 --lookahead-gamma 0.9 --lookahead-plies 40 \
    --batch 16384 --epochs 1 --seed 1 \
    --output-dir "/tmp/nnue-rust-phase3-${FEATURE_SET}"
done
```

GPUのない機械やCPUの参照経路を試す場合は、上記の`minase-trainer nnue train`に`--device cpu --threads 2`を加え、別の空の出力先を指定する。
`--device`の既定値は`cuda`であり、GPUが見つからなければ学習器は明示的なエラーで終了する。

## 4. 学習する

本番の学習は、試走と同じコマンドの`--data`に全入力を並べ、`--epochs`、`--patience`、および`--time-limit-hours`で上限を決めて実行する。
`--output-dir`には、存在しないパスか空のディレクトリを指定する。
学習率、重み減衰、補助損失の係数、およびλは共通の引数で変えられ、既定値と値域は`crates/minase-trainer/src/nnue/options.rs`のdocコメントにある。
エポック数、時間、および打ち切りの上限と既定値は`crates/minase-trainer/src/nnue/train/args.rs`の`Args`のdocコメントにある。

学習率は、更新ごとに倍率を掛けて変えられる。
`--lr-schedule`に、一定の倍率`constant:<c>`、半減期をエポック数で与える指数減衰`exponential:<半減期>:<下限>`、または総長をエポック数で与えるcosine annealing `cosine:<総長>:<下限>`を指定する。
倍率は3つのパラメータ群の学習率に同じ値で掛かり、重み減衰にも学習率を通じて掛かる。
式と引数の範囲は`crates/minase-trainer/src/nnue/schedule.rs`の`LrSchedule`のdocコメントにある。
既定値は、学習率スケジュールの計画で採用した`cosine:3:0.1`である。`constant:1`を指定した学習は、倍率を加える前の学習器とビット単位で同じである。
倍率は学習の軌跡を変えるので、再開時に変えることはできない。

`--validation-interval <更新回数>`を正の値で指定すると、その更新回数ごとに検証集合全体を評価し、`validation-interval.jsonl`へ1行ずつ記録する。
この記録は、エポックの途中で検証損失がどう動くかを見る診断用であり、最良の重みの選択、早期終了、およびチェックポイントはエポック境界のまま変わらない。
検証集合全体の評価には1回あたり数十秒かかるので、1エポックの更新回数（`ceil(訓練局面数 / バッチ)`）の数分の1程度の値を指定する。
既定値の0では記録しない。
`--threads`の既定値は物理コア数から2を引いた数である。
測定機を他の作業と共用する場合は、`--threads`を明示して他の作業の分のコアを残す。

学習は、検証損失が改善しないエポックが`--patience`回続くか、エポック数または時間の上限に達すると止まる。
時間には前処理と検証を含み、再開前の累積時間も数える。
最良の重みは、学習したエポック（1以上）の中から検証損失で選ばれる。

## 5. 結果を確認する

学習が終わったら、`--output-dir`の`summary.json`で停止理由、最良エポック、および量子化検査の結果を確かめる。
エンジンが読む重みは`nnue.bin`であり、`latest.npz`と`best.npz`は再開用のチェックポイントである。
`best.npz`は量子化前の最良モデルを兼ねる。
各ファイルの欄は、`crates/minase-trainer/src/nnue/train/record.rs`の各記録のdocコメントに、チェックポイントの形式は`crates/minase-trainer/src/nnue/model/checkpoint.rs`の`Checkpoint`のdocコメントにある。

引数、入力、または再開の条件が満たされない場合は、学習を始める前に終了コード1で終わる。
学習が始まった後に終了コード1で終わるのは、次の場合である。

- 時間の上限に達し、1エポックも検証を終えていない。
- 量子化した重みとCPUの浮動小数点参照の平均絶対誤差が上限を超え、`summary.json`に`quantization_error`が記録された。

量子化の誤差で失敗した場合も保存物は残るので、`summary.json`の誤差と`training.jsonl`の推移を見て原因を調べ、その重みを採否の測定にかけない。
時間の上限で終わった学習は、`summary.json`に収束を確認していない旨が記録されるので、測定記録にもそのことを書く。

学習曲線は`training.jsonl`で確認する。
エポック0は学習前の検証であり、以後の行で訓練損失、補助損失、検証損失、符号一致率、非線形出力の標準偏差、飽和率、および線形重みの移動量を読み、更新の不足や過学習がないか点検する。
各行には、そのエポックの最初と最後の更新で学習率に掛けた倍率も記録される。
GPUメモリ使用量の欄はGPU全体の使用量であり、他のプロセスの確保も含むので、他の作業と並行した学習ではその分を差し引いて読む。
`--validation-interval`を指定した学習では、`validation-interval.jsonl`の各行で、完了した更新回数の累計、進行中のエポック番号とその中での位置、検証損失、および符号一致率を読み、エポックの途中のどこで検証損失が最良になるかを確かめる。
各欄の定義は`crates/minase-trainer/src/nnue/train/record.rs`の`IntervalRecord`のdocコメントにある。

## 6. 中断した学習を再開する

中断した学習は、最初の実行と同じコマンドに`--resume <出力ディレクトリ>`を加えて再開する。
`--resume`には`--output-dir`と同じディレクトリを指定する。
再開時に変更できるのは`--epochs`、`--patience`、`--time-limit-hours`、`--threads`、および`--validation-interval`だけである。
`--epochs`は再開前を含む総エポック数、`--time-limit-hours`は再開前を含む累積時間に対する上限として指定する。
それ以外の設定、入力ファイルの識別（パス、バイト長、ヘッダ）、前処理の結果、またはデバイスが記録と異なると、学習器は再開を拒否する。
学習器は最後に確定したチェックポイント`latest.npz`から復元し、未確定のエポックの記録を捨てて、そのエポックをやり直す。
`validation-interval.jsonl`も、チェックポイントより後の更新で記録した行を捨てる。
再開の履歴は`config.json`の`resumes`に記録されるので、測定記録には再開の回数と変更した予算を書く。

## 7. GPUでの1更新の時間を測る

学習器の速度に関わる変更は、`minase-trainer nnue bench-step`で、実データでの1更新の時間を、ワーカーによるバッチの供給、入力の転送、およびGPUの計算に分けて測る。
最適化を有効にしたバイナリで、MNSDを`--data`に、学習と同じλを`--lambda`に指定して実行する。
次の例では訓練集合に少なくとも344,064局面が必要であり、検証集合は使わない。

```sh
DATA=/absolute/path/to/generated-1500000.bin
for FEATURE_SET in P HalfKP-hm-b8; do
  CARGO_NET_OFFLINE=true nice -n 19 cargo run --release -j 2 -p minase-trainer -- nnue bench-step \
    --feature-set "$FEATURE_SET" --width 128 --batch 16384 --steps 20 \
    --threads 6 --profile --lambda 1.0 --data "$DATA"
done
```

`--profile`はCUDAのカーネルごとの時間を表示し、`--device cpu`とは併用できない。
GPUメモリの出力はデバイス全体の使用量と確保の前後の差であり、他のプロセスによる変動も含む。
測定用のモデルの初期化と測る区間は、`crates/minase-trainer/src/nnue/gpu/bench.rs`のdocコメントにある。
性能の採否は実GPUでの測定で判断し、CUDAのコンパイルやGPUなしのテストの結果から速度を推定しない。

## 8. 学習した重みを診断する

学習した重みは、採否の測定の前に、評価値の分布、駒除去差分、および合法手の評価差の3つの診断で調べる。
3つの診断はCPUで動き、`--output`に指定した新規のJSONファイルへ報告を書く。
`--output`が既にあると何も計算せずに終了する。
入力や統計が条件を満たさない場合は、報告を書かずに終了コード1で終わり、理由を標準エラー出力に表示する。
報告の各欄の定義は、`crates/minase-trainer/src/diagnostics/`の`nnue_distribution.rs`、`nnue_removal.rs`、および`nnue_moves.rs`のdocコメントにある。
これらの診断を設けた経緯は[NNUEの完全代替](../plans/archived/nnue-replacement.md)にある。

### 評価値の分布

`diag nnue-distribution`は、基準と候補の重みを同じ標本の局面で評価し、評価値の散らばりと相関を比べる。

```sh
minase-trainer diag nnue-distribution \
  --pair <比較名> <基準のPST＋FM（FMを持つMNPT）> <候補のNNUE（nnue.bin）> \
  --data <データ...> --sample <標本数> --seed <シード> \
  --output <報告.json>
```

標本は`--data`の全局面から抽出するので、`--sample`には2以上で全局面数以下の値を指定する。
`--pair`は比較名を変えて複数回指定でき、すべての組が同じ標本で比較される。
報告の`pairs`の各要素の`verdict`は、候補と基準の標準偏差の比が0.9以上1.1以下に収まるかを示すので、評価の目盛りが基準から変わっていないかをまず確かめる。
収まらない場合は、局面の位相と基準の評価の大きさで分けた`by_phase_q`と`by_baseline_absolute_cp`で、どの帯で散らばりが変わったかを確かめる。

### 駒除去差分

`diag nnue-removal`は、検証集合の局面から王駒以外の駒を1枚ずつ除いたときの評価の変化を、量子化前と量子化後の重みで比べる。

```sh
minase-trainer diag nnue-removal \
  --float <出力ディレクトリ>/best.npz --net <出力ディレクトリ>/nnue.bin \
  --data <データ...> --sample <標本数> --seed <シード> \
  --output <報告.json>
```

`--sample`には、1以上で検証集合の局面数以下の値を指定する。
`--float`と`--net`には、同じ学習が書き出した組を与える。
報告の`reversals`は、学習を始める前の手書きの評価関数v0の駒価値による参照と整数のネットの間で除去差分の符号が反転した件数と割合であり、駒損を有利と評価する傾向の目安として読む。
量子化前の重みの2つの計算経路の差が上限を超えた場合は、`error`を含む報告を保存して終了コード1で終わるので、学習器の実装の不具合として調べる。

### 合法手の評価差

`diag nnue-moves`は、指定した局面のすべての合法手について、着手による評価の変化をNNUEとPST＋FMで比べる。

```sh
minase-trainer diag nnue-moves \
  --net <出力ディレクトリ>/nnue.bin --base <PST＋FMの重み> \
  --positions <局面のMNSD> --output <報告.json>
```

`--positions`には診断が書き出す合成の由来のMNSDも与えられるが、ヘッダの規則セットがエンジンの既定の規則セット`engine-default`でなければ拒否される。
`--base`には、PSTとFMの両方を持つMNPTを与える。
片方のモデルにしかない合法手があった場合は、`error`を含む報告を保存して終了コード1で終わるので、規則か局面の復元の不一致として調べる。

## 9. 採否を測定して記録する

診断を確認した後、基準コミットから候補ブランチを作り、学習した`nnue.bin`を候補の`crates/minase/nets/nnue.bin`へコピーし、候補で`cargo test --locked`を実行してから、[段階ゲート](sprt.md#機能採否の段階ゲートstcとltc)で採否を決める。
検証損失、符号一致率、および診断の値の改善だけを、棋力が向上した根拠にはしない。
学習の結果は`docs/measurements/<実験名>-training.md`にまとめ、学習器のコミット、実行したコマンドライン全体、入力の学習データのパスと記録数、重みのSHA-256、停止理由、および診断の要点を書く。
