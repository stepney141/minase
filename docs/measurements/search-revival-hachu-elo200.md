# 探索改良の再調整を採用した後の版の固定200ペア（HaChuとの比較）

## 目的

[探索改良の再調整](../plans/search-revival-spsa.md)を採用した時点のmaster（コミット`9027662`）をHaChuと固定200ペアで比較し、進捗指標として記録する。
前回の[SPSA採用後の版の記録](strength-stage12-hachu-elo200.md)（コミット`df0c75e`）以降、masterには探索部の不具合修正（null moveの反復の修正）、世代3の局面によるPSTの再学習、および探索改良の再調整が統合されている。
採否には使わない。

## コマンドライン

対局ハーネスは、測定対象と同じコミットに固定したworktree `data/worktrees/search-revival-hachu-runner` でビルドしたバイナリから起動した。

```console
data/worktrees/search-revival-hachu-runner/target/release/match_runner \
  --run-dir data/matches/search-revival-hachu-elo200 --seed 90270000 \
  --candidate commit:9027662d8d1761be02337eece76421f27f05cc38 \
  --baseline cecp:/home/stepney141/board-games/hachu-debian/hachu \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 19 \
  --max-ply 4096 --response-timeout 120 \
  elo --pairs 200
data/worktrees/search-revival-hachu-runner/target/release/match_report \
  --run-dir data/matches/search-revival-hachu-elo200
```

## エンジン

候補は`9027662`であり、バイナリのSHA-256は`0c56367e503fa6791cd282b6bb800e6b2549fe9f9535f42991e2a4dc785af084`である。
HaChuはDebianパッケージ収録のオリジナル版（コミット`df26f4a`）を段階8と同じ手順でビルドしたもので、規則は既定設定に対応する`L1,L3,P0,P5,P6,R2,E1,E2`を審判層とminaseの双方に与えた。
両エンジンの置換表は256 MB、minaseは`Threads=1`、HaChuは探索ワーカー数を報告しない。
runnerのSHA-256は`affa2716a26ae67b1cc3f023edc78f31cc5a302f33c6126d59d815ea539a72e0`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）である。
同時対局数は、前回の記録と同じく物理コア数から1を引いた上限の19とした。
同じ機械ではlishogi BotとminaseのGUIのコンテナが稼働していた。
2026年10月4日3時26分から5時22分に実施した。

## 結果

| 量 | 値 |
|---|---:|
| 有効ペア | 200（破棄0） |
| ペンタノミアル度数 | [0, 0, 4, 0, 196] |
| 正規化した平均得点 | 0.990 |
| Elo | +798.3 |
| 95%信頼区間 | [+678.7, +1410.0] |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 7,012.7秒（`summary.json`の累計は7,006.9秒） |

値は`match_report`が保存記録から再計算した集計による。
minaseは400局中396局に勝ち、396局はいずれもHaChuが詰みを自認して結果行を返した投了だった。
負けた4局はペア29、70、75、142の各1局であり、いずれもHaChuがminaseの最後の王駒を実際に取って終局した（296手から665手）。
4ペアとも、もう1局はminaseが勝っている。
得点率が上限に近いため、Eloの点推定と信頼区間の上限は少数の負けの有無で大きく動き、信頼区間は上側へ大きく偏っている。

## 結論

探索改良の再調整を採用した後の版は、HaChuに対して固定200ペアで+798.3 Elo（95%信頼区間[+678.7, +1410.0]）であった。
信頼区間の下限は、前回の[+530.7 Elo](strength-stage12-hachu-elo200.md)の信頼区間の上限+635.2 Eloを上回り、前回からの棋力の向上が進捗指標にも表れている。
一方で、得点率99%の領域では固定200ペアのEloの分解能が低いため、今後の進捗を同じ条件で追うには、HaChu側に時間の優位を与えるなど、得点率が上限から離れる条件の検討が必要になる。
