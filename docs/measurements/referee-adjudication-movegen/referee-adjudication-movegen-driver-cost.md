# 審判層の合法手生成の削減による探索時間の変化

## 目的

[審判層の裁定が繰り返す合法手生成の削減の設計書](../../plans/archived/referee-adjudication-movegen.md)のフェーズ3として、審判層の変更の前後でMCTSの本番の駆動部を同じ局面と乱数で動かし、出力が時間の項目を除いて一致することと、再生1手あたりに正規化した探索時間の中央値が序盤と王駒の捕獲の近傍の両方で半分以下になることを確かめる。
再生とは、探索木が新しい節点を作るたびに、根の`Game`の複製へ経路の着手を`Game::play`で順に適用して審判層の裁定を得ることをいう。
以下では、変更前の版を比較元、変更後の版を候補と呼ぶ。

## コマンドライン

測定日は2026年10月28日である。
測定道具は[driver_cost.rs](../../../tools/train/zero/feasibility/src/bin/driver_cost.rs)であり、`minase::search::mcts::Driver`を一様評価器`UniformEvaluator`で駆動する。
条件は[監査対応の測定](../alphazero-audit-remediation/alphazero-audit-remediation-driver-cost.md)と同じであり、Gumbelは根の評価に加えて最大32回、候補数は最大16手、PUCTは根の評価を含めて最大800回、バッチは256葉、規則は全局面で`engine-default`とした。
本測定の前に、Gumbelが選んだ手と方策の目標、PUCTの最善手、および終局の勝者を出力へ加え、比較元と候補の両方を同じ`driver_cost.rs`でビルドした。

比較元は、ブランチの分岐点であるmasterのコミットを`git archive`で作業用の一時ディレクトリへ展開し、測定道具のソースだけを候補と同じものに差し替えてビルドした。
本体のリポジトリと他のworktreeは変更していない。
ビルドはどちらもworktreeを作業ディレクトリとして実行し、同じ`.cargo/config.toml`（`target-cpu=native`）を適用した。
交互測定のスクリプト`run_rounds.py`は、比較元と候補を同じCPUで交互に5回ずつ`nice -n 19 taskset -c 2 <実行ファイル> --seed 20261028 --games 32 --adjudication after`の形で実行し、各回の出力と実行環境を保存する。
`analyze.py`は、時間の項目（各探索の`elapsed_ms`、全体の`summary_ms`、および`snapshots`）を除いた出力が同じ版の各回の間、および比較元と候補の間で一致することを確かめ、一致しなければ集計せずに終了する。

```console
task_root=/home/stepney141/board-games/minase/data/worktrees/referee-adjudication-movegen
scratch=/tmp/claude-1000/-home-stepney141-board-games-minase/5126b35f-9772-4372-bd23-cf52c90eb44e/scratchpad
record="$task_root/docs/measurements/referee-adjudication-movegen/referee-adjudication-movegen-driver-cost"
feasibility=tools/train/zero/feasibility
cd "$task_root"
mkdir -p "$scratch/baseline-src"
git archive 480be8ec Cargo.toml Cargo.lock .cargo crates "$feasibility" | tar -x -C "$scratch/baseline-src"
cp "$feasibility/src/bin/driver_cost.rs" "$scratch/baseline-src/$feasibility/src/bin/driver_cost.rs"
nice -n 19 cargo build --release --offline --locked --manifest-path "$scratch/baseline-src/$feasibility/Cargo.toml" --bin driver_cost
nice -n 19 cargo build --release --offline --locked --manifest-path "$feasibility/Cargo.toml" --bin driver_cost
python3 -I "$record/run_rounds.py" --before "$scratch/baseline-src/$feasibility/target/release/driver_cost" \
  --after "$feasibility/target/release/driver_cost" --out "$scratch/record/rounds" --rounds 5 --cpu 2
python3 -I "$record/analyze.py" --before "$scratch"/record/rounds/before-{1..5}.json \
  --after "$scratch"/record/rounds/after-{1..5}.json > "$record/summary.json"
python3 -I "$record/compact.py" "$scratch/record/rounds" "$record/times.json"
```

生成の順序の比較では、候補のコミットを同じく一時ディレクトリへ展開し、[群を分けずに駒種順で生成する変更](referee-adjudication-movegen-driver-cost/kind-order-core.patch)を当ててビルドした版を比較元の位置に置き、同じスクリプトで候補と交互に5回ずつ実行した（出力先は`order-rounds`、集計は`order-summary.json`と`order-times.json`）。

詰み判定で試した候補手の数は、計測用のカウンタを入れた版で数えた。
カウンタは、詰み判定で逃れる手かどうかを調べた候補手ごとに1を足す大域変数であり、`driver_cost`が各探索の直前に0へ戻して直後に読む。
比較元には[比較元のカウンタ](referee-adjudication-movegen-driver-cost/count-base-core.patch)、候補には、候補が採る3群の順（手番側の王駒、王駒へ疑似利きを持つ相手駒の升へ疑似利きが届く駒、残りの駒の順に駒ごとに生成する順序）の[カウンタ](referee-adjudication-movegen-driver-cost/count-groups-core.patch)と[駒種順のカウンタ](referee-adjudication-movegen-driver-cost/count-kind-order-core.patch)を当て、3つとも[測定道具の変更](referee-adjudication-movegen-driver-cost/count-driver-cost.patch)を加えて、`nice -n 19 <実行ファイル> --seed 20261028 --games 32 --adjudication after`を1回ずつ実行した。
計測用の版からは回数だけを読み、時間は読まない。
集計は`python3 -I "$record/count_summary.py" base=<比較元> groups=<3群の順> kind_order=<駒種順>`で行った。

αβ探索は、次のコマンドで15局面の深さ6の結果を比較した。

```console
nice -n 19 cargo build --release --offline --locked --manifest-path "$scratch/baseline-src/Cargo.toml" -p minase --bin minase
nice -n 19 cargo build --release --offline --locked -p minase --bin minase
nice -n 19 "$scratch/baseline-src/target/release/minase" dev bench --depth 6 --threads 1 > "$record/alphabeta-before.log" 2>&1
nice -n 19 target/release/minase dev bench --depth 6 --threads 1 > "$record/alphabeta-after.log" 2>&1
```

命令数の内訳は、設計書の再現手順と同じく、valgrindが解釈できる命令に限った別のビルドで、探索関数の区間だけを集計した。

```console
RUSTFLAGS="-C target-cpu=x86-64-v2" cargo build --release --offline --locked \
  --manifest-path "$feasibility/Cargo.toml" --bin driver_cost --target-dir "$scratch/callgrind-target"
valgrind --tool=callgrind --collect-atstart=no --toggle-collect='driver_cost::search' \
  --callgrind-out-file="$record/callgrind-after.out" "$scratch/callgrind-target/release/driver_cost" \
  --seed 20261028 --games 32 --adjudication after
```

比較元も、展開したソースを`--manifest-path`に、別の`--target-dir`を指定して同じ手順で集計した（`callgrind-before.out`）。

## エンジン

比較元はmasterの`480be8ecd14e21a0b48d3fb4d55df4395f3c6feb`である。
候補は`referee-adjudication-movegen`の`abea37b7265dfc73907512e7583e908b4d5316b3`であり、これより後のブランチのコミットは、試験と文書だけを変更している。
測定道具のソースのSHA-256は`82a2164ec4be2c82851d419040e783c5e2cc2bf16fb58dba538b0f4863491e95`である。
交互測定に使った実行ファイルのSHA-256は、比較元が`d7f9a1935cb9f94638846469cd2b35ce9522e4397399b653d002b4ab49e339b9`、候補が`60e970502fa7a87498c4a9f73617d639f4031fdc6c7c5fcf6288a8b6366aac17`、駒種順の版が`c10c86ec547e2b0cae1ed776cb0fa2a0bb8c0ea63b302745a1cc0f541e2fe47c`である。
αβ探索の`minase`は、比較元が`f46e513f8e98c9ad7e6096bf10b94e98bce473da5a3b59d5ab033bab7232f0d5`、候補が`a8ee52dbc66773052e86a1d93fca13e1ae64dc0055f568d64b868d57054780eb`である。
各回の出力ファイルのSHA-256は`times.json`と`order-times.json`に記録した。

測定道具はfeasibilityのreleaseプロファイルを使い、デバッグ情報を残した。
アロケータはminaseの実行ファイルと同じmimalloc 0.1.52である。
GPUの推論は実行せず、一様な方策と勝率0.5を返す評価器を使った。

## 環境

Intel Core Ultra 7 265KFの20コア、20論理CPU、メモリ30 GiBの作業機で測った。
OSはArch Linux、カーネルは`6.18.57-2-lts`、Rustは1.98.0、LLVMは22.1.8、valgrindは3.25.1である。
測定は1プロセス、1スレッド、同時探索1局面で行い、CPU 2へ固定してnice値19を指定した。
探索木の上限は64 MiBであり、MCTSは`USI_Hash`を使わない。
αβ探索のbenchは1スレッドで、組み込みの評価関数と既定の置換表サイズを使った。

比較元と候補の交互測定は17:12:46〜17:13:13 JST、生成の順序の交互測定は17:13:51〜17:13:55 JSTに実行した。
最初の実行の開始時の負荷平均は、前者が1.49 / 3.20 / 4.22、後者が0.77 / 2.67 / 3.96であった。
これは順に1分、5分、15分の平均であり、直前のビルドの負荷を含む。
各実行の間のCPU全体の稼働率は5.7〜7.5%、固定したCPU 2の稼働率は97.6〜100.0%であった。
20 CPUのうち本測定の1 CPUが占める5%を除くと、ほかのCPU負荷は小さかった。
同じ作業機で動くlishogiのBotのエンジンは、測定の開始時に動いていなかった。
各実行の時刻、負荷平均、および稼働率は[交互測定の環境](referee-adjudication-movegen-driver-cost/environment.json)と[生成の順序の環境](referee-adjudication-movegen-driver-cost/order-environment.json)に記録した。

## 結果

### 出力の一致

入力は、監査対応の測定と同じ109局面（乱択序盤32、100手の履歴28、王駒捕獲の直前23、300手の履歴14、詰みの直前8、600手の履歴1、および監査の固定局面3）である。
時間の項目を除いた出力は、比較元の5回、候補の5回、および比較元と候補の間ですべて一致した。
一致した項目には、各局面の入力、完了回数、再生した着手数、経路の深さの分布、終局の理由と勝者、Gumbelが選んだ手と方策の目標、およびPUCTの最善手が含まれる。
探索量は、Gumbelが完了3,125回、再生4,460手、PUCTが完了56,695回、再生149,710手であり、審判層の終局で確定した葉はGumbelで23葉、PUCTで101葉だった。
これらは監査対応の測定の変更後の値と同じである。
生成の順序の比較でも、駒種順の版と候補の出力は時間の項目を除いて一致した。

### 再生1手あたりの探索時間

各（局面, 探索方式）について5回の中央値を取り、再生した着手数で割ってから、局面の分類ごとに中央値を取った。
分位点は昇順の第`ceil(p × n)`標本とした。
600手の履歴と固定局面はそれぞれ1標本なので、その値から一般的な分布を推定することはできない。

| 探索 | 局面の分類 | 局面数 | 根の探索時間の中央値 変更前 ms | 変更後 ms | 再生1手あたり 変更前 µs | 変更後 µs | 比 |
|---|---|---:|---:|---:|---:|---:|---:|
| Gumbel | 乱択序盤 | 32 | 0.435 | 0.100 | 9.063 | 2.074 | 0.229 |
| Gumbel | 100手の履歴 | 28 | 0.560 | 0.128 | 11.837 | 2.779 | 0.235 |
| Gumbel | 王駒捕獲の直前 | 23 | 2.512 | 0.147 | 73.546 | 3.469 | 0.047 |
| Gumbel | 300手の履歴 | 14 | 0.846 | 0.146 | 17.632 | 3.059 | 0.174 |
| Gumbel | 詰みの直前 | 8 | 0.596 | 0.048 | 39.690 | 7.397 | 0.186 |
| Gumbel | 600手の履歴 | 1 | 2.930 | 0.241 | 61.033 | 5.026 | 0.082 |
| Gumbel | 全体 | 109 | 0.554 | 0.121 | 12.294 | 2.836 | 0.231 |
| PUCT | 乱択序盤 | 32 | 19.272 | 3.045 | 8.525 | 1.359 | 0.159 |
| PUCT | 100手の履歴 | 28 | 22.977 | 3.464 | 11.542 | 1.739 | 0.151 |
| PUCT | 王駒捕獲の直前 | 23 | 4.008 | 0.172 | 103.883 | 3.811 | 0.037 |
| PUCT | 300手の履歴 | 14 | 22.519 | 3.489 | 15.243 | 1.734 | 0.114 |
| PUCT | 詰みの直前 | 8 | 0.460 | 0.061 | 54.108 | 5.516 | 0.102 |
| PUCT | 600手の履歴 | 1 | 149.128 | 7.541 | 70.677 | 3.574 | 0.051 |
| PUCT | 全体 | 109 | 19.174 | 2.989 | 11.542 | 1.739 | 0.151 |

PUCTの全体と100手の履歴は、同じ探索がどちらの群でも中央値の標本になったので、再生1手あたりの値が一致した。
固定局面3つの値は[集計](referee-adjudication-movegen-driver-cost/summary.json)にあり、比は0.45〜0.67である。
再生1手あたりの探索時間の中央値は、序盤の局面でGumbelの32回が変更前の0.229倍、PUCTの800回が0.159倍になり、王駒の捕獲の近傍でGumbelが0.047倍、PUCTが0.037倍になった。
各局面の5回の中央値を全局面で合計した探索時間は、Gumbelで130.04 msから12.44 ms（0.096倍）、PUCTで2144.72 msから237.44 ms（0.111倍）になった。

### 詰み判定で試した候補手の数と生成の順序

次の表は、詰み判定で逃れる手かを調べた候補手の数を、局面の分類ごとに合計して再生した着手数で割った値と、駒種順の版と3群の順の候補の再生1手あたりの探索時間（各局面の5回の中央値を、局面の分類ごとに集めた中央値）である。
駒種順は`MoveGenerator::generate_moves`と同じ順序であり、比較元の生成順序を保ったまま、候補と同じ打ち切りだけを入れた版に当たる。

| 探索 | 局面の分類 | 候補手の数 比較元 | 駒種順 | 3群の順 | 再生1手あたり 駒種順 µs | 3群の順 µs | 比 |
|---|---|---:|---:|---:|---:|---:|---:|
| Gumbel | 乱択序盤 | 1.000 | 1.000 | 1.000 | 1.965 | 2.070 | 1.053 |
| Gumbel | 100手の履歴 | 3.272 | 3.272 | 1.201 | 2.701 | 2.791 | 1.033 |
| Gumbel | 王駒捕獲の直前 | 20.622 | 20.622 | 1.896 | 5.585 | 3.465 | 0.620 |
| Gumbel | 300手の履歴 | 7.978 | 7.978 | 1.461 | 3.295 | 3.066 | 0.930 |
| Gumbel | 詰みの直前 | 9.822 | 9.822 | 4.528 | 10.128 | 7.418 | 0.732 |
| Gumbel | 600手の履歴 | 18.354 | 18.354 | 17.250 | 4.871 | 5.053 | 1.037 |
| Gumbel | 全体 | 6.667 | 6.667 | 1.595 | 2.815 | 2.847 | 1.011 |
| PUCT | 乱択序盤 | 1.000 | 1.000 | 1.000 | 1.288 | 1.355 | 1.052 |
| PUCT | 100手の履歴 | 2.028 | 2.028 | 1.182 | 1.665 | 1.751 | 1.051 |
| PUCT | 王駒捕獲の直前 | 30.539 | 30.539 | 2.804 | 6.759 | 3.806 | 0.563 |
| PUCT | 300手の履歴 | 4.390 | 4.390 | 1.445 | 1.853 | 1.729 | 0.933 |
| PUCT | 詰みの直前 | 12.177 | 12.177 | 6.433 | 6.240 | 5.468 | 0.876 |
| PUCT | 600手の履歴 | 22.540 | 22.540 | 20.896 | 3.576 | 3.580 | 1.001 |
| PUCT | 全体 | 2.480 | 2.480 | 1.436 | 1.681 | 1.751 | 1.042 |

駒種順の版の候補手の数は全分類で比較元と一致し、駒ごとの生成を駒種順に並べた順序が全件生成の順序と同じであることも確かめられた。
3群の順は、王駒の捕獲の近傍で試す候補手の数を、再生1手あたりGumbelで20.6から1.9へ、PUCTで30.5から2.8へ減らし、再生1手あたりの探索時間を駒種順の0.620倍と0.563倍にした。
一方、王駒が攻撃されていない局面の多い序盤と100手の履歴では、駒を3群へ分けるための利きの逆引きの分だけ3〜5%遅く、全局面の中央値でも1〜4%遅かった。
固定局面の詰みのGumbelのように、3群の順で候補手が増えた局面もある（1.271から1.917）。
全局面の探索時間の合計では、3群の順は駒種順の0.869倍（Gumbel、14.31 msから12.44 ms）と0.982倍（PUCT、242.02 msから237.75 ms）であり、設計書のとおり3群の順を採る。

### 命令数の内訳

探索関数の区間の命令数は、比較元が66.23 G命令、候補が5.91 G命令（0.089倍）だった。
`Game::play`の包含命令数は62.17 G（93.9%）から1.85 G（31.3%）へ減り、再生1手あたり（再生154,170手）では約403 k命令から約12.0 k命令になった。
設計書が単位にした合法手の全件生成1回は約37.7 k命令なので、変更後の`Game::play`の全体の費用は全件生成の約0.32回分に当たる。
全件生成`MoveGenerator::generate_moves`の包含命令数は60.26 G（91.0%）から2.24 G（37.9%）へ減り、変更後に残る全件生成はほぼすべて葉の`Game::legal_moves`（2.24 G、37.9%）である。
変更後の`Game::play`の内訳は、合法手なしと詰みの判定`stalemate_or_mate`が1.02 G（17.3%）、着手の合法性検査が0.32 G（5.4%）、駒枯れの判定が0.31 G（5.2%）、反復規則R1（RULES.md第31条、攻撃的着手を続けた側を負けとするLishogi式の4回反復の裁定）の攻撃的着手の判定が0.25 G（4.3%）である。
変更前の内訳は設計書の「目的」の節の表にある。
プロファイルの記録は[変更前](referee-adjudication-movegen-driver-cost/callgrind-before.out)と[変更後](referee-adjudication-movegen-driver-cost/callgrind-after.out)にあり、`callgrind_annotate --inclusive=yes`で表示できる。

### 機能の検証

αβ探索は、[変更前](referee-adjudication-movegen-driver-cost/alphabeta-before.log)と[変更後](referee-adjudication-movegen-driver-cost/alphabeta-after.log)で、15局面すべての深さ、ノード数、最善手、評価値、および累算器の更新回数が一致した。

現行の全件生成による判定を試験の中に複製した参照実装との突き合わせでは、乱数対局で10規則セットの330局、32,726手を比べた。
この対局には、王駒捕獲168件、詰み37件、合法手なし1件、駒枯れの勝ち16件と引き分け4件、および裸玉の勝ち30件の終局が含まれる。
独立した`/tmp`のソースの複製に代表的な誤り6種類を1つずつ入れ、minase-coreの単体試験がそのすべてで失敗することを確かめた。
6種類は、王駒が2枚の場合も疑似利きで捕獲可能とする誤り、R1の攻撃的着手の王駒の条件を外す誤り、動かした駒の捕獲でない手を攻撃的着手に数える誤り、駒枯れの猶予を疑似利きで判定する誤り、着手の合法性検査を省く誤り、および詰み判定で候補手の即時の勝ちと引き分けを逃れる手に数えない誤りである。
このうち駒枯れの猶予の誤りは、最初の実行では検出されなかった。
王駒だけの側の王駒が余分な獅子に疑似利きを持つのに、先獅子（RULES.md第15条第1項）のために取れない局面の固定例がなかったためであり、その固定例を加えた後は検出された。
結果は[誤りの検出の記録](referee-adjudication-movegen-driver-cost/mutation-results.json)にある。
これは代表的な6種類の検査であり、網羅的な変異テストではない。

マージ前の4つの検査（`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、および`cargo test -p minase --lib --features search-stats,tuning`）は、ブランチのコードと試験の最後のコミット`c72bc4e1`で成功した。

対局による棋力比較は行っていない。
裁定の結果とαβ探索の結果が変わらないので、Elo、SPRTの尤度比、破棄ペア数、および時間切れ件数は本測定の対象外である。

## 結論

裁定と探索の出力を変えずに、再生1手あたりに正規化した探索時間の中央値は、Gumbelの32回とPUCTの800回の両方で、序盤の局面で変更前の0.16〜0.23倍、王駒の捕獲の近傍の局面で0.04〜0.05倍になり、設計書の完了条件である半減を達成した。
