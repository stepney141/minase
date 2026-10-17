# 探索の枝刈りとLMRの関数形の見直しの設計書

## 先に読む要約

本書は、探索の前向き枝刈り（読む価値が低いと見込んだ手やノードを読まずに済ませる枝刈り）と減深の式を、チェスのStockfishと将棋のYaneuraOu（やねうら王）が共通に持つ形へ寄せ、現行のmasterより強くなるかを確かめる。
[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)によれば、minaseの探索の係数の多くは両エンジンと違う形で使われている。その差は、係数の値をSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で選び直しても埋まらない。
本書が変えるのは4つである。
1つ目に、reverse futility pruning（静的評価がβを十分に上回るノードを読まずに返す枝刈り）の余裕値を深さに比例させ、improving（静的評価が同じ手番の2手前より上がっている状態）のときに減らし、適用する深さの上限を係数にする。
2つ目に、correction history（探索値と静的評価のずれを学習して静的評価を補正する手法）の補正を、futility pruningだけでなくreverse futility pruning、razoring、null move pruning、およびimprovingの判定にも使う。
3つ目に、late move pruning（後方の静かな手を読まない枝刈り）の上限を、improvingでないときに小さくする。
4つ目に、late move reductions（LMR、後方の手を浅く読む減深）の減深量に定数項、improvingの項、およびPVノードの項を加え、上限を係数にする。
新しい係数は現行と同じ探索になる値を開始値とし、既存の係数とまとめてSPSAで調整する。
調整した版を、起点のmasterと短時間条件STCおよび長時間条件LTCの2段階の逐次検定（段階ゲート）で対局させ、1つの候補として採否を決める。
完了条件は、候補の採否が測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案した。
着手は利用者の指示による。
次の一手は、利用者の指示を受けて起点を確定し、フェーズ1（形の変更の実装）を始めることである。

## 目的

本書が確かめるのは、「枝刈りと減深の式を両エンジンの形へ寄せ、新しい形の係数を調整すれば、現行の形で調整した探索より強くなる」という仮説である。
発端は、2026年10月17日に現行の形のまま探索と時間管理の38係数をNNUEに合わせてSPSAで調整し直したとき、`ReverseFutilityMargin`（歩兵価値に対する百分率）が開始値428から下限0まで動いたことである。
この調整は、本書と後続の設計書で全係数を調整し直すので、反映せずに中止した。
現行のreverse futility pruningは、深さ1〜3に共通の1つの余裕値を補正前の静的評価に当てる形なので、深さ1と深さ3の余裕値の比を1以外にできない。
Stockfishの余裕値は深さに比例して増え、improvingのときは深さ3以下でほぼ0になる（`src/search.cpp:1022-1033`）。
現行の形のままでは、全深さの余裕値を0に近づけることしかこの挙動に近づく方法がなく、SPSAの動きはそのためである可能性がある。
形を変えて係数を調整し直せば、浅い深さと深い深さで別の余裕値を選べるようになる。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、フェーズ1を始める時点のmasterの先頭のコミットである。
- **形の変更**は、本書の4項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で表（`crates/minase/src/search/alphabeta/params.rs`）の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **improving**は、`negamax.rs`の`improving`が定める判定であり、同じ手番の2手前の局面より静的評価が上がったことをいう。
- **PVノード**は、窓の幅が1より大きいノードである。窓の幅が1のノードを零窓のノードという。
- **詰み帯と専用の帯**は、詰みまでの手数を表す評価値の範囲と、千日手などの反復の結果を表す評価値の範囲（`crates/minase/src/search/mod.rs`の`MATE_THRESHOLD`と`REPETITION_BAND_START`）である。通常の評価値はこれらの外にある。
- 減深量と深さの単位は、探索の深さ1を1手と数える。

対象は次の4項目である。

- reverse futility pruningの余裕値と深さの上限（F1）。
- correction historyの補正の適用先の拡大（F2）。
- late move pruningのimprovingの項（F3）。
- late move reductionsの定数項、improvingの項、PVノードの項、および上限（F4）。

次の事項は対象外とする。王駒への利きの条件を除き、本書に続く設計書が扱う（[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)の「候補の扱い」の節に一覧がある）。

- butterfly historyの更新量の形とcontinuation history（直前の手を文脈とするhistory）の変更。手の並べ替え全体を変えるので、同じ候補に含めると採否の解釈が難しくなる。[history-forms.md](history-forms.md)で扱う。
- futility pruning、late move pruning、およびSEE（static exchange evaluation、駒の取り合いの損得の見積り）による枝刈りの深さの上限の拡張。これらの余裕値と上限は深さ1〜3の深さ別の係数で表されているので、深さ4以上へ広げるには、深さ別の係数を深さの式へ改める必要がある。[shallow-pruning-extensions.md](shallow-pruning-extensions.md)で扱う。
- null move pruningを試す条件に静的評価の余裕を加えること。条件を加えると試すノードが減るので、余裕を範囲の端まで大きくしない限り現行と同じ挙動にならず、開始値を範囲の内側に置けない。[shallow-pruning-extensions.md](shallow-pruning-extensions.md)で扱う。
- reverse futility pruningの返り値を、βと静的評価の重み付き平均にすること。[ADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked.md)は返り値の扱いを決めており、本書の4項目とは独立に測れる。[shallow-pruning-extensions.md](shallow-pruning-extensions.md)で扱う。
- 静止探索のstand-patへの補正と手数の数え方（[qsearch-forms.md](qsearch-forms.md)）、aspiration windows（[aspiration-window-forms.md](aspiration-window-forms.md)）、および時間管理の形（[time-instability-scaling.md](time-instability-scaling.md)）。
- late move reductionsを捕獲手、killer手、および根へ広げること、およびhistory補正を連続量にすること。後者は[ADR](../adr/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)が棄却しているが、[history-forms.md](history-forms.md)がSPSAで係数を決める形で扱う。根でのLMRはどの設計書も扱わない。
- 王駒に相手の利きが届くノードを枝刈りから除く条件。王駒が取られ得る局面の静的評価は探索値から大きく外れるので、除外を続ける（[ADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked.md)）。

## 依存関係

起点は、フェーズ1を始める時点のmasterの先頭であり、起点の表の既定値をSPSAの開始値とする。
本書は、探索の関数形を両エンジンの形へ寄せる一連の設計書の最初であり、後続の設計書は本書の完了後のmasterを起点とする。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、同時対局数は16を上限とする。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は次のADRの決定の一部を変える。
採用した場合は、完了時に、起点の時点で有効なADRを確かめたうえで新しいADRで置き換え、置き換えた旧ADRを`docs/adr/archive/`へ移す。決定の一部だけを変える場合は、残る決定も新しいADRへ引き継ぐ。

- [correction historyの鍵と適用先のADR](../adr/key-correction-history-by-material.md)は、補正をfutility pruningの判定にだけ使うと決めた。根拠となった表の持ち越しの診断は、補正をfutility pruningだけに使う前提で測ったものであり、適用先の拡大は測っていない。
- [improvingのADR](../adr/use-improving-only-for-futility-margins.md)は、improvingの判定に補正前の静的評価を使い、LMRの減深量をimprovingで変えないと決めた。補正後の評価で比べる案は、correction historyの学習につれて比較の基準が動くとして棄却している。F2は、比べる2つの局面の補正を同じ時点で同じ表から読むので、2つの値の間で表の状態がずれない。LMRに減深を1手加える案は、良い結果を失う割合の診断で棄却されたが、F4は増分を開始値0の係数にしてSPSAに選ばせる。
- [浅いノードの前向き枝刈りの適用条件のADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked.md)は、reverse futility pruningを残り深さ3以下に限ると決めた。F1はこの上限を係数にする。
- [late move reductionsの式のADR](../adr/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)は、上限を3とし、それを超える案を棄却した。同じADRは、この判断をSTCの到達深さが5〜6だった時期のものとしている。F4は上限を係数にする。
- [late move pruningの数え方のADR](../adr/count-only-quiet-moves-for-late-move-pruning.md)は、上限式にimprovingの項を含めなかった。理由は、improvingが同じ時期に別の項目として探索へ戻されたことである。F3は、両エンジンの上限式`(3 + d²)/(2 − improving)`（Stockfish `src/search.cpp:1199`、YaneuraOu `yaneuraou-search.cpp:3593`）に合わせてこの項を加える。

## 設計判断

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 4項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとにSPSAと段階ゲートを繰り返す案は、1項目あたりSPSAの約75時間にSTCとLTCを加えた約81〜113時間を要するので採らない。代償として、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| 新しい係数の開始値 | 開始値で現行と同じ探索になる値とする（深さの傾き0、improvingの減算0、倍率100%、定数項0、上限は現行の3）。人が選んだ値や両エンジンの値の換算を開始値にすると、その値の根拠を別に示す必要がある。現行と同じ挙動から始めれば、形の変更のコミットが探索を変えていないことを探索ノード数の一致で検証でき、最終値はSPSAが決める。補正の適用先の拡大（F2）だけは係数を持たず、開始時点から探索を変える。 |
| 新しい係数の範囲 | 両エンジンの対応する値を範囲に含め、開始値を範囲の内側に置く。minaseの余裕値は歩兵価値に対する百分率で、現行の`ReverseFutilityMargin`は428、`RazoringMargin2`は2015である。Stockfishの深さ1の余裕値（歩兵価値の約14〜41%）よりはるかに大きいので、範囲の上限は両エンジンの換算値ではなく、minaseの現行の値の数倍まで届くように取る。中将棋の歩兵は他の駒に比べて価値が小さく、歩兵価値で測った余裕値は大きな数になるからである。SPSAの最初の反復の摂動幅は最後の反復の約2.30倍（3,750^0.101、`crates/minase/src/bin/minase/spsa_runner/model.rs`）なので、`ReverseFutilityDepth`、`ReverseFutilityMaxDepth`、`LmrNonImproving`、および`LmrMaxReduction`は、序盤の摂動が範囲の下側で切り詰められる。[利得のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)はこの切り詰めを許容しており、係数ごとの摂動幅の設定は加えない。 |
| SPSAの対象 | 起点の調整対象の38係数（`FmScale`を除く）に新しい8係数を加えた46係数とする。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の6係数を含めるので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。3,750反復、1反復8ペア（計30,000ペア）、持ち時間10秒＋0.1秒、同時対局数16、`c_end`（最後の反復の摂動幅）は範囲の1/6、`r_end`（最後の反復の学習率）は0.002である（[SPSAの手引き](../guides/spsa.md)）。30,000ペアは候補を1つ作るための予算であり、最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。30,000ペアで46係数を十分に調整できるかは未検証であり、不採用はこの予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。 |
| reverse futility pruningの深さの上限 | 係数`ReverseFutilityMaxDepth`とし、他の枝刈りの上限（残り深さ3）とは切り離す。余裕値が深さに比例すれば深いノードでは余裕値が大きくなるので、上限を広げても深いノードを安易に打ち切らない。Stockfishは詰みの発見のためとして`depth < (seekMate ? 6 : 19)`に固定し（`src/search.cpp:1021-1022`）、YaneuraOuは`depth < 15`（残り深さ14まで、`yaneuraou-search.cpp:3225`）とする。どちらもチェスと本将棋で決めた値なので、範囲を1〜15としてSPSAに選ばせる。15はYaneuraOuより1手広い。 |
| reverse futility pruningの条件 | 「静的評価 − 余裕値 ≥ β」に加えて、「静的評価 ≥ β」を独立した条件とする。両エンジンも同じ条件を持つ（Stockfish `src/search.cpp:1022`、YaneuraOu `yaneuraou-search.cpp:3225`）。余裕値は深さの傾きやimprovingの減算で負になり得るので、この条件がないとβ未満の値を返して打ち切ることになる。現行の余裕値は0以上なので、この条件を加えても開始値では探索が変わらない。 |
| improvingのときの余裕値 | 余裕値から歩兵価値の`ReverseFutilityImproving`%を引く。両エンジンは、improvingのとき余裕値から深さによらない量を引く形を持つ（Stockfishは`2789 × mult/1024`、YaneuraOuは`2686 × mult/1024`）。余裕値に倍率を掛ける形は、倍率0で深いノードの余裕値まで0にするので採らない。 |
| 補正後の評価の使い方 | reverse futility pruning、razoring、null move pruningの減深量、およびimprovingの判定に、補正後の静的評価（補正前の値に`CorrectionTable::read`の値を足したもの）を使う。補正後の値は、評価関数の出力の範囲（`crates/minase/src/eval/nnue/`の`EVALUATION_LIMIT`）に収めて、詰み帯と専用の帯へ入れない。補正は、その値を使う判定の時点で読む。null move pruningの探索中にも補正表は更新されるので、null move pruningより前の判定（reverse futility pruning、razoring、null move pruning）とその後の判定（futility pruning、late move pruning、LMR）では、補正を読み直す。improvingは、現局面と2手前の局面の補正を同じ時点で読んで比べる。2手前がない場合と、比較の区間にnull moveがある場合はimprovingとみなす現行の例外は変えない。置換表の保存値、静止探索のstand-patとdelta pruning、および補正表の更新に使う差（探索値と補正前の評価の差）は変えない。静止探索も両エンジンは補正後の評価を使うが、stand-patの変更はすべての静止探索ノードの値を変え、通常探索の4項目と効果を切り分けられないので、別の候補とする。 |
| LMRの減深量の単位 | 減深量を1/100手単位の整数で計算し、最後に100で割って切り捨てる。現行の表は`floor(ln d × ln m/(LmrDivisor/100))`の整数を持つので、定数項や倍率を足すと、表の段階で切り捨てた端数が失われる。表に`floor(100x)`（xは切り捨て前の値）を持てば、整数`h`について`floor((floor(100x) + 100h)/100) = floor(x) + h`が成り立ち、新しい係数が開始値のとき現行と同じ減深量になる。両エンジンは1/1024手単位を使うが（Stockfish `src/search.cpp:1402-1403`）、minaseの表の係数は百分率で揃えているので1/100手とする。 |
| improvingを求めるノード | F3とF4のimprovingには静的評価が要る。現行でも、浅い枝刈りの対象ノード、null move pruningを試すノード、および補正表を更新するノードでは静的評価を求めている。F4では、LMRを適用するノードでまだ求めていなければ静的評価を求める。静的評価の計算は累算器の読み取りなので探索ノード数は変わらず、費用は探索速度（NPS、1秒あたりの探索ノード数）の低下として測る。 |

## 変更の内容

以下の式で、`P`は探索用の歩兵価値（評価関数の重みファイルが持つ値）、`d`はIIR（内部反復減深）を適用した後の残り深さ、`[条件]`は条件が成り立てば1、成り立たなければ0を表す。

### F1　reverse futility pruning

余裕値を次の式にする。

```math
\mathrm{margin}(d) = \frac{P \times (\mathrm{ReverseFutilityMargin} + \mathrm{ReverseFutilityDepth} \times (d - 1) - \mathrm{ReverseFutilityImproving} \times [\text{improving}])}{100}
```

reverse futility pruningは、残り深さが`ReverseFutilityMaxDepth`以下で、零窓、αとβが詰み帯と専用の帯の外、および王駒に利きが届かないという現行の条件を満たし、補正後の静的評価が「β以上」かつ「余裕値を引いてもβ以上」のときに行う。
`ReverseFutilityDepth`と`ReverseFutilityImproving`が0、`ReverseFutilityMaxDepth`が3のとき、現行と同じになる（補正後の評価を使う部分はF2で入る）。

### F2　correction historyの補正の適用先

reverse futility pruning、razoring、null move pruningの減深量の静的評価の項、およびimprovingの判定に、補正後の静的評価を使う。
補正を読む時点と値の範囲は、設計判断の表のとおりとする。
futility pruningは現行どおり補正後の評価を使う。

### F3　late move pruning

improvingでないとき、静かな手の上限`(LmpBase + LmpSlope × d²)/100`に`LmpNonImproving/100`を掛けて切り捨てる。
improvingのときの上限は現行のままである。
`LmpNonImproving`が100のとき、現行と同じになる。

### F4　late move reductions

1/100手単位の減深量`r`を次のように求める。

```math
r = \left\lfloor \frac{t \times (100 + \mathrm{LmrNonImproving} \times [\text{improvingでない}])}{100} \right\rfloor + \mathrm{LmrBase} - \mathrm{LmrPvReduction} \times [\text{PVノード}] + 100h
```

ここで`t`は表の値`floor(100 × ln d × ln m/(LmrDivisor/100))`であり、`m`は0から数えた手番号である。
`h`は現行のhistory補正であり、butterfly historyの値が`LmrHistoryThreshold`以上なら−1、`−LmrHistoryThreshold`以下なら+1、それ以外なら0である。
減深量は、`floor(r/100)`を0以上`min(LmrMaxReduction, d − 2)`以下に切り詰めた手数とする。
減深した手は残り深さ`d − 1 − 減深量`で読むので、上限`d − 2`により1以上に保たれ、静止探索へは落ちない。
対象の手（置換表の手、捕獲手、およびkiller手を除く非捕獲手）、手番号の扱い、および根で行わないことは変えない。
`LmrNonImproving`、`LmrBase`、`LmrPvReduction`が0、`LmrMaxReduction`が3のとき、現行と同じになる。

### 新しい係数

| 係数 | 開始値 | 範囲 | 範囲の根拠 |
|---|---|---|---|
| `ReverseFutilityDepth`（深さ1あたりの増分、歩兵価値に対する百分率） | 0 | −100〜400 | Stockfishの深さ1あたりの値は歩兵価値の約14〜41%（`mult = min(45 + 4d, 85) − 20×[置換表にない]`、歩兵価値208）、YaneuraOuは約61〜84%（`76 − 21×[置換表にない]`、歩兵価値90）。上限400では、現行の428に対して深さ3の余裕値が約2.9倍になり、両エンジンの深さ1と深さ3の比（約3）に届く。負の値は深さとともに余裕値が縮む形であり、開始値を内側に置くために含める |
| `ReverseFutilityImproving`（improvingのときの減算、歩兵価値に対する百分率） | 0 | −100〜600 | 歩兵価値に換算すると、Stockfishは約38〜112%（2789/1024 × 14〜41%）、YaneuraOuは約160〜230%（2686/1024 × 61〜84%）。上限600は現行の`ReverseFutilityMargin`428を上回り、improvingのとき浅い深さの余裕値を0以下にできる。負の値はimprovingのとき余裕値を増やす形であり、開始値を内側に置くために含める |
| `ReverseFutilityMaxDepth`（手） | 3 | 1〜15 | YaneuraOuは残り深さ14まで、Stockfishは18まで。設計判断の表を参照 |
| `LmpNonImproving`（improvingでないときの倍率、百分率） | 100 | 25〜150 | 両エンジンは50（上限が半分になる） |
| `LmrBase`（1/100手） | 0 | −150〜300 | Stockfishの定数項は(948 + 697)/1024で約1.6手、YaneuraOuは(1133 + 691)/1024で約1.8手（Stockfish `src/search.cpp:1350, 1917-1920`、YaneuraOu `yaneuraou-search.cpp:3886, 5148-5151`） |
| `LmrNonImproving`（improvingでないときの表の値の増分、百分率） | 0 | −50〜150 | Stockfishは表の積の197/512で約38%、YaneuraOuは206/512で約40% |
| `LmrPvReduction`（PVノードで減らす量、1/100手） | 0 | −100〜200 | StockfishはPVノードで512/1024手、つまり0.5手を減らす（`src/search.cpp:1186-1187`）。YaneuraOuのPVノードの項は窓の幅とttPvの条件に依存し（`yaneuraou-search.cpp:3879, 5150`）、単一の対応値がない |
| `LmrMaxReduction`（手） | 3 | 1〜8 | 両エンジンには上限がない。8は今回試す暫定の上限であり、最終値が上限に達した場合は測定記録に残す |

## 所要時間の見積り

時間は、同時対局数16の測定機での見積りである。
SPSAは、同時対局数16で3,000ペアの調整を完走した記録（[SPSAの手引き](../guides/spsa.md)の27,138秒、1ペア約9秒）から、30,000ペアを約75時間（約3日）と見込む。2026年10月17日に中止した38係数の調整は1反復あたり約1分で進んでおり、これより短く終わる可能性がある。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので、幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 形の変更の実装と検証（codexへ委任） | 半日程度 |
| 2 | 時間切れの事前確認（13反復、104ペア） | 約15分 |
| 3 | SPSA（30,000ペア） | 約75時間 |
| 4 | 反映、テスト、および事前対局（32ペア） | 約1時間 |
| 5 | 候補のSTC | 約1〜8時間 |
| 6 | 通過した場合のLTC | 約5〜30時間 |

測定機の占有は、段階2〜5の和でSTCで棄却される場合の約77〜84時間（約3.2〜3.5日）、段階6まで進む場合の約82〜114時間（約3.4〜4.8日）となる。

## 実装フェーズ

基本シードは、時間切れの事前確認を3200000000、セッションを3250000000、事前対局を3310000000、STCを3350000000、LTCを3400000000と仮に定める。
これらは起案時点で、保存済みの測定とSPSAの基本シード（`data/matches/`と`data/spsa/`の`manifest.json`）、および他の設計書が予約した値から、各測定のペア数以上離れた未使用の値である（[教訓](../lessons/derive-seed-adjacent-collision.md)）。
着手時に同じ方法で未使用であることを確かめ直し、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　形の変更の実装

起点からブランチ`search-pruning-forms`を切り、worktree `data/worktrees/search-pruning-forms`で作業する。
F1、F3、F4、F2の順に、1項目を1コミットにする。
探索を変えないF1、F3、F4を先に入れ、探索を変えるF2を最後に置くのは、F2による変化だけを分けて記録するためである。
F1、F3、F4の各コミットでは、`minase dev bench`の全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
F2のコミットでは同じbenchのノード数を記録し、起点との差を残す。
F4はLMRを適用するノードで静的評価を求めるので、`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点と形の変更で交互に3回ずつ測り、探索速度の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
新しい係数は`params.rs`の表へ追加し、調整用ビルド（`--features tuning`つきのビルド）が`Tune_<係数名>`のUSIオプションとして宣言することを確かめる。
新しい式の単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過、ノード数の一致と記録、および探索速度の記録である。

### フェーズ2　パラメーターファイルと時間切れの事前確認

形の変更の最後のコミット（F2まで入ったコミット）でビルドした`minase`を`data/spsa/search-pruning-forms-runner/`へ複製してrunner（測定を実行するプログラム）として固定し、以後のフェーズはこのrunnerで実行する。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除した46行のファイルを`data/spsa/search-pruning-forms.params`とする。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、使い捨てのスクリプトで機械的に突き合わせる。

続いて、手引きの第3節に従い、時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
写しの開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150へ書き換えた`data/spsa/search-pruning-forms-safety.params`で、13反復（104ペア）を実行する。

```console
<固定したminase> spsa \
  --run-dir data/spsa/search-pruning-forms-safety --seed 3200000000 \
  --engine commit:<形の変更> --params data/spsa/search-pruning-forms-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`search-pruning-forms-safety`に残す。
完了条件は、時間切れとエンジン異常が0件であることと、測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<固定したminase> spsa \
  --run-dir data/spsa/search-pruning-forms --seed 3250000000 \
  --engine commit:<形の変更> --params data/spsa/search-pruning-forms.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 3750 --pairs-per-iteration 8
```

最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`search-pruning-forms`に、手引きの第6節の項目とともに残す。
最終値を丸めた整数が範囲の上限または下限になった係数を示す。
新しい8係数については、最終値が開始値（現行と同じ挙動）からどれだけ離れたかを、`c_end`に対する比で示す。
この比は、どの形の変更が使われたかを読むための目安であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、テストの前提が新しい既定値でも成り立つようにテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で握りつぶさない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

採否測定の前に、候補が時間切れやエンジン異常を起こさないことと、時間の使い方の変化を確かめるため、STCの条件で候補と起点の32ペアの事前対局を行う。

```console
<固定したminase> match run \
  --run-dir data/matches/search-pruning-forms-smoke --seed 3310000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 elo --pairs 32
```

候補と起点の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ（反復深化で読み終えた深さ）、および探索を止めた理由の分布を集計して、測定記録`search-pruning-forms-smoke`に残す。
総思考時間の比と終局時の残り時間は、`pairs/`に保存された各手の思考時間から計算する。終局時の残り時間は、持ち時間に1手の加算を着手数だけ足し、思考時間の合計を引いた値である。
手数帯ごとの完了深さと停止理由の分布は、`scripts/clock_profile.py`を`--role candidate`と`--role baseline`で別々に実行して集計する。このスクリプトが出す時間予算の比は旧係数の予算式に基づくので、解釈に使わない。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、および測定記録の保存である。

### フェーズ5　採否測定

```console
<固定したminase> match run \
  --run-dir data/matches/search-pruning-forms-stc --seed 3350000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

STCの判定は[SPRTの手引き](../guides/sprt.md)の振分け規則に従う。
候補が有意に強い`H1`ならLTCへ進み、改善を示せない`H0`なら不採用とし、3,000ペアで判定保留ならその時点の対数尤度比の符号でLTCへ進むかを決める。
LTCは測定名の接尾辞を`-ltc`、基本シードを3400000000、持ち時間を`time=60000+200`とし、`--max-pairs`を付けない。
LTCで`H1`となり、エンジン異常、時間切れ、および拒否着手が0件なら採用し、`H0`なら不採用とする。
LTCの判定保留は、手引きのとおり`--resume`と`--target-pairs`で延長する。
結果は測定名ごとの測定記録に残す。
完了条件は、採否の確定と測定記録の保存である。

## 検証

- F1、F3、F4の各コミットで、`minase dev bench`の全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。
- 新しい8係数が調整用ビルドの`Tune_`オプションとして宣言され、パラメーターファイルの開始値と範囲が表と一致すること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`が通過すること。
- 時間切れの事前確認、SPSA、事前対局、および採否の測定で、エンジン異常、時間切れ、および拒否着手が0件であること。最初の測定で1件でも出た場合は、測定を止めて原因を調べる。

## 完了条件

- 形の変更の検証結果、時間切れの事前確認、SPSAの結果、事前対局、および採否が、測定記録に残っている。
- 採用した場合は、候補のブランチがmasterへ統合され、依存関係の節に挙げたADRが新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- 不採用の場合は、結論と測定記録だけをmasterへ残し、候補のブランチはmasterへ統合しない。
- ROADMAPの状態表が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)。
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。reverse futility pruningは1021〜1033行、late move pruningは1199行、LMRは1183〜1192行、1350行、1395〜1403行、および1917〜1920行。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。reverse futility pruningは3217〜3227行、late move pruningは3593行、LMRは3879行、3886行、および5148〜5151行。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)と[SPRTによる棋力測定の手引き](../guides/sprt.md)。
