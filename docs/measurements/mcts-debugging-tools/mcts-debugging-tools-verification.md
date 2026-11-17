# PV-MCTSのデバッグ道具の実装の前後の出力の照合

## 目的

[PV-MCTSのデバッグ道具の設計書](../../plans/archived/mcts-debugging-tools.md)の「検証」節のうち、出力の一致で確かめる項目（2.〜5.）を確かめ、3つの道具を加えた後も、既定の設定での探索の結果が変わらないかを判定する。
3つの道具は、探索の終わりに根の候補手ごとの統計を出すUSIのオプション`VerboseMoveStats`、フィーチャ`invariants`の下で探索木の全節点を検査する`Tree::validate`、および`--search mcts`で起動したminaseの`bench`である。
探索が手を選ぶ処理と統計を更新する処理を変えない計画なので、対局による棋力の測定は行っていない。

## 条件

測定は2026年11月17日に、Intel Core Ultra 7 265KF（論理CPU 20）の作業機で行った。
基準は変更前のmasterのコミット`927faf03`、候補は各フェーズの終わりのコミットであり、フェーズ1が`f0c8a25a`、フェーズ2が`1f93185d`、フェーズ3が`3887e414`である。
どのコミットも、同じ機械でRust 1.98.0の通常のrelease設定（`cargo build --release -p minase`）でビルドし、フェーズ2と3では`--features invariants`を付けたreleaseビルドも作った。

学習済みのネット（以下、実ネット）には、2回目の強化学習の実行のサイクル10のネット（ネットワーク接続の記憶装置であるNASの`cycles-rl2-az/cycle-0010/train/model.onnx`）を使った。
SHA-256は`ba89c56cab8515a26ebcaed4cedf3b651bb263f28558016e25c957d72f6c0c4d`であり、NASの`nets/cycles-rl2-az/cycle-0010.onnx`と一致する。
ONNX Runtimeは`tools/train/zero/env.sh`が読み込む1.31.1を使い、推論の実行先をCPU（`NnBackend=cpu`）にして、エンジンを`taskset -c 0-3`で論理CPU 0〜3に固定した。

## コマンドライン

実ネットの出力は、次の命令列を1つのセッションで送って集めた。
`<SFEN>`は`crates/minase/src/diagnostics/bench.rs`の`BENCH_POSITIONS`の15局面の2欄（盤面と手番）の表記であり、`position sfen`では後ろに獅子を取った升（`-`）と手数（`1`）の欄を補って4欄にした。
`info`の行の思考時間`time`と毎秒のノード数`nps`の値を伏せてから、基準と候補を全行で比べた。

```console
usi
setoption name RuleSet value JCSA
setoption name NnModel value <実ネット>
setoption name NnBackend value cpu
isready
usinewgame
position sfen <SFEN> - 1      # 15局面のそれぞれについて
go nodes 800
position startpos [moves <それまでのbestmove>...]   # 初期局面から10手まで、1手ずつ
go nodes 800
quit
```

フェーズ3の候補では、同じ命令列の`isready`の前に`setoption name VerboseMoveStats value true`を足した実行も行い、`info string movestats`の行を除いた出力を基準と比べた。

PV-MCTSの`bench`は、次の3つの条件で、論理CPU 4に固定して実行した。
終了コードが0であること、`info string error:`の行がないこと、15局面の`info string position`の行と最後の`info string end`の行があることを確かめてから、経過時間`elapsed-ms`と毎秒のシミュレーション数`sps`の値を伏せて全行で比べた。

```console
printf 'bench nodes 1\nquit\n' | taskset -c 4 <minase> --protocol usi --search mcts
printf 'bench nodes 800 batch 1\nquit\n' | taskset -c 4 <minase> --protocol usi --search mcts
printf 'bench nodes 800 batch 256\nquit\n' | taskset -c 4 <minase> --protocol usi --search mcts
```

αβ探索の`bench`は、フェーズ1の候補のコミットを展開した作業ツリーで、`scripts/bench_compare.py`に基準のコミットを3つとも与えて比べた。
同じ2つのバイナリに`bench depth 5`を送った出力全体も、`elapsed=`と`nps=`の値を伏せて比べた。

```console
scripts/bench_compare.py --reference 927faf03 --baseline 927faf03 --parent 927faf03 --depth 5 --threads 1 --repetitions 2 --rounds 1
printf 'bench depth 5\nquit\n' | taskset -c 5 <minase> --protocol usi
printf 'bench nodes 1\nbench batch 1\nquit\n' | <minase> --protocol usi
```

## 結果

基準を2回実行した実ネットの出力は全行一致したので、CPUの推論と固定回数の探索は、この機械では実行のたびに同じ結果を返す。
そのうえで、どの候補の出力も基準と一致した。

| 照合 | 基準 | フェーズ1 | フェーズ2 | フェーズ3 |
|---|---|---|---|---|
| 実ネット、15局面の`go nodes 800`（75行） | 2回の実行が一致 | 一致 | 一致 | 一致 |
| 実ネット、初期局面から10手の`go nodes 800`（50行） | 2回の実行が一致 | 一致 | 一致 | 一致 |
| 実ネット、`VerboseMoveStats`を有効にして`movestats`の行を除いた出力 | ― | ― | ― | 15局面と10手の2組とも一致 |
| PV-MCTSの`bench`、3条件（各1,490行） | ― | 2回の実行が一致 | フェーズ1と一致 | フェーズ1と一致 |
| `invariants`付きのPV-MCTSの`bench`、3条件 | ― | ― | フェーズ1と一致、パニックなし | フェーズ1と一致、パニックなし |
| αβ探索の`bench depth 5`（18行） | ― | 一致 | ― | ― |

`scripts/bench_compare.py`は、15局面の深さ、ノード数、最善手、および評価値が基準と一致すると報告した。
フェーズ1の候補は、αβ探索で起動したときの`bench nodes 1`と`bench batch 1`を、基準と同じく`unknown diagnostic argument`の1行で拒否した。

フェーズ3の候補で`VerboseMoveStats`を有効にした10手の対局では、2手目から10手目までの9回の探索で前の探索の木を持ち越していた。
持ち越した訪問数は、前の手の表で選んだ手の行の訪問回数から1を引いた値に等しかった。
たとえば1手目で選んだ`7j5h`の訪問回数は115で、2手目の要約の`reused`は114であり、2手目で選んだ`6c7e`の訪問回数355に対して3手目の`reused`は354だった。
どの探索でも、表の先頭の行は`bestmove`の手であり、表は`info string stop`の行の直前に出た。

PV-MCTSの`bench`の初期局面の要約の`value`は0.75640595であり、初期局面の鍵のFNV-1aの上位24ビット12,690,386を2²⁴で割った値（PythonでFNV-1aの定義から別に計算したもの）を32ビットの浮動小数点数で表した値と一致した。

## 結論

3つのフェーズのどの候補でも、既定の設定でのPV-MCTSの探索の結果は、実ネットのCPUの推論で基準と一致し、αβ探索の`bench`も基準と一致した。
PV-MCTSの`bench`は同じバイナリで再現し、探索木の検査を足した後と`VerboseMoveStats`を足した後もフェーズ1の出力と一致したので、設計書の「検証」節の2.〜5.を満たした。
