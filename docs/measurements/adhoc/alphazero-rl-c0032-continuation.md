# サイクル32のネットによるαβ戦の局面からの指し継ぎ

## 目的

サイクル32のネットは、[αβ探索との対局の局面](alphazero-rl-c0032-value-bias.md)で手番によらずMCTS側を有利と評価する。
この評価が「相手も同じネットである」前提では正しいのか、その前提でも読み違いなのかを、同じ局面から同じネット同士で指し継いだ得点で判定する。
判定の基準は測定の前に次のとおり定めた（[調査](../../research/alphazero-strength-gap/value-bias.md)の「診断の先行例とminaseで安く行える診断」の節）。
元の対局でMCTSが持っていた側の得点が、両方の開始局面の集合で0.8に近ければ前者、0.5に近ければ後者、両者が食い違えば手番に起因する見かけの偏りを疑う。
この測定は[DL系将棋ソフトの調査](../../proposals/alphazero-strength-gap.md)の「勧める順序」の2番目であり、どの設計書の計画にも属さない。

## コマンドライン

本記録と同じ名前のディレクトリのスクリプトを作業機で実行した。

```console
D=docs/measurements/adhoc/alphazero-rl-c0032-continuation
python3 $D/select_openings.py data/matches/alphazero-rl-restart-stc-c0032 $D/openings.jsonl 100
bash $D/run-continuation.sh data/worktrees/c0032-continuation-runner/target/release/minase 20261104000000 200
python3 $D/analyze.py data/matches/alphazero-rl-c0032-continuation $D/openings.jsonl > $D/analyze.out
source tools/train/zero/env.sh    # bashで実行する
python3 $D/start_values.py target/match-cache/45d5db5498e1739b2307b3719891a1eec24f7fbd/minase \
    /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx $D/openings.jsonl > $D/start-values.out
```

`run-continuation.sh`が実行する`minase match run`は次のとおりである。
環境変数`MINASE_DIAG_OPENINGS`に開始局面のファイル、`MINASE_DIAG_BASE_SEED`にシードを与える。

```console
minase match run --run-dir data/matches/alphazero-rl-c0032-continuation --seed 20261104000000 \
    --candidate commit:45d5db5498e1739b2307b3719891a1eec24f7fbd --baseline commit:45d5db5498e1739b2307b3719891a1eec24f7fbd \
    --candidate-args "--search mcts --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx" \
    --baseline-args "--search mcts --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx" \
    --candidate-option NnBatchSize=16 --baseline-option NnBatchSize=16 \
    --each nodes=800 --baseline-limit nodes=801 --concurrency 2 elo --pairs 200
```

開始局面は、[サイクル32のSTCの対局](../alphazero-rl/alphazero-rl-restart-stc-c0032.md)の各局の40手目から59手目で、手番側の駒の損得（歩を1とする概算）が±5未満の局面である。
各局からMCTSの手番とαβの手番の最初の局面を候補とし、候補150局面と151局面からシード20261105で各100局面を抽出した。
両方の集合に同じ対局から選ばれた局面は64組ある。

## エンジン

両エンジンともコミット45d5db54のバイナリのMCTSにサイクル32のネットを与え、規則は`engine-default`（`L0,P0,R1,E0`）である。
同じネットと同じ回数では先後を入れ替えた2局が同じ棋譜になるので、基準側を801回にして分岐させた。
ハーネスは、[修正のコミット98dcfb18](alphazero-rl-c0032-ab-ladder.md)に、開始局面をファイルから読む一時的な変更（[パッチ](alphazero-rl-c0032-continuation/temporary-opening-input.patch)）を加えたビルド（SHA-256 `bbba6096…4077`）である。
この変更は本線へ入れておらず、コマンドラインのハーネスのworktreeは測定の後に削除した。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB）で、2026年11月2日19時26分から21時31分に行った。
両エンジンの`Threads`は1、同時対局数は2、推論はTensorRTのFP16で`NnBatchSize`は16である。
対局はノード数の制限なので、同じ機械で動いていたlishogiのBotは結果の条件に影響しない。

## 結果

200ペア400局のペンタノミアル度数は[6, 16, 155, 19, 4]、800回の候補のEloの点推定は−0.9、95%信頼区間は[−15.6, +13.9]である。
破棄ペア、エンジンの異常、および時間切れは0件で、経過時間は7,502秒であった。
集計は[results.out](alphazero-rl-c0032-continuation/results.out)、[analyze.out](alphazero-rl-c0032-continuation/analyze.out)、および[start-values.out](alphazero-rl-c0032-continuation/start-values.out)にある。

元の対局でMCTSが持っていた側の得点は次のとおりである。
信頼区間はペアを単位とする正規近似の95%である。

| 開始局面の集合 | ペア数 | 指し継いだ得点 | 開始局面の探索なしの勝率の中央値（平均） |
|---|---|---|---|
| MCTSの手番 | 100 | 0.477 ± 0.072 | 0.698（0.704） |
| αβの手番 | 100 | 0.520 ± 0.071 | 0.670（0.676） |

どちらの集合でも、ペアの2局が同じ棋譜になったのは37ペアであり、残りは分岐した。
同じ対局から両方の集合に選ばれた64組では、得点はMCTSの手番が0.523、αβの手番が0.543である。
開始局面の後の手数の中央値は413〜449手であり、STCの対局の中央値より長い。
終局は400局のうち反復が151局、王駒の捕獲が135局、詰みが114局である。

## 結論

指し継いだ得点は両方の集合で0.5に近く、開始局面の勝率0.67〜0.70を下回るので、αβ戦の局面でのMCTS側への偏りは、相手が同じネットである前提でも当たらない読み違いである。
