# 捕獲履歴はStockfishの上限、初期値、および更新量で更新し、寄与を歩兵価値の定数倍に抑えて並べ替えと静止探索に使い、LMRのstatScoreにも使う

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

捕獲履歴（capture history）は、動かす駒、到達升、および取られる駒種ごとに捕獲手の成績を学習し、捕獲手の順序に使う表である。
[旧ADR](archive/bound-capture-history-and-use-it-in-qsearch.md)は、2026年10月10日に、通常探索の捕獲手を取る駒の価値の合計に捕獲履歴の換算値を加えた値で並べ、換算値の絶対値を歩兵価値の定数倍以下に抑え、重力式で更新し、静止探索では取る駒の価値の群の中の順序に使うと決めた。
加減点の量は`min(深さ², 上限)`であり、上限`CaptureHistoryLimit`は調整する係数だった。
旧ADRは、PSTの時期の測定に基づいていた。

2026年10月17日のエンジンの開発者の決定により、形や値がStockfishとYaneuraOuで違えばStockfishに合わせ、Stockfishにだけある項目は入れ、minaseにあってStockfishにない仕組みのうち中将棋の診断や測定で決めたものは残す。
[historyの形の設計書](../plans/archived/history-forms.md)は、この決定に従って捕獲履歴の更新をStockfishの形へ寄せた。

## 決定

捕獲履歴の上限を、Stockfishと同じ定数10692とし（`src/history.h:135`）、初期値を−742とする（`src/search.cpp:715`）。
更新は、[静かな手の更新のADR](order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md)と同じ加点`b`と減点`m`を使い、最善値が探索開始時のαを上回ったノードで、捕獲の最善手には`b`の1427/1024の加点を、最善手の種類によらず探索済みの捕獲手には`m`の1489/1024の減点を与える（同2024-2042行）。
探索済みの手は、ノードの手数が32以下で、読んだ時点でαを上回らなかった手に限る。
fail lowのノードでは、このノードへ至った手が捕獲手なら、その手の要素に892を与える（同1636-1643行）。
各更新は、更新量を上限で切り詰めてから重力式を使う。

通常探索の捕獲手の並べ替えは、旧ADRと同じく、取る駒の価値の合計に捕獲履歴の換算値を加え、換算値の絶対値を歩兵価値に`CaptureHistoryScale`の百分率を掛けた値以下に抑える。
`CaptureHistoryScale`の開始値は、Stockfishの`捕獲履歴 + 7 × 駒の価値`の比に当たる734とする（`src/movepick.cpp:225-226`）。
静止探索では、旧ADRと同じく、取る駒の価値の群の構造を保ったまま、群の中の順序に捕獲履歴の生の値を使う。
LMRを捕獲手にも適用するので（[LMRのADR](reduce-all-later-moves-and-root-moves-by-lmr-with-stat-score.md)）、捕獲手のstatScoreを取る駒の価値の換算値の873/128と捕獲履歴の和とする（`src/search.cpp:1371-1373`）。
捕獲手のSEEとfutilityの枝刈りの余裕値に捕獲履歴を使う既存の仕組み（[枝刈りのADR](prune-captures-and-quiet-moves-by-see-and-capture-futility-with-depth-scaled-margins.md)）は変えない。

## 検討した代案

上限を調整する係数として残す案は、更新量を上限に比例させて換算する限り、上限は値の分解能を変えるだけで探索の挙動に効かないので採らなかった。
換算値に上限を設けない案と、上限を超えたときに全体を半分にする案は、旧ADRと同じ理由で採らなかった。
並べ替えの換算をStockfishと同じ`捕獲履歴 + 7 × 駒の価値`の形に改める案は、旧ADRが中将棋の測定で決めた形なので、エンジンの開発者の決定により採らず、尺度の開始値だけをStockfishの比に置き直した。

## 結果

更新と並べ替えの式は`crates/minase/src/search/alphabeta/capture_history.rs`と`ordering.rs`の`HistoryUpdates`、`capture_stat_score`のdocコメントにあり、係数の現行値は`params.rs`にある。
寿命は[持ち越しのADR](carry-histories-within-a-game-and-share-continuation-and-pawn-histories.md)が定める。
変更の効果は、[SPSAの記録](../measurements/history-forms/history-forms.md)と[STCの記録](../measurements/history-forms/history-forms-stc.md)にある。
