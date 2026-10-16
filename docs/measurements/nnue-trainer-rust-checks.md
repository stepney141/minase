# Rust製NNUE学習器の検査と試走の記録

## 目的

[Rust製NNUE学習器（GPU）の設計書](../plans/archived/nnue-trainer-rust.md)の各フェーズの完了条件に対する検査結果と、学習の試走の速度を記録する。
棋力測定ではないので、ペンタノミアル度数やLLRは持たない。

## 環境

学習機はRTX 3060 Ti（8 GB、CUDA 13.4、ドライバ615.71）、CPUはCore Ultra 7 265KF（20コア）、メモリ30 GB、Rust 1.98.0、cudarc 0.19.10である。
検査の間、同じ機械で教師データの生成（同時12対局）とlishogiのBotが動いていた。
Pythonの照合は`tools/train/.venv`（torch 2.13.0、numpy）で行った。

## フェーズ0　`minase`の公開APIとMNUE版5の符号化

| 検査 | 方法 | 結果 |
|---|---|---|
| 符号化と復号の往復 | 埋め込みの`nets/nnue.bin`を復号して符号化し、元のバイト列と比較。ランダム重みのHalfKP-hm-b8でも往復（`cargo test`） | バイト単位で一致 |
| 符号化の拒否 | 範囲外の値、鏡映の共有違反、奇数の幅、K=0、不正な配列長、規則セット名の長さ超過（`cargo test`、13件） | すべて拒否 |
| 推論への影響 | `minase dev bench --depth 9`の15局面のノード数、最善手、評価値を変更前と比較 | 一致（総ノード数5,229,890） |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（898件） | 通過 |

## フェーズ1a　データ経路（前処理とバッチ構築）

| 検査 | 方法 | 結果 |
|---|---|---|
| 検証分割と先読み教師と教師用K | Pythonの関数の値を写した単体テスト（`hash64`のベクトル、先読みの例、Kの小例） | 一致 |
| 特徴番号の照合（実データ） | `dump-batch`で世代3の1ファイル（`data/gen3/generated-1500000.bin`）の12局面（先頭3局面、末尾2局面、途中7局面）をP型とHalfKP-hm-b8でバッチにし、`tools/train/scripts/crosscheck_batch.py`で`minase_train.nnue.feature_sets.indices`と両視点145列および線形145列を比較 | 両方の型で全列一致 |
| 逆索引の性質 | 要素数、固定順、語の往復、番兵の除外（`cargo test`、統合テスト22件） | 通過 |
| 1ファイルの読み込みと先読みの前処理 | `dump-batch`の経過時間と最大常駐メモリ（`/usr/bin/time`） | 約1.5秒、約569 MB |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer` | 通過 |

## フェーズ1b　モデルとCPU参照実装

| 検査 | 方法 | 結果 |
|---|---|---|
| 勾配の数値微分との一致 | 幅4、合成局面で、第1層、線形、バイアス、出力重み、出力バイアス、仮想行の勾配を中心差分と比較。補助損失の各区分が発火する例を含む（`cargo test`） | 通過 |
| P型の鏡映不変とエポック0の同一性 | 局面とその鏡映の評価の一致、同じシードのP型とHalfKP-hm-b8の評価の一致（`cargo test`） | 通過 |
| 量子化とMNUE版5の往復 | 量子化して符号化し、`minase`で復号して整数評価と浮動小数点の評価を比較（`cargo test`） | 通過 |
| PyTorchとの3更新の照合（実データ） | `dump-batch --width 4 --seed 71 --steps 3`で世代3の1ファイルの9局面を3バッチにし、`crosscheck_reference.py`でPyTorchの`minase_train.nnue.model`と`torch.optim.AdamW`に同じ初期状態を写して3更新し、損失、勾配、切り詰め後の重みを比較（相対10⁻⁴、絶対10⁻⁵） | 両方の型で一致。この標本では補助損失の罰則が発火しない（補助損失0）ため、線形の初期値を0倍にして除去差分を0にし下限側の罰則を全除去で発火させた標本（`--init-pst-scale 0 --init-std 0.5 --width 8`、12局面、補助損失0.17〜0.29）でも3更新を照合し、両方の型で一致した。上限側の罰則（参照の向きが駒の持ち主と逆の除去）は実データでは稀であり、数値微分の単体テストだけで検査している |
| J75のnpzの読み込み | FMの配列（`V`、`a`、`mask`）を含むnpzを読む | codexの初版は未知のメンバーを拒否したため、読み飛ばす形に直した |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（39件） | 通過 |

## フェーズ2　GPUカーネルと等価性テスト

### フェーズ2a　GPU基盤、第1層の前進、出力部、縮約、検証の前進

| 検査 | 方法 | 結果 |
|---|---|---|
| GPUとCPU参照の等価性 | `cargo test -p nnue-trainer -- --ignored gpu_`（幅2、6、8、128の両方の型と、面の環の転送）と`check-equivalence`（幅6のバッチ100、幅128のバッチ4,096と16,384、両方の型）。判定は`|GPU−CPU| ≤ 10⁻⁶ + 10⁻⁵·|CPU|`、2回実行のビット単位一致 | すべて一致。幅128・バッチ16,384の最大絶対誤差はP型6.1×10⁻⁵、HalfKP-hm-b8で2.0×10⁻⁴（損失の部分和など大きな値で生じ、許容誤差の内側） |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（GPUなし） | 通過 |

`unsafe`ブロックは`gpu`モジュールに10か所（ライブラリの存在検査2、ページ固定メモリの確保と初期化2、カーネル起動6）と、`data`のmmapに1か所である。

### フェーズ2b　補助損失の前進、第1層と線形経路の後退、仮想行の縮約

| 検査 | 方法 | 結果 |
|---|---|---|
| GPUとCPU参照の等価性 | `cargo test -p nnue-trainer -- --ignored gpu_`（幅2、6、8、128の両方の型）と`check-equivalence`（幅6のバッチ100、幅128のバッチ16,384、両方の型）。補助損失の下限と上限の罰則（上限は符号を反転した整数PSTの参照で発火させる）、不適格な除去、回転、空の逆索引の範囲を含む。判定は`|GPU−CPU| ≤ 10⁻⁶ + 10⁻⁵·|CPU|`、2回実行のビット単位一致 | すべて一致。幅128・バッチ16,384の最大絶対誤差はP型6.1×10⁻⁵、HalfKP-hm-b8で2.0×10⁻⁴ |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（GPUなし、50件） | 通過 |

カーネル起動の`unsafe`は4か所増えて、`gpu`モジュールに合計14か所である。

### フェーズ2c　AdamWと切り詰め、畳み込み、1更新の統合

| 検査 | 方法 | 結果 |
|---|---|---|
| 1更新と連続3更新の等価性 | `cargo test -p nnue-trainer -- --ignored gpu_`（19件）と`check-equivalence`（幅128、バッチ16,384、両方の型）。更新後の全パラメータと最適化器の状態、損失を比較。判定は`|GPU−CPU| ≤ 10⁻⁶ + 10⁻⁵·|CPU|`、2回実行のビット単位一致 | すべて一致 |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（GPUなし、54件） | 通過 |

### 最初の速度測定（最適化前）

`bench-step --width 128 --batch 16384 --steps 50 --workers 6`を世代3の1ファイルで実行した（2026年10月12日4時ごろ。教師データの生成が同時12対局で並走し、負荷平均は約21）。

| 項目 | P型 | HalfKP-hm-b8 |
|---|---:|---:|
| 計算ストリームの1更新 | 150.4 ms | 141.3 ms |
| 転送ストリームの1バッチ | 2.7 ms（67 MB） | 2.7 ms（67 MB） |
| ワーカーの供給速度（6ワーカー） | 372,052局面/秒 | 388,047局面/秒 |
| 実効速度 | 105,676局面/秒 | 112,078局面/秒 |
| GPUメモリ | 575 MB | 877 MB |

GPUの1更新が律速であり、`nvidia-smi dmon`ではSM使用率100%、メモリ帯域使用率1〜8%だった。`--profile`によるカーネル別の内訳（P型）は、第1層の後退70.7 ms（47%）、線形経路の後退39.1 ms（26%）、補助損失の前進29.4 ms（19%）、出力部9.1 ms（6%）、第1層の前進1.3 msである。HalfKP-hm-b8では第1層の最適化器が26.4 msを加える。原因は、後退の2本が行ごとに1スレッドで逆索引の範囲を直列に走査して出現回数の多い行が尾を引くこと、補助損失と出力部でスレッド間の読み込みが合流しないこと、最適化器が要素ごとに倍精度の`pow`を計算することであり、いずれも並列化の形の問題である。

### フェーズ2d　カーネルの並列化（1回目）

第1層と線形経路の後退を固定長の区分に分けて2段で縮約し、補助損失は1ワープ1列、出力部は1ワープ1局面にし、AdamWの補正係数をホストで計算するようにした。等価性（`gpu_`テスト23件、`check-equivalence`の幅128・バッチ16,384、両方の型）は変更前と同じ判定で一致し、2回実行はビット単位で一致した。

| カーネル（P型、ms/更新） | 2d前 | 2d後 |
|---|---:|---:|
| first_layer_backward（2d後は部分和＋縮約） | 70.7 | 13.4 |
| linear_backward（同） | 39.1 | 0.4 |
| removal_forward | 29.4 | 45.1 |
| output_backward | 9.1 | 0.4 |
| forward_acc | 1.3 | 1.3 |
| 1更新の合計 | 150.4 | 61.6 |

HalfKP-hm-b8は141.3 msから66.3 msになり、`optimizer_first`は26.4 msから4.1 msになった。実効速度はP型248,557局面/秒、HalfKP-hm-b8で230,557局面/秒である。`removal_forward`が増えたのは、CPU参照の昇順の加算順を保つために32レーンの積をレーン0へ1つずつ送る直列のシャッフルにしたためであり、縮約順をGPUの固定の木に定義し直してCPU参照がそれに合わせる方式へ改めた（フェーズ2e）が、速度は変わらなかった（43.9 ms）。

### フェーズ2f　内側ループの単精度化

補助損失の最終段と後退の区分内の積を単精度にした。補助損失の144列の和と出力部の局面方向の部分和は、局面方向で大きな項が相殺するため、単精度では出力重みの勾配が許容誤差（相対5×10⁻⁵の差）を外れ、倍精度の累算に戻した。

| カーネル（P型、ms/更新） | 2d後 | 2f後 |
|---|---:|---:|
| removal_forward | 45.1 | 36.0 |
| first_layer_backward_partials | 13.4 | 3.7 |
| 1更新の合計 | 61.6 | 42.7 |

HalfKP-hm-b8の1更新は47.0 msである。等価性（`gpu_`テスト23件、`check-equivalence`の幅128・バッチ16,384、両方の型）は同じ判定で一致し、2回実行はビット単位で一致した。この時点の実効速度（6ワーカー、生成と同居）はP型317,260局面/秒、HalfKP-hm-b8で306,218局面/秒であり、ワーカーの供給（約41万局面/秒）がGPUの更新（約38万局面/秒）と並ぶ。生成の終了後にワーカー数を増やして測り直す。補助損失のカーネル（36 ms）が残る支配項であり、1局面あたり144列×2視点×幅の行の読み込みを2回行う構造に由来する。設計書の見積り（10〜20 ms）には届いていないが、1エポック約7,600万局面が約3〜4分になる見込みなので、学習ループの実装を先に進め、カーネルの追加調整は後に回した。

### 実装レビューの対応（フェーズ2g）

codexの実装レビューの指摘に従い、補助損失のカーネルで不適格な列（空升、王駒、参照の符号が0）の除去後の評価を計算しないようにし、最終段で勾配が0の列を読み飛ばすようにした。数値は変わらない（1ファイルの試走の損失はビット単位で同じ）。

| カーネル（ms/更新） | P型 2f後 | P型 2g後 | HalfKP-hm-b8 2g後 |
|---|---:|---:|---:|
| removal_forward | 36.0 | 20.5 | 20.5 |
| 1更新の合計 | 42.7 | 27.3 | 31.6 |

世代3の1ファイルの試走は、1エポック7.9秒（356,411局面/秒）、前処理から書き出しまで22.3秒になった。あわせて、検証の最終バッチの後にも時間の上限を判定する、量子化誤差の検査の浮動小数点評価を倍精度にする、左右非対称な初期PSTを拒否する、再開時の予算の変更を`config.json`の`resumes`に記録する、`Dataset::open`を`unsafe fn`にしてmmapの契約を明示する、参照実装の入力検査を加える、という修正を入れた。等価性（`gpu_`テスト28件、`check-equivalence`）は変更前と同じ判定で一致した。

## フェーズ3　学習ループと照合、1ファイルの試走

学習器`nnue-trainer train`で、世代3の1ファイル（`data/gen3/generated-1500000.bin`、訓練2,814,063局面、検証144,088局面）について、両方の型を幅128、バッチ16,384、先読み教師（γ=0.9、n=40）、シード1で1エポック学習した。Pythonの値は[学習の記録](nnue-replacement-training.md)の「学習器の作り直しの検査と試走」から引く。測定機では教師データの生成（同時12対局）が並走していた。

| 項目 | P型（Rust） | P型（Python） | HalfKP-hm-b8（Rust） | HalfKP-hm-b8（Python） |
|---|---:|---:|---:|---:|
| エポック0の検証損失 | 0.437095 | 0.437095 | 0.437095 | 0.437095 |
| 1エポック後の検証損失 | 0.436074 | 0.436078 | 0.436615 | 0.436605 |
| 1エポック後の符号一致率 | 0.7763 | 0.7759 | 0.7767 | 0.7765 |
| 学習速度（局面/秒、検証を除く） | 294,909 | 27,021 | 287,920 | 20,860 |
| 1エポックの学習時間 | 9.5秒 | 104秒 | 9.8秒 | 135秒 |
| 前処理から書き出しまでの経過時間 | 24.7秒 | ― | 24.7秒 | ― |
| GPUメモリ（使用量） | 0.88 GB | 2.33 GiB | 1.51 GB | 3.72 GiB |
| 量子化誤差の平均と最大（上限K/40＝26.8） | 6.76、34.6 | 6.15、35.8 | 6.09、35.1 | 6.67、35.3 |

エポック0の検証損失はPythonと一致し、1エポック後の差（第1層の乱数列が異なる）は10⁻⁵の桁である。

| 検査 | 方法 | 結果 |
|---|---|---|
| 書き出したMNUE版5とPythonの整数参照の一致 | 両方の型の`nnue.bin`を`minase dev nnue-probe`で試走の21,609局面について評価し、`nnue-diagnostics compare`でPythonの整数参照と照合 | 両方とも全局面一致 |
| 再開の一致 | P型の幅32、シード3で、連続2エポックの実行と、1エポックで止めて`--epochs 2 --threads 8 --resume`で再開した実行の`latest.bin`、`best.bin`、`nnue.bin`、`nnue-float.bin`を比較 | 検証損失と`nnue.bin`は一致。チェックポイントの差は累積時間（f64）と末尾のSHA-256だけで、重みと最適化器の状態は一致 |
| 再開時のハッシュ | `--epochs`を変えて再開 | codexの初版は予算（エポック数、改善なしの許容数、時間の上限）とワーカー数を設定ハッシュに含めていたため拒否した。これらは学習の軌跡を変えないので、ハッシュから外した |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（GPUなし89件、GPU28件） | 通過 |

## フェーズ3b　係数の引数化、教師の混合比、CPU実行

利用者の要望（2026年10月12日）で、Pythonの学習器と同じ引数（群ごとの学習率と重み減衰、補助損失の係数、`--lambda-override`）と`--device cpu`を加えた。

| 検査 | 方法 | 結果 |
|---|---|---|
| 既定値での不変 | 世代3の1ファイルでP型の幅128を1エポック学習し、損失を変更前と比較 | 訓練損失、補助損失、検証損失がビット単位で同じ |
| 係数とλを変えた等価性 | `check-equivalence`（幅128、バッチ4,096、両方の型、λ=0.75と係数の変更） | 一致 |
| 係数とλを変えたPython照合 | `dump-batch --init-pst-scale 0 --init-std 0.5 --width 8`の12局面を3バッチにし、λ=0.75と係数の変更で3更新をPyTorchと比較（`crosscheck_reference.py`が係数とλを読み込んで適用する） | 両方の型で一致（補助損失0.03〜0.06が発火） |
| CPU実行の決定性 | スレッド数1、2、5で損失、全勾配、3更新後の重みと最適化器の状態を比較（`cargo test`） | ビット単位で一致 |
| CPU実行の試走 | `--device cpu --threads 6`でP型の幅128を1エポック（生成が並走） | 検証損失0.436069（GPUは0.436074。CPU参照は倍精度）、学習6,639局面/秒、1エポック424秒、GPUの約50倍遅い |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（GPUなし109件、GPU30件） | 通過 |

CPU実行は倍精度の参照実装を並列化したもので、GPUのない環境での学習と検証の手段である。速度の目標は置いていない。

## フェーズ4　全データの試走

### 既存10ファイル（約2,920万局面）

新規20ファイルの生成が終わる前に、既存の世代2と世代3の10ファイル（訓練27,745,843局面、検証1,462,975局面）でHalfKP-hm-b8の幅128を1エポック学習した（2026年10月12日5時ごろ、生成が同時12対局で並走、ワーカー18）。

| 項目 | 値 |
|---|---:|
| 前処理（検証分割、先読み、教師用K、観測マスク） | 約124秒 |
| 1エポックの学習時間 | 94.3秒（294,256局面/秒） |
| 検証（146万局面） | 6.3秒 |
| 前処理から書き出しまでの経過時間 | 231.8秒 |
| 最大常駐メモリ（mmapのページを含む） | 6.65 GB |
| GPUメモリ | 1.51 GB |
| 検証損失（エポック0→1） | 0.432565→0.430935 |
| 量子化誤差の平均と最大 | 6.51、40.7センチポーン（上限26.8） |
| 教師用K（世代2、世代3） | 758.35、765.95 |

同じ速度なら、全30ファイル（訓練約7,600万局面）の1エポックは約4.3分、検証を含めて約4.6分であり、Pythonの47〜61分に対して約12倍速い。

### 全30ファイル

NNUEの設計書のフェーズ4として、全30ファイル（訓練82,712,114局面、検証4,354,965局面）で両方の型の幅128を学習した（2026年10月12日。1回目は`e4215f6`のバイナリ、2回目はチェックポイント形式を新しくした`47d9f2c`のバイナリで、書き出したネットは両回でバイト単位に一致した）。学習の内容は[学習の記録](nnue-replacement-training.md)の「フェーズ4の学習」にある。

| 項目 | P型 | HalfKP-hm-b8 |
|---|---:|---:|
| 前処理（検証分割、先読み、教師用K、観測マスク。単一スレッド） | 約450秒（2回目は並行するビルドとの競合で1,035秒） | 約450秒 |
| 1エポックの学習時間（局面/秒） | 145秒（約56万〜59万） | 155秒（約53万） |
| 検証（435万局面） | 約20秒 | 約20秒 |
| 6エポックと書き出しを含む経過時間 | 1,453秒 | 1,534秒 |
| GPUメモリ | 836 MiB | 1,444 MiB |

全30ファイルの1エポックはPythonの学習器の見込み（47〜61分）の約20分の1である。
前処理は単一スレッドで、学習全体の約3割を占めるので、並列化は後続の改善候補である。

本学習は、生成の完了を待って両方の型を順に学習する常駐スクリプト`data/nnue-replacement-train/phase4.sh`（2026年10月12日5時13分に起動、`nice -n 19`、ワーカー12、学習器のバイナリは、学習の開始前にレビュー対応後のコミット`e4215f6`のものへ差し替えて同じディレクトリに写し、SHA-256を`trainer-sha256.txt`に記録）で行う。進捗は`phase4.log`と`<型>/training.jsonl`にある。

学習後にNNUEの設計書のフェーズ4が要求する診断は次のとおりで、結果は[学習の記録](nnue-replacement-training.md)の「フェーズ4の学習」に書く。

```console
# 量子化誤差（学習器が自動で検査し、summary.jsonに書く）
# Pythonの整数参照との一致
PYTHONPATH=tools/train/src tools/train/.venv/bin/python -m minase_train.diagnostics.nnue compare \
  --net data/nnue-replacement-train/<型>/nnue.bin --positions data/nnue-replacement-trial/generated-2000000.bin \
  --probe-command target/release/minase dev nnue-probe --output <出力>
# 駒除去差分、互角帯の順位相関と評価値の分布、着手前後の評価差
PYTHONPATH=tools/train/src tools/train/.venv/bin/python -m minase_train.diagnostics.nnue removal ...
PYTHONPATH=tools/train/src tools/train/.venv/bin/python -m minase_train.diagnostics.nnue distribution ...
PYTHONPATH=tools/train/src tools/train/.venv/bin/python -m minase_train.diagnostics.nnue moves ...
# bench（深さ9、性能コアに固定、基準Mと交互に5巡）とHalfKP-hm-b8の全再計算の割合
taskset -c 3 target/release/minase dev bench --depth 9 --repetitions 3
```

`removal`、`distribution`、`moves`の引数は`tools/train/src/minase_train/diagnostics/nnue.py`の`main`に定義がある。フェーズ5の型の比較には、`ActiveFeatureSet`と`nets/nnue.bin`だけが異なる2つのコミットが要る。

## 隠れ層計画のフェーズ0　駒除去差分の補助損失の削除

[NNUEの隠れ層と位相補間の出力](../plans/nnue-hidden-layer.md)のフェーズ0で、学習器から駒除去差分の補助損失（CUDAカーネル`removal.cu`、CPU参照の`removal_forward`、`--removal-penalty`、`training.jsonl`の`removal_loss`と`total_loss`、チェックポイントの`removal_penalty`）を削除し、代わりに書き出し時の検査として、駒除去差分の符号反転の割合を`summary.json`の`removal_sign`に記録するようにした（2026年10月16日、ブランチ`nnue-hidden-layer`）。
検査は作業機（RTX 3060 Ti、CUDA 13.4、Rust 1.98.0）で行い、検査の間、同じ機械でlishogiのBot（4スレッド）が対局していた。

| 検査 | 方法 | 結果 |
|---|---|---|
| 静的検査とCPUのテスト | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p minase-trainer`（246件） | 通過。補助損失だけを検査していた4件（`removal_uses_lane_partials_then_descending_shuffle_distances`、`removal_p_adds_both_activations_before_multiplying_the_shared_weight`、`ineligible_removals_zero_diagnostics_and_preserve_reference_fields`、`removal_inputs_cover_lower_upper_inactive_and_all_ineligible_reasons`）を削除し、旧チェックポイントの拒否など4件を加えた |
| GPUとCPU参照の等価性 | `cargo test -p minase-trainer -- --ignored gpu_`（33件）と`check-equivalence`（両方の型、幅6のバッチ100と幅128のバッチ4,096）。数値微分、1更新と連続3更新、再開の一致、混合教師と係数変更を含む | すべて一致。幅128・バッチ4,096の最大絶対誤差はP型1.144×10⁻⁵、HalfKP-hm-b8で4.843×10⁻⁶（許容誤差`1e-6+1e-5×|CPU|`以内）、2回実行はビット単位で一致 |
| 旧チェックポイントの拒否 | `removal_penalty`スカラーを含むNPZを`decode` | 余分な配列として拒否（版数は増やしていない） |

1更新の時間とワーカーの供給速度は、`bench-step`（幅128、バッチ16,384、20更新、`--profile`つき）を再生成した教師データの1本目（`generated-7100000.bin`）で測った。`--profile`はカーネルごとの計測イベントを挿入するので1更新の時間をわずかに長く見せるが、削除前の値も同じ条件で測ったものである。
削除前の値は、本書の「フェーズ2e」の表（P型27.3 ms、HalfKP-hm-b8 31.6 ms）である。

| 項目 | P型 | HalfKP-hm-b8 |
|---|---:|---:|
| 1更新（計算ストリーム） | 4.7 ms | 8.6 ms |
| カーネル時間の内訳の上位 | 第1層の後退1.8 ms（39%）、第1層の前進1.4 ms（31%） | 第1層の最適化器4.1 ms（48%）、第1層の後退1.7 ms（20%） |
| ワーカーの供給速度（6ワーカー） | 659,725局面/秒 | 712,661局面/秒 |
| ワーカーの供給速度（12ワーカー） | 1,286,215局面/秒 | 1,304,419局面/秒 |
| 実効速度（6ワーカー / 12ワーカー） | 601,669 / 950,635局面/秒 | 616,423 / 833,889局面/秒 |

1更新の時間は設計書の見込み（P型約7 ms、HalfKP-hm-b8約11 ms）より短い。
ワーカーの供給は6から12でほぼ2倍に伸び、12ワーカーでは1バッチの転送（約2.7 ms）と更新（4.7 ms）の和が律速になる。
設計書が見込んだ1エポック（約8,000万局面）の時間は、12ワーカーの実効速度では約85秒である。

## 結論

（完了後に書く。）
