# ランダム初期化のネットを使うMCTSのUSIでの対局

## 目的

[探索方式の共存とMCTSの設計書](../../plans/archived/alphazero-mcts.md)の完了条件のうち、ランダム初期化のネットを使うMCTSがUSIで対局を規則上の終局まで指し切ることを、CPUとTensorRTの両方の実行プロバイダで確かめる。
あわせて、手数に比例する固定費の教訓（[記録](../../lessons/per-move-overhead-grows-with-game-length.md)）に従い、超長手数の着手列を1手ずつ与えても1手あたりの固定費が増えないことを確かめる。

## コマンドライン

対局の駆動部は、1つのエンジンのプロセスに両側を指させ、毎手`position`と`go`を送り、エンジン自身の審判層に独自コマンド`state`で問い合わせて終局を判定する（[usi_selfplay.py](alphazero-mcts-usi-smoke/usi_selfplay.py)）。
超長手数の計測は、保存済みの対局記録の着手列を1手ずつ与え、各手で`position`の送信から`bestmove`の受信までの時間を測る（[long_game_overhead.py](alphazero-mcts-usi-smoke/long_game_overhead.py)）。
ONNX Runtimeの共有ライブラリとTensorRTのライブラリは、`tools/train/zero/setup.sh`で導入した環境の`env.sh`をbashで読み込んで与えた。

```console
source tools/train/zero/env.sh   # bashで実行する
# CPU、引き分けの値は既定の0
python3 -I usi_selfplay.py --engine <minase> --net zero-b10c128.onnx \
  --backend cpu --batch 16 --go 'nodes 32' --max-plies 3000 --log usi-cpu-game.log
# CPU、DrawValueBlackとDrawValueWhiteを-1000
python3 -I usi_selfplay.py --engine <minase> --net zero-b10c128.onnx \
  --backend cpu --batch 16 --go 'nodes 32' --max-plies 4000 --draw-value -1000 --log usi-cpu-game2.log
# TensorRT、DrawValueBlackとDrawValueWhiteを-1000
python3 -I usi_selfplay.py --engine <minase> --net zero-b10c128.onnx \
  --backend tensorrt --cache-dir <キャッシュ> --batch 64 --go 'nodes 800' \
  --max-plies 4000 --draw-value -1000 --log usi-trt-game.log
# 超長手数の着手列（TensorRT、NnBatchSize 64、毎手 go nodes 64）
python3 -I long_game_overhead.py <minase> zero-b10c128.onnx \
  data/matches/search-repetition-stc/pairs/00000000000000000016.json 1 <キャッシュ>
```

エンジンは`--protocol usi --rules engine-default --search mcts --nn-model zero-b10c128.onnx`で起動した。

## エンジン

測定したエンジンは、ブランチ`alphazero-mcts`のコミット0bbbcbb7（レビューの所見を直した最終のコミット）をリリースビルドした`minase`である。
ネットは、`tools/train/zero/feasibility/make_nets.py`が作るランダム初期化の10ブロック128チャネルのネット`zero-b10c128.onnx`（SHA-256は`61a62a623067f9328f1baa91700dc98c81b7c43146ef8448f8263cc570d01600`）である。
規則セットは`engine-default`（L0＋P0＋R1＋E0）である。
ONNX Runtimeは1.28.3である。

## 環境

作業機のCPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、GPUはメモリ8 GBのNVIDIA GeForce RTX 3060 Tiでドライバは615.78.08であり、2026年10月27日に測った。
測定のあいだ、別のプロジェクトの計算が12コアを使い、lishogiのBotも対局しており、1分間の平均負荷は24〜46であった。
MCTSの`Threads`は1、`USI_Hash`は既定の256 MiBである。
超長手数の計測に使った対局は、`search-repetition-stc`の実行ディレクトリの第16ペアの2局目で、手数上限4,096手で打ち切られたαβ探索同士の対局である。

## 結果

| 対局 | 実行プロバイダ | 探索回数 | 手数 | 終局 | `isready`の秒数 | 1手の平均と最大（秒） | 終了コード |
|---|---|---|---|---|---|---|---|
| 1 | CPU | 32 | 18 | 千日手の引き分け | 0.103 | 0.193、0.302 | 0 |
| 2 | CPU | 32 | 34 | 千日手の引き分け | 0.066 | 0.201、0.353 | 0 |
| 3 | TensorRT | 800 | 97 | 千日手の引き分け | 0.725 | 0.064、0.101 | 0 |

3局とも、エンジンの審判層が千日手の引き分けを裁定するまで指し切り、プロセスは正常に終了した。
TensorRTの`isready`の0.725秒は、前の計測で作ったTensorRTのエンジン（ネットをGPU向けに最適化したもの）のキャッシュを読んだ時間である。
キャッシュのない最初の起動では、エンジンの作成を含めて11.3秒かかった。
各`go`の`info string`は、ノード数がMCTSのシミュレーションの回数であることを示した。

3局とも、両者が獅子の「じっと」（隣の升へ動いて元の升へ戻る着手、RULES.md第3条）を繰り返して千日手になった。
引き分けの値を両側で−1,000にした対局2では、終盤の`info`の評価値が−1,000 cpで、根の側から見た千日手の勝率が設定どおり約0.28に換算されていた。
それでも千日手になる手が選ばれたのは、根ではFPUの減算が0なので未訪問の手が親の平均の価値（千日手に引き下げられた値）を受け継ぎ、ネットを呼ばずに逆伝播できる千日手の子へ32回のシミュレーションの大半が流れたからである。
ランダム初期化のネットは手の良し悪しを区別しないので、探索回数の少ない対局ではこの振る舞いが起こる。

超長手数の計測では、4,096手のすべてで`bestmove`が返り、プロセスは正常に終了した。
`position`の送信から`bestmove`までの時間は次のとおりであった。

| 手数の範囲 | 中央値（ms） | 最大（ms） |
|---|---|---|
| 0〜99 | 10.85 | 24.32 |
| 1,000〜1,099 | 10.93 | 24.77 |
| 2,000〜2,099 | 10.98 | 24.09 |
| 3,000〜3,099 | 10.99 | 25.43 |
| 3,996〜4,095 | 11.22 | 24.88 |

最大値は、どの手数の範囲でも24〜25 msであり、手数によらない負荷の揺れである。
集計は[long-game-overhead.json](alphazero-mcts-usi-smoke/long-game-overhead.json)にある。

レビューの所見を直す前のコミット86acdfd0でも同じ3局と計測を行い、3局は同じ手数と終局になった（そのときの負荷は記録していない）。
そのときの1手の平均は、対局1から順に0.068秒、0.079秒、0.044秒であった。
TensorRTの800回のシミュレーションの探索は約56 ms（毎秒約14,000回）であり、超長手数の計測の手数の範囲ごとの中央値は10.85〜12.24 msであった。

## 結論

ランダム初期化のネットを使うMCTSは、CPUとTensorRTのどちらの実行プロバイダでもUSIで対局を規則上の終局まで指し切り、1手あたりの固定費は4,096手まで約0.4 msしか増えず、持ち時間の配分式が残り時間から差し引く安全余裕30 msの内側に収まった。
