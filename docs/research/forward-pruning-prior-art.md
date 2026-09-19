# 前向き枝刈りの第3層に関する既存実装の調査

## 結論

[棋力向上段階8](../plans/strength-stage8.md)が扱う4項目について、StockfishとYaneuraOuのソースを調べた結論は次のとおりである。

- 静的評価の補正は、両エンジンとも「王手を受けているノード」と「最善手が捕獲手であるノード」では更新しない。捕獲手による駒得は評価の偏りではなく戦術上の利得であり、これを取り込むと補正が駒得の期待値を学習してしまうためである。段階8の起案時の設計にはこの除外がなく、フェーズ1の診断で観測した差の平均絶対値が歩兵12枚分と大きいことと符合する。
- 更新の向きの条件は段階8の設計と同じである。最善手がある（下限または正確な値）ノードは探索値が静的評価より高いときだけ、最善手がない（上限）ノードは探索値が静的評価以下のときだけ更新する。
- 置換表へ保存する静的評価は補正前の値であり、`improving`の比較には補正後の値を使う。段階8は`improving`の比較に補正前の値を使うと決めており、この点は異なる。
- `improving`は2手前の静的評価との比較であり、王手を受けているノードでは2手前の値を継承して偽とする。効果は、子ノードのfutility（reverse futility）の余裕値、null moveの前提条件、ProbCutの閾値、LMRの基礎値、および手数による枝刈りの閾値`(3 + depth²) / (2 − improving)`に及ぶ。親ノードのfutilityの余裕値には使わない。
- 捕獲手のSEEによる枝刈りは、余裕値を`177 × depth`に捕獲履歴の項を加えた値とし、深さの上限とPVノードの除外を持たず、探索済みの最善値が負けの値でないことだけを条件にする。YaneuraOuのコメントは、将棋ではSEEが負の手でも詰みになり得ると注意している。
- 静かな手の履歴による枝刈りは、continuation historyとpawn historyの和が`−4136 × depth`未満の手を対象にする。負の履歴値（malus）を前提にした条件であり、履歴値が非負の本エンジンへはそのまま移せない。

## 調査した版

Stockfishはmasterのコミット`17a6c8f`（2026年9月19日に取得）の`src/search.cpp`、`src/history.h`、`src/types.h`である。
YaneuraOuはローカルの複製（コミット`1308ab3`、2026年7月10日）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`である。
以下の行番号はこれらの版のものであり、数値定数は調整で頻繁に変わるので値はこの版に固有である。

## 静的評価の補正

Stockfishの更新条件は`search.cpp`の1654行から1661行にある。

```cpp
if (!ss->inCheck && !(bestMove && pos.capture(bestMove))
    && (bestValue > ss->staticEval) == bool(bestMove))
{
    auto bonus = std::clamp(int(bestValue - ss->staticEval) * depth * (bestMove ? 12 : 18) / 128,
                 -CORRECTION_HISTORY_LIMIT / 4, CORRECTION_HISTORY_LIMIT / 4);
    update_correction_history(pos, ss, *this, 1061 * bonus / 1024);
```

`bestMove`は探索値がαを上回ったときだけ設定されるので（1541行から1543行）、条件式は置換表の値の種類を直接使わずに同じ区別を表す。
鍵はpawnの配置、minor pieceの配置、および各色のpawn以外の駒の配置の4種で、直前手の駒と到達升で引く継続補正を2手前、4手前、6手前について加える（`history.h` 158行、227行から248行、`search.cpp` 87行から103行）。
補正後の値は`v + cv / 131072`を終盤データベースの値域の内側へ切り詰めたもので（109行）、補正量そのものの上限は表の要素の上限1,024から間接的に決まる。
更新量は`(探索値 − 静的評価) × depth × k / 128`を±256へ切り詰めた値で、`k`は最善手があれば12、なければ18である。
置換表には補正前の値を保存し（866行、1649行）、`improving`、null move、futility、ProbCut、および静止探索のstand patは補正後の値を使う。
補正量の絶対値は、子ノードのfutilityの余裕値（1021行）とLMRの減深量（1347行）にも入る。

YaneuraOuは6手前の継続補正を持たない点と係数を除いて同じ形である（498行から、4091行から4098行）。

## improving

定義は`search.cpp`の876行と877行にある。

```cpp
improving         = ss->staticEval > (ss - 2)->staticEval;
opponentWorsening = ss->staticEval > -(ss - 1)->staticEval;
```

王手を受けているノードは`ss->staticEval = eval = (ss - 2)->staticEval`（843行）で2手前の値を継承するので、`improving`は偽になる。
YaneuraOuは同じ継承に加えて`improving = false`を明示的に代入する（2697行から2699行）。

`improving`が変える式は次の5か所である。

- 子ノードのfutility（1016行から1021行）は、余裕値を`futilityMult × depth − (2789 × improving + 335 × opponentWorsening) × futilityMult / 1024 + abs(cv) / 198435`とする。
- null moveの前提条件（1029行）は`staticEval + 50 × priorNMPFailHigh >= beta − 13 × depth − 47 × improving + 365`である。
- ProbCut（1081行、1087行）は`probCutBeta = beta + 241 − 64 × improving`とし、探索の深さを`depth − (improving ? 5 : 3)`とする。
- LMRの基礎値（1910行）は`!improving`のときに`reductionScale × 197 / 512`を足す。
- 手数による枝刈りの閾値（1193行）は`(3 + depth × depth) / (2 − improving)`である。

razoringは`improving`を使わない（1008行）。

## 捕獲手のSEEによる枝刈り

外側の条件は`!rootNode && pos.non_pawn_material(us) && !is_loss(bestValue)`（1190行）であり、対象は捕獲手と王手の手である（1199行）。

```cpp
int margin = 177 * depth + captHist * 34 / 1024;
if ((alpha >= VALUE_DRAW || pos.non_pawn_material(us) != PieceValue[movedPiece])
    && !pos.see_ge(move, -margin))
    continue;
```

この直前に捕獲手のfutility（1205行から1211行）があり、`staticEval + 234 + 247 × lmrDepth + PieceValue[captured] + 134 × captHist / 1024 <= alpha`の手を展開しない。
YaneuraOuの余裕値は`max(167 × depth + captHist × 34 / 1024, 0)`で、条件は`alpha >= VALUE_DRAW`だけである（3312行から）。

## 手数と履歴による静かな手の枝刈り

手数の閾値を超えると、以後の静かな手の生成を打ち切る（1193行から1194行）。
履歴による枝刈りは`contHist[0] + contHist[1] + pawnHistory < −4136 × depth`を条件とする（1228行）。
続いて履歴値で`lmrDepth`を増減し、親ノードのfutility（`staticEval + 119 × lmrDepth + 90 × (staticEval > alpha) + 164 <= alpha`、`lmrDepth < 12`）と静かな手のSEE（`−23 × lmrDepth²`、1252行）を適用する。
YaneuraOuは履歴の閾値が`−4097 × depth`、LMRの除数が固定の3,220である。

## 確認していない点

YaneuraOuの`reduction()`の定数と、`#if`の分岐が既定のビルドでどちらへ解決されるかは読んでいない。
HaChuは調査の対象外である。
