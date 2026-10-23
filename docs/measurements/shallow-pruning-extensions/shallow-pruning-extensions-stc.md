# 浅い前向き枝刈りの拡張の候補のSTC

## 目的

[浅い前向き枝刈りの深さと対象の拡張](../../plans/archived/shallow-pruning-extensions.md)のフェーズ5として、[SPSA](shallow-pruning-extensions.md)の最終値を反映した候補が起点より短時間条件で強いかを、GSPRTで測る。
2026年10月18日の利用者の決定により、統括計画の各サブタスクはSTCの結果によらず採用し、STCは採否に使わない記録とし、LTCは行わない（[統括計画](../../plans/archived/search-forms.md)の設計判断「各サブタスクの採否」）。

## コマンドライン

[統括計画](../../plans/archived/search-forms.md)の決定により、SPSAを終えた調整機で、持ち時間を換算せずに実行した。

```console
target/release/minase match run \
  --run-dir /workspace/job/matches/shallow-pruning-extensions-stc --seed 3650000000 \
  --candidate commit:29443569e7328386f6858c0695232792934cef2e \
  --baseline commit:a892a5e66106e756dd94df862aa0a2de3779f326 \
  --each time=10000+100 --concurrency 191 gsprt --max-pairs 3000
```

実行ディレクトリは、終了後に測定機の`data/matches/shallow-pruning-extensions-stc`へ取得した。

## エンジン

候補と起点、およびrunnerは、[事前対局](shallow-pruning-extensions-smoke.md)と同じ実行ファイルである。
候補は`29443569e7328386f6858c0695232792934cef2e`（SHA-256 `336555125d0f28fded37615747361b4a7a7c92ede4951dec84aa3bc81a6a0973`）、起点は`a892a5e66106e756dd94df862aa0a2de3779f326`（SHA-256 `6b3114774f3bf666ab4843d56c0bd9ab505609a8092ad017206d3ddd993ab20b`）である。
runnerは、調整機で`364c10aa`をビルドした通常のminase（2.4.0、SHA-256 `d70c957afebd29773a32c23fb1849fdd1f7ab484bc0a94e04bed2c8f7dd4e3f0`）であり、起動の記録は1件で、測定は中断なく1回の起動で終えた。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、SPSAと同じvast.aiのAMD EPYC 9654の2基構成（インスタンス56567047、物理192コア、論理384スレッド、runnerが記録した論理CPUは368）である。
この機械の1コアは測定機（Intel Core Ultra 7 265KF）より遅く、負荷時の速度の比`f`は1.57である（[事前確認の記録](shallow-pruning-extensions-safety.md)）。持ち時間は換算していないので、1手の探索量は測定機の同じ条件より少ない。
候補と起点はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は191である。
手数上限は4,096手、応答タイムアウトは120秒、仮説はH0がelo=0、H1がelo=10、α = β = 0.05である。
2026年10月19日8時57分から9時31分に実施した。
STCを終えた後、調整機を削除した。作成からの費用は約20.4米ドル（約10.1時間 × 1時間2.01米ドル）であり、SPSA、事前対局、およびSTCの合計である。

## 結果

| 量 | 値 |
|---|---:|
| 判定 | `H1` |
| 判定までのペア | 1,381（有効1,363、手数上限到達による破棄18） |
| ペンタノミアル度数 | [255, 40, 712, 32, 324] |
| LLR | +2.988 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 1,254.0秒（`summary.json`の`active_wall_time_ns`） |

有効1,363ペアの度数から求めた候補の平均得点は0.524であり、Eloに直すと約+17、正規化したペア得点の分散から求めた95%信頼区間はおよそ+4〜+29である。
逐次検定は境界を越えた時点で止まるので、この点推定は真の差より大きめに出やすい。固定局数の推定ではなく、採否にも使わない。
保存されたペアは番号1〜1,920のうち1,775個であり、判定の時点で並行して進んでいた番号1,382以降のペアは統計に含まれていない。
標準出力の`elapsed`は2,051.9秒であり、`summary.json`の累計より長い。

## 結論

候補は、調整機で持ち時間を換算しないSTCで、起点より有意に強かった（`H1`）。
時間切れとエンジン異常は0件だった。
利用者の決定により、本書の変更はこの判定によらず採用し、LTCは行わない。
起点は[前の設計書](../../plans/archived/search-pruning-forms.md)を採用した版であり、前の設計書のSTCは、さらに1つ前のmaster`99ccc6bb`に対して約−17 Elo（`H0`）だった（[記録](../search-pruning-forms/search-pruning-forms-stc.md)）。
2つのSTCは条件と相手が違うので、Eloを足し合わせて元のmasterとの差とはしない。7本のサブタスクを通して元のmasterより強くなったかは、統括計画のフェーズ10で確かめる。
