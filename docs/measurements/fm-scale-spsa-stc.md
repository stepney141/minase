# fm-scale-spsa-stc

## 目的

[FMの補正の倍率をSPSAで調整する設計書](../plans/fm-scale-spsa.md)のフェーズ3として、J75のFMの補正に倍率611/1024を焼き込んだ[候補](fm-scale-spsa-candidate.md)が、master（M）より有意に強いかをSTCのGSPRTで判定する。

## コマンドライン

```console
data/worktrees/fm-scale-spsa-runner/target/release/minase match run \
  --run-dir data/matches/fm-scale-spsa-stc --seed 102000000 \
  --candidate commit:f965ba27cba16a95c3ed1e9e9436ced7d5d219e9 \
  --baseline commit:203ba2a7ff47cc9de9ccf11ba4196965644c7c4b \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

基本シードは、設計書の起案時の33000000が[棋力向上段階8の測定](strength-stage8-correction-stc.md)の基本シード33000919とペア数の範囲で重なるため、保存済みのすべての測定の基本シードからペア数以上離れた102000000に改めた。

## エンジン

候補はブランチ`fm-scale-spsa`の`f965ba2`（SHA-256 `013cd392…`）、基準はmaster `203ba2a`（SHA-256 `14bafab5…`）である。
runnerはmaster `203ba2a`のビルドを`data/worktrees/fm-scale-spsa-runner/`に固定して使った。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は16である。
同じ測定機でlishogiのBotが稼働しており、他の対局測定と学習は走っていなかった。

## 結果

167ペア（有効164ペア、手数上限による破棄3ペア）で`H1`となった。
ペンタノミアル度数は候補の得点順に[14, 0, 82, 4, 64]、LLRは+2.968（判定境界+2.944）、候補の得点率は65.9%である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は1,698秒（約28分）である。
2026年10月8日21時20分から21時48分に実施した。

## 結論

候補は`H1`であり、異常は0件なので、振分け規則に従って[LTC](fm-scale-spsa-ltc.md)へ進める。
STCの通過は採用の根拠にしない。
