# fm-scale-spsa-ltc

## 目的

[FMの補正の倍率をSPSAで調整する設計書](../plans/fm-scale-spsa.md)のフェーズ3として、[STC](fm-scale-spsa-stc.md)を通過した[候補](fm-scale-spsa-candidate.md)（J75のFMの補正に倍率611/1024を焼き込んだ重み）が、master（M）より有意に強いかをLTCのGSPRTで判定し、採否を決める。

## コマンドライン

```console
data/worktrees/fm-scale-spsa-runner/target/release/minase match run \
  --run-dir data/matches/fm-scale-spsa-ltc --seed 103000000 \
  --candidate commit:f965ba27cba16a95c3ed1e9e9436ced7d5d219e9 \
  --baseline commit:203ba2a7ff47cc9de9ccf11ba4196965644c7c4b \
  --each time=60000+200 --concurrency 16 gsprt
```

基本シードは、STCの基本シード102000000からペア数以上離れた未使用の値である。

## エンジン

候補はブランチ`fm-scale-spsa`の`f965ba2`（SHA-256 `013cd392…`）、基準はmaster `203ba2a`（SHA-256 `14bafab5…`）である。
runnerはmaster `203ba2a`のビルドを`data/worktrees/fm-scale-spsa-runner/`に固定して使った。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は16である。
同じ測定機でlishogiのBotが稼働しており、他の対局測定と学習は走っていなかった。

## 結果

169ペア（有効163ペア、手数上限による破棄6ペア）で`H1`となった。
ペンタノミアル度数は候補の得点順に[14, 1, 79, 1, 68]、LLRは+2.971（判定境界+2.944）、候補の得点率は66.6%（ロジスティック換算で約+120 Elo）である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は5,356秒（約89分）である。
2026年10月8日21時38分から23時07分に実施した。

## 結論

候補は`H1`であり、エンジン異常、時間切れ、および拒否着手が0件なので、設計書の採否の規則に従って採用する。
