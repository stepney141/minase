# late move pruningは静かな手だけを数え、improvingでないときに上限を縮め、履歴の条件を付けない

## ステータス

置き換え済み。決定日は2026年10月18日。[late move pruningは全深さとPVノードで行い、静かな手だけを数え、improvingでないときに上限を縮め、履歴の条件を付けない](../apply-late-move-pruning-at-all-depths-and-pv-nodes-counting-only-quiet-moves.md)によって置き換えられた。

## コンテキスト

late move pruningは、浅いノードで順序付けの後方にある静かな手を読まずに済ませる手法である。
[旧ADR](count-only-quiet-moves-for-late-move-pruning.md)は、2026年10月4日に、静かな手だけを数え、上限を残り深さについて単調な式とし、履歴値を枝刈りの条件にしないと決めた。
[棋力向上段階4](../../plans/archived/strength-stage4.md)の方式は、捕獲手も含めた手番号が深さごとの上限に達した後の静かな手を切り、STCが114有効ペアでLLR −2.947の`H0`だった（[測定記録](../../measurements/strength-stage4/strength-stage4-lmp-stc.md)）。
[不採用だった探索部の改良の再調整](../../plans/archived/search-revival-spsa.md)の[診断](../../measurements/search-revival-spsa/search-revival-lmp-diag.md)では、残り深さ1で枝刈りを始めるまでに探索した手は平均で捕獲手8.1手、静かな手4.5手であり、上限の枠の約3分の2を捕獲手が占めた。
残り深さ1で枝刈りしたノードの約36%は、静かな手を1手も探索しないうちに枝刈りを始めており、killer手もノードあたり平均0.56手が枝刈りされていた。
中将棋はチェスや本将棋より捕獲手が多いので、捕獲手を数えると静かな手の枠が捕獲手で埋まる。
2026年10月2日のエンジンの開発者の決定で定義を改めたうえで戻した版は、既存の係数とまとめてSPSAで調整して採用した（[採用のADR](../adopt-revived-search-items-as-a-bundle.md)）。
[棋力向上段階8](../../plans/archived/strength-stage8.md)は、履歴値の低い手だけを切るhistory pruningを診断したが、21組の候補のどれも、良い結果を失う割合が10%以下にならなかった（[診断](../../measurements/strength-stage8/strength-stage8-pruning-bench.md)）。

旧ADRの上限式には、StockfishとYaneuraOuの上限式にあるimprovingの項がなかった。
当時はimprovingを別の項目として戻していたからである。
2026年10月17日に、[探索の枝刈りとLMRの関数形の見直し](../../plans/archived/search-pruning-forms.md)で、前向き枝刈りと減深の式をチェスのStockfishの形へ寄せることにした。
両エンジンの上限式は`(3 + d²)/(2 − improving)`である（Stockfish `src/search.cpp:1199`、YaneuraOu `yaneuraou-search.cpp:3593`）。
同じ日のエンジンの開発者の決定により、Stockfishにある項目は入れ、Stockfishに同じ形の値がある既存の係数は換算値から調整し直し、静かな手だけを数えるような中将棋の診断で決めた仕組みは残す。

## 決定

置換表の手、捕獲手、成る手、およびkiller手のいずれでもない静かな手を、枝刈りで展開しなかった手も含めて数え、その手より前に数えた数が上限に達していれば、その静かな手を展開しない。
上限は、improvingのときは`(LmpBase + LmpSlope × 残り深さ²)/100`とし、improvingでないときはこれに`LmpNonImproving/100`を掛け、端数を最後に1回だけ切り捨てる。
履歴値は枝刈りの条件にしない。
数え方と履歴の条件を付けないことは旧ADRから引き継ぎ、適用するノードは[浅いノードの前向き枝刈りの適用条件](skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)に従う。
`LmpNonImproving`の開始値50は両エンジンの`/(2 − improving)`に当たり、improvingでないときの上限を半分にする。
`LmpBase`と`LmpSlope`は、Stockfishの`3 + d²`に合わせて300と100から調整し直した。

## 検討した代案

improvingの項を含めない旧ADRの上限式は、Stockfishにある項目は入れるというエンジンの開発者の決定に反するので改めた。

旧ADRで棄却した案は、この決定でも棄却したままとする。
捕獲手も数える手番号で切る段階4の方式は、上記のとおり静かな手の枠が捕獲手で埋まり、STCで`H0`だったので採らない。
捕獲手と置換表の手も数え、killer手も枝刈りの対象にする数え方は、同じ理由で採らない。
深さごとに独立した3つの上限は、単調でない値が選ばれ得るので採らない。
履歴値の低い手だけを切るhistory pruningは、上記の診断で適格な候補がなかったので見送ったままとする。

## 結果

係数は`crates/minase/src/search/alphabeta/params.rs`の`LmpBase`、`LmpSlope`、および`LmpNonImproving`であり、数え方と上限の詳細は`pruning.rs`の`late_move_limit`と`negamax.rs`の`negamax`のdocコメントにある。
improvingはnull move pruningの後の更新を反映した値を使う（[improvingのADR](judge-improving-on-the-corrected-static-eval-and-use-it-in-pruning-and-lmr.md)）。
killer手は数える対象にならないので、枝刈りもされない。
全深さへ適用を広げる変更は、未測定の候補として残る。
2026年10月18日の30,000ペアのSPSAでは、`LmpBase`が開始値300から1,799へ、`LmpNonImproving`が50から31へ動いた（[SPSAの記録](../../measurements/search-pruning-forms/search-pruning-forms.md)）。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、設計書を始めた時点のmaster`99ccc6bb`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、776ペアで`H0`となった（[STCの記録](../../measurements/search-pruning-forms/search-pruning-forms-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](../adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
