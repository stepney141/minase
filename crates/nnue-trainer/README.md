フェーズ1bは、NNUEの浮動小数点モデルと、設計書のカーネル1〜8を検証するCPU参照実装を提供する。
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

`Model::initialize`は全基底特徴の線形2端点と観測マスクを受け取る。
`shape`、`parameters`、`parameter_mut`、`masks`、`initial_linear`、`initial_virtual`が値を公開する。
`expanded_biases`、`expanded_output_weights`、`expanded_linear`、`expanded_first_layer`が共有パラメータを展開し、最後の関数はHalfKP-hm-b8の特徴分解も畳み込む。
`constrain`は有限性を検査してから切り詰め、`fix_unobserved`で未観測行と埋め草を復元する。

`RemovalReference::v0`は訓練用の参照を作り、`new`は検査用の任意の鏡映対称な整数PSTを受け取り、`deltas`は整数参照差分と切り捨て前の差分の大きさを返す。
Pythonと同じく、罰則の符号には整数評価の差を、上限には切り捨て前の差の絶対値を使う。
線形の除去差分はf64で計算し、非線形差分はf32で集約してからf64に広げる。
`Losses::auxiliary`は係数1000を含み、`Losses::total`が二値交差エントロピーとの和を返す。

`AdamW::new`は状態を0で作り、`m`、`v`、`step`、`config`で再開に必要な状態を公開する。
`AdamW::update`は全要素を更新した後で`Model::constrain`を呼ぶ。
未観測行もPyTorchと同じくモーメントを更新し、重みだけを初期値に戻すため、観測マスクで更新自体を省略する処理とは区別する。
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
