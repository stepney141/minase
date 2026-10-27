# 対局ハーネスでの`--candidate-args`によるMCTSの起動と再開

## 目的

[探索方式の共存とMCTSの設計書](../../plans/archived/alphazero-mcts.md)の「ハーネスの起動引数の拡張」のとおり、`commit:`で指定したエンジンを`--candidate-args`でMCTSとして起動して対局させ、足した起動引数が`manifest.json`に記録され、再開でもその引数で対局することを確かめる。
棋力の測定ではなく、ハーネスの経路の煙試験である。

## コマンドライン

```console
source tools/train/zero/env.sh   # bashで実行する
export XDG_CACHE_HOME=<キャッシュの親>
minase match run --run-dir data/matches/alphazero-mcts-harness-smoke --seed 20261027 \
  --candidate commit:0bbbcbb7 \
  --candidate-args '--search mcts --nn-model /home/stepney141/board-games/minase/data/alphazero-network/nets/zero-b10c128.onnx' \
  --baseline random --each nodes=200 --concurrency 1 elo --pairs 2
minase match run --resume data/matches/alphazero-mcts-harness-smoke --target-pairs 3
```

実行ディレクトリは作業用の一時ディレクトリに作り、終了後に`data/matches/alphazero-mcts-harness-smoke`へ写したが、本書の記録の後に利用者の判断で削除した。
`XDG_CACHE_HOME`は、MCTSの`NnCacheDir`の既定値（`$XDG_CACHE_HOME/minase/tensorrt`）を与えるために設定した。
ハーネスは`NnBackend`を送らないので、MCTSは既定のTensorRTで推論した。

## エンジン

候補はブランチ`alphazero-mcts`のコミット0bbbcbb7（完全なハッシュは`0bbbcbb7482eb1acb960b75ab930d5ab8c8cbc92`）に、`--search mcts --nn-model <ネット>`を足して起動したMCTSである。
ネットはランダム初期化の10ブロック128チャネルの`zero-b10c128.onnx`（SHA-256は`61a62a623067f9328f1baa91700dc98c81b7c43146ef8448f8263cc570d01600`）である。
基準は、合法手から一様に着手する`random`である。
規則セットは`engine-default`（L0＋P0＋R1＋E0）である。
思考制限は両側とも`nodes=200`であり、MCTSでは200回のシミュレーションを表す。

## 環境

作業機のCPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、GPUはメモリ8 GBのNVIDIA GeForce RTX 3060 Tiであり、2026年10月27日に測った。
MCTSの`Threads`は1、`USI_Hash`は既定の256 MiB、`NnBatchSize`は既定の256、同時対局数は1である。
測定のあいだ、別のプロジェクトの計算が12コアを使い、lishogiのBotも対局していた。

## 結果

`manifest.json`は形式の版6で、候補の同一性の欄`args`に`["--search", "mcts", "--nn-model", "<ネットのパス>"]`が、欄`working_directory`に測定を開始した作業ディレクトリが記録された。
最初の起動の2ペア4局と、再開した起動の1ペア2局は、すべて王駒の捕獲で終局した。
6局の手数は161〜384手であった。
候補のペア得点（0、0.5、1、1.5、2）ごとの度数であるペンタノミアル度数は、2ペアの時点で`[0, 0, 1, 0, 1]`、再開後の3ペアで`[0, 0, 1, 0, 2]`であり、MCTSは6局中5局に勝った。
破棄ペアは0、エンジンの異常（不正着手、クラッシュ、タイムアウト、時間切れ、拒否着手）はいずれも0件であった。
経過時間は最初の起動が19.7秒、再開した起動が10.7秒であった。
再開の起動に`--candidate-args`を加えると、`minase match run`は引数のエラーで起動を拒否した。
別の作業ディレクトリから再開を試みると、ハーネスは`engine working directory differs from the recorded working_directory`のエラーで再開を拒否した。
レビューの所見を直す前のコミット86acdfd0でも同じ手順を行い、6局の手数、勝敗、およびペンタノミアル度数は同じであった。

## 結論

`--candidate-args`で起動したMCTSはハーネスで対局を最後まで指し、足した起動引数は`manifest.json`の版6に記録されて、再開でも同じ引数で対局した。
