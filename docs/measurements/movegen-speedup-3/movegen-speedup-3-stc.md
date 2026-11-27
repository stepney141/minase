# 第3期の高速化の採用版と基準のSTC

## 目的

[第3期の設計書](../../plans/movegen-speedup-3.md)の採用版を、ブランチを切った時点のmasterの基準と、[SPRTの手引き](../../guides/sprt.md)の段階ゲートのSTCで比べ、LTCへ進むかを決めるとともに、設計書が統合を止める条件とする明確な退行がないかを確かめる。

## コマンドライン

```console
target/release/minase match run \
  --run-dir data/matches/movegen-speedup-3-stc --seed 7700000000 \
  --candidate commit:426dc065 --baseline commit:6cd40868 \
  --candidate-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --baseline-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --rules JCSA --each time=10000+100 --concurrency 11 gsprt --max-pairs 3000
```

GSPRTの仮説はelo0 = 0とelo1 = 10、誤り率はα = β = 0.05である。
runnerは、ブランチ`movegen-speedup-3`の作業ツリーでビルドしたminaseである。

## エンジン

候補は採用版のコミット`426dc065d70bce5bfedef7d74ff5d74813b6a77d`（バイナリのSHA-256は`ba6b53e9abdf6f2c…`）、基準はmasterのコミット`6cd408685f9f39cc435872690752d6a38138a188`（同`b023cdf67176ff3a…`）である。
ネットは両側とも標準の第3世代のA型のネット（`network_sha256`は`415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9`）である。
規則セットは`JCSA`（`L0,P0,R1,E0`）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）である。
候補と基準のワーカー数はともに1、`USI_Hash`はともに256 MB、同時対局数は11である。
lishogiのBotは止めずに動かした。Botは最大2局を、1局あたり4スレッドで指す設定なので、測定の同時対局数を既定の19から11に下げた（2026年11月27日の利用者の指示）。
起動は1回であり、2026年11月27日19時39分に始め、経過時間は4,865秒である。

## 結果

387ペアで`decision: H1`となった。対数尤度比LLRは2.944である。
有効ペア381、手数上限による破棄ペア6であり、ペンタノミアルは`[38, 41, 176, 52, 74]`である。
有効ペアの得点率は0.5545であり、ペンタノミアルの分散から求めたEloは+38.0（両側95%信頼区間+17.4〜+58.8）である。
不正着手、クラッシュ、応答タイムアウト、時間切れ（`time_forfeits`）、および拒否着手はすべて0件である。

## 結論

振分け規則により、採用版をLTCへ進める。
Eloの信頼区間の上端は0より大きく、異常は0件なので、設計書が定める明確な退行はない。
