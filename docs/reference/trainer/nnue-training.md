# NNUEの学習

本書は、NNUE（差分更新可能なニューラルネットワーク）の重みを学習する`minase-trainer nnue train`の挙動、引数、停止規則、保存物、および中断した学習の再開を定める。
学習の数学的な内容と、GPUおよびCPUでの実装の契約は、`crates/minase-trainer/src/nnue/`の各モジュールのdocコメントにある。
方式を選んだ理由は、ADRの[NNUEの学習器をRustのホストとNVRTCで実行時にコンパイルするCUDA C++のカーネルで書く](../../adr/write-trainer-kernels-in-cuda-cpp-via-nvrtc.md)、[NNUEの学習器は第1層の勾配をホストで作る逆索引から固定の順で足し、アトミック加算を使わない](../../adr/backpropagate-the-first-layer-through-a-reverse-index.md)、[学習器のAdamWはPyTorchと同じバイアス補正つきの式にする](../../adr/use-pytorch-adamw-with-bias-correction.md)、および[NNUEは探索値だけの先読み教師と二値交差エントロピーで学ぶ](../../adr/train-the-nnue-on-search-value-lookahead-teachers.md)にある。
入力の学習データ、教師値、検証分割は、PSTとFMの学習器と共通であり、[学習データと教師値](data-and-teachers.md)が定める。

## 実行の流れ

`minase-trainer nnue train`は前処理、指定したデバイスでの更新、全検証集合の評価、チェックポイントの保存、最良重みの書き出しを行う。
線形重みは`--init-pst`の圧縮なしNPZから初期化し、左右の鏡映対が一致することと、再量子化が`--base`のMNPTのPST部分と一致することを検査する。
非対称な初期値は拒否する。
出力用K、探索用駒価値、規則セット名はMNPTから引き継ぐ。
教師値、`--lambda-override`の適用範囲、および教師用Kの推定は[学習データと教師値](data-and-teachers.md)のとおりであり、NNUEの学習では`--lambda-override`の既定値を1.0とする。

`--device`の既定値は`cuda`であり、GPUがなければ明示的なエラーで終了する。
`--device cpu`ではGPUを初期化せず、同じ学習ループからCPU参照関数を呼ぶ。
`--threads`の既定値は物理コア数−2で、最小1である。

## 係数の引数

学習、`minase-trainer nnue check-equivalence`、`minase-trainer nnue bench-step`では、次の係数を共通の引数で指定できる。
すべて有限かつ非負とし、学習率は0を認めない。
λは0以上1以下に制限する。

| 引数 | 既定値 |
|---|---:|
| `--lr-linear` | 0.03 |
| `--weight-decay-linear` | 0 |
| `--lr-ft` | 1e-3 |
| `--weight-decay-ft` | 1e-4 |
| `--lr-head` | 1e-3 |
| `--weight-decay-head` | 1e-4 |
| `--removal-penalty` | 1000 |
| `--lambda-override` | 1.0 |

## 停止規則

学習の上限は`--epochs`で指定する50エポック以内と、`--time-limit-hours`で指定する24時間以内の早い方である。
時間には前処理と検証を含め、再開前の累積時間も引き継ぐ。
`--patience`の既定値は5であり、検証損失が厳密に改善しないエポックが5回続くと停止する。
最良エポックは1以降から選び、時間切れの未完了エポックの更新は保存候補に含めない。
検証の最終バッチ後にも期限を判定し、期限に達していればそのエポックの最良値更新とチェックポイント保存を行わない。
時間切れには「予算の上限で終了し、収束は未確認」と記録し、1エポックも検証を終えていなければ終了コード1を返す。

## 保存物

保存先は`--output-dir`で指定し、新規実行では空のディレクトリを使う。
保存物の内容は次のとおりである。

| ファイル | 内容 |
|---|---|
| `config.json` | 引数、実効ワーカー数、最適化器と補助損失の係数、λ、入力ファイルと来歴のSHA-256、教師用K、分割と観測の件数とハッシュ、デバイス名、再開履歴を保存する。 |
| `training.jsonl` | エポック0の検証と、検証を終えた各エポックを1行ずつ保存する。損失、符号一致率、非線形出力の標準偏差、飽和率、線形重みの移動量、速度、時間、GPUメモリ使用量を含む。 |
| `latest.bin` | 最後に検証を終えたモデル、AdamWの状態、更新回数、最良エポック、累積時間と照合用ハッシュを保存する。 |
| `best.bin` | 最良エポックのモデルと、その時点のAdamWの状態を保存する。 |
| `nnue.bin` | 最良モデルを量子化したMNUE版5を保存し、minaseの読み手で再検証する。 |
| `nnue-float.bin` | 最良モデルを後述の[チェックポイントの形式](#チェックポイントの形式)で保存する。 |
| `summary.json` | 停止理由、最良エポック、量子化誤差と検査結果を保存する。 |

`train_loss`は二値交差エントロピー、`removal_loss`は指定係数を含む補助損失、`total_loss`はその和であり、いずれも局面数で重み付けした平均である。
GPUメモリ使用量は、学習と検証の配列確保後にCUDAが報告したGPU全体の使用量の最大値であり、他プロセスの確保も含む。
`config.json`と`training.jsonl`に`device`を記録し、CPU実行のGPUメモリ欄は`null`とする。
符号一致率とエポック0からの改善は診断に記録する。
量子化検査には固定シードで選んだ検証局面を最大`--validation-sample`件使い、CPUの浮動小数点参照とminaseの整数推論の平均絶対誤差と最大誤差をセンチポーンで記録する。
浮動小数点参照は線形和、累算器、活性化、非線形出力をf64で計算する。
標本の抽出にはSplitMix64を使う。
平均絶対誤差がK/40を超えた場合は、保存物を残して`quantization_error`と終了コード1を返す。

## 再開

再開する場合は`--resume <output-dir>`を指定する。
`--epochs`、`--patience`、`--time-limit-hours`、`--threads`は変更できる。
`--epochs`は再開後を含む総エポック数、時間上限は再開前を含む累積時間に対する上限である。
これらの予算と並列度、保存先、再開指定を除く設定、正規化した入力パス、入力内容、再計算した前処理の結果、デバイス名が一致しなければ再開を拒否する。
再開のたびに`config.json`の`resumes`配列へ、再開時刻、エポック数、patience、時間上限、実効ワーカー数、再開時点の完了済みエポック番号を追記する。
時刻は`resumed_at_unix_seconds`にUnix時刻の秒数で保存し、残る項目は順に`epochs`、`patience`、`time_limit_hours`、`workers`、`epoch`に保存する。
各エポックではログ、最新チェックポイント、最良チェックポイントの順に保存し、最新チェックポイントの確定を保存完了の基準にする。
その後の最良チェックポイントの保存で中断した場合は、確定済みの最新チェックポイントから復元する。
未確定のログ末尾は切り捨て、同じエポック番号とシードから順列を再生成する。

## チェックポイントの形式

`latest.bin`、`best.bin`、および`nnue-float.bin`は、次の形式のチェックポイントである。
整数と浮動小数点数はすべてリトルエンディアンであり、配列の長さは構造記述文字列と幅から導くので、長さの欄を持たない。
読み書きは`nnue::model::Checkpoint`の`encode`と`decode`が行い、再開の前に`check_hashes`で実行設定と入力の一致を確かめる。

| オフセット | 内容 |
|---:|---|
| 0 | 8バイトの識別子`MNTRCKPT` |
| 8 | 版番号（u32、2） |
| 12 | MNUE版5と同じ構造記述文字列（96バイト、不足分は0） |
| 108 | 幅（u32）と出力用K（f32） |
| 116 | 更新回数、エポック番号、最良エポック番号（各u64） |
| 140 | 最良の検証損失と累積秒数（各f64） |
| 156 | 乱数のシード（u64） |
| 164 | 実行設定のSHA-256と入力のSHA-256（各32バイト） |
| 228 | 線形の群、第1層の群、出力部の群の順に、学習率と重み減衰（各f64） |
| 276 | 補助損失の係数とλの上書き値（各f64） |
| 292 | 全パラメータ、AdamWの1次モーメント、2次モーメントの順の3組。各組は`Parameter::ALL`の順（線形、第1層、仮想行、バイアス、出力重み、出力バイアス）のf32配列である。 |
| 可変 | 正準行、第1層、仮想行の順の観測マスク（1要素1バイト） |
| 可変 | 線形の初期値と仮想行の初期値（f32配列） |
| 末尾32バイト | それより前の全内容のSHA-256 |

最良エポックがまだない場合は、番号を0、検証損失を正の無限大とする。
P型の仮想行の配列とマスクは長さ0である。

## GPUの検査と速度測定

GPUのカーネルがCPU参照実装と一致することは、名前が`gpu_`で始まる`#[ignore]`のテストと`minase-trainer nnue check-equivalence`で確かめる。
一致は`|GPU−CPU| ≤ 10⁻⁶ + 10⁻⁵·|CPU|`で判定し、整数と真偽値には完全な一致を、同じ入力の2回の実行にはビット単位の一致を求める。
GPUを使わない検査とGPUを使う検査は、それぞれ次のコマンドで実行する。

```sh
cargo fmt --all --check
nice -n 19 cargo clippy -j 2 --workspace --all-targets -- -D warnings
nice -n 19 cargo test -j 2 -p minase-trainer
```

```sh
nice -n 19 cargo test -j 2 -p minase-trainer -- --ignored gpu_ --test-threads=1
for FEATURE_SET in P HalfKP-hm-b8; do
  nice -n 19 cargo run -j 2 -p minase-trainer -- nnue check-equivalence \
    --feature-set "$FEATURE_SET" --width 6 --batch 100 --seed 71
  nice -n 19 cargo run -j 2 -p minase-trainer -- nnue check-equivalence \
    --feature-set "$FEATURE_SET" --width 128 --batch 4096 --seed 71
done
```

`minase-trainer nnue bench-step`は、実データでの1更新の時間を、ワーカーの供給速度、入力の転送時間、およびGPUの計算時間に分けて測る。
最適化を有効にしたバイナリで、来歴ファイルを添えたMNSDを`--data`に指定して実行する。
次の例では訓練集合に少なくとも344,064局面が必要であり、検証集合は使わない。

```sh
DATA=/absolute/path/to/generated-1500000.bin
for FEATURE_SET in P HalfKP-hm-b8; do
  CARGO_NET_OFFLINE=true nice -n 19 cargo run --release -j 2 -p minase-trainer -- nnue bench-step \
    --feature-set "$FEATURE_SET" --width 128 --batch 16384 --steps 20 \
    --threads 6 --profile --data "$DATA"
done
```

ファイルの検査、モデルの初期化、カーネルのコンパイル、およびGPUの配列の確保を済ませ、1回のウォームアップの後で`--steps`回を測る。
シード（既定71）で並べ替えた訓練局面から、ウォームアップを含む`(steps+1)×batch`局面を重複なく使う。
測定用のモデルは、埋め草以外の全行を観測済みとし、線形をv0の駒価値、第1層を標準偏差0.05の乱数、バイアスを0.25、出力重みを±0.01の交互で初期化し、出力と全教師分類に共通のK（既定1000、`--k`で変更）を使う。
したがって教師用Kの推定、観測マスクの作成、保存、および検証は行わない。
`--profile`はCUDAのカーネルごとの時間を表示し、`--device cpu`と組み合わせるとエラーになる。
`--device cpu`では更新の時間にホストの経過時間を使い、転送量と転送時間を0とし、GPUメモリは測らない。
GPUメモリはデバイス全体の使用量と確保前後の差を出力するので、他のプロセスによる変動も含む。
性能の採否は実GPUでの測定で判断し、CUDAのコンパイルやGPUなしのテスト結果から速度を推定しない。

## 試走

本体のデータを使って世代3の1ファイルを両方の型、幅128、1エポックで試走する場合は、worktreeのルートから次を実行する。
出力先が既にある場合は、新規実行用の空のパスに変える。

```sh
TRAIN_DATA_ROOT=/home/stepney141/board-games/minase/data
for FEATURE_SET in P HalfKP-hm-b8; do
  cargo run --release --offline -j 2 -p minase-trainer -- nnue train \
    --feature-set "$FEATURE_SET" --width 128 \
    --data "$TRAIN_DATA_ROOT/gen3/generated-1500000.bin" \
    --init-pst "$TRAIN_DATA_ROOT/pst-fm-joint-training/j75/pst-float.npz" \
    --base crates/minase/nets/pst.bin \
    --lookahead-gamma 0.9 --lookahead-plies 40 \
    --batch 16384 --epochs 1 --seed 1 \
    --output-dir "/tmp/nnue-rust-phase3-${FEATURE_SET}"
done
```

CPUの1ファイル試走では、上記の`minase-trainer nnue train`に`--device cpu --threads 2`を加え、別の空の出力先を指定する。
