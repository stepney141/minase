# strength-stage5-hachu-elo200

## 目的

棋力向上段階5の最終構成（順序付けキーの重複計算の除去とLMRの減深量を採用）の外部エンジンHaChuに対する強さを、固定200ペアのEloで進捗指標として記録する。
採否の判定には使わない。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage5-hachu-elo200 --seed 20530903 \
  --candidate commit:73578d4 --baseline "cecp:../hachu-debian/hachu" \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 --concurrency 10 \
  elo --pairs 200
```

## エンジン

候補は段階5の最終構成のコミット73578d4、基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian`でビルド）であり、規則オプションはHaChuの既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）である。
規則セットは審判層とminase側の双方に`L1,L3,P0,P5,P6,R2,E1,E2`を与えた。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補の`Threads`は1、`USI_Hash`は256MB、HaChuのメモリは256MB、同時対局数は明示の10である。
開始から約50分は自己対局の固定200ペア（同時対局数9）が並行していた。

## 結果

200ペアを実行し、有効ペア200、破棄ペア0であった。
ペンタノミアル度数は[9, 0, 83, 0, 108]、Eloの点推定は+188.5（95%信頼区間+153.1〜+228.0）であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は19,344秒である。

## 結論

段階5の最終構成はHaChuに対して+188.5 Eloであり、段階4完了時の+200.2 Elo（95%信頼区間+164.8〜+239.9）と信頼区間が重なるため、HaChu戦では差を検出できない。
進捗指標として記録する。
