# 成りの保留を理由とするMCTSの拒否を外す前後での、既存の規則セットの出力の一致

## 目的

[対局用MCTSを全部の規則コードで探索できるようにする設計書](../../plans/archived/mcts-rule-codes.md)のフェーズ2として、変更の前後のminaseを比べ、規則セット`lishogi`と`JCSA`で、ニューラルネットワーク（以下、ネット）の出力と、シミュレーションの回数を固定したMCTSの結果が変わらないかを判定する。
MCTSは、AlphaZero型の対局用のモンテカルロ木探索であり、ネットで局面を評価する。
変更の前のMCTSは、成りの保留を追跡する規則コードP1、P2、またはP5を含む規則セットを、探索を始める前に拒否しており、変更はこの拒否を外す。
成りの保留は、成れる着手で成らなかった駒が後で成れるかどうかを決める状態であり、`lishogi`と`JCSA`は成りの保留を追跡しない。

## コマンドライン

測定日は2026年11月13日である。
`<main>`は変更の前のコミットの作業ツリー、`<wt>`は変更の後のコミットの作業ツリー、`<net>`は後述のネットのファイル、`<out>`は出力先のディレクトリ、`<キャッシュの親>`はTensorRTのエンジンのキャッシュを置くディレクトリの親である。

```console
source <main>/tools/train/zero/env.sh   # bashで実行する
export XDG_CACHE_HOME=<キャッシュの親>
cd <wt>
python3 docs/measurements/mcts-rule-codes/mcts-rule-codes-invariance/compare.py \
  --before <main>/target/release/minase --after <wt>/target/release/minase \
  --net <net> --nodes 800 --out <out>
```

[compare.py](mcts-rule-codes-invariance/compare.py)は、規則セットごとに、2つのバイナリを`--protocol usi --search mcts`で順に起動する。
各起動では、USI（将棋エンジンの通信規約）のオプション`RuleSet`と`NnModel`を設定して`isready`を送り、次の2つの出力を集める。

- USIの独自コマンド`nn-eval`の出力を集める。`nn-eval`は、固定の288局面について、価値と合法手ごとのlogitをJSONで書き出す。
- 固定の15局面のそれぞれに`usinewgame`、`position sfen`、`go nodes 800`を送り、最善手、予想手、評価値、シミュレーションの回数、および読み筋を集める。15局面は、αβ探索の確認に使うUSIの独自コマンド`bench`の局面集（`crates/minase-core/src/test_util.rs`の`BENCH_SFENS`）と同じであり、先頭は初期局面である。

スクリプトは、`nn-eval`の出力をバイト単位で比べ、探索の結果を局面ごとに比べて、不一致が1つでもあれば終了コード1で終わる。

マージの前の確認として、αβ探索の深さ6の`bench`も、2つのバイナリで実行して比べた。

```console
printf 'bench depth 6\nquit\n' | <main>/target/release/minase --protocol usi
printf 'bench depth 6\nquit\n' | <wt>/target/release/minase --protocol usi
```

## エンジン

変更の前のバイナリは、masterのコミット`9daa89defd5ad3b433cd37085f8bd3073dfd2af1`を`cargo build --release --bin minase`でビルドしたものであり、SHA-256は`85534fecfc6c30ead2c3fd286e808fd2c39312182b23034ed525ba11179f9e80`である。
変更の後のバイナリは、ブランチ`mcts-rule-codes`のコミット`bc8fdee0cad64e0bcb54b442a29badcf897587ec`を同じコマンドでビルドしたものであり、SHA-256は`990a8891bb9139ee5e6f0d242678129a3b0eb6175c42872397b04b1054f639d5`である。

ネットには、1回目の強化学習のサイクル32で得た、10ブロック128チャネルのネットを使った。
チェックポイント`cycle-0032/train/checkpoint.pt`（SHA-256は`8ece92ab0891937148cea95defe750416d2a09a6361afd4b17effa576ff40205`）から、変更の前のコミットの学習器の`minase_zero export-onnx`で、推論器が今受け付ける入出力の形のONNX形式のファイルへ書き出し直し、これを`<net>`とした。
書き出したファイルのSHA-256は`cdc64e02fbd3ff54a935775c1bfe7610e33b02e498d41270a55bc705792d76a6`である。

推論の設定は既定のままであり、実行プロバイダはTensorRT（FP16）、`NnBatchSize`は256、`Threads`は1である。
MCTSの手の選択には、対局用のPUCT（方策を探索項に使う上側信頼限界による手の選択）を使い、係数は既定値である。

## 環境

作業機（Intel Core Ultra 7 265KF、物理20コア、論理20コア、NVIDIA GeForce RTX 3060 Ti、ドライバ615.78.08）で、ONNX Runtime 1.31.1とTensorRT 10.14.1を使った。
測定のあいだ、ほかの棋力測定は走っていなかった。
置換表の大きさを決めるUSIのオプション`USI_Hash`は、MCTSでは使わない。対局ではないので、同時対局数にあたる設定もない。

## 結果

2つの規則セットのどちらでも、変更の前後の出力はすべて一致した。

| 規則セット | `nn-eval`の出力 | 探索の結果が一致した局面 |
|---|---|---|
| `lishogi` | 一致（435,500バイト） | 15局面のうち15 |
| `JCSA` | 一致（435,500バイト） | 15局面のうち15 |

`nn-eval`の出力のSHA-256は、2つの規則セットと2つのバイナリを組み合わせた4回の実行のすべてで`0e3e4ea0c0895e009c17386e0ddae2f05ef85e823dba45818c119bb5a781d67d`であった。
`nn-eval`は、`RuleSet`の値によらず同じ288局面と合法手を評価するので、規則セットの間でも出力は同じになる。

探索の結果は、最善手、予想手、評価値、シミュレーションの回数、および読み筋の全部が、変更の前後で一致した。
変更の後のバイナリの結果は、[lishogi.search.json](mcts-rule-codes-invariance/lishogi.search.json)と[JCSA.search.json](mcts-rule-codes-invariance/JCSA.search.json)にあり、変更の前のバイナリの結果は、これらとバイト単位で同じである。
次の表は、局面ごとの最善手と評価値（USIの`score cp`の値）である。評価値が規則セットの間で異なる局面が3つあるが、どの局面でも、変更の前後の値は同じである。

| 局面 | `lishogi`の最善手 | `lishogi`の評価値 | `JCSA`の最善手 | `JCSA`の評価値 |
|---|---|---|---|---|
| 1 | 2j2k | 7 | 2j2k | 7 |
| 2 | 3l4k | 269 | 3l4k | 269 |
| 3 | 3l4k | 2718 | 3l4k | 2718 |
| 4 | 12l12e | −419 | 12l12e | −419 |
| 5 | 8g8i | 3319 | 8g8i | 3344 |
| 6 | 6j4j | −57 | 6j4j | −57 |
| 7 | 5h6i | −523 | 5h6i | −527 |
| 8 | 4g5f | 2252 | 4g5f | 2252 |
| 9 | 5a3c | −3770 | 5a3c | −3770 |
| 10 | 10a11b | 130 | 10a11b | 130 |
| 11 | 9g7i+ | 250 | 9g7i+ | 250 |
| 12 | 2h2i+ | −65 | 2h2i+ | −65 |
| 13 | 9e6h | 197 | 9e6h | 197 |
| 14 | 6b5c | 422 | 6b5c | 422 |
| 15 | 6j5k | −87 | 6j5k | −99 |

スクリプトの実行時間は、4回の起動を合わせて約6秒であった。

αβ探索の深さ6の`bench`は、15局面の局面ごとのノード数、最善手、および評価値が2つのバイナリで一致し、ノード数の合計はどちらも56,937であった。

## 結論

成りの保留を理由とする拒否を外す変更は、規則セット`lishogi`と`JCSA`でのネットの出力とMCTSの探索の結果を変えず、αβ探索の結果も変えない。
