# 段階7の段階開始版との固定200ペア測定

## 目的

[棋力向上段階7](../plans/strength-stage7.md)の進捗指標として、採用中の鏡映共有モデルを段階開始版と固定200ペアで比較する予定であった。
世代2候補の診断後の処理方針が確定する前に開始したため、途中で停止した。
本記録は未完了の実行を保存するものであり、段階7の完了を示す測定結果ではない。

## コマンドライン

```console
data/strength-stage7/match_runner \
  --run-dir data/matches/strength-stage7-elo200 --seed 20920903 \
  --candidate commit:960c56bcbb6088d0f0466d670ceea36dffeda5b7 \
  --baseline commit:d0d3c52024c5ab4d74cc82588640a55953426aad \
  --rules engine-default --each time=10000+100 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 12 \
  --max-ply 4096 --response-timeout 120 elo --pairs 200
```

2026年9月14日2時39分32秒（日本標準時）に開始し、同日2時46分50秒にSIGINTで停止した。
runnerの終了コードはシグナルを表す−2、起動用Pythonプロセスの終了コードは254だった。
正確な引数列と起動時刻は`data/strength-stage7/elo200-invocation.json`、標準出力は`elo200.stdout`、停止後の確認は`elo200-interruption.json`に保存した。

## エンジン

候補は採用済み鏡映共有モデルのコミット`960c56bcbb6088d0f0466d670ceea36dffeda5b7`、基準は段階開始コミット`d0d3c52024c5ab4d74cc82588640a55953426aad`である。
規則は両者とも`L0,P0,R1,E0`であり、世代2の学習候補は組み込んでいない。

| 対象 | 実行ファイルのSHA-256 |
|---|---|
| 候補 | `b3728444d9a844c83bc5959b5832959cb2ce0d647a4f5addd1a5ddcebf4bc154` |
| 基準 | `63c3512f466f47b0c3b8951b3fc74e0b786bc0b91b8ef7e6449fd16159a8c79f` |
| runner | `900d67a658d6075f34ef44a906e5c930ae661232f3f435be5a4de90498bd62ad` |

起動前の確認は`data/strength-stage7/phase6-preflight.json`、シードの衝突確認は`phase6-seed-audit.json`にある。
基本シード20920903の派生前入力範囲20920904〜20921103を予約し、中断しても未使用の範囲として割り当て直さない。

## 環境

CPUはIntel Core Ultra 7 265KFの物理20コア・論理20コアで、実メモリは33,218,924,544バイトである。
両エンジンの探索ワーカー数は1、置換表は各256 MB、同時対局数は12とした。
GPU学習、教師生成、および他の対局測定は並行していなかった。

## 結果

停止後に共有ロックの取得を確認し、37ペアの保存ファイルを保持した。
保存された最大番号は41だが欠番があり、ペア番号1〜200がそろった状態ではない。
中断後もmanifest、summary、および各ペアの保存物を変更していない。
200ペアのElo、信頼区間、および全400局の異常件数は未確定である。

起動用プロセスが記録した経過時間は437.3976528990024秒だった。
保存済みsummaryの最終スナップショットは`active_wall_time_ns = 433347401377`、`invocation_active = true`、`interrupted = false`であり、強制停止後の状態が未反映である。
これらを手動で書き換えず、停止の事実を別の記録に残した。
通常完走としての`match_report`による集計とスループット校正は行っていない。

## 結論

利用者は世代2候補の除外を選ばず、[符号反転を抑える学習方法の検討](strength-stage7-removal-learning.md)を選択した。
その後の再学習で世代2の重みを採用したため、この旧構成の測定は再開せず、[最終構成の固定200ペア](strength-stage7-gen2-elo200.md)を新しい保存先とシードで完了した。
その確定前に本測定を始めたのは実行手順の誤りであり、37ペアを保存した時点で停止した。
保存済み37ペアは中断記録として保持し、最終構成の測定結果へ混ぜない。
