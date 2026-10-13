# PSTとFMの学習手順

本書は、駒の種類と位置に応じた評価表であるPSTと、PSTに加える2駒関係の補正FM（Factorization Machine）を学習する手順を定める。
学習には、学習器のクレート`crates/minase-trainer`の実行ファイル`minase-trainer`のサブコマンドを使う。
エンジンが探索に使う既定の評価関数は、2026年10月12日からNNUEである（[探索が使う評価関数をNNUEの1つにし、PSTとFMのコードは比較用に残す](../adr/use-nnue-as-the-only-search-evaluation.md)）。
PSTとFMの学習器は、NNUEの線形経路の初期値を作る場合や、PSTを土台にした研究の計画で使う。

本書は、実行者が従う手順、判断規則、および標準コマンドだけを記す。
学習データの形式とレコードの由来は`crates/minase/src/training/records.rs`、教師の分類、教師値、先読み教師、および検証分割は`crates/minase-trainer/src/data/`のdocコメントにある。
学習器が書き出すファイルの各欄は`crates/minase-trainer/src/`の`pst/record.rs`、`fm/record.rs`、`fm/report.rs`、および`joint/record.rs`に、PSTの比較診断の報告の各欄は`crates/minase-trainer/src/diagnostics/pst_compare.rs`の`Report`と`pst_statistics.rs`の各項目のdocコメントにある。
教師値、損失、量子化、および重みの範囲の式は [PSTとFMによる評価関数と学習の目的関数](evaluation-pst-fm.md) が説明し、学習器の実装の契約は`minase-trainer`の`pst`、`fm`、`joint`、`data`の各モジュールのdocコメントにある。
学習器をPythonからRustへ移した経緯と、移植時にPythonの学習器と同じ結果になることを確かめた記録は、[Python学習器の撤去の設計書](../plans/archived/python-trainer-retirement.md)と[照合の記録](../measurements/python-trainer-retirement-checks.md)にある。

学習を始める際は、[設計書の型](../plans/README.md) に従って目標局面数と比較する基準コミットを定め、[ROADMAP.md](../ROADMAP.md) に状態を登録する。
学習後の採否は [棋力測定の標準手順](sprt.md) に従う。
以下のコマンド例では、実験名を`<実験名>`、基準コミットを`<基準コミット>`、生成シードを`<シード>`と書き、明記した箇所を除いて、すべてのコマンドをリポジトリのルートで実行する。

## 1. 学習器を用意する

学習器はCPUだけで動き、`--threads`の既定値は物理コア数から2を引いた数である。
次のコマンドでビルドし、テストを実行する。

```bash
cargo build --release -p minase-trainer
cargo test --release -p minase-trainer
```

以下では、ビルドした`target/release/minase-trainer`を`minase-trainer`と書く。
学習器の変更を含む実験では、学習を始める前に学習器のコミットを決めて測定記録に書き、実行中はそのバイナリを使い続ける。

## 2. 実験を準備する

基準コミットを固定したworktreeを作り、教師データの生成器（`minase`）をビルドする。
基準のPSTを実験ディレクトリへ複製し、以後の学習と診断はこの複製を使う。

```bash
mkdir data/<実験名>
git worktree add --detach data/<実験名>/generator <基準コミット>
cargo build --release --locked --target-dir data/<実験名>/generator/target \
  --manifest-path data/<実験名>/generator/Cargo.toml --bin minase
cp <基準のPST（FMを持たないMNPT）> data/<実験名>/pst-base.bin
sha256sum data/<実験名>/pst-base.bin
```

`pst-base.bin`には、重みファイルの形式MNPTのうち、FMを持たない（FM節の次元が0の）重みを使う。
`pst train`と`fm train`は、FMを持つMNPTのPSTの部分も読める。
一方、5節のPSTの比較診断は、基準と候補がFMを持たないことを前提に独立に計算した整数のPST評価とエンジンの評価を比べるので、FMを持つ重みを与えると開始時に拒否する。
`crates/minase/nets/pst.bin`はFMを持つ重みであり、そのPSTの部分だけの重みは、倍率を焼き込む前の同時学習J75の出力`data/pst-fm-joint-training/j75/pst.bin`と`pst-float.npz`から作れる。
一般に、倍率を焼き込む前の`joint train`の出力`pst.bin`と、同じ学習の量子化前の重み`pst-float.npz`の組があれば、`minase-trainer fm scale --fm-scale 0`でFMを持たない重みを作れる。
倍率を焼き込んだ後の重みは量子化前の重みと一致しないので、この変換は拒否される。
学習の記録（`config.json`）は、基準の重みをパスとヘッダの検査和で、学習データをパス、バイト長、およびヘッダの各欄で識別するだけなので、準備の時点で基準の重みのSHA-256を測定記録に書いておく。

MNSD形式の学習データは1ファイルで完結し、付随ファイルを必要としない。
各レコードは、局面を作った経路である由来（乱数開始の自己対局、実戦開始の自己対局、実戦対局、合成）を持ち、学習器は由来とヘッダから教師の分類と検証集合を決める。
1ファイルの中で由来が混在するデータと、診断が書き出す標本のように合成の由来を持つデータは、学習に使えず、学習器が開始時に拒否する。
学習と診断のコマンドには、教師値で探索値と対局結果を混ぜる比λを`--lambda`で必ず指定する。
λは自己対局のデータにだけ適用され、実戦対局のデータでは常に0（対局結果だけを教師とする）である。
これまでのPSTの学習では0.75を使ってきた。

`gen0b.bin` は既存の対局と重複するため、入力へ加えない。

学習と診断のコマンドは、`--data`に並べた複数のファイルを入力順に読む。
教師の分類の番号は入力順に初めて現れた順で付くので、学習の記録と診断の報告を突き合わせるときは、両方で同じ順序に並べる。
入力はメモリに写像して読むため、コマンドの実行中は入力ファイルを変更も切り詰めもしない。
乱数開始の自己対局のファイルは、ファイルの間で生成シードが重ならないものだけを組み合わせる。
MNSDの形式と由来は`crates/minase/src/training/records.rs`、教師の分類、教師値、および検証集合の分け方は`crates/minase-trainer/src/data/`のdocコメントにある。

## 3. 自己対局データを生成する

生成シードは互いに生成局数以上離し、過去の生成にも使っていない範囲を選ぶ（[隣接シードの教訓](../lessons/derive-seed-adjacent-collision.md)）。
設定内の重複だけでなく、破棄されて記録されなかった対局や入力に含めていないファイルの範囲も、過去の生成ログで確認する。

シードごとに次を実行する。
生成器は、基準コミットを固定したworktreeの中で起動する（[生成コミットを固定する教訓](../lessons/pin-generation-binary-to-worktree.md)）。
数値は、これまでの実験で使った生成条件の例である（1ファイル7,500局、1手10万ノード、同時16局、4,000手で破棄、置換表16 MB、対局中のランダム着手なし）。

```bash
cd data/<実験名>/generator
target/release/minase data selfplay generate --output ../generated-<シード>.bin --seed <シード> \
  --games 7500 --nodes 100000 --random-moves 0 --concurrency 16 --max-ply 4000 --hash-mb 16 \
  2>&1 | tee ../generate-<シード>.log
target/release/minase data selfplay inspect ../generated-<シード>.bin | tee ../inspect-<シード>.log
cd -
```

生成器は`generated-<シード>.bin`を書く。
`inspect`の出力で、ヘッダの生成コミット、重みの検査和、規則、シード、探索ノード上限、およびレコードの由来が意図どおりであることを確かめる。
生成終了後は、保存局面数と手数上限による破棄数をログで確認し、理由別の除外率、勝敗数、探索値の分布、および重複率も確かめる。
途中で止まったファイルは完成品として扱わず、ログとともに別の場所へ退避してから同じシードで生成し直す。
目標の局面数に足りず生成条件を追加する場合は、別の実験として入力とシードを決め直す。

## 4. PSTを学習する

すべての生成ファイルが完成したら、次のコマンドでPSTを学習する。
例は、採用PSTの候補L（[PSTのエポック数の延長](../plans/archived/pst-longer-training.md)）の条件である。

```bash
minase-trainer pst train \
  --data <既存データ...> data/<実験名>/generated-*.bin \
  --init data/<実験名>/pst-base.bin \
  --output-dir data/<実験名>/training \
  --lr 0.03 --epochs 200 --patience 5 --batch 16384 --seed 1 \
  --removal-penalty 1000 --removal-reference init --lambda 0.75 \
  --lookahead-gamma 0.9 --lookahead-plies 40 \
  2>&1 | tee data/<実験名>/train.log
```

学習器は、`--init`のPSTを初期値として、左右の鏡映対で重みを共有する2端点（序中盤用と終盤用）のPSTを学習する。
評価値を勝率へ変換する尺度である出力用K、探索用駒価値、および規則セット名は`--init`から引き継ぐ。
`--lambda`は2節で述べたとおり自己対局のデータにだけ適用され、実戦対局のデータでは0が使われる。
`--lookahead-gamma`と`--lookahead-plies`は、同じ対局の将来の探索値を割り引いて平均した先読み教師を使う指定であり、両方を同時に与える。
`--removal-reference`は、駒を1枚除いたときの評価の変化が同じ向きになることを求める補助損失の参照であり、`init`は`--init`の整数PST、`v0`は学習を始める前の手書きの評価関数v0の駒価値を参照にする。
エポック数の上限は200である。

最良の重みは、学習したエポック（1以上）の中から、補助損失を混ぜない検証集合の二値交差エントロピーが最小のものを選ぶ。
学習前の重みの検証損失もエポック0として記録されるが、出力の候補にはならない。
`--patience`のエポック数だけ続けて最良値が更新されなければ打ち切る。
選択と打ち切りの定義は一般的な早期終了と同じであり、典拠と採用の経緯は[PST追加学習の設計書](../plans/archived/pst-longer-training.md)の「早期終了」にある。

結果は`--output-dir`に保存される。
`pst.bin`はFMを持たないMNPTの重み、`pst-float.npz`は量子化前の重み、`training.jsonl`は各エポックの損失の記録、`summary.json`は停止理由、最良エポック、量子化誤差の検査の結果の記録である。
`config.json`は引数、入力の検査和、および再開の履歴を持ち、`latest.bin`と`best.bin`は再開用のチェックポイントである。
学習曲線は`training.jsonl`で確認し、教師値の損失、補助損失、検証損失、最良エポックを読み、更新回数の不足や過学習がないか点検する。
保存時には重みを1/8センチポーン単位へ量子化し、検証標本で浮動小数点評価との平均絶対誤差が2センチポーン以下であることを確かめる。
基準を超えた場合は、出力済みのファイルを残したまま`summary.json`に失敗を記録し、終了コード1で終える。

中断した学習は、`--resume data/<実験名>/training`を加えて同じ引数で再開できる。
再開では、学習の結果に影響する設定と入力が記録と一致しなければ拒否され、`--epochs`、`--patience`、`--threads`だけを変えられる。
既存の`--output-dir`は上書きしないので、条件を変えた再試行は新しいディレクトリへ書く。

## 5. PSTを診断する

学習した重みを、次のコマンドで基準PSTと比較する。

```bash
minase-trainer diag pst-compare \
  --data <既存データ...> data/<実験名>/generated-*.bin \
  --lambda 0.75 --lookahead-gamma 0.9 --lookahead-plies 40 \
  --base data/<実験名>/pst-base.bin \
  --candidate data/<実験名>/training/pst.bin \
  --candidate-float data/<実験名>/training/pst-float.npz \
  --output-dir data/<実験名>/diagnostics --sample-size 10000 --seed 1
```

教師の設定（`--lambda`と先読み）は学習と同じにする。
`--output-dir`には存在しないディレクトリを指定する。
結果は`diagnostics/report.json`に保存され、各欄の意味は`crates/minase-trainer/src/diagnostics/pst_compare.rs`の`Report`のdocコメントにある。
教師の分類と盤上総駒数の帯の組ごとの比較は`bands`に、全検証局面での終局結果への交差エントロピーと符号一致率は`outcome_metrics`に記録される。
診断は、主に次の場合に失敗する。

- 量子化誤差の平均絶対誤差が2センチポーンを超える。
- 標本局面、成り手の着手後局面、または駒除去局面で、独立に計算した整数のPST評価とエンジンの評価が一致しない。
- PST部分の駒除去差分に、厳密な符号反転が1件でもある。
- 候補の探索用駒価値が基準と異なる。

出力ディレクトリを作った後に失敗した場合も、その時点までの`report.json`と作成済みの資料は残り、`report.json`の`error`に理由が記録され、終了コードは1になる。
失敗しなかった場合も、基準と候補の`derived_piece_values`を見て、駒損を有利と評価するなどの異常がないかを確認する。
検証損失、教師誤差、および対局結果への交差エントロピーの改善だけを、棋力が向上した根拠にはしない。
採否は自己対局の測定で決める。
本学習後の診断が失敗した場合は、測定へ進まずに原因の調査へ戻り、診断に合わせて係数や別のエポックを選び直すことはしない。

## 6. FMの補正を学習する

FMの補正は、PSTを固定してFMだけを学習する`fm train`か、PSTとFMを同時に学習する`joint train`で学習する。
次の例は、現行の重みに至った手順である。
まず[補正1/4のFMの再学習](../plans/archived/fm-quarter-current-pst.md)の候補Faを`fm train`で学習し、その補正を`fm quarter`で1/4にした。
次に、その重みと、学習したPSTとFMの実数の重みを初期値にして、[PSTとFMの同時学習](../plans/archived/pst-fm-joint-training.md)のJ75を`joint train`で学習した。

```bash
minase-trainer fm train \
  --data <既存データ...> data/<実験名>/generated-*.bin --init data/<実験名>/training/pst.bin \
  --output-dir data/<実験名>/fm \
  --epochs 200 --patience 3 --lambda 1.0 \
  --lookahead-gamma 0.9 --lookahead-plies 40

minase-trainer fm quarter --input data/<実験名>/fm/fm.bin --output data/<実験名>/fm-quarter.bin

minase-trainer joint train \
  --data <既存データ...> data/<実験名>/generated-*.bin --base data/<実験名>/fm-quarter.bin \
  --pst-init data/<実験名>/training/pst-float.npz --fm-init data/<実験名>/fm/fm-float.npz \
  --fm-reference data/<実験名>/fm/fm.bin \
  --output-dir data/<実験名>/joint --lambda 0.75 \
  --lookahead-gamma 0.9 --lookahead-plies 40
```

`fm train`の出力は、PSTの部分が`--init`と同一でFMを持つMNPTの`fm.bin`と、キー`V`、`a`、`mask`を持つ`fm-float.npz`である。
FMの成分の数は、エンジンの重みファイルの読み込み処理に合わせて32に固定されている。
`fm train`の`summary.json`の`status`が`excluded_zero_correction`の場合は、補正を1/4にすると整数の補正がすべての局面で0になるので、その重みを`fm quarter`へ進めない。
`fm quarter`は、FMの補正を1/4にした重みと、`.quarter.json`で終わる報告を書く。
指数が上限の30に達した重みは、`fm quarter`が拒否する。
`joint train`は、学習の開始時に、`--pst-init`を量子化すると`--base`のPSTに、`--fm-init`を量子化すると`--fm-reference`のFMに一致することを検査する。
出力は、FMを持つMNPTの`pst.bin`と、キー`middlegame`、`endgame`、`V`、`a`、`mask`を持つ`pst-float.npz`である。

補正に掛ける倍率は学習の損失では決まらないので、PSTまたはFMを学び直したら、[SPSAの手引き](spsa.md)の手順で、重みを固定した調整用ビルドの係数`FmScale`だけをSPSAで調整する（先例は[FMの補正の倍率のSPSAによる調整](../plans/archived/fm-scale-spsa.md)）。
得た倍率は、次のコマンドで重みファイルへ焼き込んでから段階ゲート（棋力測定）にかける。

```bash
minase-trainer fm scale --input data/<実験名>/joint/pst.bin --float data/<実験名>/joint/pst-float.npz \
  --fm-scale <倍率×1024> --output data/<実験名>/pst-scaled.bin --data <既存データ...> data/<実験名>/generated-*.bin
```

`--float`には、PSTの2端点と`V`、`a`を持つNPZが必要であり、`joint train`の`pst-float.npz`を使える。
`fm train`の`fm-float.npz`はPSTを持たないので使えない。
`fm quarter`と`fm scale`は、出力の重みや報告のファイルが既にあると拒否する。
`--fm-scale`には0から1,024までの整数を与え、`--fm-scale 0`ではFMを持たない（FM節の次元が0の）重みを書き出す。
`--data`を与えると、検証標本での整数化の誤差を`.scale.json`の報告に加える。
現行の`crates/minase/nets/pst.bin`は、J75の出力に`--fm-scale 611`を適用したものである。

## 7. 候補を作って採否を測定する

診断を確認した後、基準コミットから候補ブランチを作り、学習した重みを候補の `crates/minase/nets/pst.bin` へコピーする。
対局時の動作の変更は重みに限定し、ほかの探索改良は混ぜない。

`crates/minase/src/eval/pst/tests.rs` の `embedded_pst_matches_python_initial_position_evaluation` は、初期局面の評価を固定値で検査しているので、重みを替えると失敗する。
その期待値を、新しい重みでの初期局面の評価へ更新する。
更新する値は、診断の`report.json`の`representatives`にある初期配置の候補評価と、学習の`summary.json`の`initial_position_score`の2つで独立に確かめる。
候補で `cargo test --locked` を実行し、重みと参照値の更新をコミットする。
ほかのテストが失敗した場合は、期待値を一括変更せず原因を調べる。

棋力測定は、[sprt.md](sprt.md#機能採否の段階ゲートstcとltc) の段階ゲートで、候補コミットを新、基準コミットを旧として行う。
採否の条件と判定保留の扱いも同書に従い、検証損失による代替は行わない。
固定ノード対局とbenchは診断用途とし、生成や学習と同時に時間制御の対局を走らせない。

## 保存物と完了時の記録

`data/<実験名>` には次の保存物が残る。
大容量のデータを移動する場合も、診断や再開が済むまでは元のパスを維持する。

| 保存物 | 用途 |
|---|---|
| `pst-base.bin` | 学習の初期重みを保持する。 |
| `generated-*.bin`、`generate-*.log`、`inspect-*.log` | 生成データと、生成と検査のログを保持する。 |
| `training/`、`fm/`、`joint/` | 学習の条件と入力の識別（`config.json`）、各エポックの記録、重み、量子化前の重み、チェックポイント、停止理由と検査の結果を保持する。 |
| `diagnostics/` | 基準との比較結果、教師の分類と盤上総駒数の帯ごとの診断用標本、標本と代表局面のMNSDファイルを保持する。 |

生成と学習の結果を `docs/measurements/<実験名>-training.md` にまとめ、設計書からリンクする。
記録には、学習器のコミット、実行したコマンドライン全体、入力の学習データのパスと記録数、および重みのSHA-256を書く。
対局測定は、短時間条件のSTCと長時間条件のLTCを別の測定名で記録し、`sprt.md` の記録項目を満たす。
採用または不採用が決まったら、設計書とROADMAPの状態を更新する。
採用した場合は、完了した設計書を凍結する前に、採った方式と棄却した代案を[ADR](../adr/README.md)に記録する。

実験を終了して生成用のworktreeを片付ける際は、診断と必要な記録の保存を済ませてから次を実行する。

```bash
git worktree remove data/<実験名>/generator
```

実験ディレクトリを先に削除するとworktreeの登録が残るため、必ずこの順序で片付ける。
