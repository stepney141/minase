NNUEの浮動小数点モデル、設計書のカーネル1〜9のCPU参照実装、およびGPU基盤を提供する。
GPU側では第1層の前進、駒除去差分の補助損失、全パラメータの勾配、部分和の縮約、AdamWによる更新と切り詰め、検証を実行できる。
学習の数学的な契約は[設計書](../../docs/plans/nnue-trainer-rust.md)と`tools/train/src/minase_train`に従う。
`dump-batch`の更新処理は照合資料の生成用であり、エポックを進める学習ループは持たない。

公開APIとPython実装の対応を次に示す。
配列は行優先で、パラメータの値はf32、幅は2以上512以下の偶数である。

| ファイル | 公開型と関数 | Python側の対応 |
|---|---|---|
| `src/model/mod.rs` | `Error`、`Result`、`Parameter`、`Shape`、`Parameters`、`Masks`、`Model`を定義する。 | `pst.model.MirroredEmbedding`と`nnue.model`の線形、共有型、分解型、出力部、制約に対応する。 |
| `src/model/npz.rs` | `model::load_initial`が圧縮なしnpzを読む。 | `nnue.model.load_initial`に対応し、線形の鏡映対の平均は`pst.model.MirroredEmbedding`に合わせる。 |
| `src/model/quantize.rs` | `model::Quantized`、`model::quantize`、`Quantized::encode`を提供する。 | `nnue.train.quantized_weights`と`nnue.format`に対応する。MNUE版5の符号化は`minase::eval::nnue::encode`を使う。 |
| `src/model/checkpoint.rs` | `model::BestEpoch`、`model::Progress`、`model::Checkpoint`を提供する。 | 学習器固有の保存形式であり、PyTorchへの移植は照合スクリプトが担当する。 |
| `src/reference/mod.rs` | `Forward`、`Output`、`Losses`、`Gradients`と、`forward`、`output_backward`、`reduce_partials`、`first_layer_backward`、`linear_backward`、`reduce_virtual`、`fold`を提供する。 | `NNUE.components`、`DirectHead`とPyTorchの自動微分に対応する。 |
| `src/reference/mod.rs` | `evaluate`、`targets`、`screlu`、`screlu_derivative`、`loss_and_gradients`も公開する。 | 浮動小数点の前進、`pst.teacher.build_targets`のλ=1、SCReLU、`nnue.train.train_epoch`の1バッチに対応する。 |
| `src/reference/removal.rs` | `reference::Removal`、`reference::RemovalReference`、`reference::removal_forward`、`reference::v0_piece_values`を提供する。 | `NNUE.removal_deltas`、`pst.removal`の整数参照、罰則、`data.mnpt.initial_weights`に対応する。v0の値は`minase::eval::handcrafted::piece_value`から取る。 |
| `src/reference/optimizer.rs` | `reference::AdamWGroup`、`reference::AdamWConfig`、`reference::AdamW`、`reference::adamw`を提供する。 | `nnue.train.make_optimizer`の3群と`torch.optim.AdamW`に対応する。 |
| `src/dump.rs`と`src/lib.rs` | コマンドを拡張し、`model`と`reference`を公開する。 | `tools/train/scripts/crosscheck_reference.py`が保存物を読み、PyTorchで3更新する。 |

`Shape`は`new`で幅とKを検査し、`feature_set`、`width`、`k`、`units`、`rows`、`len`、`architecture`で配置を返す。
`Parameters::zeros`はその配置で確保し、`shape`と`validate`で形と有限性を検査できる。
`Parameter::ALL`が保存順、`Parameter::name`がPyTorchの名前を定め、添字による読み書きは配列長を保つ。
`Masks`は`new`または`from_observations`で作り、`canonical`、`first_layer`、`virtual_rows`で3種のマスクを返す。

`Model::initialize`は全基底特徴の線形2端点、観測マスク、シード、第1層の初期化標準偏差を受け取る。
標準偏差は有限かつ非負とし、`dump-batch --init-std`の既定値は0.05である。
`shape`、`parameters`、`parameter_mut`、`masks`、`initial_linear`、`initial_virtual`が値を公開する。
`expanded_biases`、`expanded_output_weights`、`expanded_linear`、`expanded_first_layer`が共有パラメータを展開し、最後の関数はHalfKP-hm-b8の特徴分解も畳み込む。
`constrain`は有限性を検査してから切り詰め、`fix_unobserved`で未観測行と埋め草を復元する。

`RemovalReference::v0`は訓練用の参照を作り、`new`は検査用の任意の鏡映対称な整数PSTを受け取り、`deltas`は整数参照差分と切り捨て前の差分の大きさを返す。
Pythonと同じく、罰則の符号には整数評価の差を、上限には切り捨て前の差の絶対値を使う。
線形の除去差分はf64で計算し、非線形差分はf32で集約してからf64に広げる。
`Losses::auxiliary`は係数1000を含み、`Losses::total`が二値交差エントロピーとの和を返す。

`AdamW::new`は状態を0で作り、`m`、`v`、`step`、`config`で再開に必要な状態を公開する。
`AdamW::update`は観測行の全要素を更新し、各群の保存範囲へ切り詰める。
バッチに現れず勾配が0だった観測行にもモーメントと重み減衰を適用し、未観測行のパラメータと両モーメントは更新せずに保つ。
HalfKP-hm-b8では仮想行の更新と切り詰めを先に行い、その値との和が±8に収まる範囲へ特徴行を切り詰める。
`AdamWConfig::default`の学習率と重み減衰は、線形が`0.03, 0`、第1層と出力部が`0.001, 0.0001`であり、`validate`で値を検査する。

チェックポイントは`Checkpoint::{encode,decode,write,read}`で読み書きする。
`check_hashes`で実行設定と入力の一致を確認してから再開する。
整数と浮動小数点数はすべてリトルエンディアンで、配列長は構造記述と幅から導く。

| オフセット | 内容 |
|---:|---|
| 0 | 8バイトの識別子`MNTRCKPT`を置く。 |
| 8 | u32の版番号1を置く。 |
| 12 | MNUEと同じ構造記述文字列を96バイトにNUL埋めして置く。 |
| 108 | 幅のu32と出力用Kのf32を置く。 |
| 116 | 更新回数、エポック番号、最良エポック番号をu64で置く。 |
| 140 | 最良の検証損失と累積秒数をf64で置く。 |
| 156 | 乱数のシードをu64で置く。 |
| 164 | 設定と入力のSHA-256をそれぞれ32バイトで置く。 |
| 228 | 線形、第1層、出力部の順に、学習率と重み減衰をf64で置く。 |
| 276 | 全パラメータ、1次モーメント、2次モーメントを順に置く。それぞれが`Parameter::ALL`順のf32配列である。 |
| 可変 | 正準、第1層、仮想行の順に、観測マスクを1要素1バイトで置く。 |
| 可変 | 線形の初期値と仮想行の初期値をf32配列で置く。 |
| 末尾32バイト | それより前の全内容のSHA-256を置く。 |

最良エポックがまだない場合は、番号を0、検証損失を正の無限大とする。
P型の仮想行の配列とマスクは長さ0である。
`gradients-N.f32`はヘッダを持たず、同じ`Parameter::ALL`順の勾配だけを保存する。
照合スクリプトはチェックポイントの形から勾配ファイルの長さを検査する。

テストの期待値は、設計書の式、Pythonの指定された例、整数推論API、および次の不変条件から定めた。
`tests/common/mod.rs`が合成局面を`Record::from_position`でMNSDへ変換し、圧縮なしnpzの試料も作る。
有限差分の計算は、逆索引や後退の中間値を使わず、各行を取り除いたネットをf64で再評価する。

| テスト名 | 検査する契約 |
|---|---|
| `finite_differences_cover_all_parameter_groups_and_penalty_regions` | 幅4、3局面で全パラメータ群の中心差分、罰則の下限と上限、不適格な除去を検査する。 |
| `p_mirror_preserves_evaluation_and_canonical_gradients` | 幅2、4、6で鏡映評価と正準行の勾配を検査する。 |
| `epoch_zero_halfkp_matches_p_and_nonzero_shared_head` | 同じシードの両型の初期評価と、非0の共有出力部を与えた評価を検査する。 |
| `screlu_derivative_includes_one_but_excludes_saturated_region` | x=0、1と飽和域の微分を検査する。 |
| `adamw_bias_correction_decay_and_zero_gradient_momentum_match_hand_calculation` | 最初の更新と、次のゼロ勾配更新を手計算で検査する。 |
| `integer_reference_uses_phase_truncation_saturation_and_excludes_royals` | 王駒、番兵、0方向への切り捨て、空局面を検査する。 |
| `integer_reference_handles_phase_boundaries_negative_truncation_and_evaluation_limit` | 駒数1、2、3、91、92、93、負数の除算、評価上限を検査する。 |
| `initialization_is_seeded_consumes_only_observed_canonical_rows_and_averages_mirrors` | 鏡映平均、乱数の決定性、観測行の昇順、標準偏差0.05を検査する。 |
| `constraints_restore_unobserved_rows_and_bound_folded_weights` | Pythonの極値の例に合わせ、切り詰めと初期値への復元を検査する。 |
| `halfkp_unobserved_feature_inherits_updated_virtual_row` | 特徴行が未観測でも、更新済み仮想行が合成値へ反映されることを検査する。 |
| `quantization_ties_even_rotation_and_overflow_match_python_examples` | Pythonの偶数丸めの例、P型の回転、上限超過、非有限値の拒否を検査する。 |
| `quantized_mnue_v5_round_trip_has_small_mean_integer_evaluation_error` | 両型のMNUE版5をminaseで復号し、小さい乱数モデルの平均絶対誤差が1 cp未満であることを検査する。 |
| `checkpoint_round_trip_restores_next_update_and_rejects_corruption_and_hash_mismatch` | 全状態の往復、次の更新のビット一致、破損とハッシュ不一致の拒否を検査する。 |
| `model_rejects_invalid_width_k_initial_and_mask_shapes` | 幅、K、初期値、観測マスク、最適化器の形の違反を検査する。 |
| `stored_npz_and_numpy_zip64_load_endpoints_and_reject_bad_archives` | NumPyが使うZIP64サイズと通常のstored ZIP、破損、圧縮指定、切断を検査する。 |
| `model::npz::tests::npy_rejects_dtype_order_shape_version_and_truncation` | NPYの型、配列順、形、版、切断を検査する。 |
| `dump_reference_writes_warmed_checkpoint_three_batches_gradients_and_states` | 両型について、先読み付きの事前3更新と、続く3更新の保存物を検査する。 |

v0の駒価値はすべて正なので、所有者に反する方向だけに掛かる上限制約は通常のv0参照では発火しない。
この区分の有限差分試験には、歩の参照値を反転した鏡映対称の整数PSTを明示的に使う。
実際の照合は常に`make_removal_reference(v0, v0)`を使う。

テストの検出力は、SCReLUの端点1の除外、第1層の後退での逆回転の削除、AdamWの1次モーメントのバイアス補正の削除を一時的に適用して確認した。
3つとも対応するテストのアサーションが失敗し、変更を戻した。
この範囲で検出されずに残った変更はない。

委任元での照合は、P型とHalfKP-hm-b8のそれぞれについて次の順で実行する。
`DATA`と`PST`を実在する入力へ置き換え、`OUTPUT_K`には基準MのMNPTに保存されたKを指定する。
通し番号はMNSDファイル列全体で0から数える。
以下の9局面は例なので、除去罰則が発火する局面を含むよう、必要に応じて番号を選ぶ。

```sh
DATA=/absolute/path/to/generated-1500000.bin
PST=/absolute/path/to/J75/pst-float.npz
OUTPUT_K=1072.6529541015625

for FEATURE_SET in P HalfKP-hm-b8; do
  DUMP_DIR="/tmp/nnue-reference-${FEATURE_SET}"
  CARGO_NET_OFFLINE=true nice -n 19 cargo run -j 2 -p nnue-trainer -- dump-batch \
    --feature-set "$FEATURE_SET" --data "$DATA" --indices 0,1,2 \
    --init-pst "$PST" --width 4 --k "$OUTPUT_K" --seed 71 --steps 3 \
    --lookahead-gamma 0.9 --lookahead-plies 40 \
    --reference-batch 0,1,2 --reference-batch 3,4,5 --reference-batch 6,7,8 \
    --output "$DUMP_DIR"
  PYTHONPATH=tools/train/src python tools/train/scripts/crosscheck_reference.py \
    --data "$DATA" --dump "$DUMP_DIR/reference.json"
done
```

`--steps N`は`--indices`のバッチでN回更新し、その直後を`checkpoint.bin`へ保存する。
続く3更新は`--reference-batch`の指定順に行い、損失を`reference.json`、勾配を`gradients-N.f32`、更新後の全状態を`after-N.bin`へ保存する。
Pythonは教師用Kと先読み教師を独立に計算し、特徴番号、損失、全パラメータ群の勾配、切り詰め後の重み、AdamWの両状態を照合する。
浮動小数点数は`numpy.isclose`の`rtol=1e-4, atol=1e-5`で検査し、不一致の場所と差を表示して終了コード1を返す。
Pythonとの3更新の実照合は委任元で行うため、Rustの単体テストの成功とは別に記録する。

補助損失が発火する資料を照合する場合は、`dump-batch`に`--init-std 0.2`などを指定し、Pythonの照合に`--require-auxiliary`を付ける。
このフラグは`reference.json`の3更新のうち少なくとも1更新の補助損失が正であることを要求する。
標準偏差だけでは発火を保証しないため、全更新が0ならスクリプトは明示的なエラーで終了する。

GPUの実装は次のファイルに置く。
各CUDAソースは`include_str!`で埋め込み、改行で連結してNVRTCで実行時にコンパイルする。
デバイスから取得した計算能力を`--gpu-architecture=sm_主番号副番号`で指定し、NVRTCの診断ログは`gpu::Error::Compile`に保持する。
幅や特徴集合は起動引数で渡し、ソースの置換やテンプレートによる特殊化は行わない。

| ファイル | 公開型と役割 |
|---|---|
| `src/gpu/mod.rs` | `gpu::Error`と`gpu::Result`を定義し、GPU不在、コンパイル、転送、入力不正、等価性違反を区別する。 |
| `src/gpu/device.rs` | `Device`がCUDA文脈、計算ストリーム、転送ストリームを所有し、デバイス名と計算能力を返す。 |
| `src/gpu/ring.rs` | `HostRing`がホスト3面とデバイス2面を所有し、`BatchTicket`で投入世代と所有する環を検査する。 |
| `src/gpu/engine.rs` | `Evaluator`がモデル、整数参照PST、作業配列を保持し、`Evaluation`へ前進、補助損失、全勾配、検証統計を回収する。 |
| `src/gpu/optimizer.rs` | 全自由パラメータとAdamWの両モーメントをデバイスに保持し、群ごとに更新する。 |
| `src/gpu/step.rs` | `Step`がカーネル1〜8を投入し、`StepResult`が損失と更新時間を遅れて回収する。 |
| `src/gpu/bench.rs` | `bench-step`が実データの供給速度、入力転送時間、GPUの更新時間とメモリ使用量を測る。 |
| `src/gpu/equivalence.rs` | `Report`、`run`、`check`が合成入力でCPUとの一致と反復実行のビット一致を検査する。 |
| `src/gpu/step_equivalence.rs` | 非0のモデルと最適化器の状態から1更新と連続3更新をCPU参照と照合する。 |
| `src/reference/validation.rs` | `reference::Validation`と`validation_forward`が検証統計のCPU参照を提供する。 |
| `kernels/forward.cu` | 第1層と線形経路の前進を実装する。 |
| `kernels/head.cu` | 出力部の後退と、後退を伴わない検証を実装する。 |
| `kernels/removal.cu` | 整数参照の除去差分、罰則、後退に使う除去ごとの微分と局面ごとの部分和を実装する。 |
| `kernels/backward.cu` | 逆索引による第1層と線形経路の後退、および仮想行の縮約を実装する。 |
| `kernels/reduce.cu` | 勾配のブロック部分和と固定順の縮約を実装する。 |
| `kernels/optimizer.cu` | AdamW、群ごとの切り詰め、HalfKP-hm-b8の畳み込みとP型出力部の展開を実装する。 |
| `tests/validation.rs` | 勝敗の符号、引き分け、飽和の端点、標準偏差の単位をGPUなしで検査する。 |

`BatchSlot::from_records`は記録と教師配列から合成バッチを作り、既存のデータ経路と同じ特徴番号と逆索引を構築する。
`Evaluator::new(device, model, removal_reference, capacity)`は整数参照PSTをデバイスへ一度転送し、評価器が生存する間保持する。
通常の参照には`RemovalReference::v0()`を渡す。
`Evaluator::evaluate`はカーネル1〜7と検証を実行し、`Evaluator::validate`は前進と検証のカーネルだけを起動する。
結果のホストへの同期回収は等価性検査用であり、エポックを進める処理とGPU上のパラメータ更新は含めない。

`Step::new(device, model, optimizer, removal_reference, capacity)`は更新に必要なモデル、最適化器、作業配列をデバイスへ確保する。
`Step::enqueue(ring, ticket, targets)`は前進、補助損失、出力部、縮約、第1層後退、線形後退、仮想行縮約、AdamW群の順で計算ストリームへ投入する。
全行の勾配は後退または縮約が書くため、更新後のゼロ化は行わない。
損失は2面のページ固定メモリへ非同期にコピーし、次の更新を投入してから`StepResult::collect`で回収する。
同時に保持できる未回収結果は2個であり、3個目の投入前に最初の結果を回収する。
末尾の結果は最後に回収し、全パラメータと最適化器の状態が必要なときだけ`Step::snapshot`で同期する。

更新では、未観測行のパラメータとモーメントを切り詰めも含めて固定する。
HalfKP-hm-b8は仮想行を先に更新し、特徴行の更新と`C=W+V[base]`の書き込みを同じ起動で行う。
特徴行が未観測でも、合成重みには更新済みの仮想行を反映する。
P型のバイアスと出力重みは両半幅の勾配を足した自由パラメータを更新し、全幅の配列に展開する。

カーネルの配列はCPU参照と同じ行優先順である。
Bを局面数、Hを全幅、UをP型ならH/2、それ以外ならHとする。
Rは第1層の行数で、P型は6,841、HalfKP-hm-b8は埋め草を含め109,441である。
全カーネルのブロックは`(128, 1, 1)`で、Gを`ceil(B/128)`、Cを`3U+1`とする。
以下の格子はx方向のブロック数を示し、y方向とz方向は1である。

| カーネル | 引数を宣言順に記す | 格子と共有メモリ |
|---|---|---|
| `forward_acc` | `features, weights, bias, acc, B, H, p`を受ける。 | 格子は`ceil(2B ceil(H/4)/128)`である。 |
| `forward_linear` | `features, weights, lin, B`を受ける。 | 格子は`ceil(2B/128)`である。 |
| `removal_forward` | `features, linear_features, counts, weights, linear, output, bias, acc, lin, reference, e_r, delta_r, ref_r, s_r, eligible, wrong_r, pen_r, d_r, S, removal_acc, removal_output, removal_linear, removal_loss, B, H, p, K`を受ける。 | 格子はB、静的共有メモリは1,928バイトである。 |
| `output_backward` | `acc, lin, counts, output, bias, targets, S, removal_acc, removal_output, removal_linear, removal_loss, f, h, logits, loss, g, d_acc, d_output, c, loss_partials, B, H, p, K`を受ける。 | 格子はG、共有メモリは2,048バイトである。 |
| `gradient_partials` | `d_acc, d_output, g, partials, B, H, units, chunk`を受ける。 | 格子はG、`units=U`、`chunk=128`である。 |
| `first_layer_backward` | `first_starts, first_entries, features, weights, acc, output, d_r, d_acc, d_first, rows, H, p`を受ける。 | 格子は`ceil(R ceil(H/4)/128)`、`rows=R`である。 |
| `linear_backward` | `linear_starts, linear_entries, counts, d_r, c, d_linear, rows, K`を受ける。 | 格子は`ceil(6841/128)`、`rows=6841`である。 |
| `reduce_virtual` | `d_first, d_virtual, H`を受ける。 | HalfKPだけで起動し、格子は`ceil(13681 ceil(H/4)/128)`である。 |
| `reduce_partials` | `partials, sums, blocks, values`を受ける。 | Qを列数とすると格子は`ceil(Q/128)`、`blocks=G`、`values=Q`である。 |
| `validation_head` | `acc, lin, counts, outcomes, output, bias, targets, partials, B, H, p, K`を受ける。 | 格子はG、共有メモリは7,168バイトである。 |

次の表で、上記引数の型と形を定める。
`p`はP型なら1、それ以外なら0で、B、H、`p`、`rows`、`units`、`chunk`、`blocks`、`values`はu32、Kはf32である。
そのほかの配列も、表に示す要素型をCUDA側の対応する型へそのまま渡す。

| 配列 | 型と形 |
|---|---|
| 第1層の`features, weights, bias, acc` | 順に`u32[B,2,145]`、`f32[R,H]`、`f32[H]`、`f32[B,2,H]`である。 |
| 線形前進の`features, weights, lin` | 順に`u32[B,145]`、`f32[6841,2]`、`f32[B,2]`である。 |
| 出力部の`counts, outcomes, output, bias` | 順に`u8[B]`、`u8[B]`、`f32[2,H]`、`f32[1]`である。 |
| `targets, S, logits, loss, g` | いずれも`f32[B]`である。 |
| `removal_acc, removal_output, f, h, d_acc, d_output` | いずれも`f32[B,2,H]`である。 |
| `removal_linear, c` | いずれも`f32[B,2]`である。 |
| 除去の`reference, linear_features, linear` | 順に`i64[6841,2]`、`u32[B,145]`、`f32[6841,2]`である。参照は1/8センチポーン単位の整数である。 |
| `e_r, delta_r, pen_r` | いずれも`f64[B,144]`で、評価、除去差分、罰則をセンチポーン単位で保持する。 |
| `ref_r, s_r, eligible, wrong_r, d_r` | 順に`i32[B,144]`、`f32[B,144]`、`u8[B,144]`、`u8[B,144]`、`f32[B,144]`である。真偽値は0か1である。 |
| `removal_loss` | `f64[B]`で、適格な除去の平均に係数1000を掛けてKで割った値を保持する。 |
| `first_starts, first_entries` | u32配列である。開始位置はP型で6,842個、HalfKPで109,441個とし、要素数は実際の発火数とする。 |
| `linear_starts, linear_entries` | u32配列である。開始位置は6,842個とし、要素数は手番側の発火数とする。 |
| `d_first, d_linear, d_virtual` | 順に`f32[R,H]`、`f32[6841,2]`、`f32[13681,H]`であり、埋め草行を含む。P型には仮想行配列を確保しない。 |
| `loss_partials` | `f64[G,2]`で、二値交差エントロピーと補助損失の順に保持し、縮約後にBで割る。 |
| 勾配の`partials` | `f64[G,C]`で、出力重み2U個、バイアスU個、出力バイアス1個の順である。 |
| 検証の`partials` | `f64[G,7]`で、BCEの和、符号一致数、非引き分け数、非線形出力の和、二乗和、下側飽和数、上側飽和数の順である。 |
| 縮約の`partials, sums` | `f64[G,Q]`と`f64[Q]`で、Qは2、C、7のいずれかである。 |

加算順はCPU参照に合わせ、第1層では左右の升を先に対にし、P型の出力部では左右半幅の活性化を足してから重みを掛ける。
乗算と加算の融合は無効にし、SCReLUの微分は0と1の端点を含める。
Hが4の倍数でない場合も、最後のスレッドがH未満の出力だけを処理する。
大きな中間配列はf32で保持し、ブロック部分和をf64で書いてからブロック番号順に縮約する。
アトミック加算は使わない。

`removal_forward`が生成した`S, removal_acc, removal_output, removal_linear`を出力部へ渡す。
これらはそれぞれCPU参照の`Removal::s, d_acc_removal, d_output, linear`に対応する。
`removal_linear`はCPU参照と同じ`S(u_r-u)/K`で、出力部が`g u/K`を加えて`c`を組み立てる。
整数参照は負数も0方向へ切り捨て、±28,999で切り詰めて符号を取り、罰則の大きさは切り捨て前の差分から求める。
除去の共有領域は144列のスカラーだけを保持し、累算器と出力重みへの寄与は各スレッドのf64レジスタに列順で足す。
`(B,144,H)`の中間配列は作らない。

第1層と線形経路の後退は、逆索引の範囲を固定順に読み、空範囲と埋め草行にも0を書く。
列144の先獅子には除去の補正を掛けず、P型の第1層だけ半幅の逆回転を行う。
HalfKPの埋め草行は逆索引に含まれないため、その範囲を読む前に0を書く。
仮想行の縮約は8区分を番号順に足し、最後の埋め草行に0を書く。

非線形出力の和と二乗和はセンチポーン単位で集め、分散は局面数で割った母分散とする。
符号一致率は引き分けを除き、ロジット0は勝ち予測に含めず、全局面が引き分けなら`None`を返す。

ホスト面は「書き込み可能→転送中→転送完了→書き込み可能」の順に再利用する。
各面は`BatchSlot`と同じ順の全配列を持ち、通し番号も含めて`alloc_pinned_with_flags(len, 0)`で確保する。
逆索引の要素配列は両方とも`最大B×2×145`語を確保し、実際の局面数と2つの要素数を別に持つ。
転送前のコピーは、そのホスト面の転送完了イベントをホストで待ってから行う。
転送時は`PinnedHostSlice`をそのまま`memcpy_htod`へ渡し、容量全体をコピーする。
カーネルが読む範囲は実際の局面数と要素数で制限する。

デバイス面は「空き→投入済み→後退完了を記録済み→次の転送」の順に進む。
次の転送ストリームは、その面の後退完了イベントを待つ。
転送後にホスト面の転送完了イベントを記録し、計算ストリームがそのイベントを待ってから面を読む。
`finish`は最後の使用を計算ストリームへ投入した後に呼び、その時点までの後退完了を記録する。
完了を記録していない面、古い世代の券、別の環の券は拒否し、部分転送の失敗後はその環の再利用も拒否する。
ストリーム間の`join`は呼ばない。

GPU処理の`unsafe`はすべて`gpu`モジュールにある。
各起動の前にRust側で配列長を検査し、各ブロックの直前に前提条件を書く。
既存のデータ読み込みにあるmmapの`unsafe`は変更していない。

| 所在 | 前提条件 |
|---|---|
| `device.rs`のドライバ存在検査 | OSがロードするCUDA共有ライブラリを信頼し、GPUポインタを渡さない。 |
| `device.rs`の固定メモリ確保 | 要素数とバイト数を検査し、フラグ0で確保する。読み出す前に全要素を初期化する。 |
| `device.rs`のゼロ初期化 | ポインタは整列済みでlen要素を保持し、`ValidAsZeroBits`を満たす。参照や転送がまだ存在しない。 |
| `device.rs`のNVRTC存在検査 | OSがロードするNVRTC共有ライブラリを信頼し、未検証のポインタを渡さない。 |
| `engine.rs`の第1層起動 | 特徴配列、R×Hの重み、Hのバイアス、B×2×Hの出力を検査する。特徴番号は`BatchSlot`の不変条件で範囲内か番兵と保証する。 |
| `engine.rs`の線形前進起動 | B×145、6,841×2、B×2の長さを検査し、正準行番号と番兵の契約を使う。 |
| `engine.rs`の補助損失起動 | B×144のスカラー、B×2×Hの寄与、6,841×2の整数PSTを検査する。1ブロックが1局面を担当する。 |
| `engine.rs`の出力部起動 | B、B×2、B×2×Hの各配列とG×2の部分和を検査し、2×128個のf64の共有メモリを確保する。 |
| `engine.rs`の勾配部分和起動 | 入力のB×2×HとB、出力のG×Cを検査し、UがHまたはH/2であることを`Shape`で保証する。 |
| `engine.rs`の第1層後退起動 | 逆索引、B×144の微分、B×2×Hの入力、R×Hの出力を検査する。HalfKPの埋め草では逆索引を読まない。 |
| `engine.rs`の線形後退起動 | 6,842個の開始位置、逆索引、B局面分の係数、6,841×2の出力を検査する。列144の微分を読まない。 |
| `engine.rs`の仮想行縮約起動 | 109,441×Hの入力と13,681×Hの出力を検査する。8区分を読み、埋め草は入力を読まず0にする。 |
| `engine.rs`の縮約起動 | G×Qの部分和とQ個の出力を検査し、1スレッドが出力1列だけを担当する。 |
| `engine.rs`の検証起動 | 前進と教師配列、B個の結果、G×7の部分和を検査し、7×128個のf64の共有メモリを確保する。 |
| `optimizer.rs`の4種類の更新起動 | 自由パラメータ、勾配、両モーメント、行マスクと展開先の長さを検査し、同じ計算ストリーム上で仮想行を特徴行より先に更新する。 |

カーネルと配列は同じ`Device`から作り、書き込み先には排他的な参照を渡す。
cudarcのイベント追跡を有効に保ち、転送中または起動中のメモリの解放も追跡する。
CPUテストでは、CUDAの引数宣言とRustの起動引数の順序、ソースの連結、面の状態遷移、入力範囲、検証統計を確認する。
GPUを要するテストには`#[ignore]`を付け、名前はすべて`gpu_`で始める。

等価性検査の合成記録は、回転と番兵、先獅子、補間率の境界、空の逆索引範囲、3分類の教師値を含む。
幅2、6、8、128と両特徴集合について、バッチ64、4,096、100、1、129を同じ評価器へ順に渡し、面の再利用も検査する。
CPUとの判定は既存のPython照合と同じ許容誤差の形式で、`|GPU-CPU| ≤ 1e-6 + 1e-5 |CPU|`とする。
非有限値は拒否し、全中間値と縮約値を同一入力で2回実行してビット単位でも比較する。
検証専用の起動も2回実行し、出力部の後退を伴う場合の検証値とビット一致を要求する。

補助損失の比較には、`d_r`、`S`、累算器と出力重みへの寄与、線形係数、局面ごとの損失を含める。
除去後の評価、差分、整数参照、適格性、罰則も比較し、第1層と線形経路の全行、およびHalfKPの仮想行の勾配を照合する。
整数と真偽値には完全一致を要求する。
CLIと各幅のGPU試験は、v0参照のランダム入力に続けて、歩の符号を反転した鏡映対称の整数PSTによる4局面を照合する。
この入力は、下限罰則、上限罰則、適格で無罰則、参照符号0を含む。
王将と太子には非零の参照差分を与え、王駒という理由で除外されることも確かめる。
CPUテスト`removal_inputs_cover_lower_upper_inactive_and_all_ineligible_reasons`が、この各区分の発火を両特徴集合と幅2、6、8、128で確認する。
期待値は設計書の罰則区分とCPU参照に基づき、実GPU上の一致と反復の決定性はignoreされたテストで確認する。

更新の等価性検査は、非0のモデル、両モーメント、更新回数から始める。
1更新後と連続3更新後の全パラメータ、モーメント、更新回数、合成重み、出力部の展開と損失をCPU参照と照合する。
同じ初期状態から2回実行した結果にはビット単位の一致を要求する。

GPUを使わない検査は次のコマンドで実行する。

```sh
cargo fmt --all
nice -n 19 cargo clippy -j 2 --workspace --all-targets -- -D warnings
nice -n 19 cargo test -j 2 -p nnue-trainer
```

GPUを使える環境では次のコマンドを実行する。
カーネルのコンパイル成功と、実GPUでの値の一致は別々に確認する。

```sh
nice -n 19 cargo test -j 2 -p nnue-trainer -- --ignored gpu_ --test-threads=1
for FEATURE_SET in P HalfKP-hm-b8; do
  nice -n 19 cargo run -j 2 -p nnue-trainer -- check-equivalence \
    --feature-set "$FEATURE_SET" --width 6 --batch 100 --seed 71
  nice -n 19 cargo run -j 2 -p nnue-trainer -- check-equivalence \
    --feature-set "$FEATURE_SET" --width 128 --batch 4096 --seed 71
done
```

実データでの測定は、最適化を有効にしたバイナリで行う。
次の例では訓練集合に少なくとも835,584局面が必要であり、検証集合は使わない。
各ファイルには来歴を記した`.provenance.json`を添える。

```sh
DATA=/absolute/path/to/generated-1500000.bin
for FEATURE_SET in P HalfKP-hm-b8; do
  CARGO_NET_OFFLINE=true nice -n 19 cargo run --release -j 2 -p nnue-trainer -- bench-step \
    --feature-set "$FEATURE_SET" --width 128 --batch 16384 --steps 50 \
    --data "$DATA"
done
```

`bench-step`はファイル検査、モデルの初期化、カーネルのコンパイル、GPU配列の確保を済ませ、1回のウォームアップの後で指定回数を測る。
シード71で訓練局面を並べ替え、ウォームアップを含む`(steps+1)×batch`局面を重複なく使う。
`--seed`、`--device`、`--workers`で条件を指定でき、ワーカー数の既定値はLinuxが報告する物理コア数から2を引いた数で、最小1、最大は測定更新数とする。
CPUの構成を取得できない場合は`--workers`を明示する。

供給側は既存の`build_batches`でワーカー数以下のバッチを並列に構築し、順列の順に投入する。
ワーカーの供給速度は、復号、特徴抽出、逆索引構築、教師値変換、スレッドの起動と回収を含む構築時間から求め、GPU待ちによる送信の停止時間を除く。
完成バッチを渡すチャネルの容量は3とし、供給側にはワーカー数以下のバッチを別途保持する。
転送には`HostRing`の固定メモリ3面とデバイス2面を使う。
転送時間は、通し番号を含む`BatchSlot`の全配列と教師確率について、転送ストリームのイベントで測ったコピー時間を合計する。
入力面の再利用待ちとホスト内のコピーをこの指標から除き、計算時間は教師確率の到着後の前進からAdamW群の完了までを計算ストリームのイベントで測る。
全体の経過時間には供給、固定メモリへのコピー、投入、損失回収を含めるため、供給と転送を重ねた実効速度も確認できる。

測定用モデルでは、埋め草以外の全行を観測済みとし、線形をv0の駒価値、第1層を標準偏差0.05の乱数で初期化する。
バイアスは0.25、出力重みは±0.01を交互に置き、AdamWは既定の係数を使う。
出力と全教師分類に共通のKを使い、既定値1000は`--k`で変更できる。
これは学習を始める前の速度測定であり、教師Kの推定、観測マスクの全訓練集合からの作成、保存、検証は行わない。
GPUメモリはデバイス全体の使用量と、入力面と更新器を確保する前後の差をバイト単位で出力するため、他のプロセスによる変動も含む。
