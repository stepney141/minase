# late move pruningは全深さとPVノードで行い、静かな手だけを数え、improvingでないときに上限を縮め、履歴の条件を付けない

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

late move pruningは、順序付けの後方にある静かな手を読まずに済ませる手法である。
[旧ADR](archive/count-only-quiet-moves-for-late-move-pruning-and-shrink-the-limit-when-not-improving.md)は、2026年10月18日に、静かな手だけを数え、上限を`(LmpBase + LmpSlope × 残り深さ²)/100`とし、improvingでないときに`LmpNonImproving/100`を掛けて縮め、履歴値を枝刈りの条件にしないと決めた。
[棋力向上段階4](../plans/archived/strength-stage4.md)の方式は、捕獲手も含めた手番号が深さごとの上限に達した後の静かな手を枝刈りし、STCが114有効ペアでLLR −2.947の`H0`だった（[測定記録](../measurements/strength-stage4/strength-stage4-lmp-stc.md)）。
[不採用だった探索部の改良の再調整](../plans/archived/search-revival-spsa.md)の[診断](../measurements/search-revival-spsa/search-revival-lmp-diag.md)では、残り深さ1で枝刈りを始めるまでに探索した手は平均で捕獲手8.1手、静かな手4.5手であり、上限の枠の約3分の2を捕獲手が占めた。
残り深さ1で枝刈りしたノードの約36%は、静かな手を1手も探索しないうちに枝刈りを始めており、killer手もノードあたり平均0.56手が枝刈りされていた。
中将棋はチェスや本将棋より捕獲手が多いので、捕獲手を数えると静かな手の枠が捕獲手で埋まる。
[棋力向上段階8](../plans/archived/strength-stage8.md)は、履歴値の低い手だけを切るhistory pruningを診断したが、21組の候補のどれも、良い結果を失う割合が10%以下にならなかった（[診断](../measurements/strength-stage8/strength-stage8-pruning-bench.md)）。

旧ADRは、適用するノードを[浅いノードの前向き枝刈りの適用条件](archive/skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)に従わせ、残り深さ3以下の零窓のノードに限っていた。
そのうえで、全深さへ適用を広げる変更を未測定の候補として残していた。
深さの上限3は、評価関数がPSTとFMだった時期に段階4で静的評価に基づく枝刈りを入れたときの値を引き継いだものである。
Stockfishのlate move pruningには深さの上限がなく、PVノードでも働く（`src/search.cpp:1199`）。
2026年10月17日のエンジンの開発者の決定により、Stockfishにある項目は入れて形をStockfishに合わせ、静かな手だけを数えるような中将棋の診断で決めた仕組みは残すことにした。
[浅い前向き枝刈りの深さと対象の拡張](../plans/archived/shallow-pruning-extensions.md)は、この決定に従って適用範囲だけを広げた。

## 決定

置換表の手、捕獲手、成る手、およびkiller手のいずれでもない静かな手を、枝刈りで展開しなかった手も含めて数え、それまでに数えた静かな手の数が上限に達していれば、その手を展開しない。
上限は、improvingのときは`(LmpBase + LmpSlope × 残り深さ²)/100`とし、improvingでないときはこれに`LmpNonImproving/100`を掛け、端数を最後に1回だけ切り捨てる。
履歴値は枝刈りの条件にしない。
数え方、上限の式、および履歴の条件を付けないことは旧ADRから引き継ぐ。

適用するノードは、残り深さの上限を持たず、零窓のノードとPVノードの両方とする。
適用するノードには、αとβが詰み帯と専用の帯の外にあること、手番側の王駒に相手の疑似利きが届かないこと、および探索済みの最善値が負の詰み帯と専用の帯の外にあることを要求する。
これらの共通の条件は[浅い前向き枝刈りのADR](extend-shallow-pruning-to-stockfish-ranges-and-skip-it-when-a-royal-is-attacked.md)が定める。
判定は手ごとの枝刈りの判定の最初に行い、late move pruningで読まない手は静かな手のfutility pruningの見積りで最善値を上げない。

## 検討した代案

残り深さ3以下の零窓のノードに限り続ける旧ADRの適用範囲は、Stockfishにある形に合わせるというエンジンの開発者の決定により改めた。
旧ADR自身も、全深さへの適用を未測定の候補として残していた。

旧ADRで棄却した案は、この決定でも棄却したままとする。
improvingの項を含めない上限式は、Stockfishにある項目は入れるというエンジンの開発者の決定に反するので採らない。
捕獲手も数える手番号で切る段階4の方式は、上記のとおり静かな手の枠が捕獲手で埋まり、STCで`H0`だったので採らない。
捕獲手と置換表の手も数え、killer手も枝刈りの対象にする数え方は、同じ理由で採らない。
深さごとに独立した3つの上限は、単調でない値が選ばれ得るので採らない。
履歴値の低い手だけを切るhistory pruningは、上記の診断で適格な候補がなかったので見送ったままとする。

## 結果

係数は`crates/minase/src/search/alphabeta/params.rs`の`LmpBase`、`LmpSlope`、および`LmpNonImproving`であり、数え方と上限の詳細は`pruning.rs`の`late_move_limit`と`LateMoveCoefficients`、および`negamax.rs`の`negamax`のdocコメントにある。
3つの係数の開始値は、起点の既定値である。
improvingはnull move pruningの後の更新を反映した値を使う（[improvingのADR](judge-improving-on-the-corrected-static-eval-and-use-it-in-null-move-pruning-but-not-futility-margins.md)）。
killer手は数える対象にならないので、枝刈りもされない。
起点のSPSAは残り深さ3以下だけで働く形で係数を調整したので、残り深さ4以上の上限は、本決定の後のSPSAで初めて調整される。

`minase dev bench`の深さ3、5、7の全15局面の探索ノード数の合計は、適用範囲を広げたコミットで直前より33.4%減り、設計書の項目の中で最も大きく減った（[benchの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-bench.md)）。

2026年10月18日から19日にかけての30,000ペアのSPSAでは、上限の切片`LmpBase`が開始値1,799から6,436へ、傾き`LmpSlope`が48から346へ上がり、上限を広げる側へ動いた。improvingでないときの倍率`LmpNonImproving`は31から26になった（[SPSAの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions.md)）。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、[前の設計書](../plans/archived/search-pruning-forms.md)を採用した版`a892a5e6`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、1,381ペアで`H1`となった（[STCの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc-on-the-measurement-machine.md)）。
