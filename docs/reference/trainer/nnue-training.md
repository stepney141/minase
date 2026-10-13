# NNUEの学習

本書は、NNUE（差分更新可能なニューラルネットワーク）の重みを学習する`minase-trainer nnue train`の挙動、引数、停止規則、保存物、および中断した学習の再開を定める。
学習の数学的な内容と、GPUおよびCPUでの実装は[Rust製NNUE学習器の設計書](../../plans/nnue-trainer-rust.md)にある。
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
| `nnue-float.bin` | 最良モデルを[チェックポイント形式](../../plans/nnue-trainer-rust.md#チェックポイントの形式)で保存する。 |
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
