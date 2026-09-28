# SPSA採用後の版の固定200ペア（HaChuとの比較）

## 目的

SPSAの2回の調整を採用した後の版（[棋力向上段階12](../plans/strength-stage12.md)の段階開始版、コミット`df0c75e`）をHaChuと固定200ペアで比較し、進捗指標として記録する。
段階12は採用項目なしで完了し、最終構成は段階開始版と同じなので、この記録は段階12の最終構成の進捗指標を兼ねる。
採否には使わない。

## コマンドライン

対局ハーネスは、段階開始版に固定したworktree `data/worktrees/strength-stage12-runner` でビルドしたバイナリから起動した。

```console
data/worktrees/strength-stage12-runner/target/release/match_runner \
  --run-dir data/matches/strength-stage12-hachu-elo200 --seed 82500000 \
  --candidate commit:df0c75ef980b74040e25313c8d60aff17ddb985a \
  --baseline cecp:/home/stepney141/board-games/hachu-debian/hachu \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 19 \
  --max-ply 4096 --response-timeout 120 \
  elo --pairs 200
data/worktrees/strength-stage12-runner/target/release/match_report \
  --run-dir data/matches/strength-stage12-hachu-elo200
```

## エンジン

候補は`df0c75e`である。
HaChuはDebianパッケージ収録のオリジナル版（コミット`df26f4a`）を段階8と同じ手順でビルドしたもので、規則は既定設定に対応する`L1,L3,P0,P5,P6,R2,E1,E2`を審判層とminaseの双方に与えた。
両エンジンの置換表は256 MB、minaseは`Threads=1`、HaChuは探索ワーカー数を報告しない。
runnerのSHA-256は`ac3485eff3a5f5c8005d7c80e135a8d64f031422b923aefff190711ae7ada075`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）である。
同時対局数は、利用者の指示により、物理コア数から1を引いた上限の19とした。
同じ機械ではlishogi BotとminaseのGUIのコンテナが稼働していた。
2026年9月28日18時04分から20時21分に実施した。
同日17時56分に同時対局数10で開始した回は、完了ペアが0件の時点で中止し、実行ディレクトリを削除して同じシードでやり直した。

## 結果

| 量 | 値 |
|---|---:|
| 有効ペア | 200（破棄0） |
| ペンタノミアル度数 | [0, 0, 18, 0, 182] |
| 正規化した平均得点 | 0.955 |
| Elo | +530.7 |
| 95%信頼区間 | [+463.6, +635.2] |
| エンジン異常 | 不正着手1（HaChu側）、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 8,204.7秒（`summary.json`の累計は8,198.0秒） |

値は`match_report`が保存記録から再計算した集計による。
不正着手はペア80の第2局（1,012手目）で、HaChu（先手）が審判層で不合法な手を返したものであり、段階8と段階9の記録と同じくminaseの勝ちとして算入した。
時間切れは同時対局数19でも0件だった。

## 結論

SPSA採用後の版は、HaChuに対して固定200ペアで+530.7 Elo（95%信頼区間[+463.6, +635.2]）であり、段階9の完了時の[+449.4 Elo](strength-stage9-hachu-elo200.md)と信頼区間が重なる。
