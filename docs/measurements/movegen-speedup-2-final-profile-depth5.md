# 第2期高速化の最終の採用版の残存負荷（bench深さ5）

## 目的

第2期の最終の採用版（コミット`24ddf4f`）について、関数別の自己時間と静止探索ノードの終わり方の件数を測り直し、次期の高速化の起点として記録する。

## コマンドライン

プロファイルは、通常のrelease設定にフレームポインタだけを加えたビルド（`RUSTFLAGS="-C target-cpu=native -C force-frame-pointers=yes"`、別ターゲットディレクトリ）で取った。[起案時の診断](movegen-speedup-2-diagnostics-depth5.md)と異なり`#[inline(never)]`は付けていないので、静止探索の初期化、グループ構築、`next`は`quiesce`へ、手選択器は`negamax`へインライン化されて集計される。

```console
perf record -F 6000 --call-graph fp -o perf.data target/prof/release/bench --depth 5 --threads 1 --repetitions 2
perf report --no-children --sort dso,symbol -g none
perf stat -e cycles,instructions,LLC-load-misses target/prof/release/bench --depth 5 --threads 1 --repetitions 1
```

測定機はハイブリッド構成（性能コアと効率コア）なので`perf report`は事象ごとに2つの表を出す。効率コアの標本は97.6%が`bench::main`から呼ばれた`libc`（計測区間外の置換表クリア）であり、性能コアの表を集計に使った。性能コアの表でも`libc`の11.7%は`bench::main`から呼ばれた置換表クリアなので除き、残りを100%として正規化した。

件数は、静止探索の入口に一時的なカウンタを加えた別ターゲットディレクトリのビルドで`bench --depth 5 --threads 1 --repetitions 1`を1回実行して数えた。

## エンジン

最終の採用版であるコミット`24ddf4f`（段階1、2、4、5、6、7、8、9を含む）である。`bench`は規則セット`engine-default`を固定で使い、総ノード数は1,201,518である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、rustc 1.98.0であり、2026年9月17日に他の負荷を走らせずに1スレッドで測った。

## 結果

### 関数別の自己時間

性能コアの探索中の標本に対する自己時間（置換表クリアを除いて正規化）は次のとおりである。

| 関数 | 自己時間 | 含まれるインライン化された処理 |
|---|---:|---|
| `Searcher::quiesce` | 27.7% | 静止探索の初期化（相手駒の走査、特殊候補の構築）、グループ構築、`next`、SEE本体 |
| `MoveGenerator::ordinary_capturer` | 11.9% | 到達範囲の検査、利き線の事前選別、走り計算 |
| `Position::attackers_to_by` | 11.3% | SEEの逆引き（近傍走査、走りの逆引き、特殊利き） |
| `TranspositionTable::probe` | 10.5% | 主探索と静止探索の照合 |
| `Searcher::negamax` | 9.2% | 手選択器、着手順、枝刈り |
| `piece_control_without_special` | 3.3% | 主探索の生成と特殊駒の候補 |
| `Pst::update_accumulator_after_move` | 3.0% | |
| `Position::captured_squares` | 2.6% | |
| `push_with_promotion` | 2.3% | 公開生成の成り展開 |
| `Position::make_move_with_captures_unchecked` | 2.1% | |
| 特殊駒の捕獲生成（`generate_special_piece_captures`） | 1.9% | |
| `MoveRules::special_move_is_legal` | 1.7% | |
| `MoveRules::promotion_choice_for` | 1.4% | |
| `Searcher::enter_node` | 1.4% | |
| `QsearchBuffers::initialize`の閉包 | 1.1% | |
| `Position::remove_piece_without_hash` | 1.1% | |
| 非捕獲手の整列（`quicksort`） | 0.9% | |

ハードウェアカウンタ（`perf stat`、置換表クリアを含む）では、性能コアで命令数9.16G、サイクル2.45G、IPC約3.7、LLCロードミス39.2万件であり、起案時（命令数11.9G、サイクル3.47G、IPC約3.4、LLCロードミス78.8万件）から命令数は23%、LLCロードミスは50%減った。

### 静止探索ノードの終わり方

| 終わり方 | 件数 | 割合 |
|---|---:|---:|
| 静的評価で打ち切り（置換表に触れない） | 228,842 | 32.7% |
| 候補が空（置換表に触れない） | 169,811 | 24.3% |
| 置換表を照合 | 300,734 | 43.0% |
| うち照合が的中 | 26,097 | 照合の8.7% |
| うち置換表で打ち切り | 23,470 | 照合の7.8% |
| うち合法な捕獲として残る置換表の手あり | 2,217 | 照合の0.7% |

静止探索ノード（`MAX_PLY`の判定を通過した699,387件）のうち置換表を照合するのは43.0%であり、起案時の全ノード照合（757,927件）から照合回数は60%減った。

## 結論

残存負荷の上位は静止探索の本体（初期化と候補処理を含む27.7%）、通常駒の捕獲可能升の計算（11.9%）、SEEの逆引き（11.3%）、置換表の照合（10.5%）、主探索の本体（9.2%）である。置換表の照合は回数を6割減らしたが照合1回あたりのDRAM待ちは残り、`unsafe`を許可しない方針のもとでは先読みによる短縮はできない。次期の候補は、静止探索の初期化における相手駒の走査と特殊候補の構築、SEE逆引きの走り部分、および主探索の手選択器である。
