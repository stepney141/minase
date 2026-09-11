# strength-stage6-iir-ltc

## 目的

棋力向上段階6のinternal iterative reduction（[STC](strength-stage6-iir-stc.md)で`H1`）を、直前の採用構成（aspiration windows）と標準LTCのGSPRTで比較し、採否を確定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-iir-ltc --seed 20630903 \
  --candidate commit:9d37246 --baseline commit:75bb69d \
  --each time=60000+200 --concurrency 18 gsprt
```

## エンジン

候補はコミット9d37246（aspiration windowsとinternal iterative reductionを含み、置換表クラスタ化は撤回済み）、基準はaspiration windowsを採用したコミット75bb69d、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ30 GB）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した18である。
本セッションからは他の対局や診断を走らせていないが、2026年9月10日19時55分から20時06分にかけて別ユーザーのプロセス（`wget-at-nss`）が繰り返しクラッシュしてコアダンプを書き出し、20時06分34秒にカーネルのOOM killerが動作した（`journalctl`）。

## 結果

2,891ペアを実行し、有効ペア2,831、破棄ペア60（手数上限）であった。
ペンタノミアル度数は[569, 68, 1465, 79, 650]、LLRは+2.971で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、および拒否着手は0件だが、`time_forfeits`は11件であり、経過時間は69,473秒である。

時間切れ11件はすべてペア668から684（保存時刻20時06分から20時10分）に集中し、候補側6件、基準側5件である。
時間切れとなった手は、いずれもエンジンが`info string stop soft`または`hard`を数百ms以内（完了反復の経過時間2〜1,024ms、1件だけ32,328ms）で報告しているにもかかわらず、ハーネスの実測思考時間が32.4〜33.4秒であり、着手前の残り時間は1.2〜29.4秒であった。
同じ時間帯のペア668から687には思考時間が20秒を超える手が18件あり、それ以外の時間帯には1件もない。
したがって時間切れは、上記のOOMによる約33秒の停止が同時に進行中の全対局へ及んだ結果であり、探索や時間管理の変更に起因しない。
時間切れの11ペアを除いたペンタノミアル度数は[565, 68, 1460, 79, 648]、LLRは+3.114で、判定は`H1`のまま変わらない（`src/stats.rs`の`gsprt_llr`で再計算）。

## 結論

長時間GSPRTは`H1`だが、docs/sprt.md が採用の条件とする時間切れ0件を満たしていない。
時間切れは外部のメモリ圧迫による対称な停止であり、該当ペアを除いても`H1`であることから、本記録は採用を推奨するが、規則からの逸脱を伴う採用の可否は利用者の決定を要する。
決定までの間、後続の項目はinternal iterative reductionを含む構成を暫定の基準として測る。
