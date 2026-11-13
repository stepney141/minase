# 1回目の学習のネットと固定したαβ探索の基準の対局

## 目的

[強さを測る固定の相手の設計書](../../plans/mcts-fixed-opponents.md)のフェーズ1として、1回目の強化学習のサイクル10、20、および32のネットのMCTSを、コミット45d5db54のαβ探索の1手500ノードと規則セット`lishogi`で100ペアずつ対局させ、2回目の学習の測定値と比べる基準のEloを求める。
MCTSは、ニューラルネットワーク（以下、ネット）で局面を評価しながら探索するAlphaZero型探索のモンテカルロ木探索である。
この対局は、同じ相手に対するネットの成績の変化を追うものであり、MCTSとαβ探索のどちらが強いかを判定するものではない。

## コマンドライン

測定日は2026年11月13日である。
[固定の相手との対局のスクリプト](mcts-fixed-opponents-scripts/)の`run_match.py`を、作業機のリポジトリの根で、次のように3回実行した。
`<c>`は`0010`、`0020`、`0032`であり、この順に続けて実行した。3つのネットは同じシードで対局したので、開始局面の列は同じである。

```console
source tools/train/zero/env.sh   # bashで実行する
export XDG_CACHE_HOME=$HOME/.cache
W=data/worktrees/mcts-fixed-opponents
python3 -I $W/docs/measurements/mcts-fixed-opponents/mcts-fixed-opponents-scripts/run_match.py alphabeta \
  --harness $W/target/release/minase --mcts-commit c1c84466476eb36cf43abc529792469862b8e856 \
  --net data/mcts-fixed-opponents/nets/cycle-<c>.onnx --nodes 500 \
  --name mcts-fixed-opponents-alphabeta-baseline-c<c> --seed 20261113100000 --pairs 100 --concurrency 4
```

スクリプトが起動した対局ハーネス`minase match run`のコマンドは次のとおりである（`<net>`はネットのファイルの絶対パス）。

```console
data/worktrees/mcts-fixed-opponents/target/release/minase match run \
  --run-dir data/matches/mcts-fixed-opponents-alphabeta-baseline-c<c> --seed 20261113100000 \
  --candidate commit:c1c84466476eb36cf43abc529792469862b8e856 --candidate-args '--search mcts' \
  --candidate-option NnModel=<net> --candidate-option NnBatchSize=16 --candidate-limit nodes=800 \
  --baseline commit:45d5db5498e1739b2307b3719891a1eec24f7fbd --baseline-args '--rules lishogi' \
  --baseline-limit nodes=500 --rules lishogi --max-ply 4096 --concurrency 4 elo --pairs 100
```

1ペアは、ハーネスが初期局面から8〜12手を乱択で指した同じ開始局面で、先後を入れ替えた2局である。
サイクル10の対局は、スクリプトがログの先頭にコマンドの行を書くように直す前の版で起動した。直した後の版（コミットe28f3f89）とは、ハーネスへ渡すコマンドが同じである。
本番の前に、同じスクリプトでサイクル32のネットを2ペアだけ対局させ（実行ディレクトリ`mcts-fixed-opponents-script-smoke-alphabeta`、シード20261113400000）、最後まで異常なく動くことを確かめた。

## エンジン

候補（`--candidate`）は、masterのコミットc1c84466（対局用MCTSの規則コードへの対応をマージした後）のバイナリに起動引数`--search mcts`を渡したMCTSである。ハーネスがビルドしたバイナリのSHA-256は`60e6611ef44f6b23669be6da2164b21dce4c0ef383b218813086fd7ee06e7c36`である。
思考の制限は1手800回のシミュレーションであり、手の選択には対局用のPUCT（方策を探索項に使う上側信頼限界による手の選択）を既定の係数で使った。方策の温度（`PolicyTemperature`）は既定値140、すなわち1.4である。

ネットは、1回目の強化学習の各サイクルのチェックポイント`/mnt/synology_nas/minase/alphazero/cycles/cycle-<c>/train/checkpoint.pt`を、コミットc1c84466の学習器の`minase_zero export-onnx`で、推論器が今受け付ける入出力の形のONNX形式のファイルへ書き出し直したものである。いずれも10ブロック128チャネルのネットである。

| サイクル | チェックポイントのSHA-256 | 書き出したONNXのSHA-256 |
|---|---|---|
| 10 | `20d80885b577fc07fcbfd8029fda8ca4782aa06cbfaab340d814a61beaa979a1` | `55a4859f46f1a02232ee31546e13f53f1cd56f6db272406c240cd595e7a5cf08` |
| 20 | `3dac1a65acad373ea1c66c8009a7047f6c1a35de27ea32a102c1877200ecbb9c` | `a27e4e749b2cba4cd540f0e15a0f10398fd0c7be04386ca749670ee2bb44a961` |
| 32 | `8ece92ab0891937148cea95defe750416d2a09a6361afd4b17effa576ff40205` | `cdc64e02fbd3ff54a935775c1bfe7610e33b02e498d41270a55bc705792d76a6` |

サイクル32のONNXは、[対局用MCTSの規則コードへの対応の記録](../mcts-rule-codes/mcts-rule-codes-invariance.md)で同じチェックポイントから書き出したファイルとSHA-256が一致し、書き出しが決定的であることを確かめた。
ハーネスは、エンジンが`isready`で返したネットのチェックサムを`manifest.json`の`network_sha256`に記録しており、各実行の値は上の表のONNXのSHA-256と一致した。

基準（`--baseline`）は、コミット45d5db54のαβ探索と、そのコミットに埋め込まれた評価関数NNUE（差分計算する小型のニューラルネットワーク）である。ハーネスがビルドしたバイナリのSHA-256は`e94fbeab5551442c960c936d93c79f4ee1cafd207d9649cd054b8bae17463ba8`である。
このコミットは規則を起動引数`--rules`で受ける版なので、起動引数`--rules lishogi`を足した。思考の制限は1手500ノードである。

規則は、オンライン対局サイトlishogiの実装に近い規則セット`lishogi`（`L1,L2,P0,P3,R1,E1,E3`）を、審判層と両エンジンに与えた。45d5db54の規則セット`lishogi`の規則コードの組は、今のmasterと同じである。
手数の上限`--max-ply`は、ハーネスの既定値と同じ4,096手を明示した。
対局のハーネスは、ブランチ`mcts-fixed-opponents`のworktreeでビルドしたコミットc1c84466のバイナリ（SHA-256 `67d8aeb058dc7f6fc1cdcc85f4363fdc535348adaffc77a9e4991b5ebc243456`）である。

## 環境

作業機（Intel Core Ultra 7 265KF、物理20コア、論理20コア、NVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08）で、ONNX Runtime 1.31.1とTensorRT 10.14.1を使った。
両エンジンの`Threads`は1、`USI_Hash`はどちらも既定の256 MB（MCTSは使わない）、同時対局数は4、応答タイムアウトは120秒である。MCTSの推論はTensorRTのFP16で、`NnBatchSize`は16である。
サイクル10の対局の最初の約2分は、HaChuとの対局のスクリプトの2ペアの確認と重なった。同じ機械では、lishogiのBotが通信対局を指していた。
対局はシミュレーション数とノード数の制限なので、これらの負荷は時間には影響するが、対局の結果には影響しない。

## 結果

全部の対局で、破棄ペア、エンジンの異常終了、応答タイムアウト、時間切れ（`time_forfeits`）、不正着手（`illegal_moves`）、および拒否着手（`rejected_moves`）は0件であった。
ペンタノミアル度数は、先後を入れ替えた2局のMCTSの得点が0、0.5、1、1.5、および2であるペアの数である。
得点率は、引き分けを0.5点としたMCTSの得点の割合である。
集計は、`summarize.py`の出力（[summary.json](mcts-fixed-opponents-alphabeta-baseline/summary.json)）であり、Eloと95%信頼区間はハーネスの最終行の値と一致した。

| サイクル | ペンタノミアル度数 | 得点率 | Eloの点推定 | 95%信頼区間 | 経過時間（秒） |
|---|---|---|---|---|---|
| 10 | [93, 0, 7, 0, 0] | 3.5% | −576 | [−798, −478] | 751 |
| 20 | [45, 0, 46, 2, 7] | 31.5% | −135 | [−187, −88] | 1,437 |
| 32 | [23, 1, 47, 1, 28] | 52.5% | +17 | [−31, +67] | 1,540 |

終局の内訳は次のとおりである。
王駒は王将と、酔象が成った太子であり、規則E1では詰みで終局せず、相手の最後の王駒を取った側が勝つ。
裸玉は、規則E3の裁定であり、盤上の駒が（最奥段で動けない歩兵と香車を除いて）王駒1枚だけになった側を、相手の駒の数などの条件のもとで負けとする（[競技規則](../../rules/chu-shogi.md)第32条）。
千日手は、規則R1による同一局面の4回目の出現の引き分けである（同第31条）。

| サイクル | MCTSの王駒の捕獲による勝ち | MCTSの裸玉による勝ち | αβの王駒の捕獲による勝ち | αβの裸玉による勝ち | 千日手の引き分け | 1局の手数（最小、中央値、最大） |
|---|---|---|---|---|---|---|
| 10 | 7 | 0 | 168 | 25 | 0 | 55、207、590 |
| 20 | 56 | 6 | 125 | 11 | 2 | 114、343、3,351 |
| 32 | 84 | 20 | 84 | 10 | 2 | 75、377、2,961 |

サイクル10の得点率3.5%は、設計書が相手の強さを改める目安とする1割を下回る。
サイクル32の+17は、同じネットを日本中将棋連盟の規則に基づく規則セット`JCSA`で同じ500ノードのαβ探索と対局させた+56（95%信頼区間[+8, +107]、[記録](../mcts-royal-capture-temperature/mcts-royal-capture-temperature-t140-ladder.md)）と、信頼区間が重なる。
その対局は、MCTSのコミットが0b7c3800で、ネットは書き出し直す前の入出力の形のONNX（同じチェックポイントから学習器が書き出したもの）であり、規則、MCTSのコミット、および開始局面の列が本記録と異なる。

## 結論

固定したαβ探索の500ノードに対するEloは、サイクル10の−576から、サイクル20の−135、サイクル32の+17へ上がり、点推定の差はサイクル10から20で+441、20から32で+152であった。この3つの値を、規則セット`lishogi`の基準として、2回目の学習の測定値と比べる。
