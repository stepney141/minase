# 第3期の高速化の採用版と基準のLTC

## 目的

[第3期の設計書](../../plans/movegen-speedup-3.md)の採用版を、ブランチを切った時点のmasterの基準と、[SPRTの手引き](../../guides/sprt.md)の段階ゲートのLTCで比べ、設計書が統合を止める条件とする明確な退行がないかを確かめる。

## コマンドライン

```console
target/release/minase match run \
  --run-dir data/matches/movegen-speedup-3-ltc --seed 7700100000 \
  --candidate commit:426dc065 --baseline commit:6cd40868 \
  --candidate-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --baseline-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --rules JCSA --each time=60000+200 --concurrency 11 gsprt
```

GSPRTの仮説はelo0 = 0とelo1 = 10、誤り率はα = β = 0.05である。
基本シードは、[STC](movegen-speedup-3-stc.md)の基本シード7700000000から、STCのペア数387より離した値である。
runnerは、ブランチ`movegen-speedup-3`の作業ツリーでビルドしたminaseである。

## エンジン

候補と基準は[STC](movegen-speedup-3-stc.md)と同じであり、候補は採用版のコミット`426dc065d70bce5bfedef7d74ff5d74813b6a77d`、基準はmasterのコミット`6cd408685f9f39cc435872690752d6a38138a188`である。
ネットは両側とも標準の第3世代のA型のネット（`network_sha256`は`415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9`）である。
規則セットは`JCSA`（`L0,P0,R1,E0`）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）である。
候補と基準のワーカー数はともに1、`USI_Hash`はともに256 MB、同時対局数は11である。
lishogiのBotは、[STCの記録](movegen-speedup-3-stc.md)に書いた理由で開始時には止まっており、22時20分に起動し直した後は動いていた。Botは最大2局を1局あたり4スレッドで指す設定なので、同時対局数11と合わせても20コアに収まる。
起動は1回であり、2026年11月27日21時01分に始め、経過時間は9,384秒である。

## 結果

242ペアで`decision: H1`となった。対数尤度比LLRは2.969である。
有効ペア240、手数上限による破棄ペア2であり、ペンタノミアルは`[18, 28, 102, 39, 53]`である。
有効ペアの得点率は0.5844であり、ペンタノミアルの分散から求めたEloは+59.2（両側95%信頼区間+33.2〜+85.9）である。
不正着手、クラッシュ、応答タイムアウト、時間切れ（`time_forfeits`）、および拒否着手はすべて0件である。

## 結論

LTCは`decision: H1`であり、異常は0件なので、手引きの採用の規則も満たす。
Eloの信頼区間の上端は0より大きく、設計書が定める明確な退行はないので、採用版をmasterへ統合する。
