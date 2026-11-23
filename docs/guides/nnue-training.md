# NNUEの学習と診断の手順

本書は、中将棋エンジンminaseの評価関数NNUE（差分更新可能なニューラルネットワーク）を、学習器`minase-trainer`で学習、検査、診断するための標準レシピ、手順、オプションを定める。
学習器は、将棋のNNUEの学習器tataraに倣い、ネットの構造ごとにサブコマンドを分けている。
`nnue train layerstack`は、王の位置を使わない入力特徴集合P型を、第1層の出力どうしの積と2段の隠れ層を持つtataraのLayerStackで学ぶ。
`nnue train simple`は、自玉の升ごとに別の重みを引く入力特徴集合HalfKP_vmを、両視点の第1層の出力を連結して隠れ層へ通すtataraのSimpleで学ぶ。
2つのサブコマンドは同じ引数をとり、学習したネットをtataraの形式で書き出す。
ネットの構造と計算式は[NNUEの評価関数の手引き](evaluation-nnue.md)に、設計判断の記録（ADR、architecture decision record）は`docs/adr/`にある。

| したいこと | 読む場所 |
|---|---|
| 学習を実行する | [標準レシピ](#標準レシピ)と[手順](#手順) |
| vast.aiで教師ファイルを生成する | [vast.aiで教師ファイルを生成する](#vastaiで教師ファイルを生成する) |
| 世代反復の世代を続けて作る | [連続生成スクリプトで世代を作る](#連続生成スクリプトで世代を作る) |
| 中断した学習を再開する | [中断した学習を再開する](#中断した学習を再開する) |
| メモリに収まらない教師データで学習する | [メモリに収まらない教師データで学習する](#メモリに収まらない教師データで学習する) |
| 学習器のコードを変更した | [学習器を変更したとき](#学習器を変更したとき) |
| 引数を調べる | [オプション一覧](#オプション一覧) |
| 標準レシピの根拠を調べる | [標準レシピの設定と根拠](#標準レシピの設定と根拠) |
| 計算式、保存物の項目、方式の理由を調べる | [詳細の置き場所](#詳細の置き場所) |

**前提**

- 学習を始める前に、[設計書の型](../plans/README.md)に従って目的と基準コミットを定め、[ROADMAP](../ROADMAP.md)に状態を登録する。
- コマンドは、明記した箇所を除いてリポジトリのルートで実行する。
- `minase-trainer`は、`cargo build --release -p minase-trainer`でビルドした`target/release/minase-trainer`を指す。

## 標準レシピ

標準レシピv3では、教師を対局単位で訓練用と検証用に分け、訓練に使わなかった対局で教師の探索値を最もよく予測した重みを棋力測定に送る。
P型を同じ教師ファイル30本で学習したところ、2.83エポック以降は訓練損失が下がり続けても検証損失は悪化した。8.02エポックのモデルは2.05エポックのモデルより、共通の相手と開始局面で測った得点率も低かった。エポック数は教師を替えるたびに選び直す。[測定記録](../measurements/adhoc/tatara-p-generalization.md)

学ぶネットは、P型の入力を受けるLayerStackに、駒と升ごとの点数を足す表PSQT（piece-square table）を並べた構造である。
第1層の幅は256であり、LayerStackの後段とPSQTはどちらも、局面の進行度などに応じて重みの組を切り替える区分を1つだけ持つ。
構造を選んだ理由は[構造のADR](../adr/compose-p-as-a-tatara-layerstack-with-a-psqt-and-halfkp-vm-as-a-simple-network.md)にある。

1エポックは、訓練集合をひととおり学ぶ期間である。superbatchは、指定した回数のバッチ更新をまとめる単位である。
検証は約0.5エポックごとに行う。更新後の最初の検証から最小損失を記録し、2回続けて最小値を更新しなければ学習を止める。同率なら先の重みを選ぶ。
選択に使うのは量子化前の勝率の平均二乗誤差であり、選んだ重みを整数化した後、別の開始局面を使った対局で棋力を判定する。
前の世代のネットで教師を生成して次の世代のネットを学習する世代反復では、`scripts/vast/nnue/generation_chain.py`がこの手順を続けて実行する（[連続生成スクリプトで世代を作る](#連続生成スクリプトで世代を作る)）。

**コマンド**

```sh
# 学習
minase-trainer nnue train layerstack \
  --data <教師...> --rules lishogi --teacher-cp-per-logit <K_t> \
  --output-dir <新規または空のディレクトリ> \
  --lambda 1.0 \
  --batch 16384 --batches-per-superbatch <B> --superbatches <S> \
  --lr-schedule cosine:<T>:0.027 --validation-superbatches 1 --validation-patience 2 \
  --seed 1 --device cuda --threads 16

# 診断
minase-trainer diag nnue-removal \
  --float <学習の出力>/best.npz --net <学習の出力>/nnue.bin \
  --data <教師...> --sample 10000 --seed 1 --output <新規の報告.json>
```

- `--data`には、その世代の教師だけを指定する。第n世代の教師は、1億局面に相当する本数k（見込みは37）のn倍の教師ファイルである。
- `--rules`には、教師の生成器と同じ規則セット`lishogi`（対局サイトLishogiに近い対局条件の規則コードの組）を渡す。学習器は、全ての教師ファイルのヘッダの規則セットがこれと一致することを確かめ、一致しなければ学習を始めずに止まる。
- `--teacher-cp-per-logit`には、教師の探索値を勝率へ換算する尺度K_tを数値で渡す。単位は1ロジットあたりのcp（センチポーン、評価値の単位で歩兵1枚の100分の1）である。K_tは第1世代の教師の最初の4本から一度だけ求め、全世代に同じ値を渡す。求め方は[学習データと初期値を用意する](#学習データと初期値を用意する)にある。
- 訓練局面数をM、1エポックの更新回数をU＝ceil(M / 16,384)とし、検証間隔にはB＝ceil(U / 2)更新を使う。間引く場合のMには抽出後の局面数を用いる。ceilは整数への切り上げを表す。Mは同じ教師の `config.json` にある `preprocessing.training_positions`、間引いた場合は `preprocessing.train_subsample.training_positions_after` で確認する。初めて使う教師では約5%の対局を検証に充てるため、生成時の適格局面数の95%に訓練局面の抽出率を掛けて見積もり、学習記録の `epochs` で実際の間隔を確認する。
- 学習率の減衰期間は、従来と同じ更新回数（第1〜4世代は390,656回、第5世代は488,320回）に揃える。その回数をBで割った値がTであり、S＝ceil(T)を `--superbatches` の上限とする。例えば訓練79,458,082局面ならU＝4,850、B＝2,425となり、第1世代ではT≃161.095、S＝162となる。
- 更新に使える局面数が限られる場合は、すべての棋譜から局面を抽出する。例えば `--train-subsample positions:4:0` は各訓練局面を約1/4の割合で採り、検証対局をすべて残す。追加で生成する教師は異なるシードの対局から集める。全局面を使える場合に間引くことの棋力上の利点は未確認である。
- `--threads`は教師のバッチを組むスレッドの数であり、標準は16とする。基準とした作業機（RTX 3060 Ti、Core Ultra 7 265KFの20コア）では、4スレッドではバッチの供給が律速で毎秒約70万局面にとどまり、16スレッドで毎秒約106万局面になる。物理コア数が18に満たない機械では、物理コア数から2を引いた数を使う。GPUかCPUがこの作業機より強力な機械では、[学習する](#学習する)の「スレッド数を決める」の手順で16以上のスレッド数から選ぶ。RTX 5090を載せたvast.aiの生成機の実績は32である。
- 学習で出力された`nnue.bin`については、ファイル全体のSHA-256を求め、NASの`/mnt/synology_nas/minase/nnue/nets/`に`<SHA-256の16進数>.bin`の名前で置く。エンジンには、オプション`EvalFile`でこのファイルを渡す。gitにはコミットしない。
- 上のコマンド例では、診断は駒除去差分だけを示した。採否の測定の前に、[残りの2つの診断](#学習した重みを診断する)も行う。

HalfKP_vmのネットは、`nnue train layerstack`を`nnue train simple`に替え、同じ引数で学ぶ。
HalfKP_vmは、自陣の王将の升（王将がなければ太子の升）に応じて第1層の行を選び、両陣営の王将を入力の駒から除いて、太子は駒として含める。
Simpleは、両視点の幅256の第1層の出力を連結した512成分を、幅32、幅32、出力1の層へ通す構造（256x2-32-32）である。
学習器は、tataraの特徴分解と同じく、学習の間だけ王の位置によらない13,392行の仮想行を第1層に足し、書き出しのときに実の行へ畳み込む。
HalfKP_vmの第1層は964,224行あり、幅256ではファイルの中で約494 MBを占める。
HalfKP_vmをバッチ16,384局面で学ぶと、学習器がGPUに確保する配列が約3.94 GiBになり、RTX 3060 Tiで測ったデバイスの使用量の増分は4.26 GBだった。

**止める条件**

| 段階 | 次のときに中止する |
|---|---|
| 学習 | `summary.json`の`status`が`candidate`でない。 |
| 診断 | 終了コードが0以外である。 |
| NASへ置いた後 | エンジンに`setoption name RuleSet value lishogi`、`setoption name EvalFile value <NASのネット>`、`isready`を順に送った後の、`bench depth 5`が完走しない。`bench`は、将棋のエンジンとGUIの通信規約USIにminaseが足した独自のコマンドである。 |

## 手順

次の順に進める。

1. [学習データと初期値を用意する](#学習データと初期値を用意する)
2. [試走する](#試走する)
3. [学習する](#学習する)
4. [結果を確認する](#結果を確認する)
5. [学習した重みを診断する](#学習した重みを診断する)
6. [採否を測定して記録する](#採否を測定して記録する)

### 学習データと初期値を用意する

**学習データ**

- 教師データの形式と生成の手順は、NNUEより前の評価関数であるPSTとFMの学習器と共通である。PSTは、NNUEのPSQTと同じく駒と升ごとの点数の表だけからなる評価関数であり、FMは2駒の関係を補う因子分解機である。
- [PSTとFMの学習手順](pst-training.md)の2節と3節に従って、minaseの教師データの形式MNSDのファイルを生成し、`--data`に入力順で指定する。教師の分類（後述）の番号は初出順に付く。
- 世代反復の教師ファイルをvast.aiで生成するときは、[vast.aiで教師ファイルを生成する](#vastaiで教師ファイルを生成する)に従う。
- MNSDは対局の手順を記録しているので、学習器は開始時に全ファイルの手順を再生して局面を復元する。この時間は`config.json`の`preprocessing_seconds`に残る。

**初期値**

- 学習器は前の世代のネットを読まず、重みを毎回`--seed`の乱数から初期化する。
- LayerStackのPSQTはP型の入力特徴ごとに1行を持ち、手書きの評価関数v0の駒割を、出力の換算の尺度K_o（後述）で割った値から始める。そのうち王駒の行と、先獅子の対象の升を表す行は0とする。王駒は王将と太子であり、先獅子は獅子の取り合いを制限する中将棋の規則である。その他の初期化の分布は[引数で変えられない固定値](#引数で変えられない固定値)にある。
- 第1世代の教師は、v0を表したネット（以下、v0のネット）を`EvalFile`で渡したエンジンで生成する。v0のネットは次のコマンドで書き出す。

```sh
minase-trainer nnue export-handcrafted --output <新規のパス>
```

- v0のネットはP型のLayerStackであり、第1層と後段の重みとバイアスがすべて0で、PSQTにv0の駒割と位置評価を持つ。後段の寄与がないので、このネットを`EvalFile`で渡すと、探索はv0の評価で動く。
- 書き出しには勝率尺度の引数がなく、整数の出力をcpへ換算する除数`fv_scale`は1に固定する。K_tはv0のネットが生成した教師から求めるので書き出しの時点では決まっておらず、後段を持たないこのネットは除数を1にしてもv0の評価値を正確に再現するからである。
- コマンドは既存のパスを拒否し、書き出した後に読み直して検査してから、パス、`fv_scale`、およびファイル全体のSHA-256を1行ずつ標準出力へ書く。

**教師値**

- `--lambda`は、教師値のうち探索値が占める割合λであり、残りを対局結果が占める。自己対局を使う標準レシピでは1.0とする。終盤の比較実験では残り25枚以下だけλ＝0.75にして追加学習した。`--lambda 0.75` を指定すると全局面に適用される。[終盤の測定](../measurements/adhoc/tatara-p-generalization.md)
- λは自己対局のデータにだけ適用され、実戦対局のデータでは常に0である。
- 標準の学習は、同じ対局の先の局面の探索値を割引率で混ぜる先読み教師を使わない。比較で使うときは`--lookahead-gamma`と`--lookahead-plies`を同時に指定する。

**教師の勝率尺度（`--teacher-cp-per-logit`）**

- `--teacher-cp-per-logit`は必須の引数であり、K_tを正の有限な数で渡す。世代反復では、K_tを第1世代の教師の最初の4本から次のコマンドで一度だけ求め、全世代の学習に同じ値を渡す。

```sh
minase-trainer data calibrate --data <第1世代の番号0〜3の教師> --output <新規の較正.json>
```

- 出力のJSONの`classes`には、教師の分類ごとに1項目が並ぶ。分類とは、生成コミット、生成に使ったネットのSHA-256、探索の規模、規則セット、由来、および先読みの条件がすべて同じ教師の組である。
- 第1世代の4本は同じ条件で生成するので、`classes`は1項目になる。その項目の`teacher_cp_per_logit`の値を、`--teacher-cp-per-logit`に渡す。
- 同じ項目の`at_search_bound`が`true`なら、推定値がK_tを推定する範囲（50〜4,000 cp）の端に達しているので、その値を使わない。連続生成スクリプトも、この場合は止まる。
- 較正は、学習と同じ検証分割で訓練集合を選ぶ。先読み教師を使う比較では、先読み教師の引数も学習と揃える。
- 学習器は、K_tから、ネットの整数の出力をcpへ換算する除数m＝round(8,128÷K_t)と、出力の換算の尺度K_o＝8,128÷mを求め、損失を`[σ(K_o y / K_t) − σ(s / K_t)]²`とする（[ADR](../adr/separate-the-nnue-teacher-win-rate-scale-from-the-output-scale.md)）。yはネットの実数の出力、sは教師の探索値、σはシグモイド関数である。8,128は、後段の出力の量子化の尺度127×64である。例えば、K_t＝679.03 cpならm＝12、K_o＝677.33 cpとなる。
- mは、書き出すネットの構造記述に`fv_scale`として書く。mが1以上の32ビットの整数にならないK_tは、学習を始める前に拒否する。例えば、K_tが16,256 cpを超えるとmが0になり、換算を定義できない。
- 使った値は、`config.json`の`preprocessing`の`teacher_cp_per_logit`と、`architecture`の`K_t`、`m`、`K_o`に残る。
- 訓練データの量や間引き方を変える比較でも、すべての学習に同じK_tを渡せば、教師値が同じに保たれる。

### 試走する

本番の前に教師1ファイルを使った短い学習で試走し、前処理、更新、検証、保存が最後まで通ることを確かめる。
`--batches-per-superbatch`を小さくすると、1 superbatchの更新回数を減らして短く試せる。

```sh
for STRUCTURE in layerstack simple; do
  minase-trainer nnue train "$STRUCTURE" \
    --data <1ファイル> --rules <教師の規則セット> --teacher-cp-per-logit <K_t> \
    --lambda 1.0 \
    --batch 16384 --superbatches 1 --batches-per-superbatch 100 \
    --lr-schedule cosine:1:0.027 --validation-superbatches 1 --seed 1 \
    --output-dir "/tmp/nnue-trial-${STRUCTURE}"
done
```

- `--rules`には、教師ファイルのヘッダと同じ規則セットの名前（例えば`lishogi`）か、規則コードの列を渡す。
- GPUのない機械やCPUの参照実装を試すときは、`--device cpu --threads 2`を加える。

### 学習する

- 本番では標準レシピのコマンドに全入力を並べ、`--time-limit-hours`で累積時間の上限を決める。
- 測定機を他の作業と共用するときは、`--threads`を明示して他の作業の分のコアを残す。

**スレッド数を決める**

学習の速さは、教師からバッチを組んで供給する処理とGPUでの更新のうち遅いほうで決まり、供給はスレッド数にほぼ比例して速くなる。
GPUかCPUが基準の作業機より強力な機械では、本番の前にスレッド数を変えながら次の短い学習を走らせ、学習の速さを比べる。
`--threads`には、16から8ずつ増やした値を、物理コア数から2を引いた数を上限として順に渡す。

```sh
minase-trainer nnue train layerstack \
  --data <教師4ファイル> --rules <教師の規則セット> --teacher-cp-per-logit <K_t> \
  --lambda 1.0 --batch 16384 --superbatches 1 --batches-per-superbatch 2000 \
  --lr-schedule cosine:1:0.027 --validation-superbatches 1 --seed 1 --device cuda \
  --threads <N> --output-dir <新規の出力先>
```

- 出力先の`training.jsonl`から、`superbatch`が1の行にある`training_positions_per_second`を読み、その最大値の97%以上になる最も小さいスレッド数を本番に使う。基準の作業機では1回が約1分（前処理15秒、学習31秒）で、16スレッドで毎秒1,055,979局面だった。
- 測る間は、同じ機械のBotや対局の測定を止める（[教訓](../lessons/compare-trainer-speed-with-co-resident-load-stopped-and-gpu-bound-threads.md)）。

**学習の長さと書き出すネット**

- `--superbatches`×`--batches-per-superbatch`が更新回数の上限である。標準レシピでは検証損失が改善しなくなれば上限より早く止まる。
- エポック末の短いバッチも1更新と数える。superbatchの境界はエポックの途中に来てもよい。
- 検証は学習前、指定した間隔ごと、および予定した最後のsuperbatchで行う。更新0回の測定は初期値との比較に使い、候補は更新後の重みから選ぶ。

**主な調整用オプション**（値域と式は[オプション一覧](#オプション一覧)）

- 学習率は `--lr-schedule` で指定する。余弦の総長Tと指数減衰の半減期hの単位はsuperbatchである。途中で止まっても減衰期間を縮めない。
- `--validation-superbatches`: 検証1回の時間は教師の本数に比例し、37ファイルの教師で約30秒、185ファイルの教師で約2.7分かかる見込みである。

### 結果を確認する

**保存物**

| ファイル | 内容 |
|---|---|
| `nnue.bin` | 選んだチェックポイントを量子化した、エンジンが読むtatara形式の重み。 |
| `latest.npz` | 最後に確定した重みと最適化器の状態。中断からの再開に使う。 |
| `best.npz` | `--validation-patience`を指定したときに保存する、検証損失が最小になった重みと最適化器の状態。駒除去差分の診断に使う。 |
| `summary.json` | 状態、停止理由、実行した更新回数、選択時点と停止時点の検証損失、量子化誤差、駒除去差分の符号反転の割合 |
| `training.jsonl` | 学習前の検証と、superbatchごとの学習曲線（1行目は更新0回の検証） |
| `config.json` | 引数、構造とK_t、m、K_o（`architecture`）、入力、前処理の結果、再開の履歴 |

**確かめること**

- `summary.json`の`status`が`candidate`であることを確かめる。`selected_superbatch`が指す`training.jsonl`の行から採用候補のエポック数を読み、`selected_validation_loss`と量子化誤差を記録する。`completed_steps`は停止までの更新回数であり、検証で早く止まると`planned_steps`より小さくなる。
  - `quantization`は、最大`--validation-sample`個の検証局面で、実数の評価と書き出したネットの整数の評価を比べた結果である。差の平均絶対値`mean_absolute_cp`、符号つきの平均`mean_signed_cp`、最大値`max_absolute_cp`、および層ごとの飽和の割合が入る。飽和の割合は、活性化の前の値が0以下か1以上になった成分の割合である。
  - 飽和の割合の欄は、LayerStackでは`ft_saturation`、`l1_square_saturation`、`l1_linear_saturation`、`l2_saturation`であり、Simpleでは`ft_saturation`、`l1_saturation`、`l2_saturation`である。
  - 量子化誤差に合否の閾値はなく、棋力は対局で判断する。誤差が大きい原因を調べるときは、整数化の段を1つずつ適用して、誤差の大きい段を特定する（[教訓](../lessons/check-quantization-scale-against-the-win-rate-scale.md)）。
- `summary.json`の`removal_sign`には、同じ検証局面から王駒以外の盤上の駒を1枚ずつ除き、書き出したネットで評価し直した差の符号が、v0の駒価値の差と逆になった除去の割合が入る（[ADR](../adr/drop-the-piece-removal-auxiliary-loss-from-the-nnue-trainer.md)）。
  - 欄は、局面数`positions`、除去の数`removals`、評価を定義できない除去の数`terminal_removal_count`、および反転の集計`reversals`（件数`count`、対象の数`total`、割合`fraction`、差の平均絶対値`mean_absolute_cp`、最大値`max_absolute_cp`）である。
  - Simpleでは王将は入力にないので除去せず、王将のない側の最後の太子を除く除去は評価を定義できないので`terminal_removal_count`に数えて集計から除く。
  - この記録にも閾値はなく、`status`に影響しない。候補が対局で負けた場合の原因の手がかりにする。
- `training.jsonl`: 更新の不足や過学習がないかを点検する。学習前の行を除くすべての行に訓練損失と学習率の倍率があり、エポック数はすべての行にある。検証損失は検証を行った行にあり、検証を行わなかった行では`null`になる。
  - 構造に固有の統計は`statistics`にある。LayerStackでは、PSQTの初期値からの移動量`psqt_movement`がすべての行にある。検証を行った行には、引き分けを除く局面でネットの出力の符号が勝敗と一致した割合`sign_accuracy`、PSQTを除いた後段の出力の標準偏差`nonlinear_std_cp`、および層ごとの飽和の割合もある。Simpleでは、層ごとの飽和の割合だけが検証を行った行にある。
  - GPUメモリの欄はGPU全体の使用量であり、他のプロセスの確保も含む。
- 損失は勝率の差の2乗なので、二値交差エントロピーで学んだ過去の学習の損失とは値を比べられない。

**失敗と時間切れの扱い**

- `summary.json`の`status`が`candidate`のときだけ終了コード0で終わり、次の場合は終了コード1で終わる。
  - 引数、入力、または再開の条件を満たさない（学習を始める前）。教師ファイルの規則セットが`--rules`と異なる場合も、これにあたる。
  - 予定の更新を終える前に時間の上限に達した（`status`は`incomplete`）。`nnue.bin`は書き出さず、最後に確定した`latest.npz`を残すので、[再開](#中断した学習を再開する)の手順で続きを学習する。時間切れの実行は候補として使わない。
  - 学習の途中か書き出しで処理を続けられなくなった（`status`は`error`）。`summary.json`の`reason`に原因が入る。
- 書き出しの失敗には、量子化した値が格納先の整数型の範囲を超えた場合と、書き出したネットがエンジンの読み込みの検査に通らなかった場合がある。学習器は範囲を超えた値を切り詰めずに止まるので、`reason`に示された配列の名前と要素の番号から原因を調べる。

### 学習した重みを診断する

採否の測定の前に、次の3つの診断を行う。

- いずれもCPUで動き、`--output`に指定した新規のJSONへ報告を書く。`--output`が既にあれば、何も計算せずに終わる。
- 入力や統計が条件を満たさなければ、報告を書かずに終了コード1で終わる。計算どうしの照合に失敗したときだけは、`error`を含む報告を保存してから終了コード1で終わる。
- 3つの診断は、LayerStackとSimpleのどちらのネットにも使える。
- 報告の各項目の定義は`crates/minase-trainer/src/diagnostics/`にある。診断を設けた経緯は[NNUEの完全代替](../plans/archived/nnue-replacement.md)にある。

評価値の分布と合法手の評価差は、NNUEより前の評価関数PSTとFMを合わせたPST＋FMを基準にする。
PST＋FMの重みのファイルの形式をMNPTと呼ぶ。

#### 評価値の分布

基準と候補を同じ標本で評価し、散らばりと相関を比べる。

```sh
minase-trainer diag nnue-distribution \
  --pair <比較名> <基準のMNPT> <候補のnnue.bin> \
  --data <データ...> --sample <標本数> --seed <シード> \
  --output <報告.json>
```

- 基準には次元32のFMを持つMNPT版3を、候補にはtatara形式のネットを指定する。
- 標本は全局面から抽出する。`--sample`は2以上で全局面数以下とする。
- `--pair`は複数回指定でき、すべての組を同じ標本で比べる。
- 報告の`pairs`の`verdict`で、標準偏差の比が0.9以上1.1以下に収まるかをまず確かめる。
- 収まらないときは、盤上の駒数で分けた`by_phase_q`と、基準の評価値の絶対値で分けた`by_baseline_absolute_cp`で、どの帯で散らばりが変わったかを確かめる。

#### 駒除去差分

検証集合の局面から駒を1枚ずつ除いたときの評価の変化を、量子化の前後で比べる。
除く駒は、LayerStackでは王駒以外の駒であり、Simpleでは王将以外の駒である。

```sh
minase-trainer diag nnue-removal \
  --float <出力ディレクトリ>/best.npz --net <出力ディレクトリ>/nnue.bin \
  --data <データ...> --sample <標本数> --seed <シード> \
  --output <報告.json>
```

- `--sample`は1以上で検証集合の局面数以下とする。
- `--float`と`--net`には、同じ学習で出力されたファイルの組を指定する。標準レシピでは`best.npz`が`nnue.bin`の量子化前のモデルである。固定回数で学習した場合は`latest.npz`を渡す。2つのファイルの構造、第1層の幅、および除数mが一致しなければ、報告を書かずに終わる。
- 報告の`reversals`は、v0の駒割で求めた参照の除去差分と、整数のネットの除去差分とで、符号が反転した件数と割合である。駒損を有利と評価する傾向の目安になる。
- Simpleでは、王将のない側の最後の太子を除いた局面は静的評価を定義できないので、終局の除去として`terminal_removal_count`に数え、差分の集計から除く。
- 量子化前の評価で、累算値から行を引く一括の計算と、除去後の局面から計算し直した値との差が上限の1e-6 cpを超えると、`error`を含む報告を保存して終了コード1で終わる。学習器の不具合として調べる。

#### 合法手の評価差

指定した局面のすべての合法手について、着手による評価の変化をNNUEとPST＋FMで比べる。

```sh
minase-trainer diag nnue-moves \
  --net <出力ディレクトリ>/nnue.bin --base <PST＋FMの重み> \
  --positions <局面のMNSD> --output <報告.json>
```

- `--base`には、次元32のFMを持つMNPT版3を与える。
- `--positions`には、ヘッダの規則セットが`lishogi`（L1,L2,P0,P3,R1,E1,E3）と同じ規則であるMNSDを指定する。由来が合成のMNSDも指定できる。
- 終局する着手は静的評価に渡さず、終局として`terminal_count`に数え、差分の集計から除く。
- 片方のモデルにしかない合法手があると、`error`を含む報告を保存して終了コード1で終わる。規則か局面の復元の不一致として調べる。

### 採否を測定して記録する

- 候補のネットをNASに`<SHA-256の16進数>.bin`の名前で置く。
- 対局の実行器`minase match run`で、候補と基準の両方に同じコミットを指定し、候補に`--candidate-option EvalFile=<候補のネット>`、基準に`--baseline-option EvalFile=<基準のネット>`を渡して、[段階ゲート](sprt.md#機能採否の段階ゲートstcとltc)で採否を決める。コミットを作る必要はない。
- 基準には、候補と比べたいネットを渡す。標準のネットは、[NNUEの評価関数の手引き](evaluation-nnue.md)に記す。
- 検証損失、符号一致率、診断の値の改善だけを、棋力が向上した根拠にしない。
- 結果は`docs/measurements/<設計書名>/<実験名>-training.md`にまとめる。どの設計書も計画していなければ`adhoc/`に置く。
- 記録には、学習器のコミット、コマンドライン全体、入力のパスと記録数、重みのファイル全体のSHA-256（NASのファイル名と同じ値）、停止理由、`summary.json`の量子化誤差、診断の要点を書く。

## 必要なときの操作

### vast.aiで教師ファイルを生成する

教師ファイルは、計算機を時間単位で借りられる市場vast.aiの中断可能インスタンスを最低入札額で借りて生成する（[ADR](../adr/rent-interruptible-instances-at-the-minimum-bid-for-teacher-generation.md)）。
作業機の制御スクリプト`scripts/vast/nnue/teacher_generation.py`が生成機を1台ずつ借り、確定した塊を60秒おきに回収し、生成機が止められたら別の機械を借りて続きから生成する（[ADR](../adr/generate-teacher-files-on-one-rented-machine-and-continue-from-collected-chunks-on-another.md)）。
仕様はスクリプトのdocコメントにある。

借りる前に`vastai show user`が残高を返すことを確かめ、2段階認証の要求で失敗すれば`vastai tfa login`でログインし直す。
制御スクリプトは、作業機のセッションが終わっても動き続けるように`systemd-run --user`のサービスとして起動する。
次は、第N世代の教師ファイルを番号0から生成する例であり、`<最後の番号>`は1世代の本数から1を引いた値である。
引数名の`book`は、教師ファイル1本を指す。

```sh
systemd-run --user --unit=<サービス名> --setenv=PATH="$PATH" -p WorkingDirectory=<worktree> \
  /bin/bash -c "exec python3 scripts/vast/nnue/teacher_generation.py run --run-dir <実行ディレクトリ> \
    --repo <リポジトリ> --ref <生成コミットを含む参照> --commit <生成コミット> --minase-bin <検査に使うminase> \
    --eval-file <教師生成に使うネット> \
    --generation <N> --first-book 0 --last-book <最後の番号> --seed-base <番号0のシード> --seed-book-step 100000 \
    --games 7500 --nodes 20000 --random-moves 0 --max-ply 4000 --hash-mb 16 --chunk-games 250 \
    --rental interruptible --max-price <時間単価の上限> --deadline <UTCの絶対期限> \
    --max-cost <累計費用の上限> --max-rentals <借りる回数の上限> >> <ログ> 2>&1"
```

- 受け入れの検査に通った教師ファイルと生成ログは、実行ディレクトリの`gen-<N>/teacher/`と`gen-<N>/logs/`に置かれる。連続生成スクリプトで学習するときは、これらを連続生成スクリプトの実行ディレクトリの同じ位置へハードリンクで置く。その世代の教師がすべてそろっていれば、連続生成スクリプトは教師を生成せず、`--train-place generator`と`--offer <学習機の出品>`ではGPUつきの学習機を借りて学習と診断だけを行い、`--train-place local`では作業機で学習する（[連続生成スクリプトで世代を作る](#連続生成スクリプトで世代を作る)）。
- 連続生成スクリプトは、第N世代の番号iの教師ファイルのシードを`seed_base + seed_generation_step × N + seed_book_step × i`として照合する。制御スクリプトの`--seed-base`には、この式でiを0とした値（`seed_base + seed_generation_step × N`）を、`--seed-book-step`には連続生成スクリプトと同じ`seed_book_step`を渡す。生成コミットは連続生成スクリプトと同じ反復のコミットとし、`--eval-file`にはNASにある第N−1世代のネット（第1世代ではv0のネット）のファイルを渡す。
- `--eval-file`のネットはtatara形式であり、生成機のエンジンがファイルの構造記述からLayerStackかSimpleかを判定するので、どちらの構造のネットも同じ引数で渡せる。
- 途中で止めるときは、`systemctl --user kill --kill-whom=main --signal=SIGTERM <サービス名>`で主プロセスに終了シグナルを送る。制御スクリプトは最後の回収と生成機の削除の確認を行ってから終わる。`systemctl --user stop`は90秒で強制終了するので使わない。
- 同じ引数で起動し直すと、受け入れ済みの教師ファイルを飛ばして続きを生成する。累計費用、絶対期限、および借りる回数の上限は実行ディレクトリに保存され、起動し直しても初期化されない。
- 費用を残高の差で記録するときは、削除した機械の課金が数分遅れて反映されるので、残高が変わらなくなるまで待ってから読む。
- 2026年11月14日の試行では、EPYC 7B13機で教師ファイル1本あたり約27.9分、約0.44米ドルであった（[測定記録](../measurements/interruptible-teacher-generation/interruptible-teacher-generation-trial.md)）。

### 連続生成スクリプトで世代を作る

連続生成スクリプト`scripts/vast/nnue/generation_chain.py`は、世代ごとに、足りない教師の生成、教師の検査、学習、駒除去差分の診断、ネットの保管、およびエンジンでの動作確認を続けて行う。
引数の一覧は`python3 scripts/vast/nnue/generation_chain.py run --help`で確かめ、標準レシピv3では上記の検証間隔と停止条件を渡す。1回の実行で指定できるB、T、Sは1組なので、教師量が変わる場合は世代ごとに計算して実行する。世代反復での求め方と世代ごとの見込みの値は、[世代反復の設計書](../plans/nnue-generation-iteration.md#世代の手順)にある。
標準レシピに関わる引数と動作は次のとおりである。

- `--structure layerstack`はP型の学習器を、`--structure simple`はHalfKP_vmの学習器を選ぶ。
- `--gen0-network`には、`nnue export-handcrafted`で書き出したv0のネットを渡す。スクリプトは、このネットを第0世代のネットとして`gen-0/generation.json`に記録する。
- 第1世代の教師が`--calibration-books`の本数だけそろうと、スクリプトは番号の小さいほうからその本数の教師で`data calibrate`を実行し、結果を実行ディレクトリの`calibration.json`に置く。第1世代の最初の4本からK_tを求める標準の手順では、`--calibration-books`に4を渡す。
- スクリプトは、較正の結果の`teacher_cp_per_logit`をK_tとして`gen-0/generation.json`に書き、以後の世代の記録にも同じ値を書く。`classes`が1項目でないか、`at_search_bound`が`true`なら止まる。
- 学習には、記録したK_tを`--teacher-cp-per-logit`に、規則セット`lishogi`を`--rules`に渡す。スクリプトの`--batch`、`--batches-per-superbatch`、`--superbatches`、`--validation-superbatches`、`--validation-patience`、`--train-subsample`、`--device`、`--lr-schedule`、および`--lambda`は学習器の同名の引数へ、`--train-seed`と`--train-threads`は学習器の`--seed`と`--threads`へ渡す。`--train-threads`には、標準レシピの`--threads`と同じく16か、[学習する](#学習する)の「スレッド数を決める」の手順で選んだ値を渡す。
- `--ref`と`--commit`は、教師の生成器と、教師ファイルのヘッダの照合に使うコミットを指す。`--trainer-ref`と`--trainer-commit`は、学習機で学習器をビルドするコミットを指す。同じコミットで学ぶ場合も、両方を明示して渡す。借りた機械では生成と学習が1つの作業ツリーを使うので、2つのコミットを分けられるのは、その世代の教師がすべてそろっていてスクリプトが教師を生成しない場合だけであり、生成が残っていればスクリプトは機械を借りる前に止まる。`--train-place local`では`--trainer-bin`の実行ファイルをそのまま使うので、その実行ファイルを`--trainer-commit`からビルドしておく。世代の記録`gen-<N>/generation.json`には、`commit`に加えて`trainer_commit`が書かれる。
- スクリプトは、学習を始めるときに、学習器のコミットと、実行する場所によらない学習の引数を`gen-<N>/train-request.json`に書く。同じ実行ディレクトリを別の条件で起動し直すと、記録と一致しないので止まり、別の条件で終えた学習の成果物を使うことはない。学習の成果物をほかの実行ディレクトリへ移すときは、`train-request.json`も一緒に移す。
- `--validation-patience`を渡さない実行では、学習器は最後の重みを書き出す。スクリプトは世代を記録する前に`train/training.jsonl`を読み、最後の検証損失が、更新後の検証損失の最小値の1.02倍を超えていれば、世代を記録せずに止まる。閾値の根拠は[学習の長さと書き出すネットのADR](../adr/train-the-nnue-for-four-epochs-decaying-the-learning-rate-to-the-end-and-export-the-last-network.md)にある。
- 学習の`summary.json`の`status`が`candidate`でなければ止まる。学習の後は、その世代の教師だけで駒除去差分の診断を行い、報告を`gen-<N>/removal.json`に書く。
- 最後に、ネットをSHA-256の名前で`--network-store`へ置き、`setoption name RuleSet value lishogi`、`setoption name EvalFile value <ネット>`、`isready`の後の`bench depth <--bench-depthの値>`が完走することを確かめてから、`gen-<N>/generation.json`に世代の完了を記録する。
- 教師がすべて作業機にある世代を`--train-place generator`と`--offer`で実行すると、スクリプトは指定した出品を学習機として借り、コードと教師を送り、学習機でコミットを展開して学習器をビルドしてから学習と診断を行う。結果を作業機へ取得した後に学習機を止めて削除し、ネットの保管と`bench`は作業機で行う。学習機の停止期限は`--training-wait-hours`に世代の数を掛けた時間であり、教師の転送、ビルド、学習、および診断をこの時間に収める。
- 学習機の作業プロセスは、学習が終わるまで`--idle-limit-hours`の自己停止を行わない。学習が終わらないまま作業機との接続が絶えた場合は、停止期限で学習機が止まる。
- 学習機で行った各ステップのログは`gen-<N>/training-logs/`に取得される。`train.rusage.json`の`max_rss_kib`は学習器のプロセスの最大常駐メモリ（単位はKiB）であり、学習機を削除した後も読める。

### 中断した学習を再開する

- 最初と同じコマンドに、`--output-dir`と同じパスで`--resume <出力ディレクトリ>`を加える。
- 再開時に変えられるのは、`--superbatches`、`--validation-superbatches`、`--time-limit-hours`、`--threads`、`--validation-sample`、`--bucket-positions`、`--scratch-dir`だけである。検証で重みを選ぶ場合は検証間隔も固定する。`--batches-per-superbatch`と`--lr-schedule`は学習率の進み方を変えるので、変えられない。
- `--superbatches`と`--time-limit-hours`は、再開前を含む総量である。`--superbatches`を完了済みの数より小さくするとエラーになり、大きくすると完了した学習からさらに続きを学習できる。
- それ以外の設定、入力ファイル（パスとファイル全体のSHA-256）、前処理の結果、またはデバイスが異なると、エラーになり再開できない。構造、規則セット、およびK_tも、照合する設定に含まれる。
- 最後に確定した`latest.npz`から復元し、エポックの途中であれば同じ並べ替えを作り直して続きの局面から学習する。`training.jsonl`の確定していない行は捨て、前の実行の`nnue.bin`と`summary.json`は再開の始めに消す。止めずに通した学習と同じネットになる。
- 再開の履歴は`config.json`の`resumes`に残るので、測定記録に再開の回数と変更した上限を書く。
- 学習器が構造ごとのサブコマンドに分かれる前に保存したチェックポイントからは、再開できない。

### メモリに収まらない教師データで学習する

- 既定の`--buckets 1`は、全局面を圧縮表現でメモリに持ち、毎エポック全体を並べ替える。
- 収まらないときは、`--buckets <B>`（2以上）、`--bucket-positions <上限>`、`--scratch-dir <ディレクトリ>`を指定する。
- Bが2以上なら、局面を一時ファイルへ書き出し、エポックごとにB個のbucketへ無作為に振り分ける。bucketを1つずつメモリへ読んで並べ替えるので、全局面が一様に並べ替わる（[ADR](../adr/shuffle-the-whole-training-set-per-epoch-with-rao-sandelius-buckets.md)）。

**値の選び方**（Nは訓練局面の数）

- `--buckets <B>`: 次の3つの合計が使える実メモリに収まるように選ぶ。
  - 学習プロセスの固定のメモリ（ネット、最適化器、GPUのドライバなど）。
  - bucket1つ分（局面1つにつき一時表現の138バイトと順列の8バイト）。
  - 先行して組み立てるバッチ（`--threads × --batch`局面）。
- `--bucket-positions <上限>`: `N/B + 6√(N/B)`以上を指定する（超える確率は約10⁻⁹）。超えると学習器が局面数を示して止まるので、値を増やしてやり直す。
- `--scratch-dir`: 訓練局面1つにつき約300バイトの空きを用意する。一時ファイルは測定機の空き容量の4分の1までとする（2026年10月17日の利用者の決定）。
- 1エポックの読み書きは一時表現の総量の約3倍である。時間への影響は[B=16の測定記録](../measurements/out-of-core-shuffle/out-of-core-shuffle-time-memory.md)にある。

**一時ファイルの後始末**

- 一時ファイルは`<scratch-dir>/nnue-<UNIX秒>-<プロセスID>-<連番>/`に作られ、正常終了時と、学習器が捕捉した異常終了時に消える。
- 強制終了や電源断で残ったものは、再開にも別の学習にも使えない。`lock`のプロセスIDが`ps -p <ID>`で実行中でないことを確かめてから、ディレクトリごと`rm -r`で消す。

## 学習器を変更したとき

### 学習器を検査する

学習器を変更したら、学習の前に次の検査を通す。

**GPUなしの検査**

```sh
cargo fmt --all --check
nice -n 19 cargo clippy -j 2 --workspace --all-targets -- -D warnings
nice -n 19 cargo test -j 2 -p minase-trainer
```

**GPUありの検査**（GPUを載せた機械で実行）

```sh
nice -n 19 cargo test -j 2 -p minase-trainer -- --ignored gpu_ --test-threads=1
for STRUCTURE in layerstack simple; do
  nice -n 19 cargo run -j 2 -p minase-trainer -- nnue check-equivalence "$STRUCTURE" \
    --batch 4096 --seed 71 --teacher-cp-per-logit 1072.65 --lambda 1.0
done
```

- `check-equivalence`は、合成した局面で、GPUのカーネルの順伝播、損失、勾配、および3回の更新がCPUの参照実装と一致することを確かめる。第1層の幅は構造ごとに256で固定である。
- `--teacher-cp-per-logit`は合成した局面の教師値の換算に使うだけなので、正の有限な値ならよい。例の値は、学習器のテストが使う値と同じである。
- `gpu_`のテストは、これに加えて次の3つを確かめる。
  - CUDAでエポックの途中から再開した学習が、止めずに通した学習と一致する。
  - `--validation-superbatches`によらず学習の軌跡が変わらない。
  - CUDAで1 superbatchを学んだ結果が、CPUで学んだ結果と許容差の中で一致する。
- 許容差（|GPU−CPU| ≤ 1e-6 ＋ 1e-5×|CPU|）は、`crates/minase-trainer/src/nnue/layerstack/commands.rs`と`simple/commands.rs`の`ATOL`と`RTOL`にある。

### GPUでの1更新の時間を測る

学習器の速度に関わる変更をしたときは、`nnue bench-step`で1更新の時間を、バッチの供給、入力の転送、GPUの計算に分けて測る。

```sh
for STRUCTURE in layerstack simple; do
  CARGO_NET_OFFLINE=true nice -n 19 cargo run --release -j 2 -p minase-trainer -- nnue bench-step "$STRUCTURE" \
    --batch 16384 --steps 20 --seed 71 --teacher-cp-per-logit 1072.65 \
    --threads 6 --profile --lambda 1.0 --data <MNSDファイル>
done
```

- この例には、訓練集合が16,384局面以上要る。訓練集合を使い切ると並べ替え直して続けるので、更新の回数に比例した局面数は要らない。検証集合は使わない。
- `--steps`は2以上とし、省略すると10になる。
- `--lambda`は学習と揃える。
- `--profile`は、順伝播、逆伝播、および最適化器の時間の内訳を表示する。`--device cpu`と併用すると、逆伝播の時間に順伝播の時間が含まれる。
- GPUメモリの出力のうち、`device_array_bytes`は学習器が確保した配列のバイト数であり、`gpu_memory_used_bytes`は確保した後のデバイス全体の使用量である。後者は他のプロセスの確保も含む。Simpleでは、学習と検証の配列の内訳も出力する。
- 測る区間は`crates/minase-trainer/src/nnue/layerstack/bench.rs`と`simple/bench.rs`にある。
- 性能の採否は実GPUの測定で決める。CUDAのコンパイルやGPUなしのテストの結果から速度を推定しない。

## オプション一覧

値はソースの既定値を写したものである。食い違うときはソースと`--help`を正とする。
`--resume`のときに変えられる引数は、[中断した学習を再開する](#中断した学習を再開する)にある。

### `nnue train layerstack`と`nnue train simple`

2つのサブコマンドは同じ引数をとる。
第1層の幅などの構造に固有の値は引数で変えられず、[引数で変えられない固定値](#引数で変えられない固定値)にある。

**入力と出力**

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--data <FILE...>` | 必須 | MNSDファイルを全体のレコード順に指定する。 |
| `--rules <RULES>` | 必須 | 規則セットの名前（例えば`lishogi`）または規則コードの列。全ての教師ファイルのヘッダの規則セットと照合し、異なれば学習を始めない。 |
| `--output-dir <DIR>` | 必須 | 新規または空のディレクトリ。 |
| `--resume <DIR>` | なし | 再開する学習の出力ディレクトリ。`--output-dir`と同じパスを指定する。 |
| `--train-subsample <SPEC>` | 全件 | `games:<k>:<r>`または`positions:<k>:<r>`。k≥2、0≤r<k。 |

`--train-subsample`の補足は次のとおりである。

- `games`は少数の対局から全局面を、`positions`は多数の対局から疎に局面を取り、両者を同じ局面数で比べるための引数である。
- 式は`crates/minase-trainer/src/data/subsample.rs`の`TrainSubsample`にある。
- 先読み教師などの前処理は、間引く前の訓練集合全体で行う。検証集合は間引かない。
- 1エポックの更新回数は抽出後の局面数で決まる。同じ引数を渡せば、更新回数の上限と各更新での学習率は間引いても変わらない。

**教師**

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--lambda <VALUE>` | 必須 | 探索値の割合。0以上1以下。 |
| `--teacher-cp-per-logit <K_t>` | 必須 | 教師の勝率尺度（1ロジットあたりのcp）。正の有限な数で、round(8,128÷K_t)が1以上の32ビットの整数になる値。 |
| `--lookahead-gamma <VALUE>` | なし | 先読みの割引率。0より大きく1より小さい。`--lookahead-plies`と同時に指定する。 |
| `--lookahead-plies <N>` | なし | 先読みの手数。1以上4096以下。`--lookahead-gamma`と同時に指定する。 |

**最適化**

最適化器AdamWは、パラメータを`ft`と`head`の2つの群に分け、群ごとに学習率と重み減衰を持つ。
`ft`群は、LayerStackでは第1層の重みとPSQTであり、Simpleでは第1層の重みと特徴分解の仮想行である。
`head`群は、どちらの構造でも、第1層のバイアスと後段の重みとバイアスである。

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--lr-ft <VALUE>` | 0.001 | `ft`群の学習率。正の有限値。 |
| `--lr-head <VALUE>` | 0.001 | `head`群の学習率。正の有限値。 |
| `--weight-decay-ft <VALUE>` | 0 | `ft`群の重み減衰。非負の有限値。 |
| `--weight-decay-head <VALUE>` | 0 | `head`群の重み減衰。非負の有限値。 |
| `--lr-schedule <SPEC>` | 必須 | 下表の3つの形。 |
| `--batch <N>` | 16384 | 1更新の最大局面数。エポックの最後のバッチは少なくてもよい。 |
| `--seed <N>` | 1 | 初期化、順列作成、検証標本抽出の乱数シード。 |

| `--lr-schedule`の形 | 倍率m(t) | 条件 |
|---|---|---|
| `constant:<c>` | c | c>0 |
| `exponential:<h>:<f>` | max(f, 2^(−t/(h·B))) | h>0、0<f≤1 |
| `cosine:<T>:<f>` | f + (1−f)·(1+cos(π·min(t/(T·B), 1)))/2 | T>0、0<f≤1 |

- tは完了した更新回数、Bは`--batches-per-superbatch`である。hとTの単位はsuperbatchである。
- 倍率は2つの群の学習率に同じ値で掛かり、重み減衰にも学習率を通じて掛かる。

**学習の長さと時間**

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--superbatches <N>` | 必須 | 再開前を含むsuperbatch数。1以上。N×Bがu64を超える指定は拒否する。 |
| `--batches-per-superbatch <B>` | 6104 | 1 superbatchの更新回数。1以上。 |
| `--time-limit-hours <VALUE>` | 24 | 前処理、検証、再開前を含む累積時間の上限。24以下。 |

**検証**

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--validation-superbatches <V>` | 必須 | V superbatchごとに全検証局面の損失を測る。学習前と予定した最後のsuperbatchでも測る。 |
| `--validation-patience <P>` | 指定なし | P回連続して検証損失が最小値を更新しなければ止め、更新後の最小損失の重みを書き出す。Pは1以上とし、指定時は `--superbatches` をVの倍数にする。省略時は予定した最後の重みを書き出す。 |
| `--validation-sample <N>` | 10000 | 量子化誤差の測定に使う検証局面数の上限。1以上。 |

**実行環境**

| 引数 | 既定値 | 値域と意味 |
|---|---|---|
| `--device <NAME>` | `cuda` | `cuda`または`cpu`。`cuda`はGPUが見つからなければエラーで終わる。 |
| `--threads <N>` | 物理コア数−2（最小1） | バッチ供給のスレッド数。 |
| `--buckets <N>` | 1 | bucket数。1以上4096以下。1は全局面をメモリに持つ。 |
| `--bucket-positions <N>` | なし | 1つのbucketの局面数の上限。`--buckets`が2以上のとき必須。 |
| `--scratch-dir <DIR>` | なし | 一時ファイルを置く既存のディレクトリ。`--buckets`が2以上のとき必須。 |

### その他のサブコマンド

同名の引数の値域と既定値は`nnue train layerstack`と同じである。括弧内はそれと異なる値や条件を示す。
「学習率と重み減衰」は、`--lr-ft`、`--lr-head`、`--weight-decay-ft`、`--weight-decay-head`の4つを指す。
`nnue check-equivalence`と`nnue bench-step`は、`nnue train`と同じく構造を`layerstack`か`simple`のサブコマンドで選び、どちらの構造でも同じ引数をとる。

| コマンド | 必須の引数 | 任意の引数 |
|---|---|---|
| `nnue export-handcrafted` | `--output <FILE>`（新規のパス） | なし |
| `data calibrate` | `--data`、`--output`（新規のパス） | `--lookahead-gamma`、`--lookahead-plies`、`--threads` |
| `nnue check-equivalence layerstack`、`nnue check-equivalence simple` | `--batch`、`--seed`、`--teacher-cp-per-logit`、`--lambda` | 学習率と重み減衰 |
| `nnue bench-step layerstack`、`nnue bench-step simple` | `--batch`、`--seed`、`--teacher-cp-per-logit`、`--lambda`、`--data` | `--steps`（2以上、既定10）、`--threads`、`--device`、`--profile`、学習率と重み減衰 |
| `diag nnue-distribution` | `--pair <名前> <MNPT> <tatara形式のネット>`（複数可）<br>`--data`、`--sample`、`--seed`、`--output` | なし |
| `diag nnue-removal` | `--float <量子化前のチェックポイント>`、`--net <tatara形式のネット>`<br>`--data`、`--sample`、`--seed`、`--output` | なし |
| `diag nnue-moves` | `--net`、`--base`、`--positions`、`--output` | なし |

### 引数で変えられない固定値

変えるにはソースの変更が要る。場所は`crates/minase-trainer/src/`からの相対パスである。
LayerStackの値は`nnue/layerstack/`に、Simpleの値は`nnue/simple/`にある。

| 項目 | 値 | 場所 |
|---|---|---|
| AdamWの係数 | β1=0.9、β2=0.999、ε=1e-8 | `nn/adam.rs` |
| 第1層の幅 | 256 | 各構造の`model.rs`の`H` |
| 初期値（LayerStack） | 第1層の重みは±√(1/13,680)の一様分布であり、13,680はP型の入力特徴の数である。後段の重みは±0.01の一様分布、バイアスは0とする。PSQTはv0の駒割をK_oで割った値とし、王駒と先獅子の行は0とする | `nnue/layerstack/model.rs` |
| 初期値（Simple） | 第1層を含む全ての重みとバイアスは±0.01の一様分布、特徴分解の仮想行は0 | `nnue/simple/model.rs` |
| 更新後の切り詰め（第1層） | LayerStackは重みとバイアスを±224/127。Simpleは実の行と仮想行の重みをそれぞれ±112/127、バイアスを±224/127 | 各構造の`model.rs`の`Parameter::limit` |
| 更新後の切り詰め（後段） | LayerStackの幅16の層の重みは区分ごとの組と共有の組をそれぞれ±127/128。それ以外の後段の重みと、出力のバイアスを除く後段のバイアスを±127/64。出力のバイアスとPSQTは切り詰めない | 各構造の`model.rs`の`Parameter::limit` |
| 量子化の型と尺度 | 第1層は16ビット整数で尺度127、後段の重みは8ビット整数で尺度64、後段のバイアスとPSQTは32ビット整数で尺度127×64＝8,128。範囲を超える値はエラーで止める | 各構造の`export.rs` |

## 標準レシピの設定と根拠

対局で測ってから決める残りの項目は、[調査メモ](../research/nnue-training-recipes.md#対局で測ってから決める項目)にある。
表の根拠にある第1世代と第2世代は、[中断した世代反復](../plans/archived/nnue-generation-reset.md)の世代である。
表のSTCは持ち時間10秒、1手ごとの加算0.1秒の対局で行う逐次検定であり、`H1`は候補が強いと判定されたこと、`H0`は強いとは言えないと判定されたことを表す（[棋力測定の手引き](sprt.md)）。

| 項目 | 値 | 根拠 |
|---|---|---|
| 教師の量 | 第n世代は1億局面に相当する本数k（見込みは37）のn倍のファイル、1ファイル7,500局、1手2万ノード | 第2世代の教師では、37ファイルが、その半分に対してSTCで`H1`（得点率63.2%）で、まだ頭打ちではない（[記録](../measurements/nnue-generation-reset/nnue-generation-reset-gen2-volume.md)）。自己生成で育てたSaplingとAstraは、世代を追って1世代の量を増やした（[調査の付録](../research/teacher-generation-strong-engines/zero-chess-and-variants.md)）。局面数をそろえると対局の多い教師が`H1`（得点率56.5%）なので、量は対局数で定める（[記録](../measurements/nnue-generation-reset/nnue-generation-reset-gen2-diversity.md)） |
| リプレイバッファ | その世代の教師だけで学ぶ | 直前の世代の教師を併用した学習はSTCで`H0`（[記録](../measurements/nnue-standard-recipe/nnue-standard-recipe-window-stc.md)） |
| λ | 1.0 | 終盤に勝敗を25%混ぜると勝敗の予測誤差は小さくなったが、直接対局で棋力の改善は確認できなかった。[終盤の比較](../measurements/adhoc/tatara-p-generalization.md#終盤に勝敗を混ぜた効果) |
| 構成 | P型のLayerStack（第1層の幅256、後段の区分1つ）に、区分1つのPSQTを並べた構造 | 2026年11月15日にエンジンの開発者が、P型もtataraの枠組みへ載せ替え、重みをtataraの形式にすると決めた（[ADR](../adr/compose-p-as-a-tatara-layerstack-with-a-psqt-and-halfkp-vm-as-a-simple-network.md)） |
| 最適化手法 | AdamW（β1=0.9、β2=0.999、ε=1e-8） | Stockfishの本番と同じ値（[調査メモ](../research/nnue-training-recipes.md)） |
| 基準の学習率 | `ft`群と`head`群をどちらも0.001 | 0.001は先行の初期値の範囲内。PSQTはtataraと同じく第1層と同じ`ft`群に入れ、第1層の学習率を使う。載せ替える前の構造が持っていた線形の経路（駒と升ごとの点数を駒数で序盤と終盤の間で補間する部分）の学習率0.03はcp単位の値であり、ネットの出力の単位で表すPSQTへはそのまま移せない。理由の詳細は[ADR](../adr/compose-p-as-a-tatara-layerstack-with-a-psqt-and-halfkp-vm-as-a-simple-network.md)にある |
| 学習の長さ | 対局単位の検証損失で世代ごとに選ぶ。 | 同じ30ファイルでは、訓練損失が下がり続けても検証損失と棋力は悪化した。[測定記録](../measurements/adhoc/tatara-p-generalization.md) |
| 書き出すネットと停止 | 連続2回改善しなければ止まり、検証損失が最小の重みを書き出す。 | 2回は単発の上振れで止めないための運用値である。検証損失は教師の探索値を再現する精度を測るので、棋力の採否には独立した対局を用いる。 |
| 学習率の予定 | 旧レシピと同じ更新回数を期間とする余弦減衰で、下限は初期値の0.027倍とする。 | 検証間隔を変えても、同じ更新回数での学習率を保つ。[旧レシピの根拠](../adr/archive/train-the-nnue-for-64-or-80-superbatches-by-the-generation-volume-and-export-the-last-network.md) |
| 損失 | 勝率の差の2乗 | bullet、tatara、variant-nnue-pytorchが2乗を、nnue-pytorch、Stockfish、tanuki-が勝率の差のべき乗を使う（同じ調査メモ）。指数は調整していない基本の形の2とする |
| 教師の勝率尺度 | 第1世代の教師の最初の4本から`data calibrate`で一度だけ求めたK_tを、全世代の学習に`--teacher-cp-per-logit`で明示して渡す | 調べた13件の実装のうち11件が、手で決めた定数か一度推定して固定した値を使う（[調査メモ](../research/win-rate-scale-k-prior-art.md)）。tataraの形式は実数の勝率尺度の欄を持たないので、前の世代のネットから引き継がず引数で渡す（[ADR](../adr/separate-the-nnue-teacher-win-rate-scale-from-the-output-scale.md)） |
| 重み減衰 | どの層にも掛けない | nnue-pytorch、tatara、tanuki-、variant-nnue-pytorchが0である（[調査メモ](../research/nnue-training-recipes.md)） |
| バッチ | 16,384局面 | 先行の多くと同じ |
| 学習のスレッド数 | 16。GPUかCPUが基準の作業機より強力な機械では、短い学習で速さを比べて選ぶ | 基準の作業機（RTX 3060 Ti、20コア）では、4スレッドは供給が律速で毎秒約70万局面、16スレッドは毎秒約106万局面だった（[記録](../measurements/adhoc/layerstack-backward-coalesce.md)） |
| 検証 | 約0.5エポックごとに全検証対局で測る。 | 第2世代の旧設定では初回の更新後検証が8.29エポックだった。改善から悪化に転じる時期を見るために間隔を短くする。 |

## 詳細の置き場所

| 知りたいこと | 場所 |
|---|---|
| 構造と計算式、学習の目的関数 | [NNUEの評価関数の手引き](evaluation-nnue.md) |
| 採否の判定 | [棋力測定の手引き](sprt.md) |
| 世代反復の手順 | [世代反復の設計書](../plans/nnue-generation-iteration.md) |
| 引数の既定値と値域の定義 | `crates/minase-trainer/src/nnue/`の`options.rs`、`train/args.rs`、`schedule.rs` |
| 学習ループと保存物の各項目 | `crates/minase-trainer/src/nnue/train/mod.rs`と`train/record.rs` |
| 構造ごとの式、初期化、切り詰め、量子化 | `crates/minase-trainer/src/nnue/layerstack/`と`nnue/simple/` |
| チェックポイントの形式 | `crates/minase-trainer/src/nnue/layerstack/checkpoint.rs`と`nnue/simple/checkpoint.rs`の`Checkpoint` |
| 学習データの形式、教師値、検証集合の分け方 | `crates/minase/src/training/`と`crates/minase-trainer/src/data/`（PSTとFMの学習器と共通） |
| 方式を選んだ理由 | ADRの[学習器の構成](../adr/split-the-nnue-trainer-into-a-shared-loop-and-structure-backends.md)、[構造と量子化](../adr/compose-p-as-a-tatara-layerstack-with-a-psqt-and-halfkp-vm-as-a-simple-network.md)、[教師の勝率尺度](../adr/separate-the-nnue-teacher-win-rate-scale-from-the-output-scale.md)、[学習の後に補正しないこと](../adr/use-each-trained-nnue-outside-git-for-the-next-generation.md)、[カーネルの書き方](../adr/write-trainer-kernels-in-cuda-cpp-via-nvrtc.md)、[第1層の勾配](../adr/backpropagate-the-first-layer-through-a-reverse-index.md)、[AdamWの式](../adr/use-pytorch-adamw-with-bias-correction.md)、[教師と損失](../adr/train-the-nnue-on-search-value-teachers-with-the-squared-win-rate-difference.md)、[学習の長さと書き出すネット](../adr/select-the-nnue-checkpoint-on-held-out-games.md) |
