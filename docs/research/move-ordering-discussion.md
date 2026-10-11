# 手の順序付け（move ordering）の改善余地に関する調査と議論

## 結論

minaseの通常探索の手の順序付けには改善の余地がある。βカットの遅れは、件数で見ると「置換表（探索済み局面の結果を保存して再利用する表）の記録手がなく、残り深さが1で、最初の捕獲手が静的交換評価（SEE、到達升での駒の取り合いを見積もる計算で、正なら得、負なら損）で負になる」というノードに集中するが、その遅れが探索ノード数としてどれほどの損失に当たるかは測られておらず、これまでの指標では換算できない。
2026年10月9日に、Claude（本書の書き手）とcodexが現行のソースと測定記録を読み合わせ、2巡の反論を経て次の結論に達した。

- 初手βカット率（通常探索のβカットのうち最初に探索した手によるものの割合）は、現行masterで深さ5では66.4%、深さ8では70.1%である（[move-ordering-baseline-bench](../measurements/move-ordering-baseline-bench.md)）。この値は、Chess Programming Wikiが順序付けの目安とする90%超を下回る。
- この指標はβカットを件数で集計するので、件数の83〜87%を占める残り深さ1のノード（[2026年9月27日の監査](../audits/beta-cutoff-first-move-2026-09-27.md)の値であり、現行masterでは再測定していない）に支配される。残り深さ1でβカットが遅れたときに無駄になるのは静止探索（読みの末端で捕獲手だけを読み進める探索）の部分木であり、残り深さの大きいノードで遅れたときに無駄になるのは通常探索の部分木である。両者の大きさの差は未測定であり、件数の指標ではこの差を区別できない。したがって順序付けの損失をノード数で測るには、βカットした手より前に読んだ手の部分木ノード数の和（以下、先行費用W）を残り深さ別・先頭手の種別別に集計する診断が必要であり、これが設計書の最初のフェーズになる。
- 各枝刈りはいずれも「探索済みの最善値が詰み帯（詰みを表す評価値の範囲）の外である」ことを適用条件にするので、手のループの最初の手は決して枝刈りされない。置換表の記録手がないノードではMVV-LVA順（取る駒の価値の降順、取る側の駒の価値の昇順）の先頭がSEEで負になる捕獲手であることが多く、その手だけは枝刈りの対象にならずに必ず読まれる。
- history表（βカットを起こした静かな手の統計。静かな手とは相手の駒を取らない手）の値は、加点、半減、減衰の更新しか行われないので負にならない。したがってLMR（late move reductions、順序付けの後方の静かな手を浅く読む手法）の減深量を決める`pruning.rs`の`lmr_reduction`にある「history値が−146以下なら減深を1増やす」分岐は、現行では発動しない。
- 候補は8件に絞り、実装費用が小さく新しい表や手生成を要しないものから、1主張1測定で順に検証する。費用の小さい候補を先にするのは、効果の有無を早く確かめ、大きな表を要する候補の設計をその結果の後に回すためである。各候補の順位は本書の「候補の順位」節に記す。

具体的な設計は[手の順序付けの改善](../plans/move-ordering.md)で定める。

## 調査した版

調査対象のminaseは、masterのコミット`6b97cc26`（2026年10月9日）である。
参照したStockfishのコードは、手元の複製のコミット`17a6c8f1`（2026年9月19日取得）の`src/movepick.cpp`、`src/search.cpp`、`src/history.h`であり、本書の行番号はこの版のものである。
数値定数は調整で頻繁に変わるので、値はこの版に固有である。

## 現行の順序付け

通常探索の手選択器`MovePicker`（`crates/minase/src/search/alphabeta/ordering.rs`）は、次の順に合法手を1手ずつ返す。

1. 置換表の記録手。
2. MVV-LVA順の捕獲手。捕獲価値には捕獲履歴（capture history、捕獲手の成績を学習する表）の補正を加える。
3. killer手（根からの同じ手数のノードで直前にβカットを起こした静かな手。同じ手数の近いノードでも有効と見込む）2手。
4. 残りの静かな手。butterfly history（手番、移動元、移動先を鍵とし、βカットを起こした回数に基づく成績を記録する表）の降順で並べ、同点は生成順（駒種の順、次に升の順）とする。

静止探索（`quiesce.rs`）は、置換表の記録手を先頭にし、次に取る駒の価値のランク群ごとに捕獲手を遅延生成して、群内では取る側の駒の価値の昇順で返す。
捕獲履歴は静止探索では使わない。
読む捕獲手の数は`QsearchMoveLimit`（現行4）で制限し、最後の王駒の捕獲、直前の捕獲への取り返し、および獅子の2枚取りを例外とする。

根の探索（`root.rs`）では、通常の順序付けキーで整列したうえで置換表の記録手を先頭に置き、直前の反復の最善手をさらにその前へ回す。

history表は根の探索ごとに`HistoryDecay`（現行23%）だけを残して持ち越す。
捕獲履歴とkiller表は探索ごとに初期化する。

## 確認した機構

以下はソースで確認した事実である。

- `negamax.rs`のfutility pruning、late move pruning、SEE pruningの3つの枝刈りは、いずれも`best_score > -MATE_THRESHOLD`を条件に含む。`best_score`は`-INFINITY`から始まるので、手のループの最初の手はこれらで枝刈りされない。2手目以降のSEEが負の捕獲手は、残り深さ3以下の零窓ノードで損が余裕値を超える場合に枝刈りされる。
- 2026年9月27日の監査では、捕獲段の候補の42%がSEEで負と判定された。βカットしたノードのうち最初に探索した手がSEE負の捕獲手だった局面において、その手自身がβカットを起こした割合は深さ5で18.4%、深さ8で40.3%だった（残り深さは限定していない。[監査](../audits/beta-cutoff-first-move-2026-09-27.md)）。
- butterfly historyの更新は`depth²`の加点、上限超過時の全体の半減、および根ごとの減衰だけなので、値は常に非負である。したがって`pruning.rs`の`lmr_reduction`にある「`history <= -threshold`なら減深を1増やす」という条件は成立しない。
- `bench`は局面ごとに置換表とhistory表を消す（`bench.rs`）。したがって`bench`の統計は対局内の持ち越しの効果を含まない。
- 捕獲履歴の更新は、捕獲手がβカットを起こしたときにだけ行われる。静かな手がβカットを起こしたノードで先に読んだ捕獲手は減点されない（`negamax.rs`の`record_cutoff`の呼び出し条件）。
- 段階5でmalus（β打ち切りを起こさなかった静かな手への減点）を実装した対局候補`c002357`のLMRは、残り深さ3以上かつ4手目以降の固定減深であり、history値を参照していない。

## 現状の測定

[move-ordering-baseline-bench](../measurements/move-ordering-baseline-bench.md)の値は次のとおりである。

| 深さ | βカット | 初手βカット | 初手βカット率 | 全手走査ノードの最善手順位 1位 / 4位以降 |
|---:|---:|---:|---:|---|
| 5 | 31,195 | 20,726 | 66.4% | 50.9% / 44.0% |
| 8 | 488,713 | 342,486 | 70.1% | 51.3% / 42.0% |

監査の値（db716df、59.5%と76.7%）とは、静止探索の手数制限、捕獲履歴、history持ち越し、およびFM（factorization machine、2駒関係の評価補正）の有無が異なるため、時系列としては比べない。

## 議論の経過

Claudeが7つの見立て（R1からR7）を提示し、codexがコードと記録で検証して反論し、Claudeが再反論した。
結果として変わった点を記す。

| 論点 | Claudeの当初の読み | codexの検証 | 結果 |
|---|---|---|---|
| R1 指標 | 初手βカット率は残り深さ1に支配され、損失を過大に見積もる。ノード重み付きの先行費用Wで測るべきである。 | Wの定義と、既存の`search-stats`に近い費用で実装できることを支持した。ただし「残り深さ1の損失は数ノード」という前提は静止探索が再帰するため未確認とし、先行費用の全量が削減可能とは限らないこと、祖先と子孫で重複計上することを注記した。 | Wを診断の主指標とする。分母、カット手の種別、捕獲段で終わり静かな手を生成しなかった件数を加える。 |
| R2 最初の手の枝刈り免除 | pruning nodeでは先頭のSEE負捕獲だけが必ず読まれ、2番目以降は枝刈りされるという非対称性がある。 | 肯定した。ただし2手目以降も余裕値、王駒の捕獲、王駒への利きで保護される。 | 候補2の根拠とする。 |
| R3 malus不採用の原因 | 負のhistoryがLMRの減深を増やした影響が混在していた。 | 否定した。当時のLMRは固定減深でhistoryを参照していない。 | Claudeが撤回した。再測定するなら順位変更、更新式、減深補正を分けて測る。 |
| R4 静かな手の同点 | 同点は生成順で情報がなく、PST（piece-square table、駒と升ごとの評価値の表）の差分は費用ゼロの同点破りである。 | 同点の問題があることは認めつつ、「費用ゼロ」を修正した。候補ごとの表参照、成る手の駒の扱い、序中盤と終盤の補間が必要なので、費用は小さいが0ではない。 | 候補5とする。費用が0でないので、実装前に同点群の大きさと、成功手が同点群のどこにあるかを測り、PST差分が成功手を前へ動かす場合だけ進める。 |
| R5 静止探索への捕獲履歴 | 静止探索は呼び出し数の63〜79%を占め、手数制限が4なので順序付けがどの捕獲手を読むかを決める。 | 支持した。ただし呼び出し数は時間比ではなく、整数除算で補正が0になり得るので順位が実際に変わる率を測る。 | 候補3とする。 |
| R6 counter move / continuation history | 持ち越しが23%の弱い保持なので再診断しても参照率は低い。 | 23%はbutterflyの係数であり新表に同じ率を使う必然はないと修正した。再開条件は成立している。 | 候補6として再診断する。表2枚で約70 MBの費用は別に設計する。 |
| R7 根の順序 | 効果はaspiration windowsの再探索に限られる。 | 当初は根でもPVSの再探索と部分木の探索量に効くとして3位に置いた。Claudeの「直前最善手を先頭に回すので順序が効くのは最善手が交替する反復に限る」という反論に対し、交替する反復の利得が主であることは認めつつ、根でもβでの打ち切りと全窓の読み直しがあり、先に読んだ部分木が置換表とhistoryを更新する効果は他の反復にもあり得るとして、効果が交替時だけに限られるという見方は否定した。そのうえで現行版での費用削減の証拠がないことを理由に7位へ下げた。 | 候補7とし、最善手が交替した反復のノード比を先に数える。 |

codexは追加の候補として、静かな手がβカットしたときにも探索済み捕獲手の捕獲履歴を減点する案（候補1）と、捕獲履歴の根をまたぐ持ち越し（候補4）を挙げた。
前者はStockfishが`update_all_stats`で行う更新と同じ構造である。

測定の単位については、候補1、2、3を1つにまとめず別々に測ることで一致した。
まとめると1回の測定で分かるのは組合せ全体の効果だけであり、どの項目が効いたかを特定するには各項目を1つずつ外した版との比較が必要になるからである。

## Stockfishの現行方式

比較のために、参照版の手選択器を記す。

- 段は`MAIN_TT`、`CAPTURE_INIT`、`GOOD_CAPTURE`、`QUIET_INIT`、`GOOD_QUIET`、`BAD_CAPTURE`、`BAD_QUIET`の順であり、killer手の段はない（`movepick.cpp` 33〜41行）。
- 捕獲手の順序付けの値は、捕獲履歴の値に取られる駒の価値の7倍を加えた値であり、`see_ge(m, -value / 18)`を満たす手を良い捕獲として先に返し、残り（悪い捕獲）を良い静かな手の後、悪い静かな手の前に回す（`movepick.cpp` 225〜226行、311行、334〜360行）。
- 静かな手の順序付けの値は、butterfly historyの2倍、pawn historyの2倍、continuation history（1、2、3、4、6手前の手で引く表）、王手を掛ける手への加点（SEEが−75以上の手に限る）、価値の低い駒に狙われる升から逃げる手への加点と狙われる升へ入る手への減点、および浅いplyでのlow ply historyの項の和である（`movepick.cpp` 233〜251行）。
- βカットまたは最善手の確定時に、最善手が静かな手なら探索済みの静かな手を減点し、最善手の種別によらず探索済みの捕獲手を減点する。bonusとmalusは深さに応じたそれぞれ別の一次式で決まる（`search.cpp` 1995〜2034行）。
- 表の上限は`ButterflyHistory`が7,183、`CapturePieceToHistory`が10,692、`PieceToHistory`（continuation historyの要素）が30,000である（`history.h` 128〜143行）。

## 候補の順位

次の順に、1主張1測定で進める。順位は実装費用と既存の土台で決めており、効果の大きさの順ではない。

| 順位 | 候補 | 一般名 | 過去の結果との関係 | 最初の診断 |
|---:|---|---|---|---|
| 1 | 静かな手のβカット時にも探索済み捕獲手の捕獲履歴を減点する | capture history malus | 未検証。探索済み捕獲手の列と表は既にある。 | 影の表で後続ノードの順位変更率と成功手の順位差を測る。 |
| 2 | 捕獲段の先頭1手だけSEEを判定し、負なら合法なkiller手を先に返す | killers before losing captures | 段階12の項目2（全捕獲手のSEEを生成時に計算して静かな手の後ろへ回す、STC `H0`）の適用範囲を限定した変種。追加のSEEはノードあたり最大1回。 | 先頭が負かつ合法killerがある件数、その先行費用W、静かな手の生成が前倒しになる率。 |
| 3 | 静止探索の価値群内の順序に捕獲履歴を使う | capture history in quiescence ordering | 未検証。段階12は通常探索への適用と別の主張と定めている。 | 候補が2つ以上の群の数、先頭が変わる群の率、αを固定した手数制限の集合が変わる率。 |
| 4 | 捕獲履歴を根をまたいで持ち越す | capture history aging | 段階12の項目5が条件付きで定めた未実施の項目。 | 対局再生で持ち越し表と初期化表の順位変更率を比べる。 |
| 5 | 静かな手のhistory同点をPST差分で破る | PST-delta tiebreak | 未検証。段階5のpiece-to history（STC `H0`）とは信号が異なる。 | 同点群の大きさの分布と、PST差分が成功手を前へ動かす率。 |
| 6 | counter move historyとcontinuation historyを持ち越し状態で再診断する | counter move history、continuation history | 段階5で非ゼロ参照率（静かな手の順序付けで表の値が0でない手の割合）が0.73%と0.62%と低かったため見送り。段階12の再開条件は成立済み。 | 対局再生で非ゼロ参照率と順位変更率を測る。 |
| 7 | 根の手を前回反復の情報で整列する | root move ordering | 未検証。 | 深さ別の最善手交替率と、交替した反復のノード比。 |
| 8 | malusをbonusと別係数で再測定する | history malus | 段階5でSTC上限到達時のLLRが−1.58となり不採用。再び候補とする理由は、当時のbenchで探索ノード数が17.9%減っていたことと、係数を持つ改良は既存の係数と調整し直してから捨てるという教訓である。現行では負のhistory値がLMRの減深を増やす経路が新たに動くので、順序の効果と減深の効果を分けて測る必要がある。 | 影の表で順位差とLMRの減深変更率を別々に測る。 |

## 一次資料

- Stockfish `src/movepick.cpp`、`src/search.cpp`、`src/history.h`（コミット17a6c8f1、GPL-3.0-or-later）: [movepick.cpp](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/movepick.cpp)、[search.cpp](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/search.cpp)、[history.h](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/history.h)。
- Chess Programming Wiki「Move Ordering」: <https://www.chessprogramming.org/Move_Ordering>。初手βカット率の目安の出典である。
- [最初の手でβカットが起きる割合の原因調査](../audits/beta-cutoff-first-move-2026-09-27.md)（2026年9月27日、コミットdb716df）。
- [棋力向上段階5](../plans/strength-stage5.md)、[棋力向上段階12](../plans/strength-stage12.md)、[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)。
- 測定記録: [strength-stage5-malus-stc](../measurements/strength-stage5-malus-stc.md)、[strength-stage5-malus-bench](../measurements/strength-stage5-malus-bench.md)、[strength-stage5-piece-history-stc](../measurements/strength-stage5-piece-history-stc.md)、[strength-stage5-cmh-bench](../measurements/strength-stage5-cmh-bench.md)、[strength-stage5-continuation-bench](../measurements/strength-stage5-continuation-bench.md)、[strength-stage12-bad-captures-stc](../measurements/strength-stage12-bad-captures-stc.md)、[strength-stage12-bad-captures-bench](../measurements/strength-stage12-bad-captures-bench.md)、[strength-stage6-iteration-diag](../measurements/strength-stage6-iteration-diag.md)、[move-ordering-baseline-bench](../measurements/move-ordering-baseline-bench.md)。
- 教訓: [不採用の枝刈りは数え方を点検し、係数を調整し直してから捨てる](../lessons/retune-rejected-pruning-before-discarding.md)、[失う良手の割合は深さの上限のない再帰での枝刈りの損失を抑えない](../lessons/recall-loss-does-not-bound-recursive-pruning.md)。
