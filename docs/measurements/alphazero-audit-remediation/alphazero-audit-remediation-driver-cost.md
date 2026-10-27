# 探索木の終局判定共有に伴う費用

## 目的

[AlphaZero型探索の監査対応](../../plans/alphazero-audit-remediation.md)のフェーズ1として、審判層の終局裁定を共有したMCTSの本番駆動部を変更前と同じ局面と乱数で測り、経路再生の費用と探索量の変化を確かめる。

## コマンドライン

測定日は2026年10月28日である。
[driver_cost.rs](../../../tools/train/zero/feasibility/src/bin/driver_cost.rs)は、`minase::search::mcts::Driver`を`UniformEvaluator`で駆動する。
各局面で、Gumbelは根の評価に加えて最大32回、候補数は最大16手、PUCTは根の評価を含めて最大800回、バッチは256葉とした。
Gumbel乱数は自己対局と同じ53ビットの中点から作り、局面ごとの固定シードを変更前後で共有した。
成り規則と終局規則は全局面で`engine-default`とした。

変更前は、道具のソースとCargo.tomlを次の一時ディレクトリへ複製し、path依存だけをmasterのcrateへ向けた。
本体のリポジトリとgit管理領域は変更していない。
依存の版は`tools/train/zero/feasibility/Cargo.lock`で固定する。

```console
task_root=/home/stepney141/board-games/minase/data/worktrees/alphazero-audit-remediation
baseline=/tmp/claude-1000/-home-stepney141-board-games-minase/6df15eae-20d7-4230-8efc-cdb9a3906246/scratchpad/driver-cost-before
feasibility="$task_root/tools/train/zero/feasibility"
record="$task_root/docs/measurements/alphazero-audit-remediation/alphazero-audit-remediation-driver-cost"
cd "$task_root"
mkdir -p "$baseline/src/bin"
cp "$feasibility/src/bin/driver_cost.rs" "$baseline/src/bin/driver_cost.rs"
sed 's|../../../../crates/|/home/stepney141/board-games/minase/crates/|g' "$feasibility/Cargo.toml" > "$baseline/Cargo.toml"
cp "$feasibility/Cargo.lock" "$baseline/Cargo.lock"
nice -n 19 cargo build --release --offline --locked --manifest-path "$baseline/Cargo.toml" --bin driver_cost
nice -n 19 cargo build --release --offline --locked --manifest-path "$feasibility/Cargo.toml" --bin driver_cost
nice -n 19 taskset -c 2 "$baseline/target/release/driver_cost" --seed 20261028 --games 32 --adjudication before > "$record/before.json"
nice -n 19 taskset -c 2 "$feasibility/target/release/driver_cost" --seed 20261028 --games 32 --adjudication after > "$record/after.json"
```

`--adjudication`は測定後の集計にだけ使い、探索方式を切り替えるフラグではない。
変更前の駆動部でも同じソースをビルドでき、変更前に取得しない正確な終局の理由別件数と`Game::play`呼出し数は0として記録する。
この0は、規則上の終局が存在しなかったことを意味しない。

αβ探索は、次のコマンドで15局面の深さ6の結果を比較した。

```console
nice -n 19 cargo build --release --offline --locked --manifest-path /home/stepney141/board-games/minase/Cargo.toml --target-dir /tmp/phase1-before-target -p minase --bin minase
nice -n 19 cargo build --release --offline -p minase --bin minase
nice -n 19 /tmp/phase1-before-target/release/minase dev bench --depth 6 --threads 1 > "$record/alphabeta-before.log" 2>&1
nice -n 19 target/release/minase dev bench --depth 6 --threads 1 > "$record/alphabeta-after.log" 2>&1
```

## エンジン

比較元はmasterの`255fb48c59d1ee4da876b0c0c5d4f59de0d447d6`であり、測定前に作業ツリーが未変更であることを確認した。
候補は、本記録と同じコミットに含まれる変更後のソースであり、測定の時点では`alphazero-audit-remediation`の`ad8b5f0012f19e80cae13e0d770ce58e2944e233`に未コミットの差分を加えた状態であった。
この2コミット間に`crates/`の差分はない。
候補のソースは[ファイルごとのSHA-256](alphazero-audit-remediation-driver-cost/source-sha256.json)で識別する。
道具のSHA-256は`874b81325a5f29f7c0e627ca8702b2a236f442150cc8a612d8b2ba88d42e0ffd`である。

測定道具はfeasibilityのreleaseプロファイルを使い、デバッグ情報を残し、ワークスペースの`.cargo/config.toml`の`target-cpu=native`を適用した。
アロケータはminaseの実行ファイルと同じmimalloc 0.1.52である。
GPUの推論は実行せず、一様な方策と勝率0.5を返す評価器を使った。

## 環境

Intel Core Ultra 7 265KFの20コア、20論理CPU、メモリ30 GiBの作業機で測った。
OSはArch Linux、カーネルは`6.18.57-2-lts`、Rustは1.98.0、LLVMは22.1.8である。
測定は1プロセス、1スレッド、同時探索1局面で行い、CPU 2へ固定してnice値19を指定した。
探索木の上限は64 MiBであり、MCTSは`USI_Hash`を使わない。
αβのbenchは1スレッドで、組み込みの評価関数と既定の置換表サイズを使った。

変更前は04:14:48〜04:14:49 JST、変更後は04:14:49〜04:14:54 JSTに実行した。
開始時の負荷平均は、変更前が2.24 / 5.16 / 3.24、変更後が2.46 / 5.16 / 3.25であった。
これは順に1分、5分、15分の平均であり、直前のビルド負荷も含む。
測定中のCPU全体の稼働率は変更前5.91%、変更後5.78%、固定したCPU 2はそれぞれ100.00%、99.80%だった。
20 CPUのうち本測定の1 CPUが占める約5%を除くと、ほかのCPU負荷は小さかった。
時刻、負荷平均、CPU時間の読み取り値は[環境の記録](alphazero-audit-remediation-driver-cost/environment.json)に残した。

## 結果

### 入力と計測範囲

入力は、`datagen::opening::generate_opening`の8〜16手の乱択序盤から一様ランダムに進めた32対局と、監査報告の3つの固定局面から採った109局面である。
各局面の開始局面、実着手列、シード、履歴の長さ、および根の局面キーをJSONへ保存し、変更前後で全件一致することを確認した。
各探索の前に、その実着手列を`Game::play`で再生して根の`Game`を作った。
固定局面は明示的な開始局面と空の着手列を持ち、過去の履歴を推定しない。

| 入力の種類 | 局面数 |
|---|---:|
| 乱択序盤 | 32 |
| 100手の履歴 | 28 |
| 王駒捕獲の直前 | 23 |
| 300手の履歴 | 14 |
| 詰みの直前 | 8 |
| 600手の履歴 | 1 |
| 固定局面の駒枯れ勝ち | 1 |
| 固定局面の駒枯れ引き分け | 1 |
| 固定局面の詰み | 1 |

時刻の計測は`SearchSnapshot::from_game`の直前に始め、駆動部の作成、選択、一様評価、逆伝播、および最終手の選択までを含む。
Gumbelでは方策目標の計算も含む。
入力対局の再生、木の破棄、および測定後の統計集計は含めない。
各局面と各探索方式の実行は1回であり、分位点は繰り返し測定のばらつきではなく、局面間の分布を表す。
分位点は昇順の第`ceil(p × n)`標本とした。

回数の集計は、完了後の木をたどって行った。
容量不足、探索の取り消し、木の再利用がないため、根以外の作成済み節点の数が新しく分類した葉の数であり、その深さの総和が経路再生の着手数に等しい。
変更後は、再生した着手ごとに`Game::play`を1回呼ぶので、同じ総和が駆動部による裁定の呼出し数となる。
これはコードの契約から木の構造を使って算出した回数であり、実行時のカウンタを使った計測ではない。
反復の近似が攻撃側の判定のために行う追加の局面再生、入力を作る再生、および統計集計の再生は、この回数に含めない。
測定後の集計では、仮想損失が残っていないことと、正確な終局の節点が辺を持たないことも確認した。

### 根の探索時間

| 探索 | 版 | 局面数 | 中央値 ms | 90パーセンタイル ms | 99パーセンタイル ms |
|---|---|---:|---:|---:|---:|
| Gumbel | 変更前 | 109 | 0.094107 | 0.119720 | 0.131244 |
| Gumbel | 変更後 | 109 | 0.554948 | 3.136880 | 6.705520 |
| PUCT | 変更前 | 109 | 2.094241 | 2.822903 | 3.080619 |
| PUCT | 変更後 | 109 | 19.128315 | 29.554312 | 149.772073 |

中央値の比はGumbelで5.90倍、PUCTで9.13倍となった。
一方、正確な終局を早く認識すると探索量自体が減るので、要求した32回または800回だけを分母にして費用を比較することはできない。
次の表は、実際に完了した回数で正規化した値を併記している。

| 探索 | 版 | 完了回数 | 新規分類数 | 再生着手数 | Game::play呼出し数 | 探索時間の合計 ms | 完了1回あたり µs |
|---|---|---:|---:|---:|---:|---:|---:|
| Gumbel | 変更前 | 3,226 | 3,117 | 4,651 | 0 | 9.395 | 2.912 |
| Gumbel | 変更後 | 3,125 | 3,001 | 4,460 | 4,460 | 130.029 | 41.609 |
| PUCT | 変更前 | 59,410 | 58,672 | 160,617 | 0 | 175.494 | 2.954 |
| PUCT | 変更後 | 56,695 | 55,257 | 149,710 | 149,710 | 2145.660 | 37.846 |

入力の種類ごとの中央値は次のとおりである。
駒枯れ勝ちと固定詰みのPUCTでは、正確な裁定によって探索が早く終わった。
600手の履歴と固定局面はそれぞれ1標本なので、この値から一般的な分布を推定することはできない。

| 入力の種類 | Gumbel変更前 ms | Gumbel変更後 ms | PUCT変更前 ms | PUCT変更後 ms |
|---|---:|---:|---:|---:|
| 乱択序盤 | 0.072462 | 0.434996 | 2.100517 | 19.290139 |
| 100手の履歴 | 0.097689 | 0.558303 | 2.579291 | 22.922596 |
| 王駒捕獲の直前 | 0.110092 | 2.476738 | 0.129913 | 3.997579 |
| 300手の履歴 | 0.107045 | 0.862447 | 2.520633 | 22.442007 |
| 詰みの直前 | 0.063170 | 0.594103 | 0.148524 | 0.465160 |
| 600手の履歴 | 0.125504 | 2.929103 | 2.732000 | 149.772073 |
| 固定局面の駒枯れ勝ち | 0.028615 | 0.023193 | 0.899774 | 0.030468 |
| 固定局面の駒枯れ引き分け | 0.020813 | 0.042218 | 0.705089 | 0.349788 |
| 固定局面の詰み | 0.031942 | 0.135105 | 0.233694 | 0.015900 |

### 深さと終局

新しく分類した葉の深さの分布は次のとおりである。
根の初回評価と、すでに確定した葉の再訪問は数えない。

| 根からの深さ | Gumbel変更前 | Gumbel変更後 | PUCT変更前 | PUCT変更後 |
|---|---:|---:|---:|---:|
| 1 | 1,600 | 1,547 | 7,179 | 6,811 |
| 2 | 1,500 | 1,449 | 17,659 | 16,941 |
| 3 | 17 | 5 | 17,780 | 17,011 |
| 4 | 0 | 0 | 15,528 | 14,486 |
| 5 | 0 | 0 | 488 | 8 |
| 6 | 0 | 0 | 38 | 0 |

変更後に審判層の終局で確定した葉はGumbelで23葉、PUCTで101葉だった。
これとは別に、反復の近似による打ち切りが各1葉あった。
変更前に確定または引き分けとなった葉はそれぞれ16葉と63葉だが、正確な理由別の集計は取得せず、次の表では0としている。

| 正確な裁定理由 | Gumbel変更前 | Gumbel変更後 | PUCT変更前 | PUCT変更後 |
|---|---:|---:|---:|---:|
| 王駒捕獲の勝ち | 0 | 11 | 0 | 37 |
| 詰みの勝ち | 0 | 6 | 0 | 17 |
| 合法手なしの勝ち | 0 | 0 | 0 | 0 |
| 駒枯れの勝ち | 0 | 2 | 0 | 20 |
| 駒枯れの引き分け | 0 | 4 | 0 | 27 |
| 正確なR1の勝ち | 0 | 0 | 0 | 0 |
| 正確なR1の引き分け | 0 | 0 | 0 | 0 |
| 裸玉の勝ち | 0 | 0 | 0 | 0 |
| 裸玉の引き分け | 0 | 0 | 0 | 0 |

### スナップショット

履歴が0手、100手、300手、600手の対局について、構築と複製をそれぞれ1,000回測った。
値はµsであり、生成した写しの破棄を含む。
100手以降は、上の局面集でその長さに初めて到達した対局を使い、変更前後で同じ局面を測った。

| 履歴の手数 | 操作 | 変更前の中央値 | 変更前の90パーセンタイル | 変更前の99パーセンタイル | 変更後の中央値 | 変更後の90パーセンタイル | 変更後の99パーセンタイル |
|---|---|---:|---:|---:|---:|---:|---:|
| 0 | 構築 | 1.122 | 1.156 | 1.257 | 1.148 | 1.192 | 1.286 |
| 0 | 複製 | 0.044 | 0.057 | 0.071 | 0.080 | 0.091 | 0.115 |
| 100 | 構築 | 5.038 | 5.147 | 5.356 | 5.153 | 5.261 | 5.438 |
| 100 | 複製 | 0.058 | 0.070 | 0.085 | 0.122 | 0.147 | 0.171 |
| 300 | 構築 | 13.911 | 14.146 | 14.511 | 14.137 | 14.362 | 14.830 |
| 300 | 複製 | 0.074 | 0.081 | 0.090 | 0.253 | 0.265 | 0.282 |
| 600 | 構築 | 26.401 | 26.764 | 27.581 | 26.771 | 27.748 | 28.486 |
| 600 | 複製 | 0.091 | 0.096 | 0.104 | 0.473 | 0.491 | 0.508 |

600手の履歴では、スナップショット複製の中央値は0.091 µsから0.473 µsに増えた。
構築の中央値は26.401 µsから26.771 µsであり、今回の根の探索時間の増加をスナップショット構築の増分だけで説明することはできない。

### 機能の検証

αβ探索は、[変更前](alphazero-audit-remediation-driver-cost/alphabeta-before.log)と[変更後](alphazero-audit-remediation-driver-cost/alphabeta-after.log)で、15局面すべての深さ、ノード数、最善手、および評価値が一致した。
`cargo fmt --all --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test -p minase-core`、`cargo test -p minase --lib`、および`cargo test -p minase --bin minase -- zero_selfplay`が成功した。
coreは単体試験278件と統合試験2件、minaseのライブラリは894件、自己対局は19件が通った。
既存の無効化された試験はcoreが8件、minaseが1件である。
MCTS関連の109件はすべて通った。
Cargoが既存のcrate内releaseプロファイルを無視する通知は残るが、Clippyとコンパイラのコード警告はない。
測定道具単体のClippyも`-D warnings`で成功した。

独立した`/tmp`のソース複製では、勝者の視点の反転、正確な裁定への近似の印の付与、全応手の敗北に必要な近似依存の欠落、および審判状態の比較の欠落を個別に入れ、対応する試験が4件とも失敗することを確認した。
結果は[誤変更の検出記録](alphazero-audit-remediation-driver-cost/mutation-results.json)にある。
これは代表的な4種類の検査であり、網羅的な変異テストではない。

局面ごとの時間、完了回数、経路の深さ、再生量、および終局理由は[変更前の全結果](alphazero-audit-remediation-driver-cost/before.json)と[変更後の全結果](alphazero-audit-remediation-driver-cost/after.json)に保存した。
対局による棋力比較は行っていないため、Elo、SPRTの尤度比、破棄ペア数、および時間切れ件数は本測定の対象外である。

## 結論

終局判定の共有により今回の109局面では正確な終局を評価前に確定できたが、根の探索時間の中央値はGumbelで5.90倍、PUCTで9.13倍となったため、強化学習サイクルの所要時間はGPU推論を含む後続の試走で評価する必要がある。
