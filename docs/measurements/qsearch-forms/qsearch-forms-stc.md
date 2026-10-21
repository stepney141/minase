# 静止探索の補正と枝刈りの形の変更の候補のSTC

## 目的

[静止探索の補正と枝刈りの形の見直しの設計書](../../plans/archived/qsearch-forms.md)のフェーズ5として、SPSAの最終値を反映した候補と起点を、短時間条件（10秒＋1手0.1秒）の逐次検定STCで対局させ、強さの変化を記録する。
[統括計画](../../plans/search-forms.md)の決定により、この判定は採否に使わない。

## コマンドライン

統括計画の決定により、SPSAを終えた調整機で、持ち時間を換算せずに実行した。

```console
target/release/minase match run \
  --run-dir /workspace/job/matches/qsearch-forms-stc --seed 4550000000 \
  --candidate commit:c6cf0c729a6b8209e34c2a1df0f9744a87aae733 \
  --baseline commit:662c1ab6789e05ae8ef3c994772055f190d34996 \
  --each time=10000+100 --concurrency 191 gsprt --max-pairs 3000
```

実行ディレクトリは、終了後に測定機の`data/matches/qsearch-forms-stc`へ取得した。

## エンジン

候補は、形の変更に[SPSAの最終値](qsearch-forms.md)を反映したコミット`c6cf0c729a6b8209e34c2a1df0f9744a87aae733`である。
起点は、設計書を始めた時点のmaster`662c1ab6789e05ae8ef3c994772055f190d34996`（[サブタスク4](../../plans/archived/correction-history-tables.md)を採用した版）である。
runnerは、調整機で形の変更の最後のコミット`e9abbc86`をビルドした通常のminase（版2.4.0、SHA-256`1585bb71…`）である。
規則は`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiのインスタンス56989145（AMD EPYC 9654の2基構成、物理192コア、論理384スレッド、スペイン）である。測定機との1コアの速さの比`f`は1.26である（[時間切れの事前確認](qsearch-forms-safety.md)）。
両エンジンの`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は191である。
GSPRTの仮説は既定のelo0 = 0、elo1 = 10、α = β = 0.05である。

## 結果

| 量 | 値 |
|---|---:|
| 判定 | `H1` |
| 判定までのペア | 107（有効107、手数上限到達による破棄0） |
| ペンタノミアル度数 | [1, 1, 32, 7, 66] |
| LLR | +2.946 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 243.8秒（`summary.json`の`active_wall_time_ns`） |

有効107ペアの度数から求めた候補の平均得点は0.818であり、Eloに直すと約+261、正規化したペア得点の分散から求めた95%信頼区間はおよそ+211〜+322である。
逐次検定は境界を越えた時点で止まるので、この点推定は真の差より大きめに出やすい。固定局数の推定ではなく、採否にも使わない。
保存されたペアは番号1〜370のうち322個である。逐次検定はペア番号の順に結果を取り込み、107番目のペアで判定が確定したので、その時点で並行して進んでいた番号108以降のペアは統計に含まれていない。
標準出力の`elapsed`は369.1秒であり、`summary.json`の累計より長い。

## 結論

候補は、調整機で持ち時間を換算しないSTCで、起点より有意に強かった（`H1`）。
時間切れとエンジン異常は0件だった。
2026年10月18日の利用者の決定により、設計書の変更はこの判定によらず採用し、LTCは行わない。
起点はサブタスク4（[correction historyの表の設計書](../../plans/archived/correction-history-tables.md)）を採用した版であり、サブタスク4のSTCは、その1つ前の版に対して約+30 Elo（`H1`）だった（[記録](../correction-history-tables/correction-history-tables-stc.md)）。
2つのSTCは条件と相手が違うので、Eloを足し合わせて元のmasterとの差とはしない。7本のサブタスクを通して元のmasterより強くなったかは、統括計画のフェーズ10で確かめる。
