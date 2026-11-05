# 高速化の後に残る負荷の測定

## 目的

[教師生成の高速化](../../plans/teacher-generation-speedup.md)の候補に残る負荷を、旧版および基準版と比べて種類別と関数別に分け、まだ減らせる箇所があるかを判断する。

## コマンドライン

種類別の内訳は、[探索結果を変えない高速化の測定](teacher-generation-speedup-engine.md)の付属資料の`ecore_cmp.py`で採取した。
12個のE-coreで12局を同時に動かし、3版の実行順序を入れ替えながら各3回実行した。

```console
python3 ecore_cmp.py --cpus 8-19 --concurrency 12 --rounds 3 --max-ply 300 --cwd <固定のworktree> \
  --events 'cpu_atom/cycles/u,cpu_atom/instructions/u,cpu_atom/branch-misses/u,cpu_atom/branch-instructions/u,cpu_atom/topdown-retiring/u,cpu_atom/topdown-bad-spec/u,cpu_atom/topdown-fe-bound/u,cpu_atom/topdown-be-bound/u,cpu_atom/mem_bound_stalls_load.all/u,cpu_atom/mem_bound_stalls_ifetch.all/u' \
  old=<旧版> base=<基準版> int4=<候補>
```

細目の測定では、同じスクリプトに`--rounds 1`を与え、旧版と候補について、`topdown_fe_bound`、`topdown_be_bound`、`topdown_bad_speculation`、`mem_bound_stalls_load`、および`br_misp_retired`の各イベントを3組に分けて1回ずつ採取した。
イベントの数がカウンタの数を超えるので、値は多重化の補正を受けている。

性能コアの内訳は次の形で、候補と旧版を1回ずつ採取した。

```console
taskset -c 3 perf stat -x, \
  -e '{cpu_core/slots/u,cpu_core/topdown-retiring/u,cpu_core/topdown-bad-spec/u,cpu_core/topdown-fe-bound/u,cpu_core/topdown-be-bound/u,cpu_core/topdown-heavy-ops/u,cpu_core/topdown-br-mispredict/u,cpu_core/topdown-fetch-lat/u,cpu_core/topdown-mem-bound/u}' \
  -e cpu_core/cycles/u,cpu_core/instructions/u,cpu_core/branch-misses/u -- \
  <バイナリ> data selfplay generate --output <出力> --seed 9010200000 --games 2 --concurrency 1 \
  --nodes 20000 --random-moves 0 --max-ply 200 --hash-mb 16 --allow-dirty
```

関数別の内訳は、フレームポインタを残した候補のビルドを次の形で採取し、付属資料の`inline_prof.py`でインライン展開まで解決して集計した。
イベントと周期の組は、`cpu_atom/cycles/u`と4,000,037、`cpu_atom/branch-misses/u`と40,009、`cpu_atom/mem_bound_stalls_load.llc_miss/u`と1,000,003、および`cpu_atom/mem_bound_stalls_load.l2_hit/u`と700,001である。

```console
taskset -c 8-19 perf record --user-callchains --call-graph fp -e <イベント> -c <周期> -- \
  <バイナリ> data selfplay generate --output <出力> --seed 9010200000 --games 12 --concurrency 12 \
  --nodes 20000 --random-moves 0 --max-ply 300 --hash-mb 16 --allow-dirty
```

`cpu_atom/topdown_fe_bound.all/u`も同じ形で試したが、標本が得られなかった。
そのため、命令フェッチの不足がどの関数で起きるかは分かっていない。
集計の全行は[付属資料](teacher-generation-speedup-remaining-load/)にある。

## エンジン

旧版は試走のコミット`9f393ecc`、基準版はmasterのコミット`9380aa7f`、候補はブランチ`teacher-speedup`のコミット`6988af2b`であり、いずれも各コミットのネットを埋め込んでいる。
関数別の採取には、候補へ`-C force-frame-pointers=yes`を足したビルドを使った。
規則セットは`L0,P0,R1,E0`である。

## 環境

測定機はIntel Core Ultra 7 265KF（性能コア8、高効率コア12、スレッドは計20）で、Linux `6.18.57-2-lts`、Rust `1.98.0`である。
探索ワーカーは1局あたり1、置換表は16 MiBである。
E-coreの測定は論理CPU 8〜19に、性能コアの測定は論理CPU 3に固定した。

2026年11月5日の7時46分から7時54分に測定した。
同じ機械で別の計画の学習（`nice 19`、4スレッド）が動いており、lishogi Botの状態は確かめていない。
候補の探索速度が[以前の測定](teacher-generation-speedup-engine.md)より低いのは、この影響かもしれない。
割合の比較を目的とする測定なので、そのまま記録する。

## 結果

### 種類別の内訳

下表は、E-coreの3回の中央値を探索ノードあたりで示す。

| 指標 | 旧版 | 基準版 | 候補 |
|---|---:|---:|---:|
| 探索速度 | 9,237,258ノード/秒 | 4,248,977ノード/秒 | 7,122,393ノード/秒 |
| 命令数 | 16,607.1 | 31,429.4 | 17,669.7 |
| サイクル数 | 4,585.3 | 10,128.0 | 6,871.7 |
| 分岐命令数 | 2,363.7 | 4,081.4 | 2,287.4 |
| 分岐予測ミス | 17.52 | 41.72 | 36.44 |
| ロードの待ち | 699.1 | 1,923.1 | 1,705.8 |
| 命令フェッチのメモリ待ち | 4.6 | 27.2 | 30.7 |

候補の命令数は旧版の1.064倍まで戻ったが、サイクル数は1.499倍である。
分岐予測ミスの率は、旧版の0.74%に対して候補は1.59%であり、基準版の1.02%より高い。

CPUの命令発行の枠（1サイクルあたり8個）を、命令の完了（retiring）、分岐予測ミスなどで捨てた分（bad speculation）、命令の供給不足（frontend bound）、および実行資源やメモリの待ち（backend bound）の4種類に分けると、次のとおりである。
値は枠の数を8で割ってサイクルに換算した。

| 種類 | 旧版 | 基準版 | 候補 | 候補の割合 | 候補−旧版 |
|---|---:|---:|---:|---:|---:|
| 命令の完了 | 2,173 | 4,105 | 2,302 | 33.3% | +129 |
| 分岐予測ミスなど | 574 | 1,454 | 1,132 | 16.4% | +558 |
| 命令の供給不足 | 367 | 929 | 1,090 | 15.8% | +723 |
| 実行資源やメモリの待ち | 1,490 | 3,694 | 2,386 | 34.5% | +897 |
| 計 | 4,605 | 10,182 | 6,911 | 100% | +2,306 |

候補は基準版より命令が43.8%少ないのに、命令の供給不足は基準版より大きい。

### 内訳の細目

下表は、E-coreで1回ずつ採取した細目である。
ロードの待ちは実際のサイクル数、予測ミスの回数はノードあたりの回数、ほかは枠の数を8で割ったサイクル換算である。

| 細目 | 旧版 | 候補 |
|---|---:|---:|
| ロードの待ちのうち、最終キャッシュのミス | 303 | 1,024 |
| ロードの待ちのうち、L2のヒット | 391 | 655 |
| ロードの待ちのうち、最終キャッシュのヒット | 8 | 55 |
| 実行待ちのうち、メモリのスケジューラ | 203 | 686 |
| 実行待ちのうち、メモリ以外のスケジューラ | 774 | 1,126 |
| 実行待ちのうち、レジスタ | 169 | 358 |
| 供給不足のうち、デコード | 276 | 635 |
| 供給不足のうち、命令キャッシュ | 21 | 129 |
| 供給不足のうち、分岐の再誘導と検出 | 30 | 109 |
| 供給不足のうち、プリデコード | 10 | 73 |
| 分岐予測ミスの無駄 | 523 | 1,043 |
| マシンクリアの無駄 | 52 | 97 |
| 条件分岐の予測ミスの回数 | 15.62 | 34.38 |
| 間接分岐の予測ミスの回数 | 1.82 | 2.15 |

旧版との差が最も大きい細目は、最終キャッシュのミスの待ちである（+721サイクル）。

性能コアでの内訳は次のとおりである（2局、200手、750万ノード、1回）。

| 種類 | 旧版 | 候補 | 旧版の割合 | 候補の割合 |
|---|---:|---:|---:|---:|
| 命令の完了 | 1,968 | 2,123 | 48.6% | 35.1% |
| 分岐予測ミスなど | 365 | 729 | 9.0% | 12.0% |
| 命令の供給不足 | 699 | 1,446 | 17.3% | 23.9% |
| 命令の供給不足のうちフェッチの遅延 | 238 | 629 | 5.9% | 10.4% |
| 実行資源やメモリの待ち | 1,017 | 1,757 | 25.1% | 29.0% |
| 実行待ちのうちメモリの待ち | 302 | 760 | 7.5% | 12.5% |

サイクル数は旧版が4,048.7、候補が6,056.6、命令数は旧版が16,923.3、候補が18,375.9、分岐予測ミスは旧版が15.8回、候補が33.3回である。
命令の供給不足の増加は、μopキャッシュを持たないE-coreだけの現象ではなく、性能コアでも起きている。

### 関数別の内訳

サイクル数の自己時間の上位は次のとおりで、最大でも7.48%である。

| 関数 | 自己時間 |
|---|---:|
| `tt::unpack_critical`（置換表の読み出し） | 7.48% |
| `MovePicker::next` | 5.04% |
| `PairwiseL16L32::output_shared_256` | 4.50% |
| `ContinuationHistory::read` | 4.38% |
| `Bitboard::pop_lsb` | 3.90% |
| `Bitboard::intersects` | 3.31% |
| `Network::add_hidden` | 2.69% |
| `PairwiseL16L32::hidden2_sum` | 2.58% |
| `Searcher::negamax` | 2.40% |
| `AttackTables::ray` | 1.88% |

分岐予測ミスのうち各関数の自身で起きた割合の上位は、`MovePicker::next`が12.51%、`Bitboard::pop_lsb`が8.64%、`Searcher::negamax`が5.34%、`Move`の比較が4.95%、`Position::piece_at`が3.95%、`Square::new`が2.87%である。

最終キャッシュのミスの待ちの標本は、次のように分かれた。
この種のイベントの標本は、待ちの原因のロードより後の命令に付くことがある。
`corrected_static_eval`は足し算と範囲の制限だけの関数なので、補正表の読み出しの待ちが付いたものと解釈し、下表ではそのように分類した。

| 処理 | 割合 | 集計に含めた関数 |
|---|---:|---|
| 補正表 | 24.8% | `corrected_static_eval`、`correction_from_weighted_sum`、`correction_weighted_sum_at` |
| 置換表の照会 | 19.2% | `TranspositionTable::probe` |
| NNUEの累算値 | 17.3% | `evaluate_at`（`ensure_accumulator`と累算値の複写を含む） |
| 静かな手の採点 | 16.7% | `quiet_move_ordering_value`（continuation historyの読み出しを含む） |

### 生成機との関係

生成機では測っていない。
生成機での教師生成の速さの測定（ブランチ`nnue-generation-reset`の`docs/measurements/nnue-generation-reset/nnue-generation-reset-generator-speed.md`）によれば、基準版より後の`6f874692`の探索速度は、同時対局数を96から144へ増やしても2,135万から2,324万ノード/秒へ約9%しか伸びない。
また、同時384局を、NUMAノードごとに同時192局を動かす2プロセスへ分けると約9%伸びる。
これは、共有のキャッシュとメモリの競合が生成機で強いことを示唆するが、メモリの待ちの割合は測っていない。

## 結論

候補には、自己時間が8%を超える関数がない。
旧版との差は命令数ではなく、実行資源やメモリの待ち（+897サイクル）、命令の供給不足（+723サイクル）、および分岐予測ミス（+558サイクル）にある。
最終キャッシュのミスの待ちは補正表、置換表、累算値、および静かな手の採点に分かれ、大きな表のランダムアクセスが中心なので、探索木を変えずに減らしにくい。
探索結果を変えない変更で残る余地は、1件あたり1%前後の改善の積み重ねになると見ている。
命令の供給不足の原因は確かめておらず、生成機でのメモリの待ちの割合も未測定である。
