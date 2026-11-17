# PV-MCTSがN本の読み筋を出すようにした変更の、既定値1での出力の一致

## 目的

[MultiPVの設計書](../../plans/archived/multipv.md)のフェーズ3として、PV-MCTSが根の上位N手の読み筋を出すようにした変更が、既定値1でエンジンの出力を変えないかを判定する。
PV-MCTSは、方策と価値を出すネットで木を広げるAlphaZero型のモンテカルロ木探索であり、変更は探索を変えずに、探索の終了時に根の辺（根から指せる手）を最善手と同じ比較規則で並べて上位N本を出す。
判定には、設計書の「検証」の節の4.（PV-MCTSの`go nodes`の出力の一致）を使う。
あわせて、探索を変えていないことの確認として、`MultiPV`を3にしたときの1本目の読み筋と`bestmove`が、既定値1のときと一致するかを調べた。

## コマンドライン

測定日は2026年11月17日である。
手順は、[イベントの読み筋を列へ改めた変更の記録](multipv-events-default-equivalence.md)の検証4と同じである。
変更の後のバイナリを`--protocol usi --search mcts`で起動し、`RuleSet`を`JCSA`、`Threads`を1、`NnBackend`を`cpu`として、`bench`の15局面（`crates/minase/src/diagnostics/bench.rs`の`BENCH_POSITIONS`）のそれぞれに`usinewgame`、`position sfen <局面> - 1`、および`go nodes 800`を送り、`bestmove`までの出力を集めた。
出力の`time`と`nps`の値は伏せてから、変更の前のバイナリの出力と比べた。
続けて、同じ手順に`setoption name MultiPV value 3`を加えて実行した。
`MultiPV`が1のときは`info`の行に`multipv`の欄を出さないので、各局面の`multipv 1`の行から`multipv 1`の語を除いた行と`bestmove`の行を、既定値1の出力の`info depth`の行と`bestmove`の行と比べた。

## エンジン

変更の前のバイナリは、masterのコミット`7a0b267612ea063093bf6622f59069687648456a`のビルドであり、SHA-256は`deb9ae5f4fb2850e0174ba7a7e0c39eb494c0c36e4895693890a86b5408349a4`である。
比べた出力は、[イベントの読み筋を列へ改めた変更の記録](multipv-events-default-equivalence.md)の検証4で集めたものである。
変更の後のバイナリは、ブランチ`multipv`のコミット`4c99f8ed`と同じ内容の作業ツリーを`cargo build --release --bin minase`でビルドしたものであり、SHA-256は`1c179382c598c4ea73c640965b48e44591e99754233e48621508b8141bfcda47`である。

ネットは、AlphaZero型探索の強化学習の1回目のサイクル32で得た10ブロック128チャネルのネットを、コミットc1c84466の学習器でONNX形式に書き出し直したファイルであり、SHA-256は`cdc64e02fbd3ff54a935775c1bfe7610e33b02e498d41270a55bc705792d76a6`である。
書き出しの経緯は、同じ記録の「エンジン」の節にある。

## 環境

作業機（Intel Core Ultra 7 265KF、物理20コア、論理20コア）で測り、探索のプロセスを`taskset -c 14-19`で論理CPU 14〜19に固定した。
ONNX Runtimeの環境は、bashで`tools/train/zero/env.sh`を読み込んで用意した。
この記録は出力の一致だけを判定するので、負荷の影響を受けない。

## 結果

既定値1では、`readyok`以後の出力が15局面の`go nodes 800`のすべてで、変更の前のバイナリと一致した。

`MultiPV`を3にした実行では、15局面のそれぞれで`multipv 1`、`multipv 2`、および`multipv 3`の3行が出て、合わせて45行であった。
どの局面でも、`multipv 1`の行から`multipv 1`の語を除いた行は既定値1の`info depth`の行と、`bestmove`の行は既定値1の`bestmove`の行と一致した。
PV-MCTSは2本目以降の未訪問の手の行に評価値を出さないが、`go nodes 800`（800回のシミュレーション）では、どの局面でも上位3手がすべて訪問済みであり、評価値を持たない行はなかった。

## 結論

PV-MCTSがN本の読み筋を出すようにした変更は、既定値1でPV-MCTSの`go nodes 800`の出力を変えない。
`MultiPV`を3にしても、1本目の読み筋と`bestmove`は既定値1のときと同じであり、探索は変わっていない。
