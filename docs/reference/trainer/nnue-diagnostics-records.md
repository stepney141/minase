# NNUEの診断の記録

本書は、NNUEの重みを診断する3つのコマンド`minase-trainer diag nnue-distribution`、`diag nnue-removal`、`diag nnue-moves`が保存する報告の欄を定める。
`diag nnue-removal`が標本を抽出する検証集合は、[学習データと教師値](data-and-teachers.md)の検証分割で決まる。

## 共通の事項

`diag nnue-distribution`、`diag nnue-removal`、`diag nnue-moves`はCPUで実行し、`--output`の新規JSONファイルへ報告する。
重みと入力の同一性は正規化した`path`と`sha256`で記録し、数式と診断の目的は[NNUEの完全代替](../../plans/nnue-replacement.md)に従う。

## 評価値の分布

`diag nnue-distribution`は`--pair NAME BASELINE CANDIDATE`でMNPT版3とMNUE版5を指定し、複数の組を同じ標本で比較する。
`--data`の全局面を母集団とし、SplitMix64で`--sample`件を非復元抽出して通算番号の昇順に並べる。
`sample_count`、`seed`、`sample_indices_sha256_le_i64`、`input_files`、`population_count`、`sampling`に抽出条件を保存する。
標本のハッシュはリトルエンディアンi64列を対象とし、`input_files`は入力順のパス、SHA-256、`record_count`を持つ。

`evaluation_perspective = "side_to_move"`、`standard_deviation_ddof = 0`、`percentile_method = "linear"`、`spearman_tie_method = "average"`に統計の定義を記録する。
`regression_equation`は回帰式を記録する。
`pairs`の各要素は比較名`name`、重みの`baseline`と`candidate`、`sample_count`、次の統計を持つ。

| 欄 | 定義 |
|---|---|
| `baseline_statistics`、`candidate_statistics` | `mean_cp`、母標準偏差`std_cp`、`mean_absolute_cp`、絶対値の5、25、50、75、95百分位点`absolute_percentiles_cp`を持つ。 |
| `std_ratio_candidate_over_baseline` | 候補の標準偏差を基準の標準偏差で割った比である。 |
| `regression_slope`、`regression_intercept_cp` | `candidate = intercept + slope * baseline`の最小二乗回帰の傾きと切片である。 |
| `pearson_correlation`、`spearman_correlation` | 評価値の相関係数と、同順位に平均順位を与えた順位相関係数である。 |
| `regression_residual_std_cp`、`difference_std_cp` | 回帰残差と候補−基準の差の母標準偏差である。 |
| `absolute_difference_percentiles_cp`、`absolute_difference_over_100_cp_fraction` | 絶対差の50、90、99百分位点と、100 cpを厳密に超える割合である。 |
| `verdict` | 標準偏差比が閉区間`[0.9, 1.1]`内かどうかで、散らばりの尺度を判定する。区間は`unchanged_scale_interval_inclusive`にも保存する。 |
| `by_phase_q`、`by_baseline_absolute_cp` | 同じ統計を、位相分子qの0〜29、30〜59、60〜90、および基準の絶対評価0〜99、100〜299、300〜999、1,000 cp以上に分ける。各帯は両端の値と標本数を持つ。 |

qは盤上総駒数から2を引いて0〜90に制限した値である。
全体またはいずれかの帯の標本が2件未満、あるいは基準か候補の分散が0なら報告の生成を拒否する。

## 駒除去差分

`diag nnue-removal`は`--float`の`nnue-float.bin`と`--net`のMNUE版5を検査する。
`--data`の検証集合から`--sample`件を抽出し、王将と太子以外の駒を1枚ずつ除いたときの差分を測る。
標本の同一性は分布診断と同じ欄に記録し、`split = "validation"`、検証集合の`population_count`、重みの`float`と`net`、除去件数`removal_count`を加える。

`float_batch_vs_recomputed`は、累算器からの行の減算と局面を作り直したf64再計算の最大差`max_absolute_cp`を、`limit_cp = 1e-6`で検査して`passed`を記録する。
実数の再計算には整数ネットのKを使う。
`float_vs_integer_recomputed`は実数と整数の除去差分の`mean_absolute_cp`と`max_absolute_cp`である。

`reference`は両端点にv0の駒価値を使うことを示し、`reversals`はその参照と整数ネットの差分の厳密な符号反転を集計する。
`count`は反転件数、`total`は参照差分が非0の件数、`fraction`はその比、`mean_absolute_cp`と`max_absolute_cp`は反転した差分の絶対値の統計である。
分母や対象が空の場合の統計は`null`とし、同じ集計を駒状態番号ごとの`by_piece_state`にも保存する。
実数の2経路の差が上限を超えた場合は`error`付きの報告を保存して終了コード1を返す。

## 合法手の評価差

`diag nnue-moves`は`--positions`の全局面の合法手を、`--net`のNNUEと`--base`のPST＋FMで比較する。
入力MNSDに来歴ファイルは不要で、規則セットはエンジンの既定規則とする。
`net`、`base`、`positions`にファイルの同一性、`position_count`に局面数、`evaluation_perspective = "side_to_move_before_move"`に視点を記録する。
`standard_deviation_ddof = 0`は母標準偏差を使うことを示す。

着手前をB、着手後をB′、着手前の手番側をtとすると、差分は`−F_opp(B′)−F_t(B)`である。
`by_position`は入力順の`index`、合法手の`matched_count`、片側にしかない`candidate_only_count`と`baseline_only_count`を持つ。
`candidate`と`baseline`は各モデルの着手差分、`candidate_minus_baseline`と`candidate_minus_baseline_pst`は共通の合法手で測った候補と基準全体、候補と基準PSTだけの差を表す。
各統計は`count`、`mean_cp`、母標準偏差`std_cp`、`mean_absolute_cp`、`max_absolute_cp`を持ち、空の集合では件数以外を`null`とする。
片側だけの手数は最上位にも合計し、非0なら`error`付き報告を保存して終了コード1を返す。
