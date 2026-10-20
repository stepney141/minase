# correction historyの表の変更の候補のSTC

## 目的

[correction historyの表の追加と持ち越しの設計書](../../plans/archived/correction-history-tables.md)のフェーズ5として、SPSAの最終値を反映した候補と起点を、短時間条件（10秒＋1手0.1秒）の逐次検定STCで対局させ、強さの変化を記録する。
[統括計画](../../plans/search-forms.md)の決定により、この判定は採否に使わない。

## コマンドライン

統括計画の決定により、SPSAを終えた調整機で、持ち時間を換算せずに実行した。

```console
target/release/minase match run \
  --run-dir /workspace/job/matches/correction-history-tables-stc --seed 4250000000 \
  --candidate commit:5ed505eca0a525dff9f21c1d6d3e166f82d04089 \
  --baseline commit:e21b765545d679d55add9b324c1a3b08d77e8f6d \
  --each time=10000+100 --concurrency 191 gsprt --max-pairs 3000
```

実行ディレクトリは、終了後に測定機の`data/matches/correction-history-tables-stc`へ取得した。

## エンジン

候補は、形の変更に[SPSAの最終値](correction-history-tables.md)を反映したコミット`5ed505eca0a525dff9f21c1d6d3e166f82d04089`である。
起点は、設計書を始めた時点のmaster`e21b765545d679d55add9b324c1a3b08d77e8f6d`（[サブタスク3](../../plans/archived/history-forms.md)を採用した版）である。
runnerは、調整機で形の変更の最後のコミット`e695176a`をビルドした通常のminase（版2.4.0、SHA-256`d3e7ea8c…`）である。
規則は`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiのインスタンス56856484（AMD EPYC 9K84の2基構成、物理192コア、論理384スレッド、ベトナム）である。測定機との1コアの速さの比`f`は1.29である（[時間切れの事前確認](correction-history-tables-safety.md)）。
両エンジンの`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は191である。
GSPRTの仮説は既定のelo0 = 0、elo1 = 10、α = β = 0.05である。

## 結果

| 量 | 値 |
|---|---:|
| 判定 | `H1` |
| 判定までのペア | 625（有効610、手数上限到達による破棄15） |
| ペンタノミアル度数 | [96, 27, 315, 20, 152] |
| LLR | +2.973 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 819.3秒（`summary.json`の`active_wall_time_ns`） |

有効610ペアの度数から求めた候補の平均得点は0.543であり、Eloに直すと約+30、正規化したペア得点の分散から求めた95%信頼区間はおよそ+12〜+48である。
逐次検定は境界を越えた時点で止まるので、この点推定は真の差より大きめに出やすい。固定局数の推定ではなく、採否にも使わない。
保存されたペアは番号1〜1,159のうち1,095個である。逐次検定はペア番号の順に結果を取り込み、625番目のペアで判定が確定したので、その時点で並行して進んでいた番号626以降のペアは統計に含まれていない。
標準出力の`elapsed`は1,311.4秒であり、`summary.json`の累計より長い。

## 結論

候補は、調整機で持ち時間を換算しないSTCで、起点より有意に強かった（`H1`）。
時間切れとエンジン異常は0件だった。
利用者の決定により、設計書の変更はこの判定によらず採用し、LTCは行わない。
起点はサブタスク3（[historyの形の設計書](../../plans/archived/history-forms.md)）を採用した版であり、サブタスク3のSTCは、その1つ前の版に対して約+80 Elo（`H1`）だった（[記録](../history-forms/history-forms-stc.md)）。
2つのSTCは条件と相手が違うので、Eloを足し合わせて元のmasterとの差とはしない。7本のサブタスクを通して元のmasterより強くなったかは、統括計画のフェーズ10で確かめる。
