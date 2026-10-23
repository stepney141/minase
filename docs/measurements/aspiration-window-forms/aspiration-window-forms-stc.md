# aspiration windowsの形の変更の候補のSTC

## 目的

[aspiration windowsの幅と窓の移し方の見直しの設計書](../../plans/archived/aspiration-window-forms.md)のフェーズ5として、SPSAの最終値を反映した候補と起点を、短時間条件（10秒＋1手0.1秒）の逐次検定STCで対局させ、強さの変化を記録する。
[統括計画](../../plans/archived/search-forms.md)の決定により、この判定は採否に使わない。

## コマンドライン

統括計画の決定により、SPSAを終えた調整機で、持ち時間を換算せずに実行した。

```console
target/release/minase match run \
  --run-dir /workspace/job/matches/aspiration-window-forms-stc --seed 4850000000 \
  --candidate commit:469009043d784a28259d7e59f5a2c232a9b23ad9 \
  --baseline commit:03f5d79dab86eac1ad2cdb15a3c0faaf3d867ad6 \
  --each time=10000+100 --concurrency 191 gsprt --max-pairs 3000
```

実行ディレクトリは、終了後に測定機の`data/matches/aspiration-window-forms-stc`へ取得した。

## エンジン

候補は、形の変更に[SPSAの最終値](aspiration-window-forms.md)を反映したコミット`469009043d784a28259d7e59f5a2c232a9b23ad9`である。
起点は、設計書を始めた時点のmaster`03f5d79dab86eac1ad2cdb15a3c0faaf3d867ad6`（[サブタスク5](../../plans/archived/qsearch-forms.md)を採用した版）である。
runnerは、調整機で形の変更の最後のコミット`7631da34`をビルドした通常のminase（版2.4.0、SHA-256`ffaf3f22…`）である。
規則は`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiのインスタンス57080553（AMD EPYC 9K84の2基構成、物理192コア、論理384スレッド、ベトナム）である。測定機との1コアの速さの比`f`は1.37である（[時間切れの事前確認](aspiration-window-forms-safety.md)）。
両エンジンの`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は191である。
GSPRTの仮説は既定のelo0 = 0、elo1 = 10、α = β = 0.05である。

## 結果

| 量 | 値 |
|---|---:|
| 判定 | 判定保留（上限の3,000ペアに到達） |
| ペア | 3,000（有効2,946、手数上限到達による破棄54） |
| ペンタノミアル度数 | [501, 188, 1,506, 167, 584] |
| LLR | +2.177 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 2,459.9秒（`summary.json`の`active_wall_time_ns`） |

有効2,946ペアの度数から求めた候補の平均得点は0.512であり、Eloに直すと約+8.6、正規化したペア得点の分散から求めた95%信頼区間はおよそ+0.6〜+16.5である。
LLRは上限の時点で+2.177であり、`H1`の境界+2.944に届かなかった。[SPRTの手引き](../../guides/sprt.md)の振分け規則ではLLRが0以上なのでLTCへ進む扱いになるが、統括計画の決定により、本書ではLTCを行わない。
終局の内訳は、詰みによる勝ち5,499局、反復による引き分け387局、駒枯れによる勝ち47局と引き分け7局、王駒の捕獲による勝ち6局、および手数上限の打ち切り54局だった。

標準出力の`elapsed`は4,053.2秒であり、`summary.json`の累計より約1,593秒長い。
runnerは、最後のペアを書き終えた後、最終の集計を出力する前に、保存した全ペアの棋譜を1スレッドで再生して先読みの予想手の数を数え直す（`match_runner`の`ponder_summary`）。
3,000ペア、6,000局の再生に、この差の約1,593秒がかかった。前の5本のサブタスクのSTCは最大625ペアで判定が確定したので、この再生の時間は短かった。

## 結論

候補は、調整機で持ち時間を換算しないSTCで、3,000ペアの上限までに起点との差を判定できなかった（判定保留、LLR +2.177、約+8.6 Elo）。
時間切れとエンジン異常は0件だった。
2026年10月18日の利用者の決定により、設計書の変更はこの判定によらず採用し、LTCは行わない。
起点はサブタスク5（[静止探索の形の設計書](../../plans/archived/qsearch-forms.md)）を採用した版であり、サブタスク5のSTCは、その1つ前の版に対して約+261 Elo（`H1`）だった（[記録](../qsearch-forms/qsearch-forms-stc.md)）。
2つのSTCは条件と相手が違うので、Eloを足し合わせて元のmasterとの差とはしない。7本のサブタスクを通して元のmasterより強くなったかは、統括計画のフェーズ10で確かめる。
