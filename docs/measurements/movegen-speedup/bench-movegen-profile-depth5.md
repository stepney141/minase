# 合法手生成高速化の基準プロファイルと試作のbench NPS（深さ5）

## 目的

現行の探索で実行時間を占める関数を関数別に測り、合法手生成高速化の着手順を決める根拠にする。あわせて、利き逆引きを片側化した試作がノード数を変えずに探索速度をどれだけ上げるかを確認する。

## コマンドライン

プロファイルは、frame pointerを有効にした別ターゲットディレクトリのビルドで次のとおり取った。速度測定には使わない。

```console
RUSTFLAGS="-C target-cpu=native -C force-frame-pointers=yes" CARGO_TARGET_DIR=<別ディレクトリ> cargo build --release --bin bench
perf record -F 2000 --call-graph fp -o perf.data -- <別ディレクトリ>/release/bench --depth 5 --threads 1 --repetitions 1
perf report -i perf.data --no-children --stdio -g none
perf report -i perf.data --children --stdio -g none
```

速度は、通常の`cargo build --release --bin bench`（`.cargo/config.toml`の`target-cpu=native`と`Cargo.toml`のLTOが効く）で作った`target/release/bench`を各版で次のとおり実行した。

```console
target/release/bench --depth 5 --repetitions 3 --threads 1
```

## エンジン

基準はコミット`0e3dfa6b59c489049c15e2cd00455e190a3fbf52`（master）であり、`git archive`で展開した別ディレクトリでビルドした。試作は同コミットに対して`Position::attackers_to`を片側化し、走りの逆引きを方向主導に、特殊利きの走査を獅子・角鷹・飛鷲に限定したブランチ`movegen-speedup`の作業ツリーである。`bench`は規則セット`engine-default`を固定で使う。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）であり、2026年9月15日に他の対局を走らせずに1スレッドで測った。置換表は既定の容量を1個使い回し、局面ごとの`clear()`は計測区間外である。perfはこのCPUの性能コア（`cpu_core`）と高効率コア（`cpu_atom`）の標本を別集計で報告する。高効率コアの標本は起動時の利き表構築と重みの復号だけで全体の約4%なので、以下の割合は性能コアの標本を分母とし、計測区間外の置換表クリアと起動を含む。

## 結果

基準の関数別の自己時間は次のとおりである。`bench::main`から呼ばれるlibcの未解決シンボル（置換表クリアの`memset`に相当）が4.4%を占める。

| 関数 | 自己時間 | 主な呼び出し元 |
|---|---|---|
| `ordinary_attackers_to` | 23.2% | 静止探索の静的交換評価 |
| `piece_control_without_special` | 13.2% | `generate_captures`と足の判定 |
| `MoveGenerator::generate_captures`（自己） | 9.9% | 静止探索 |
| `TranspositionTable::probe` | 9.3% | 主探索と静止探索 |
| `special_attackers_to` | 6.7% | 静止探索の静的交換評価 |
| `AttackTables::sliding_control` | 5.8% | 利き計算 |
| `Pst::update_accumulator_after_move` | 3.5% | 着手適用 |
| `Searcher::quiesce`（自己） | 2.9% | 主探索 |
| `Position::captured_squares` | 2.4% | 規則判定と整列 |
| `MoveRules::special_move_is_legal` | 1.9% | 捕獲生成 |
| `Searcher::negamax`（自己） | 1.7% | 主探索 |
| `generate_piece_moves::<false>` | 1.6% | 主探索の全手生成 |
| `move_order_key` | 1.3% | 捕獲手の整列 |
| `RawVec::grow_one`と`finish_grow` | 1.0% | 生成バッファの確保 |

包含時間（同じ分母）では、`quiesce`配下が76.2%、`generate_captures`配下が31.2%、主探索の`generate_moves::<false>`配下が5.1%であった。静的交換評価の利き逆引き（`ordinary_attackers_to`と`special_attackers_to`）の自己時間は合計29.8%である。

基準と試作のbench NPSは次のとおりである。総ノード数は一致する。局面別の最善手と探索値の比較経路は未実装のため、その一致は未確認である。

| 版 | ノード数 | 経過（中央値） | NPS（中央値） |
|---|---|---|---|
| 基準 | 1,268,805 | 0.845秒 | 1,501,790 |
| 試作（片側化、特殊利きの限定、方向主導の逆引き） | 1,268,805 | 0.639秒 | 1,986,936 |

試作はNPSが32.3%増えた。

試作後の関数別の自己時間は、`piece_control_without_special`が16.9%、`generate_captures`が13.2%、`TranspositionTable::probe`が11.1%、`attackers_to_by`が10.3%、`sliding_control`が7.6%であった。包含時間では`quiesce`配下が67.8%、`generate_captures`配下が40.5%、`generate_moves::<false>`配下が6.1%である。

## 結論

静的交換評価の利き逆引きが最大の負荷であり、片側化と特殊利きの限定を中心とする試作で総ノード数を変えずにNPSが約32%上がる。次の負荷は静止探索の捕獲生成であり、試作後は`generate_captures`配下が包含時間の4割を占める。走り利きの計算（`piece_control_without_special`と`sliding_control`）は逆引きと捕獲生成の両者に共通する。
