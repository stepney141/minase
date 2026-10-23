# 持ち時間の伸縮の形の変更の候補のSTC

## 目的

[探索の不安定さに応じた持ち時間の伸縮の設計書](../../plans/time-instability-scaling.md)のフェーズ5として、SPSAの最終値を反映した候補と起点を、短時間条件（10秒＋1手0.1秒）の逐次検定STCで対局させ、強さの変化を記録する。
[統括計画](../../plans/search-forms.md)の決定により、この判定は採否に使わない。

## コマンドライン

統括計画の決定により、SPSAを終えた調整機で、持ち時間を換算せずに実行した。

```console
target/release/minase match run \
  --run-dir /workspace/job/matches/time-instability-scaling-stc --seed 5150000000 \
  --candidate commit:1930dcb29cec7d2fc3ebe5bdb65a8f47c9e24eb5 \
  --baseline commit:370f707780548b0fa65d3e1303881a9fd609c05b \
  --each time=10000+100 --concurrency 95 gsprt --max-pairs 3000
```

実行ディレクトリは、終了後に調整機から測定機の`data/matches/time-instability-scaling-stc`へ複製した。

## エンジン

候補は、形の変更に[SPSAの最終値](time-instability-scaling.md)を反映したコミット`1930dcb29cec7d2fc3ebe5bdb65a8f47c9e24eb5`である。
その後に加えたコミット`903ab780`はテストだけを直したものであり、エンジンの実行ファイルには影響しない（[事前対局の記録](time-instability-scaling-smoke.md)）。
起点は、設計書を始めた時点のmaster`370f707780548b0fa65d3e1303881a9fd609c05b`（[サブタスク6](../../plans/archived/aspiration-window-forms.md)を採用した版）である。
runnerは、調整機で形の変更の最後のコミット`68d3a4d4`をビルドした通常のminase（版2.4.0、SHA-256`462b83de…`）である。
規則は`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiのインスタンス57236964（AMD Ryzen Threadripper PRO 9995WX、物理96コア、論理192スレッド、ギリシャ）である。測定機との1コアの速さの比`f`は1.00である（[時間切れの事前確認](time-instability-scaling-safety.md)）。
両エンジンの`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は95である。
GSPRTの仮説は既定のelo0 = 0、elo1 = 10、α = β = 0.05である。

## 結果

| 量 | 値 |
|---|---:|
| 判定 | `H1` |
| ペア | 303（有効297、手数上限到達による破棄6） |
| ペンタノミアル度数 | [32, 17, 151, 27, 70] |
| LLR | +2.945 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 784.0秒（`summary.json`の`active_wall_time_ns`） |

有効297ペアの度数から求めた候補の平均得点は0.572（Eloに直すと約+51）であり、正規化したペア得点の分散による95%信頼区間はおよそ+27〜+75である。
LLRは303ペア目で`H1`の境界+2.944を越えた。
終局の内訳は、詰みによる勝ち551局、反復による引き分け47局、駒枯れによる勝ち2局、および手数上限の打ち切り6局だった。
`pairs/`には、303より大きい番号のペアも含めて計522ペアが保存されている。runnerは終局したペアを番号の順に逐次検定へ取り込むので、統計に入ったのは判定までの1〜303番のペアだけである。

## 結論

候補は、調整機で持ち時間を換算しないSTCで、起点に対して`H1`となった（LLR +2.945、約+51 Elo）。
時間切れとエンジン異常は0件だった。
2026年10月18日の利用者の決定により、設計書の変更はこの判定によらず採用し、LTCは行わない。
候補は時間管理の形に加えて、探索の係数もSPSAで調整し直しているので、この差は時間管理の変更だけによるものではない。
起点はサブタスク6を採用した版であり、サブタスク6のSTCは、その1つ前の版に対して上限の3,000ペアで判定保留（約+8.6 Elo）だった（[記録](../aspiration-window-forms/aspiration-window-forms-stc.md)）。
2つのSTCは条件と相手が違うので、Eloを足し合わせて元のmasterとの差とはしない。7本のサブタスクを通して元のmasterより強くなったかは、統括計画のフェーズ10で確かめる。
