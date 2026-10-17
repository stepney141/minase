# 探索と時間管理の係数の関数形に関する既存実装との比較

## 結論

minaseがSPSAで調整する探索と時間管理の係数（`crates/minase/src/search/alphabeta/params.rs`の表）の多くは、StockfishとYaneuraOuが共通に持つ関数形と違う形で使われている。
違いの大半は、係数の値をどう選んでも埋まらない構造の差であり、SPSAで値を調整し直すだけでは解消しない。
最も影響が大きいと見込む差は、次の5つである。

1. 浅い前向き枝刈り（reverse futility pruning、futility pruning、late move pruning、SEEによる枝刈り）を、すべて残り深さ3以下の零窓のノードに限っている。Stockfishのreverse futility pruningは深さ19未満、YaneuraOuは15未満で働き、late move pruningとSEEによる捕獲手の枝刈りには深さの上限がない。
2. reverse futility pruningの余裕値が深さに比例しない1つの値であり、improving、置換表、補正量の項もない。StockfishとYaneuraOuは余裕値を深さに比例させ、深さ1と3の余裕値の比は約3になる。
3. correction history（静的評価の補正）を、futility pruningの判定にだけ使っている。StockfishとYaneuraOuは、補正後の評価をすべての枝刈り、improvingの判定、および静止探索に使う。
4. late move reductions（LMR）の減深量に定数項がなく、上限が3手であり、improving、PVノード、cutNode、補正量への依存もない。
5. butterfly historyの更新量が深さの2乗に比例し、1回の更新が上限に占める割合が深さ10で約0.6%と小さい。StockfishとYaneuraOuは上限つきの線形で、同じ割合が約15%になり、continuation historyなど直前の手を文脈とする表も併せ持つ。

一方、`HistoryDecay`、`CaptureHistoryDecay`、`CorrectionCap`、`CorrectionWeight`、null moveの減深量の切片と傾き、`DeltaMargin`の大きさ、およびYaneuraOuと比べた場合の時間管理の残り手数と加算の配分は、関数形が同じかほぼ同じで値だけが違い、SPSAで調整できる。

違いのうち、late move reductionsのhistory補正を段差にしたこと、aspiration windowsで外れた側だけを広げること、correction historyを駒種別の枚数で引いて探索ごとに初期化することは、ADRで意図して選んだ形である。
ただし、それらの判断の多くはPSTとFMを既定の評価関数としていた時期のもので、STCの到達深さが今より浅い条件で測られている。
この比較は、2026年10月17日に現行の形のまま探索と時間管理の38係数をNNUEに合わせてSPSAで調整し直したとき、`ReverseFutilityMargin`が開始値428から下限0まで動き、`RazoringMargin1`と`RazoringMargin2`が範囲の上限（razoringが事実上働かない値）の近くまで動いたことを受けて行った。この調整は、後続の関数形の見直しで全係数を調整し直すので、反映せずに中止した。
形を変える候補は[探索の枝刈りとLMRの関数形の見直し](../plans/search-pruning-forms.md)が扱う。

## 調査の範囲

比較の対象は、2026年10月17日時点のminaseのmaster（c3ddaa93）、Stockfish（コミット5c4cef0b）、およびYaneuraOu（コミットc1b80eaa）である。
Stockfishのソースは`src/search.cpp`、`src/history.h`、`src/movepick.cpp`、`src/timeman.cpp`、YaneuraOuのソースは`source/engine/yaneuraou-engine/yaneuraou-search.cpp`（以下`yaneuraou-search.cpp`）と`source/timeman.cpp`を参照した。
YaneuraOuの既定のビルドでは`#if STOCKFISH`の区間が無効になる（`source/config.h:337`）。
minaseのファイルは、断りのない限り`crates/minase/src/search/alphabeta/`からの相対パスで示す。

評価値の尺度はエンジンごとに違う。
Stockfishの歩兵価値は208（`src/types.h:199`）、YaneuraOuは90（`source/evaluate.h:78`）であり、minaseの探索用の歩兵価値は評価関数の重みファイルが持つ値である。
中将棋の歩兵は最も弱い駒の1つなので、余裕値の絶対量は比べず、深さの間の比や、依存する変数の有無を比べる。

各差は、次の5種に分けた。
「同型」は式の形が同じで値だけが違う。
「依存変数の欠落」は、他のエンジンが参照する変数をminaseが参照しない。
「適用条件の違い」は、働くノードや手の範囲が違う。
「関数形の違い」は、同じ変数に対する式の形が違う。
「対応なし」は、他のエンジンに相当する仕組みがない。
「SPSAで埋まるか」の欄は、係数の値の調整だけでStockfishとYaneuraOuの形を再現できるかを示す。

## 浅い前向き枝刈り

| 手法（係数） | minaseの形 | StockfishとYaneuraOuの形 | 差の種類 | SPSAで埋まるか |
|---|---|---|---|---|
| 共通の適用条件 | reverse futility、futility、late move pruning、SEEは、IIR後の残り深さ3以下、零窓、αとβが詰み帯の外、王駒に利きが届かないノードに限る（`negamax.rs:114-118`） | reverse futilityは深さ19未満（詰みを探す間は6未満、Stockfish `search.cpp:1022`）、15未満（YaneuraOu `yaneuraou-search.cpp:3225`）。静かな手のfutilityはlmrDepth 12未満（Stockfish `search.cpp:1247`）。late move pruningとSEEには深さの上限がなく、PVノードでも働く（Stockfish `search.cpp:1196-1258`） | 適用条件の違い | 埋まらない |
| reverse futility pruning（`ReverseFutilityMargin`） | `補正前の静的評価 − 歩兵価値×係数/100 ≥ β`なら静的評価を返す。余裕値は深さ1〜3で共通（`pruning.rs:88-90`、`negamax.rs:131-136`） | 余裕値は`mult×depth − (improving, opponentWorsening)の項 + 補正量の項`で、`mult`は置換表のヒットで変わる。置換表の手が静かな手なら行わない。返り値はβ寄りの混合値（Stockfish `search.cpp:1022-1033`、YaneuraOu `yaneuraou-search.cpp:3217-3227`） | 依存変数の欠落、関数形の違い | 埋まらない（深さ1と3の比が1に固定される） |
| futility pruning（`FutilityMargin1`〜`3`、`NonImprovingFutility1`〜`3`） | ノードごとに1回、補正後の静的評価に深さ別の余裕値を足し、improvingでなければ深さ別の倍率を掛ける（`pruning.rs:44-63`、`negamax.rs:174-184`） | 手ごとに`staticEval + a + b×lmrDepth + c×(staticEval>α)`を求め、lmrDepthを通して手の順番とhistoryに依存する（Stockfish `search.cpp:1240-1251`、YaneuraOu `yaneuraou-search.cpp:3661-3677`） | 依存変数の欠落、適用条件の違い | 深さ1〜3の値とimprovingは埋まる。手ごとの依存と深さの上限は埋まらない |
| late move pruning（`LmpBase`、`LmpSlope`） | 静かな手を数え、`(基数 + 傾き×d²)/100`に達したら以後を読まない。improvingに依存しない（`pruning.rs:71-74`、`negamax.rs:220-241`） | `moveCount ≥ (3 + d²)/(2 − improving)`。improvingでなければ上限が半分になる（Stockfish `search.cpp:1199`、YaneuraOu `yaneuraou-search.cpp:3593`） | 依存変数の欠落 | 深さの比は埋まる。improvingの切り替えは埋まらない |
| razoring（`RazoringMargin1`、`2`） | 深さ2以下で`補正前の静的評価 + 余裕値 ≤ α`なら静止探索を呼び、その値もα以下なら返す（`negamax.rs:137-145`） | Stockfishは全ノード（allNode）で`eval < α − 342×depth`なら静止探索の値をそのまま返す（`search.cpp:1017-1018`）。YaneuraOuは非PVで`α − 502 − 306×depth²`（`yaneuraou-search.cpp:3191-3192`）。どちらも深さの上限がない | 適用条件の違い | 深さ1と2の値は埋まる。深さの上限と静止探索の値の返し方は埋まらない |
| SEEによる捕獲手の枝刈り（`SeeMargin1`〜`3`） | 捕獲手だけを対象に、`SEE < −歩兵価値×深さ別の係数/100`なら読まない（`pruning.rs:77-85`、`negamax.rs:246-262`） | 余裕値は`a×depth + capture history×34/1024`で、深さの上限がない。静かな手にも`−c×lmrDepth²`の枝刈りがある（Stockfish `search.cpp:1222-1225, 1258`） | 依存変数の欠落、適用条件の違い | 深さ1〜3の値は埋まる。capture historyへの依存と静かな手への適用は埋まらない |
| null move pruning（`NullMoveBase`、`NullMoveSlope`、`NullMoveEvalScale`） | 減深量は`(基数 + 傾き×d)/1200 + clamp(静的評価の余裕, 0, 3)`。試す条件は深さ3以上などの4つだけ（`pruning.rs:108-115`、`negamax.rs:154-172`） | 減深量は`7 + depth/3 + 静的評価の余裕/256`（Stockfish `search.cpp:1044`）。試す条件にcutNodeと`staticEval ≥ β − c₁×depth − c₂×improving + c₃`を要求する（Stockfish `search.cpp:1037-1039`、YaneuraOu `yaneuraou-search.cpp:3236`） | 減深量は同型、試す条件は依存変数の欠落 | 減深量は埋まる。試す条件は埋まらない |

StockfishとYaneuraOuは、reverse futility pruningとrazoringの判定に、補正後の評価を置換表の値で上書きした値を使い、null move pruningを試す条件と減深量には補正後の静的評価を使う（Stockfish `search.cpp:860-865, 1037-1044`、YaneuraOu `yaneuraou-search.cpp:3236`）。
minaseのreverse futility pruning、razoring、null move、およびimprovingの判定は、補正前の評価を使う（`negamax.rs:24-35, 119-166`）。
[段階4の診断](../measurements/strength-stage4/strength-stage4-pruning-bench.md)では、reverse futility pruningの深さ別の余裕値が3つの深さとも候補の最小値になっており、共通の余裕値は深さごとの差を測れないまま決まった可能性がある。

## late move reductions、aspiration windows、静止探索

| 手法（係数） | minaseの形 | StockfishとYaneuraOuの形 | 差の種類 | SPSAで埋まるか |
|---|---|---|---|---|
| late move reductionsの基本表（`LmrDivisor`） | `floor(ln d × ln m/(除数/100))`を、history補正の後に0以上`min(3, d − 2)`以下へ切り詰める。置換表の手、捕獲手、killer手、根を除く（`pruning.rs:39-41, 124-136`、`negamax.rs:263-272`） | `ln d × ln m`の積に、improvingでなければ約1.38倍（Stockfish 197/512）を掛け、定数項を足し、PVで減らし、ttPv、cutNode、補正量、`α − eval`などで増減する。1/1024手単位の連続値で上限はなく、負なら延長する。深さ2以上の全手に適用する（Stockfish `search.cpp:1183-1192, 1350-1403, 1917-1920`、YaneuraOu `yaneuraou-search.cpp:5148-5151`） | 依存変数の欠落、適用条件の違い | 傾きだけ埋まる。定数項、improving、上限、適用範囲は埋まらない |
| late move reductionsのhistory補正（`LmrHistoryThreshold`） | butterfly historyが閾値以上なら−1、以下なら+1の段差（`pruning.rs:124-135`） | `statScore×439/4096`を引く連続量で、statScoreは主historyと2つのcontinuation historyの和（Stockfish `search.cpp:1375-1381`） | 関数形の違い、依存変数の欠落 | 埋まらない |
| aspiration windowsの初期幅（`AspirationDelta`） | 歩兵価値の55%の定数で、中心は直前の反復の値。深さ5以上に限る（`root.rs:27-33, 69-71`） | `5 + スレッド番号 + |meanSquaredScore|/c`で、評価値の大きさとともに広がり、中心は平均値。全深さで使う（Stockfish `search.cpp:389-392`、YaneuraOu `yaneuraou-search.cpp:1670-1673`） | 依存変数の欠落、適用条件の違い | 埋まらない |
| aspiration windowsの拡大（`AspirationGrowth`） | 外れた側の端だけを広げ、幅を1.9倍にする（`root.rs:40-55, 74-77`） | 返った値を基準に窓を移し、反対側の端も寄せる。幅を約1.37倍（YaneuraOuは約1.33倍）にし、fail highの回数で深さを浅くする（Stockfish `search.cpp:410-455`） | 依存変数の欠落 | 倍率は埋まる。返った値による移動と減深は埋まらない |
| delta pruning（`DeltaMargin`） | `stand-pat + 取る駒の価値 + 余裕値 ≤ α`なら読まない。stand-patは補正前の評価（`quiesce.rs:76-97, 144-148`） | 補正後の評価に定数を足した基準を使い、余裕値をSEEの閾値にも入れ、読まない手の値で最善値を更新する。王手になる手、直前の到達升への手、成る手を除く（Stockfish `search.cpp:1794-1843`） | 同型（付随部分に差） | 余裕値は埋まる |
| 静止探索の手数制限（`QsearchMoveLimit`） | 実際に読んだ捕獲手が4手に達したら残りを読まない（`quiesce.rs:154-169`） | 生成順の位置が2手を超えたら読まない。王手になる手、直前の到達升への手、成る手を例外にし、王手を受けている局面では使わない（Stockfish `search.cpp:1816-1826`） | 適用条件の違い | 埋まらない |

aspiration windowsでfail high時に深さを浅くする方式と反対側の端を寄せる方式は、[ADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)で棄却済みである（fail high時の減深は、STCで改善を示せない`H0`だった）。
返った値を基準に窓を移す形は、そのADRの代案に含まれていない。
history補正を連続量にする案は、[late move reductionsのADR](../adr/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)が自由度の増加を理由に棄却している。
同じADRは、上限3と段差の補正をSTCの到達深さが5〜6だった時期の判断としている。

## historyとcorrection history

| 手法（係数） | minaseの形 | StockfishとYaneuraOuの形 | 差の種類 | SPSAで埋まるか |
|---|---|---|---|---|
| butterfly historyの更新量（`HistoryLimit`、`HistoryBonusScale`、`HistoryMalusScale`） | 加点と減点は`min(d²×尺度/100, 上限)`、重力式`v += b − v×|b|/上限`。β打ち切りのときだけ更新する（`ordering.rs:70-88`、`negamax.rs:289-314`） | 加点は`min(133d − 81, 1487)`に置換表の手と直前の手のstatScoreの項を足す。減点は`min(968d − 235, 2244)`で手ごとに減衰させる。最善手があれば更新し、置換表の打ち切りとfail lowでも更新する。continuation history、pawn history、low-ply historyを同時に更新する（Stockfish `search.cpp:2004-2089`） | 関数形の違い、依存変数の欠落、対応なし（文脈の表） | 学習率の水準は埋まる。形と文脈の表は埋まらない |
| butterfly historyの持ち越し（`HistoryDecay`） | 根の探索ごとに23%を残す（`team.rs:195-201`） | Stockfishは`go`ごとに約71%を残す（`search.cpp:341-343`）。YaneuraOuは減衰させない | 同型 | 埋まる |
| capture historyの持ち越し（`CaptureHistoryDecay`） | 根の探索ごとに23%を残す（`team.rs:202-204`） | 対局中は減衰させない（Stockfish `search.cpp:715`） | 同型 | 埋まる |
| capture history（`CaptureHistoryLimit`、`CaptureHistoryScale`） | 並べ替えで駒価値に最大で歩兵価値の4倍ぶんを加える。静止探索では同じ価値の群の中だけで使う（`capture_history.rs:59-120`、`ordering.rs:232-241`、`quiesce.rs:472-493`） | 並べ替えの点数は`capture history + 7×駒価値`。捕獲手のfutility、SEEの余裕値、およびLMRにも使う（Stockfish `movepick.cpp:225-226`、`search.cpp:1208-1225, 1371-1373`） | 同型（加算）、対応なし（並べ替え以外の用途） | 加算の大きさは範囲を広げれば埋まる。用途は埋まらない |
| correction history（`CorrectionCap`、`CorrectionWeight`） | 駒種別の枚数の鍵1つで、探索ごとに初期化する。補正はfutility pruningの判定にだけ加える（`correction.rs:31-104`、`negamax.rs:174-184`） | pawn、minor、非pawn、continuationの複数の表の重み付き和を、すべての枝刈り、improving、静止探索に使い、補正量の大きさで余裕値とLMRも変える。表は対局内で持ち越す（Stockfish `search.cpp:95-143, 860-1034, 1353, 1754-1776`、`history.h:152-214`） | 適用条件の違い、依存変数の欠落 | 上限と重みの水準は埋まる。適用先、鍵、寿命は埋まらない |

correction historyを駒種別の枚数で引き、futility pruningにだけ使い、探索ごとに初期化することは、[ADR](../adr/key-correction-history-by-material.md)が診断に基づいて決めた。
ただし、表の持ち越しを見送った根拠の発動率（STC相当で0.69%）は、補正をfutility pruningにだけ使う前提で測った値であり、補正の適用先を広げれば結論が変わり得る。
複数の表の補正を足し合わせる形は、同じADRが未測定と記している。
butterfly historyの学習率が小さいため、minaseでは重力式による忘却がほとんど働かず、根の探索ごとの減衰が忘却の役割をほぼ担っていると推測される。

## 時間管理

| 手法（係数） | minaseの形 | StockfishとYaneuraOuの形 | 差の種類 | SPSAで埋まるか |
|---|---|---|---|---|
| 残り手数の見積り（`ExpectedPlies`、`MinMoves`） | `max(MinMoves, (ExpectedPlies − ply)/2)`で残り時間を割る（`time.rs:88-91, 113-139`） | YaneuraOuは`min(手数制限の項, 地平)/2`で、地平は時間制御の種類で変わる（`source/timeman.cpp:203-213`）。Stockfishは手数のべき乗と残り時間の対数で決まる割合を残り時間に掛ける（`src/timeman.cpp:122-132`） | YaneuraOuとは同型。Stockfishとは関数形の違い、依存変数の欠落 | YaneuraOuの形は埋まる。残り時間の水準への依存は埋まらない |
| 加算の配分（`IncrementShare`） | 加算の76%を目安時間へ足す独立の項（`time.rs:124`） | YaneuraOuは加算の全量を残り時間の見積りに入れる（`source/timeman.cpp:246-257`）。Stockfishは将来の加算を残り時間に足し込んでから同じ割合を掛ける（`src/timeman.cpp:113-114, 159`） | YaneuraOuとは同型。Stockfishとは依存変数の欠落 | YaneuraOuの形は埋まる |
| 上限時間と目安時間の比（`HardSoftRatio`） | 定数の倍率4.51（`time.rs:130-135`） | YaneuraOuは定数5.0（`source/timeman.cpp:263`）。Stockfishは手数と残り時間の対数に依存する（`src/timeman.cpp:128, 134, 160-162`） | YaneuraOuとは同型。Stockfishとは依存変数の欠落 | YaneuraOuの形は埋まる |
| 残り時間に対する上限（`HardRemainingShare`） | 上限時間を残り時間の27%に秒読みの80%を足した値以下にする（`time.rs:131`） | Stockfishは残り時間を基準量とする（`src/timeman.cpp:131, 161`）。YaneuraOuは将来の加算と秒読みを含めた見積りの30%（`source/timeman.cpp:277`） | Stockfishとは同型。YaneuraOuとは基準量の違い | Stockfishの形は埋まる |
| 次の反復の完了時刻の予測（`IterationRatio`） | 経過時間の2.63倍が上限時間以内なら次の反復を始める（`time.rs:76-85`） | 次の反復を予測せず、目安時間の50%を過ぎたら深さを増やさずに同じ深さを読み直す（Stockfish `search.cpp:623, 634`、YaneuraOu `yaneuraou-search.cpp:2027, 2051`） | 対応なし | 埋まらない |
| 探索の不安定さによる伸縮（係数なし） | 最善手が4反復続いたら判定を厳しくする二値の短縮だけ（`time.rs:33-65`） | 目安時間に、評価値の下落、最善手の変化回数、最善手の変化からの深さ、最善手のノード占有率の4因子を掛け、両方向に連続で伸縮させる（Stockfish `search.cpp:592-613`、YaneuraOu `yaneuraou-search.cpp:1969-1990`） | 依存変数の欠落 | 埋まらない |

minaseは秒読みのない時計では上限時間で中断した反復の結果を捨てるので（`team.rs:246`、`root.rs:116-118`）、上限時間の倍率は、中断した反復の結果も使うStockfishの倍率と同じ意味の値ではない。
延長方向の時間配分は、[ADR](../adr/stop-early-on-stable-best-move-without-extensions.md)に記録された二値の延長がSTCで`H0`だった。4因子の積は、[持ち時間の効率的な使用](../plans/archived/time-management-efficiency.md)の第3段階でStockfishの定数に正規化定数0.48を掛けた形で実装した（[事前判定](../measurements/time-management-efficiency/time-management-efficiency-stage3-bench.md)）。ただし、途中結果の採用と予算の再定義を積んだ再構成の一部としてSTCで`H0`となったものであり、単独では測っていない。続く分解測定は、負けの原因を途中結果の採用と予算の再定義に絞っている。
Stockfishは相手との持ち時間の差でも目安時間を下げるが（`src/timeman.cpp:151-156`）、minaseの時計の入力には相手の残り時間がない（`crates/minase/src/search/limits.rs:145-155`）。

## 候補の扱い

構造の差のうち、[探索の枝刈りとLMRの関数形の見直し](../plans/search-pruning-forms.md)が扱うのは、reverse futility pruningの形と深さの上限、補正後の評価の適用先、late move pruningのimprovingの項、およびlate move reductionsの定数項、improving、PVノード、上限である。
これらは、新しい係数の開始値を現行と同じ挙動になる値に設定できるので、形だけを変えたコミットを探索ノード数の一致で検証できる。
残りの差は、次の6本の設計書がこの順の連鎖として扱う。順序は提案であり、各設計書は1つ前の設計書の完了後のmasterを起点とする。
[浅い前向き枝刈りの深さと対象の拡張](../plans/shallow-pruning-extensions.md)は、futility pruning、late move pruning、SEEによる枝刈り、およびrazoringの深さの上限、捕獲手のfutility pruningと静かな手のSEEによる枝刈り、SEEの余裕値のcapture historyの項、null moveを試す条件、およびreverse futility pruningの返り値を扱う。
[historyの更新量の形とcontinuation history](../plans/history-forms.md)は、butterfly historyの更新量と更新の条件、continuation history、LMRのhistory補正の連続化、およびLMRの捕獲手とkiller手への拡張を扱う。
[correction historyの表の追加と持ち越し](../plans/correction-history-tables.md)は、駒配置を鍵にした表、continuation correction history、表の持ち越し、および補正量の余裕値とLMRへの利用を扱う。
[静止探索の補正と枝刈りの形](../plans/qsearch-forms.md)は、stand-patへの補正、delta pruningの余裕値のSEEへの利用、手数の数え方、および取り返しの例外を扱う。
[aspiration windowsの幅と窓の移し方](../plans/aspiration-window-forms.md)は、初期の幅、窓の中心、窓を使い始める深さ、および外れたときの窓の移し方を扱う。
[探索の不安定さに応じた持ち時間の伸縮](../plans/time-instability-scaling.md)は、評価値の下落、最善手の安定と変化、最善手のノード占有率、および相手との持ち時間の差による目安時間の伸縮を扱う。
王駒に相手の利きが届くノードを枝刈りから除く条件は、関数形の差ではなく中将棋の規則から来る判断なので、どの設計書でも変えない。

## 参考資料

- Stockfish（コミット5c4cef0b、手元の複製`../stockfish`）。
- YaneuraOu（コミットc1b80eaa、手元の複製`../YaneuraOu`）。
- [前向き枝刈りの第3層に関する既存実装の調査](forward-pruning-prior-art.md)。
