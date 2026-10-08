# minase-train

本ディレクトリは、minaseの評価関数のうちPST（駒の種類と位置に応じた評価表）を、自己対局の教師データから学習するPythonのツール群である。
エンジン本体はRustで書かれており、学習ツールとはファイルを介してだけやり取りする。
本体の`minase data selfplay`が自己対局の局面と探索値を書き出し、学習ツールがそれを読んでPSTを学習する。
学習した重みは、エンジンが読み込める重みファイル`crates/minase/nets/pst.bin`の形式で保存する。
学習の手順、設定項目、および採否の判断は [PSTの学習手順](../../docs/guides/pst-training.md) が定めており、本書は環境の構築、ファイル形式と記録の各欄、およびソースの構成を説明する。

## 環境の構築

依存関係は [uv](https://docs.astral.sh/uv/) で管理する。
Python 3.12以上、PyTorch、およびNumPyの版は `uv.lock` に固定されており、次のコマンドで `tools/train/.venv` に同じ環境を作れる。

```bash
uv sync --project tools/train
```

コマンドはすべてリポジトリのルートで実行する。
`uv run --project tools/train` は作業ディレクトリを変えないので、設定ファイルやデータのパスはリポジトリのルートから解釈される。
GPUで学習する場合は、この環境からCUDAを利用できる必要がある。

依存関係の版を上げる場合は、`uv lock --upgrade-package <パッケージ名>` で `uv.lock` を更新する。
学習結果の来歴（`environment.json`）にはPyTorchなどの版が記録されるので、版の更新は学習条件の変更として扱う。

## テストの実行

テストは標準ライブラリのunittestで書かれており、`tests/` にある。

```bash
uv run --project tools/train python -m unittest discover -s tools/train/tests
```

## コマンド

`pyproject.toml` は次の7つのコマンドを定義する。
いずれも `uv run --project tools/train <コマンド> --help` で引数を確認できる。

| コマンド | 用途 |
|---|---|
| `pst-workflow` | 学習の準備、自己対局の生成、学習、診断の4工程を、入力と出力の検査和を記録しながら順に実行する。通常の学習はこのコマンドだけを使う |
| `train-fm` | 固定PSTへのFMの学習、学習後の補正1/4への変換、および補正の倍率の重みファイルへの焼き込みを行う |
| `fm-diagnostics` | FMの整数参照評価とRustの評価を照合する |
| `train-pst` | 学習器を直接呼ぶ。初期重みの作成（`init`）、教師値の尺度Kの推定（`estimate-k`）、学習（`train`）を持つ |
| `pst-diagnostics` | 基準のPSTと候補のPSTを、局面帯別の損失、駒の除去、および成りで比較する |
| `taper-report` | 教師データの駒数分布と、序中盤と終盤の2つの重み（端点）を区別して学習できるかどうかを報告する |
| `lookahead-diag` | 先読み教師を使う前に、先読み値と探索値の差の分布と教師Kの変化を調べる |

たとえば学習工程の準備は次のように実行する。
設定ファイルの書き方は、[設定例](pst.example.toml) と学習手順の文書にある。

```bash
uv run --project tools/train pst-workflow prepare --config <実験名>.toml
```

学習器の`train`と`estimate-k`、および`pst-diagnostics`を`pst-workflow`を介さずに呼ぶ場合は、設定ファイルの項目を次の引数で指定する。
教師混合比の上書きは`--lambda-override 1.0`のように指定する。
先読みは`--lookahead-gamma 0.9 --lookahead-plies 40`のように両方を指定し、両方を省略した場合は先読みなしとし、片方だけの指定は拒否する。
付け直しファイルは`--rescore`に`--data`と同じ個数と順序でパスを渡し、付け直さないファイルには`-`を渡す。
`train`では追加損失の係数`--removal-penalty`が必須であり、`pst-workflow`は設定の`train.removal_penalty`をそのまま渡す。

`lookahead-diag`は、先読み教師を使う前の事前診断である。
基本の教師のMNSD、段階9の定義2による118列の追加特徴を収めたMNKF、追加特徴を含まない学習PST、および王駒の露出局面と対照局面からなる予備標本を渡す。
予備標本の定義は[先読み教師値の設計書](../../docs/plans/lookahead-teacher.md)にある。

```bash
uv run --project tools/train lookahead-diag \
  --data <MNSD...> --king-features <MNKF...> --pst <MNPT> \
  --output-k 1072.6529541015625 --gammas 0.9,0.7,0.95 --plies 40 \
  --exposed-sample <予備標本のJSON> \
  --output <JSON>
```

この診断は学習器と同じ訓練分割を使い、減衰係数γごとの先読み値と探索値の差、駒数帯ごとの差、その差が−300センチポーン以下となる割合、窓の記録数、および退避率を出す。
分布には平均、母標準偏差、および線形補間による5、25、50、75、95パーセンタイルを記録する。
最初の記録までの手数は、記録が1件以上ある窓だけで集計する。
γの一覧には0.9を含め、教師K、露出局面と対照局面の教師勝率差、およびMNKFの118列との残差相関はγ=0.9で比較する。
教師勝率はλ=0.75で混合し、残差は教師勝率からPSTの予測勝率を引いた値とする。
基本の教師に合わせて来歴のλも0.75を要求し、標本のファイル検査和、行番号、訓練分割への所属を照合する。
空の分布と分散0の相関は`null`とし、理由を記録する。

## 固定PSTへのFMの学習

`train-fm`は、採用PSTを固定したまま、2駒の位置関係を表すFactorization Machine（FM）の補正だけを学習する。
既存データの読み込み、教師の分類、対局単位の分割、先読み、教師Kの推定、および早期終了の判定はPST学習と共通である。
FMは生成工程もPSTの学習条件も必要としないため、起動には専用コマンドを使い、条件と入力の検査和を学習JSONへ保存する。

先読みなしの候補は、リポジトリのルートで次のように学習する。
入力MNSDには来歴ファイルが必要であり、教師Kは訓練集合だけから分類順に推定する。
出力Kは`--init`のMNPT v2から読み込み、PSTの両端点と探索用駒価値とともに固定する。

```bash
uv run --project tools/train train-fm train \
  --init crates/minase/nets/pst.bin \
  --data data/strength-stage7/gen2/generated-*.bin data/gen3/generated-*.bin \
  --output data/fm-fn/fm.bin --epochs 200 --patience 3 \
  --lambda-override 1.0 --device cuda
```

先読みつきの候補では、出力を`data/fm-fa/fm.bin`に変え、`--lookahead-gamma 0.9 --lookahead-plies 40`を加える。
先読み後の探索値を、教師Kの推定と訓練、検証のすべてに使う。
既定値は潜在次元32、AdamW、学習率0.001、重み減衰0.0001、補正の正則化係数0.0001、バッチ4,096、乱数シード1であり、上限エポック数とpatienceは必須である。
CPUの合成データで検証するときは`--device cpu`を使う。

出力は、補正を縮める前のMNPT v3である`fm.bin`、量子化前の埋め込み`V`と出力係数`a`と観測マスク`mask`を持つ`fm-float.npz`、および`fm.training.json`である。
エポック0は検証損失の記録だけに使い、最良の重みは学習したエポックから選ぶ。
最良値を厳密に更新しないエポックがpatienceだけ続いたら打ち切り、最良の重みを復元する。
エポック0より損失が高いことだけでは候補を除外しない。

`fm.training.json`は、入力と来歴と初期重みのSHA-256、訓練と検証の件数と添字のSHA-256、`teacher_classes`と同じ順の`teacher_ks`、教師Kの推定件数、`lambda_override`、`lookahead`、および全学習条件を保存する。
`epochs`には各エポックの二値交差エントロピー`train_loss`と`validation_loss`、正則化を加えた`total_loss`、補正のロジット分布`phi`を記録する。
`best_epoch`、`last_epoch`、`max_epochs`、`patience`、`improved_over_epoch_zero`、更新回数、特徴の観測回数、および最初の5更新の勾配も記録する。
`correction_cp`は同じ検証標本で測った整数補正の標準偏差を縮小前後について保存し、`quantization`はその標本における縮小前後の平均絶対誤差と最大誤差を保存する。
標本数の既定値は10,000であり、標本の乱数シードも記録する。
縮小前後のいずれかの平均絶対誤差が2センチポーンを超えた場合はエラーで停止する。

`quarter_zero_check`は全特徴対の整数補正が0かどうかと、最大145特徴の任意の和が整数補正0になる十分条件を別々に検査する。
各対の値が1未満でも、対を足してから切り捨てると非0になり得るため、対の検査だけで見送らない。
絶対値による上界で全局面の補正0を保証できた場合は、保存物を残して`status = "excluded_zero_correction"`とし、対局候補から除く。
この上界は十分条件なので、上界が1以上の場合に補正が非0の合法局面が存在するとは保証しない。

学習後、次のコマンドで補正1/4の候補を作る。
指数を1増やし、PST、出力K、探索用駒価値、埋め込み、および符号を保持する。
入力と出力のSHA-256、指数、および全0の検査結果を`fm-quarter.quarter.json`に保存する。
指数30の入力は、変換後の指数が形式の上限を超えるため拒否する。

```bash
uv run --project tools/train train-fm quarter \
  --input data/fm-fn/fm.bin --output data/fm-fn/fm-quarter.bin
```

FMの補正に掛ける倍率bを自己対局で決めた後は、`train-fm scale`でその倍率を重みファイルへ焼き込む。
`--fm-scale`には、bを1024倍した0から1024までの整数を指定する。
FMの補正は出力係数に比例するので、このコマンドは整数化前の出力係数`a`を`--fm-scale`/1024倍してから学習時と同じ整数化を行い、PST、探索用駒価値、および出力Kは入力のまま書く。
倍率0ではFMを含まないMNPTバージョン2を出力し、倍率1024では入力と同じバイト列を出力する。
書き込む前に、`--float`の両端点を整数化した値が入力の両端点と一致し、埋め込み`V`と出力係数`a`を整数化した値が入力のFMと一致することを確かめる。
これは、入力のMNPT v3と整数化前の重みが同じ学習の出力であることの検査であり、一致しなければエラーで停止する。
出力先または報告がすでに存在する場合も、エラーで停止する。

報告`<出力>.scale.json`には、入力、整数化前の重み、出力のそれぞれのSHA-256のほか、倍率、変換前後の指数、出力の埋め込みの最大絶対値、および出力の形式の版を保存する。
`--data`を指定すると、検証集合から乱数シード`--seed`（既定1）で`--validation-sample`（既定10,000）局面を選び、その標本での次の値を`validation`に加える。
`float_vs_integer`は、浮動小数点の評価と出力の重みによる整数評価の絶対誤差の平均と最大である。
`tuning_vs_baked`は、調整用ビルドの補正と出力の重みによる整数補正の差の絶対値の平均と最大である。
調整用ビルドの補正は、入力のFMの2駒の組の寄与を整数で足した分子Nと、入力の指数eによる分母D=2^(2e+1)から、0方向への切り捨てでtrunc(N·FmScale/(1024·D))として求める。
`correction_cp`は、出力の重みによる補正と調整用ビルドの補正のそれぞれの標準偏差である。
このコマンドは誤差の大きさを判定せず、記録だけを行う。

```bash
uv run --project tools/train train-fm scale \
  --input data/pst-fm-joint-training/j75/pst.bin \
  --float data/pst-fm-joint-training/j75/pst-float.npz \
  --fm-scale 512 --output data/fm-scale-spsa/pst-512.bin \
  --data <MNSD...>
```

Rust側の評価との照合には、検証したい局面を収めたMNSDと、FMに対応した実行ファイルを指定する。
`--probe-command`には`minase dev pst-probe`のほか、独立した`pst_probe`のパスも指定でき、どちらも`--pst`と`--positions`で呼び出す。
照合は入力順の全局面についてFM込みの`eval`と固定PSTの`eval_pst`を比較し、局面の欠落もエラーにする。
実行ファイルのビルドはこのコマンドでは行わない。

```bash
uv run --project tools/train fm-diagnostics \
  --base crates/minase/nets/pst.bin --candidate data/fm-fn/fm-quarter.bin \
  --positions data/fm-samples.bin --output data/fm-fn/rust-agreement.json \
  --probe-command target/release/minase dev pst-probe
```

PSTも更新した候補とRustの評価を照合するには、`fm-diagnostics`へ`--pst-changed`を付ける。
この指定では探索用駒価値とKだけを基準との不変条件にし、Rustの`eval_pst`を候補自身のPSTと比較する。
FMを含む`eval`の照合も同時に行う。

```bash
uv run --project tools/train fm-diagnostics \
  --base crates/minase/nets/pst.bin --candidate <候補のMNPT-v3> --pst-changed \
  --positions data/fm-samples.bin --output <照合結果のJSON> \
  --probe-command target/release/minase dev pst-probe
```

## ファイル形式

学習ツールが読み書きするファイルは、いずれも先頭4バイトの識別子を名前とする独自のバイナリ形式である。

| 形式 | 内容 | 書き出す側 | 読み込むモジュール |
|---|---|---|---|
| MNSD | 自己対局の局面、探索値、および対局結果 | `minase data selfplay generate`、`minase data lishogi` | `data/mnsd.py` |
| MNRS | MNSDの各局面に別の探索で付け直した教師値 | `minase data selfplay rescore` | `data/mnsd.py` |
| MNKF | MNSDの各局面に対応する追加の評価特徴の列 | 段階9の実験ブランチ（masterには書き出す側がない） | `data/mnsd.py` |
| MNPT | PSTの重み、探索用の駒価値、および出力の尺度K | `train-fm` | 固定PSTへのFMの学習と、学習後の補正1/4への変換を行う |
| `fm-diagnostics` | FMの整数参照評価とRustの評価を照合する |
| `train-pst` | `data/mnpt.py`、本体の評価関数 |

### 来歴と教師の分類

各MNSDには、パスの末尾に `.provenance.json` を付けた来歴ファイルが必須であり、`data/mnsd.py` が検査和とともに検証する。
来歴は`format = "minase-provenance"`、`version = 1`、元のMNSD全体のSHA-256である`mnsd_sha256`、`teacher`、`result_origin`、`start_origin`、`lambda`、`games`を持つ。
既存データにも明示的に用意し、欠落や検査和の不一致があれば読み込みを停止する。

教師の分類は、`teacher`内の`generation_commit`、`network_checksum`、`nodes`、`rule_set`、`search_condition`と、`result_origin`、`start_origin`の7項目で定まる。
`search_condition`は`"in-game"`または`"standalone"`、`result_origin`は`"selfplay"`または`"human"`、`start_origin`は`"random"`または`"human-game"`とする。
混合率は全体の`train.lambda`や`--lambda`では指定せず、各来歴の`lambda`に0以上1以下の有限値を記す。
[評価関数の設計書](../../docs/plans/evaluation.md#教師値と損失)のとおり、自己対局の分類は0.75、実戦棋譜の対局結果だけを使う分類は0とする。
同じ分類の実効混合比が一致しない入力は拒否する。
実効λが0の分類ではKを推定せず、教師値には対局結果だけを使う。

### 教師混合比の上書き

`train.lambda_override = 1.0`を指定すると、`result_origin = "selfplay"`の分類だけで探索側の割合を1にする。
実戦棋譜の分類のλ=0は変えず、来歴ファイルにも書き戻さない。
教師の分類には`lambda_override`を加え、適用しない分類では`null`、適用する分類では指定値を記録し、`lambda`には実効値を記録する。
先読みと併用した分類は、先読みの属性と上書きの値を両方持つ。
上書きの設定は準備記録、`training/inputs.json`、`<出力名>.training.json`、診断結果に保存し、学習、診断、再開時に照合する。
λ=1でも教師Kは訓練分割の対局結果から推定し、探索値または先読み値の勝率への換算に使う。

### 先読み教師値

先読みを使う場合は、同じ対局の将来の探索値を手数差の偶奇で現在の手番側へそろえ、幾何加重平均した実数を教師に使う。
計算式と記録が欠けた場合の正規化は[先読み教師値の設計書](../../docs/plans/lookahead-teacher.md#先読み値の計算)に従う。
窓に記録がない局面だけは元の探索値を使う。
入力は対局番号の非減少順、同一対局内では手数の厳密増加順を要求し、並びが崩れていれば拒否する。
先読み値はMNSDの整数欄へ書き戻さず、別の実数配列に保持し、教師Kの推定、訓練、検証損失、教師探索値との比較に共通して使う。

先読みを使う分類には、生成時の7項目に`lookahead_gamma`と`lookahead_plies`を加え、教師Kを訓練分割から推定し直す。
来歴ファイル自体は変更せず、先読みなしの分類ではこの2項目を`null`として記録する。
`train.lookahead`は準備記録、`training/inputs.json`、`<出力名>.training.json`、診断結果に保存し、学習、診断、再開時に照合する。
`train.rescore`に`"-"`以外が1つでもあれば先読みとの併用を拒否する。

### 検証分割

検証分割は由来に応じて生成シードと対局番号、または棋譜IDによって固定され、約5%の対局が検証用になる。
乱数開始の自己対局では来歴の`games`を`null`とし、検証の所属は`hash64(seed, game) % 20 == 0`で決める。
その他の由来では`games`を`{"game": 1, "id": "棋譜ID"}`形式の配列とし、実戦開始では各要素に開始手数`ply`も記す。
全記録の対局番号が`games`の配列に必要であり、配列にない番号があれば停止する。
棋譜IDのUTF-8バイト列のSHA-256の先頭8バイトをリトルエンディアンの整数として読み、20で割った余りが0のIDを検証に使う。
これにより、同じ棋譜の局面と、その棋譜から始めた自己対局は全ファイルを通して同じ側に入る。

### 付け直しファイル

付け直しファイルMNRSはMNSDごとに1つまで指定できる。
元のMNSDの検査和、記録数、対象一覧の検査和、および固定長240バイトのヘッダに16バイト×記録数を加えたファイル長を照合し、書きかけのファイルは拒否する。

状態1の記録は探索値を差し替え、教師の分類をMNRSの探索条件と元の来歴の由来から決め直す。
混合率は元の来歴から引き継ぐ。
差し替えた探索値の絶対値が29,000以上の記録、最善手が捕獲または成りの記録、および深さ1未完了の状態2の記録は、訓練と検証の両方から除く。
理由別件数は重複を含み、`rescore_exclusions.total`は重複を除いた件数として、標準出力と学習JSONに記録する。
元のMNSDを書き換えず、MNKFは除外後も元の行番号で参照する。

## 学習と診断の記録

本節は、`pst-workflow`の各工程が実行ディレクトリへ書き出す記録の欄を説明する。
判断に使う規則と停止時の対処は、学習手順の文書が定める。

### 端点の識別性の報告

`taper-report`は、`mirrored`では鏡映対の観測を正準特徴ごとに統合してから補間係数φの平均と偏差平方和を計算し、`features.csv`へ6,840個の特徴を記録する。
`tapered`では元の13,680特徴を別々に集計する。
`report.json`の`training_mean_phi`は、検証局面を除く全訓練局面に等しい重みを与えたφの平均である。

### 学習の記録

教師Kは、訓練集合の添字を明示して教師の分類ごとに推定し、λ=0の分類ではJSONで`null`とする。
モデル出力の尺度には`train.k`をそのまま渡し、λが0でない分類の混合データから推定したKは標準出力と`training/inputs.json`の`mixed_k`へ参考値として記録する。
全分類でλ=0の場合は`mixed_k`も`null`とする。
同ファイルの`k`には学習に使う指定値を記録し、`train.k`を省略した設定は準備時に拒否する。
分類ごとの尺度、訓練と検証の局面数、1エポックと全体の更新回数も `training/inputs.json` に記録する。
同ファイルの`options`には`removal_penalty`を含む全学習設定を保存する。
`total_steps`は`train.epochs`までの上限での総更新回数であり、実際の更新回数は`<出力名>.training.json`の`total_updates`で読む。
Python、PyTorch、CUDA、導入パッケージの版は `training/environment.json` に残す。

重みファイル `training/pst.bin` はMNPTバージョン2であり、序中盤用と終盤用の2組の重みに加え、基準から引き継いだ探索用駒価値47個を持つ。
静的評価は盤上総駒数で両端点を線形補間し、探索用駒価値は学習で変えない（[PSTの序中盤と終盤の補間](../../docs/plans/tapered-pst.md)）。
診断が量子化誤差を測れるよう、量子化前の重みを `training/pst-float.npz` に併置する。
量子化の丸め、範囲の検査、および重みの範囲射影は[評価関数の設計書](../../docs/plans/evaluation.md#段階1の学習pst)が定める。

検証損失は、由来、教師の分類、駒数帯、および王駒を2枚持つ側を含むかどうかのそれぞれで区分し、初期状態のエポック0から毎エポック記録する。
駒数帯は補間係数φの5等分を使い、空の区分の損失は`null`とする。
`game_half`は`hash64(seed, game) % 40 == 0`となる検証局面の平均である。
`human_game_mean`は、対局結果が実戦棋譜に由来する局面の損失を棋譜IDごとに平均し、その値を全IDについて等しい重みで平均したものである。
`sign_agreement`は引き分けを除き、評価値の符号と対局結果の一致率を3つの由来ごとに記録する。
評価値0は勝敗のいずれにも一致しないものとする。
これらは`<出力名>.training.json`の`validation`に入り、最良エポックは全検証局面の平均損失で選ぶ。

`train.log`の`train_loss`は教師値の二値交差エントロピー、`removal_loss`は係数を掛ける前の追加損失、`total_loss`は両者を指定係数で合成した学習目的の値である。
係数0では追加損失の計算を省略し、`removal_loss`を0、`total_loss`を`train_loss`と同じ値にする。
打ち切りで止めたときは学習ログの`best epoch:`行の直前に`early stop:`行を出し、`<出力名>.training.json`に指定値`patience`と最後に回したエポック`last_epoch`を記録する。

### 診断の記録

`diagnostics/report.json`の`bands` は、補間係数を5等分した局面帯と教師の分類の組ごとに、基準と候補を比較する。
比較する値は、訓練と検証の局面数、全検証局面の検証損失、および教師探索値との平均絶対誤差と相関である。
平均絶対誤差と相関は最大 `diagnose.sample_size` 局面の標本で計算し、平均絶対誤差は出力Kと教師Kの比で換算した値と生の値の両方を記録する。
λ=0の分類は教師探索値との比較から除外し、`teacher_comparison_excluded`へ除外件数を記録する。
検証損失、量子化誤差、およびRustとの評価値の照合にはその分類も含める。
出力JSONの`generation`と`generations`の軸は教師の分類を表し、`teacher_classes`に生成時の7項目、先読みのγと手数上限、`lambda_override`、および実効混合比`lambda`を記録する。
相関が定義できない場合は `null` と理由を出力し、空の帯は理由を記録して標本を作らない。
抽出した局面番号も帯ごとに保存するので、入力ファイルの一覧と合わせて標本を特定できる。
`quantization` は全帯の標本を合わせた量子化誤差である。
`rust_agreement` は、同じ標本を `minase dev pst-probe` で評価したRustの値がPythonの整数参照評価と全件一致したことを示す。

`outcome_metrics`は、基準と候補のそれぞれについて、全検証局面の評価値を各重みファイルの固定した出力Kで勝率へ換算し、対局結果への二値交差エントロピーを記録する。
結果は負け0、引き分け0.5、勝ち1とし、`bce_position_mean`は局面の平均、`bce_game_mean`は対局ごとの平均の平均とする。
対局単位は学習の分割と同じ識別子を使い、実戦棋譜のIDが複数ファイルに現れる場合も1対局として集計する。
`sign_agreement`は引き分けと評価値0を除いた符号一致率であり、`sign_records`と`sign_matches`に分母と一致件数を残す。
`sign_excluded`は除外局面の総数、`draw_records`と`zero_score_records`は重複を含む理由別の件数であり、対象がない場合の率は`null`とする。
これらは教師の混合比に依存しない診断値である。

`representatives` は、初期配置と各帯の標本のうち最小の通算番号を持つ代表局面について、王駒以外の各駒を1枚除いたときの評価の変化を基準と候補で並べる。
駒を除くと補間係数も変わるため、評価差をその駒固有の価値と同一視しない。
同じ代表局面の合法な成り手は `minase dev pst-probe` が実際に適用し、着手前の手番側視点の評価差を `promotions` に記録する。
成り手がない局面は `promotion_reason` にその旨を残す。
`after`の着手後局面からPythonで計算した評価差がRustの値と一致した成り手の件数を`rust_promotion_agreement`に記録する。

駒除去の`evaluations`と`delta_cp`はPST部分を表し、`total_evaluations`と`total_delta_cp`は評価全体を表す。
PST部分の厳密な符号反転は`pst_removal_sign_reversals`へ集計する。
評価全体の除去差分には、残った駒の利きが変わる効果が入るため、符号の保存を合格条件にしない。
除去局面は`removals.bin`へ保存してRustで評価し、`--skip-invalid`により復元不能のレコードを入力番号と理由つきで残す。
`rust_removal_agreement`は一致した件数と除外した件数をモデル別に記録する。

`derived_piece_values` は、各端点の全升平均から導出した駒価値と固定した探索用駒価値を並べ、端点ごとの静的な駒価値が固定値からどれだけ離れたかを示す。

## ソースの構成

ソースは `src/minase_train/` にあり、機能ごとに4つのサブパッケージへ分かれている。
依存の向きは、`workflow.py` と `diagnostics/` が `pst/` を、`fm/` が `pst/` を、学習器が `data/` を使う一方向であり、`data/` は学習の手法に依存しない。

`data/` は、学習データと重みファイルの読み書き、および学習器と診断が共有する前処理を持つ。

| ファイル | 内容 |
|---|---|
| `data/mnsd.py` | MNSD、MNRS、MNKF、および来歴を検証して読み込み、複数のファイルを1つの通し番号で扱う `Dataset` を提供する。訓練と検証の分割、教師の分類、教師値の付け直しの適用もここで行う |
| `data/mnpt.py` | MNPTの読み書き、駒価値の検証、初期重み、および実数の重みを16ビット整数へ丸める量子化を提供する |
| `data/features.py` | 盤上の駒を「駒状態×升」の特徴番号へ変換し、手番側から見た視点変換と左右の鏡映を行う |
| `data/taper.py` | 盤上の駒数から序中盤と終盤の重みを混ぜる補間係数を計算し、局面を駒数帯へ分ける |
| `data/lookahead.py` | 同じ対局の将来の探索値を、手数差による幾何加重平均で先読み教師値へ変換する |

`pst/` は、PSTの学習そのものを持つ。

| ファイル | 内容 |
|---|---|
| `pst/model.py` | 線形PSTのモデル（単一、2端点、左右鏡映で重みを共有する2端点の3種類）と、評価値を勝率へ換算するロジットを計算する |
| `pst/teacher.py` | 探索値と対局結果を混ぜて教師の勝率を作り、探索値を勝率へ換算する尺度Kを推定する |
| `pst/removal.py` | 駒を1枚取り除いたときの評価差が、初期の重みと同じ符号になることを促す追加の損失を計算する |
| `pst/evaluate.py` | 整数と実数の重みで局面を評価する参照実装であり、量子化誤差の検査とRust側との一致確認に使う |
| `pst/train.py` | 検証損失、学習ループ、および `train-pst` のコマンドラインを持つ |

`fm/train.py`は、FMのモデル、学習、整数参照評価、および補正1/4への変換を持つ。

`diagnostics/` は、学習済みの重みや教師データを調べる報告を持つ。

| ファイル | 内容 |
|---|---|
| `diagnostics/fm.py` | `fm-diagnostics`の本体。MNPT v3の評価をRustと照合する |
| `diagnostics/comparison.py` | `pst-diagnostics` の本体。本体の `minase dev pst-probe` を呼び、Rustの評価とPythonの参照評価の一致も確認する |
| `diagnostics/taper_report.py` | `taper-report` の本体 |
| `diagnostics/lookahead_teacher.py` | `lookahead-diag` の本体 |

残る2つのファイルは、パッケージ全体から使われる。

| ファイル | 内容 |
|---|---|
| `workflow.py` | `pst-workflow` の本体。基準コミットのworktreeで `minase data selfplay` と `minase dev pst-probe` をビルドし、生成、学習、診断の各工程の入力と出力を検査和で照合する。準備の時点で本パッケージのソースの検査和も記録し、ソースが変わった実行ディレクトリの続行を拒否する |
| `checksum.py` | ファイル全体のSHA-256を計算する |

## テストの構成

`tests/` のテストファイルは、検査する主な関数の定義元モジュールごとに `test_<モジュール名>.py` と名付けている。
たとえば `data/mnsd.py` のテストは `tests/test_mnsd.py` に、`workflow.py` のテストは `tests/test_workflow.py` にある。
複数のテストファイルが使う人工データの作成と、実装から独立した参照計算は `tests/helpers.py` にまとめている。
