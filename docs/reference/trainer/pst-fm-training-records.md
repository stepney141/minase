# PST、FM、同時学習の記録

本書は、PSTの学習`minase-trainer pst train`、FMの学習`fm train`、両者の同時学習`joint train`、およびFMの変換`fm quarter`と`fm scale`が保存するファイルと、各記録の欄を定義する。
入力の学習データと教師値の扱いは[学習データと教師値](data-and-teachers.md)に、学習を実行する手順は[PSTとFMの学習手順](../../guides/pst-training.md)にある。

## 学習の実行条件

`pst train`は鏡映対で重みを共有するPSTを学び、`fm train`は`--init`の整数PSTを固定してランク32のFMを学ぶ。
`joint train`はPSTとFMを2群のAdamWで更新する。
PSTの補間と除去損失の式は[PSTとFMによる評価関数と学習の目的関数](../../guides/evaluation-pst-fm.md)にあり、方式を選んだ理由は[線形の評価の2組の重みを盤上総駒数で補間する](../../adr/interpolate-the-linear-path-by-total-piece-count.md)と[駒除去差分の符号を守らせる片側絶対値の補助損失を学習に加える](../../adr/penalize-wrong-signed-piece-removal-differences.md)にある。
同時学習の損失と2群のAdamWの実装は、`crates/minase-trainer/src/joint/`のdocコメントにある。
出力用K、47個の探索用駒価値、規則セット名は初期MNPTから引き継ぐ。

3コマンドともCPUで実行し、新規実行では空の`--output-dir`を使う。
`--resume`には同じ出力ディレクトリを指定し、完了したエポックの境界から再開する。
再開時は総エポック数、`patience`、スレッド数を変更できるが、学習の軌跡を変える条件と入力は照合する。
各エポックの順列にはSplitMix64を使い、FMと同時学習の鏡映はシード、エポック番号、局面番号から決める。
このため再開とスレッド数の変更で局面の順序や鏡映は変わらない。

## 出力ファイル

最良エポックは1以降から選び、全検証局面の平均損失が厳密に改善しないエポックが`patience`回続くと停止する。
エポック0より損失が高いという理由だけでは候補を除外しない。
各コマンドが保存するファイルは次のとおりである。

| ファイル | 内容 |
|---|---|
| `config.json` | 引数、再開時に照合する設定、入力の検査和、前処理の結果、再開履歴を保存する。 |
| `training.jsonl` | 学習前のエポック0と、検証を完了した各エポックを1行ずつ保存する。 |
| `latest.bin` | 最後に検証を完了したモデル、最適化器の状態、進捗、累積時間、照合用ハッシュを保存する。 |
| `best.bin` | 最良エポックのモデルと、その時点の最適化器の状態を保存する。 |
| `summary.json` | 停止理由、最良エポック、量子化検査、書き出した重みの初期局面評価を保存する。 |
| `pst.bin`、`pst-float.npz` | `pst train`がMNPT版2と実数のPSTを保存し、`joint train`がMNPT版3と実数のPSTとFMを保存する。 |
| `fm.bin`、`fm-float.npz` | `fm train`が縮小前のFMを含むMNPT版3と実数のFMを保存する。 |

NPZは圧縮なしで、PSTの`middlegame`と`endgame`は各13,680要素のf32配列をセンチポーン単位で持つ。
FMの`V`は形`[13680, 32]`、`a`は形`[32]`のf32配列であり、`mask`は13,680要素の真偽値配列である。
`joint train`の`pst-float.npz`にはこの5配列をまとめ、`fm train`の`fm-float.npz`には`V`、`a`、`mask`だけを保存する。

## 設定と前処理

`config.json`の各欄は、条件の記録と再開時の照合に使う。

| 欄 | 定義 |
|---|---|
| `device`、`resolved_threads` | 実行デバイス`"cpu"`と実効ワーカー数を記録する。 |
| `arguments` | 新規実行時の引数を保存する。 |
| `settings` | モデル名、最適化器の係数、教師条件など、再開で変更できない設定を保存する。 |
| `inputs.files` | MNSDと来歴の正規化したパスとSHA-256、およびヘッダの`header_sha256`を入力順に保存する。 |
| `inputs.init` | PSTとFMの学習で、初期MNPTのパスとSHA-256を保存する。 |
| `inputs.base`、`inputs.pst_init`、`inputs.fm_init`、`inputs.fm_reference` | 同時学習で、基準MNPT、実数PST、実数FM、照合用の縮小前FMのパスとSHA-256を保存する。 |
| `preprocessing.teacher_classes` | 教師の7項目、`lookahead`、適用した`lambda_override`を分類順に保存する。検査和は32バイトの配列、先読みは`{gamma, plies}`または`null`である。 |
| `preprocessing.teacher_ks`、`teacher_lambdas` | 分類順の教師用Kと実効λを保存する。 |
| `preprocessing.teacher_k_source`、`output_k` | 推定元`"training"`と、固定した出力用Kを記録する。 |
| `preprocessing.training_positions`、`validation_positions` | 訓練と検証の局面数を保存する。 |
| `preprocessing.training_indices_sha256`、`validation_indices_sha256` | 入力を連結した0始まりの通算番号を、リトルエンディアンu64列にしたSHA-256を保存する。 |
| `preprocessing.feature_observations`、`unobserved_features` | FMと同時学習で、訓練集合とその全鏡映による全13,680特徴の観測回数と未観測特徴数を保存する。 |
| `preprocessing.mask_mismatches`、`removal_reference` | 同時学習で、読み込んだマスクと数え直したマスクの不一致数、および除去参照`"v0"`を保存する。 |
| `config_sha256`、`input_sha256` | それぞれ`settings`と、`inputs`および`preprocessing`をまとめた内容のSHA-256を保存する。 |
| `preprocessing_seconds` | 新規実行で設定を書き出すまでの秒数を記録する。 |
| `resumes` | 再開ごとに`resumed_at_unix_seconds`、`epochs`、`patience`、`workers`、完了済みの`epoch`を追記する。 |

同時学習では、実数PSTの再量子化が`--base`のPSTと一致し、実数FMの再量子化が`--fm-reference`のFMと一致することを開始時に検査する。
観測マスクは今回の訓練集合で作り直し、未観測のFM行を0にする。

## エポックごとの記録

`training.jsonl`の`device`は`"cpu"`、`epoch`は完了したエポック番号、`samples`は全検証集合の局面数である。
損失は局面数で重み付けした平均であり、端数バッチも含める。

| 欄 | 定義 |
|---|---|
| `train_loss` | 教師確率に対する訓練集合の二値交差エントロピーである。 |
| `removal_loss` | PSTと同時学習の、係数を掛ける前の駒除去損失である。 |
| `regularization_loss` | FMの`lambda_res·mean(Φ²)`であり、係数を含む。 |
| `phi_squared_mean` | 同時学習で、係数を掛ける前の`mean(Φ²)`を記録する。 |
| `total_loss` | PSTでは`train_loss + removal_penalty·removal_loss`、FMでは`train_loss + regularization_loss`、同時学習では`train_loss + lambda_res·phi_squared_mean + removal_penalty·removal_loss`である。 |
| `validation_loss` | 全検証集合の二値交差エントロピーであり、除去損失と正則化項を含めない。 |
| `phi` | FMと同時学習で、全検証局面のFM補正Φの`mean`、母標準偏差`std`、`max_absolute`、`samples`を記録する。Φはロジット単位であり、同時学習では`unit = "logit"`と`std_cp = K·std`も持つ。 |
| `training_seconds`、`validation_seconds` | そのエポックの訓練と検証にかかった秒数である。 |
| `training_positions_per_second` | 訓練局面数を訓練秒数で割った速度である。 |
| `update_seconds` | 1更新あたりの平均計算秒数である。 |
| `elapsed_seconds` | 前処理と再開前の時間を含む累積秒数である。 |
| `steps`、`total_steps` | そのエポックと、実行開始からの累積の更新回数である。 |

エポック0では訓練損失、訓練時間、速度、更新時間を`null`、更新回数を0とし、検証値を記録する。
PSTと同時学習の除去係数が0なら除去計算を省略し、学習後の`removal_loss`は0となる。

## 終了時の記録と量子化検査

`summary.json`の`status`は`candidate`、`quantization_error`、`error`のいずれかであり、FMには`excluded_zero_correction`もある。
`stopped_by`は`max_epochs`または`early_stopping`、`reason`は失敗や除外の理由、`convergence`は`null`である。
`last_epoch`、`best_epoch`、`best_validation_loss`、`initial_validation_loss`、`improved_over_epoch_zero`、`total_steps`、`elapsed_seconds`に最終進捗を記録する。
処理途中で失敗した場合は、判明している進捗と`reason`だけを持つ短い報告になることがある。

`initial_position_score`は、書き出した整数重みで初期局面を評価した値であり、手番側の視点のセンチポーンである。
FMと同時学習ではPSTにFM補正を加え、静的評価の範囲±28,999へ切り詰める。
重みを書き出せなかった場合は`null`または未記録となる。
量子化の平均絶対誤差が上限を超えた場合も、書き出した重みと評価値の記録は残す。

`quantization`は検証集合から最大`--validation-sample`件を選んだ検査であり、`samples`、`seed`、`split = "validation"`、抽出順の`indices_sha256`を持つ。
`mean_absolute_error_cp`と`max_absolute_error_cp`は実数評価と整数評価の絶対誤差である。
FMでは1/4補正の`quarter_mean_absolute_error_cp`と`quarter_max_absolute_error_cp`も持ち、FMと同時学習では量子化の`exponent`も記録する。
`limit_cp = 2.0`と`passed`で判定を保存し、FMは縮小前後の両方でこの上限を満たす必要がある。
同時学習では学習前にも同じ検査を行って`initial_quantization`に保存し、各量子化報告の`correction_cp_std`には整数FM補正の母標準偏差を記録する。
不合格なら保存物を残して終了コード1を返す。

FMと同時学習の`gradients`は、最初の最大5更新の`step`と勾配のユークリッドノルムを持つ。
FMでは`embedding`と`a`、同時学習ではさらに`pst`を記録し、再開時にも引き継ぐ。
終了時の`phi`は最良モデルの全検証集合の分布である。
FMの`correction_cp`は量子化検査と同じ標本の`std_before_quarter`と`std_after_quarter`をセンチポーンで記録し、`samples`、`seed`、`split`も保存する。

FMの`quarter_zero_check`は、1/4補正が常に0になる十分条件を検査する。
`all_pair_corrections_zero`は各特徴対を単独で評価すると整数補正が0であること、`identically_zero`は最大145特徴の和でも0であることを示す。
`max_absolute_pair_numerator`は異なる特徴対の符号付き内積の最大絶対値、`absolute_sum_bound`は全対の絶対値和と最大内積×最大対数の小さい方、`pair_denominator`は1対の分母`2^(2e)`である。
`identically_zero`が真なら`status = "excluded_zero_correction"`とする。
各対が0でも和は非0になり得るため、対の検査だけでは除外しない。

## FMの変換の記録

`fm quarter`はMNPT版3の指数だけを1増やし、補正を1/4にする。
`--output out.bin`なら`out.bin`と`out.quarter.json`を作り、入力、出力のSHA-256である`input_sha256`、`output_sha256`、変換前後の`exponent_before`、`exponent_after`、および上記と同じ定義の`zero_check`を報告する。
指数30の入力は拒否する。

`fm scale`は実数の出力係数`a`へ`--fm-scale / 1024`を掛け、FMを再量子化する。
倍率は0〜1,024の整数であり、0ではFMを含まないMNPT版2、1,024では入力と同じ重みを書き出す。
`--float`にはPSTの2端点と`V`、`a`を持つNPZが必要であり、同時学習の`pst-float.npz`を使える。
保存前にPSTとFMの再量子化が入力MNPTと一致することを検査し、PST、出力用K、探索用駒価値を保持する。
式の根拠は[FMの倍率のSPSA調整](../../plans/fm-scale-spsa.md)にある。

`--output out.bin`なら、重みと`out.scale.json`に次の欄を保存する。
両変換とも、重みか報告の出力先が既存なら拒否する。

| 欄 | 定義 |
|---|---|
| `input_sha256`、`float_sha256`、`output_sha256` | 入力MNPT、実数NPZ、出力MNPT全体のSHA-256である。 |
| `fm_scale`、`format_version` | 指定した整数倍率と出力MNPTの版を記録する。 |
| `exponent_before`、`exponent_after`、`max_absolute_embedding` | 入力と出力の指数、出力埋め込みの最大絶対値を記録する。出力が版2なら後2項目は`null`となる。 |
| `validation.samples`、`seed`、`split` | 検証標本数、抽出シード、`"validation"`を記録する。 |
| `validation.float_vs_integer` | 実数評価と出力の整数評価の`mean_absolute_cp`と`max_absolute_cp`を記録する。 |
| `validation.tuning_vs_baked` | 調整時の補正と焼き込んだ補正の絶対差を、同じ2欄で記録する。 |
| `validation.correction_cp` | 焼き込んだ補正の母標準偏差`output_std`と、調整時の補正の`tuning_std`をセンチポーンで記録する。 |

`validation`は`--data`を指定した場合だけ保存し、SplitMix64で検証集合から最大`--validation-sample`件を非復元抽出して昇順に並べる。
調整時の補正は、入力FMの分子Nと指数eから`trunc(N·fm_scale / (1024·2^(2e+1)))`として0方向へ切り捨てる。
この変換は誤差を記録するが、誤差の大きさによる合否判定は行わない。
