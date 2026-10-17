# NNUEの学習と診断の手順

本書は、NNUE（差分更新可能なニューラルネットワーク）の重みを`minase-trainer nnue train`で学習し、GPUの実装を検査し、学習した重みを診断する手順を定める。
学習器は、クレート`crates/minase-trainer`の実行ファイル`minase-trainer`のサブコマンドとして動く。
NNUEの計算式と学習の目的関数は[NNUEによる評価関数と学習の目的関数](evaluation-nnue.md)が説明する。

本書は、実行者が従う手順、判断規則、および標準コマンドだけを記す。
引数の既定値と値域、保存物の各欄、チェックポイントの形式、および検査の許容誤差は、`crates/minase-trainer/src/nnue/`と`crates/minase-trainer/src/diagnostics/`の各項目のdocコメントにあり、本書の各節で該当する項目を示す。
学習データの形式と対局の由来、教師の分類、教師値、および検証集合の分け方は、駒の種類と位置に応じた評価表PSTと、2駒の組の関係を補う補正FMの学習器と共通であり、`crates/minase/src/training/`と`crates/minase-trainer/src/data/`のdocコメントにある。
方式を選んだ理由は、ADRの[NNUEの学習器をRustのホストとNVRTCで実行時にコンパイルするCUDA C++のカーネルで書く](../adr/write-trainer-kernels-in-cuda-cpp-via-nvrtc.md)、[NNUEの学習器は第1層の勾配をホストで作る逆索引から固定の順で足し、アトミック加算を使わない](../adr/backpropagate-the-first-layer-through-a-reverse-index.md)、[学習器のAdamWはPyTorchと同じバイアス補正つきの式にする](../adr/use-pytorch-adamw-with-bias-correction.md)、および[NNUEは先読みを使わない探索値だけの教師と二値交差エントロピーで学ぶ](../adr/train-the-nnue-on-search-value-teachers-without-lookahead.md)にある。

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
評価値を勝率へ変換する勝率尺度（1ロジットあたりのcp）、探索用駒価値、および規則セット名は、`--base`に与えた重みファイル（MNPT形式）から引き継ぐ。
学習器は開始時に、`--init-pst`の左右の鏡映対が一致することと、`--init-pst`を量子化すると`--base`のPSTの部分に一致することを検査し、一致しなければ停止する。
したがって、`--init-pst`と`--base`には同じ学習から得た組を与える。
学習データの規則セットも、`--base`の規則セットと一致していなければならない。

教師値で探索値と対局結果を混ぜる比λは`--lambda`で必ず指定し、NNUEの学習では探索値だけを教師とする1.0を指定する。
λは自己対局のデータにだけ適用され、実戦対局のデータでは常に0である。
NNUEの標準の学習は先読み教師を使わない。比較のために先読み教師を使う場合は、`--lookahead-gamma`と`--lookahead-plies`を同時に指定する。
教師の探索値を勝率へ換算する教師の勝率尺度は、必須の引数`--teacher-cp-per-logit`で与える。
値は、この学習の訓練集合から教師の分類ごとに推定する`calibrate`か、較正コマンド`minase-trainer data calibrate`が書き出した較正ファイル（JSON）のパスのどちらかであり、既定値はない。
通常の学習では`calibrate`を指定する。
訓練集合の間引きや分割の変更のように、入力を変えても教師値を固定したい比較では、間引く前の全入力から較正ファイルを作り、それぞれの学習に同じファイルを渡す。
較正コマンドは学習と同じ`--data`、`--lookahead-gamma`、`--lookahead-plies`を受け、学習と同じ検証分割で訓練集合を選んで較正する。
学習器は較正ファイルの項目を分類の属性（生成コミット、重みの検査和、探索の規模、規則セット、由来、先読み条件）で突き合わせ、λが0でない分類に項目がなければ学習を始めずに停止する。
`config.json`の`preprocessing`には、教師の勝率尺度`teacher_cp_per_logit`、勝率尺度を教師の勝率尺度で割った尺度補正係数`teacher_score_multiplier`、および出所`teacher_cp_per_logit_source`（`calibrate`か、較正ファイルのパスと内容）が残る。

```sh
cargo run --release --offline -j 2 -p minase-trainer -- data calibrate \
  --data "$TRAIN_DATA_ROOT/nnue-lr-schedule-gen/generated-"*.bin \
  --output /tmp/teacher-calibration.json
```

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
    --lambda 1.0 --teacher-cp-per-logit calibrate \
    --batch 16384 --epochs 1 --seed 1 \
    --output-dir "/tmp/nnue-rust-phase3-${FEATURE_SET}"
done
```

GPUのない機械やCPUの参照経路を試す場合は、上記の`minase-trainer nnue train`に`--device cpu --threads 2`を加え、別の空の出力先を指定する。
`--device`の既定値は`cuda`であり、GPUが見つからなければ学習器は明示的なエラーで終了する。

## 4. 学習する

本番の学習は、試走と同じコマンドの`--data`に全入力を並べ、`--epochs`、`--patience`、および`--time-limit-hours`で上限を決めて実行する。
`--output-dir`には、存在しないパスか空のディレクトリを指定する。
学習率、重み減衰、およびλは共通の引数で変えられ、既定値と値域は`crates/minase-trainer/src/nnue/options.rs`のdocコメントにある。
エポック数、時間、および打ち切りの上限と既定値は`crates/minase-trainer/src/nnue/train/args.rs`の`Args`のdocコメントにある。

ネットの出力部は`--head`で選ぶ。
`direct-screlu`（既定。両視点の活性化と出力重みの直接内積）、`direct-screlu-phase2`（序盤用と終盤用の2つの出力を盤上総駒数で補間する）、`screlu-hidden16`（幅16の隠れ層の寄与を直接内積に加える）、および`screlu-hidden16-phase2`（隠れ層を共有した2端点の補間）の4つであり、名前は重みファイルの構造記述文字列の`head=`と同じである。
出力部ごとの式、パラメータの配列、初期値、および切り詰めは`crates/minase-trainer/src/nnue/model/mod.rs`の`HeadKind`と`Parameter`、`crates/minase-trainer/src/nnue/reference/head.rs`のdocコメントにある。
出力部は学習の軌跡を変えるので、再開時に変えることはできない。
`check-equivalence`と`bench-step`にも同じ`--head`がある。
隠れ層を持つ出力部では、`training.jsonl`の`hidden_saturation`に隠れ層の活性化が0以下と1以上であった割合が記録され、他の出力部では`null`である。
エンジンは`crates/minase/src/eval/nnue/mod.rs`の`ActiveHead`に決めた出力部の重みだけを読むので、別の出力部で学んだ`nnue.bin`を埋め込むには`ActiveHead`も合わせて変える。

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

`--train-subsample games:<k>:<r>`または`--train-subsample positions:<k>:<r>`を指定すると、更新に使う訓練集合を対局単位または局面単位で間引く（kは2以上、rは0以上k未満）。
`games`は対局ごとのハッシュをkで割った余りがrの対局の全局面を残し、`positions`は局面ごとのハッシュで1局面ずつ選ぶ。
同じ局面数で「少数の対局から全局面」と「多数の対局から疎に」を比べるための引数であり、ハッシュの式は`crates/minase-trainer/src/data/subsample.rs`の`TrainSubsample`にある。
教師の勝率尺度の推定、先読み教師、観測マスク、および初期値は間引く前の訓練集合全体から作り、検証集合は間引かない。
1エポックの更新回数、学習率の倍率、および`--validation-interval`は抽出後の局面数から決まる。
指定した値と抽出後の局面数、対局数、および局面の番号列の検査和は`config.json`に記録され、再開時には変更できない。
省略した学習は、この引数を加える前の学習器とビット単位で同じである。

学習は、検証損失が改善しないエポックが`--patience`回続くか、エポック数または時間の上限に達すると止まる。
時間には前処理と検証を含み、再開前の累積時間も数える。
最良の重みは、学習したエポック（1以上）の中から検証損失で選ばれる。

### メモリに収まらない教師データで学習する

学習器は、既定の`--buckets 1`では、学習に使う全局面を圧縮表現でメモリに持ち、毎エポック全体を並べ替える。
教師データがメモリに収まらない規模になったら、`--buckets <B>`、`--bucket-positions <上限>`、および`--scratch-dir <ディレクトリ>`の3つを指定する。
Bを2以上にすると、学習器は開始時にMNSDを再生して局面を一時ファイルへ書き出し、エポックごとに各訓練局面を無作為にB個のbucket（区画）のどれかへ振り分け、bucketを1つずつメモリへ読んで中を並べ替えて順に学習に流す。
各局面の振り分けが独立で一様であり、bucketの中も一様に並べ替えるので、連結した全体は全局面の一様な並べ替えになり、混ぜる範囲がデータの規模に依存しない。
方式を選んだ理由は[毎エポック訓練集合全体を一様に並べ替えるADR](../adr/shuffle-the-whole-training-set-per-epoch-with-rao-sandelius-buckets.md)にある。
Bは順列を決めるので、同じシードでもBを変えると別の順列になり、再開時には記録と一致していなければならない。
上限と作業ディレクトリは順列を変えないので、再開時に変えてよい。

Bと上限は次のように選ぶ。
訓練局面の数をNとすると、1つのbucketに入る局面数は二項分布Binomial(N, 1/B)に従い、平均はN/B、標準偏差はおよそ√(N/B)である。
上限には、平均に標準偏差の6倍を足した`N/B + 6√(N/B)`以上の値を指定する。
6倍なら1つのbucketが上限を超える確率は約10⁻⁹であり、超えた場合は学習器が超えたbucketの局面数、上限、およびBを示して止まるので、上限を大きくして実行し直す。
Bは、学習プロセスの固定のメモリ（ネット、最適化器、GPUのドライバなど）、bucket1つぶんのメモリ（局面1つにつき一時表現の138バイトと順列の8バイト）、および先行して組み立てるバッチ（`--threads × --batch`局面）の合計が、使える実メモリに収まるように選ぶ。
`--scratch-dir`の下には、元の一時表現と1エポックぶんのbucket（それぞれ訓練局面1つにつき138バイト）、検証局面の一時表現、および教師の勝率尺度の推定の入力（訓練局面1つにつき5バイト）が書き出されるので、おおむね訓練局面1つにつき300バイトのディスクの空きを用意する。
一時ファイルに使う容量は、測定機の空き容量の4分の1までとする（2026年10月17日の利用者の決定）。
1エポックの読み書きは一時表現の総量の約3倍であり、所要時間への影響は[B=16の測定記録](../measurements/out-of-core-shuffle/out-of-core-shuffle-time-memory.md)にある。

一時ファイルは`<scratch-dir>/nnue-<UNIX秒>-<プロセスID>-<連番>/`に作られ、学習の正常終了時と、学習器がエラーとして捕捉した異常終了時に削除される。
強制終了や電源断で残ったものは、別の学習の入力にも再開にも使えない。
`lock`に書かれたプロセスIDが`ps -p <ID>`で実行中でないことを確かめてから、ディレクトリごと`rm -r`で削除する。

## 5. 結果を確認する

学習が終わったら、`--output-dir`の`summary.json`で停止理由、最良エポック、量子化検査の結果、および駒除去差分の符号反転の割合を確かめる。
符号反転の割合（`removal_sign`）は、量子化検査と同じ検証標本について、王駒以外の盤上の駒を1枚ずつ除いた局面を書き出した整数ネットで評価し直し、評価差の符号が手書きの評価関数v0の駒価値の差と逆になる除去の割合である。
閾値はなく、学習の成否には影響しない。駒損を有利と評価する傾向の目安として、採否の測定で負けた場合の原因の手がかりに使う。
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
エポック0は学習前の検証であり、以後の行で訓練損失、検証損失、符号一致率、非線形出力の標準偏差、飽和率、および線形重みの移動量を読み、更新の不足や過学習がないか点検する。
各行には、そのエポックの最初と最後の更新で学習率に掛けた倍率も記録される。
GPUメモリ使用量の欄はGPU全体の使用量であり、他のプロセスの確保も含むので、他の作業と並行した学習ではその分を差し引いて読む。
`--validation-interval`を指定した学習では、`validation-interval.jsonl`の各行で、完了した更新回数の累計、進行中のエポック番号とその中での位置、検証損失、および符号一致率を読み、エポックの途中のどこで検証損失が最良になるかを確かめる。
各欄の定義は`crates/minase-trainer/src/nnue/train/record.rs`の`IntervalRecord`のdocコメントにある。

## 6. 中断した学習を再開する

中断した学習は、最初の実行と同じコマンドに`--resume <出力ディレクトリ>`を加えて再開する。
`--resume`には`--output-dir`と同じディレクトリを指定する。
再開時に変更できるのは`--epochs`、`--patience`、`--time-limit-hours`、`--threads`、`--validation-interval`、`--bucket-positions`、および`--scratch-dir`だけである。
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
学習の結果は、その学習を計画した設計書の名前のディレクトリ（どの設計書も計画していなければ`adhoc/`）へ`docs/measurements/<設計書名>/<実験名>-training.md`としてまとめ、学習器のコミット、実行したコマンドライン全体、入力の学習データのパスと記録数、重みのSHA-256、停止理由、および診断の要点を書く。
