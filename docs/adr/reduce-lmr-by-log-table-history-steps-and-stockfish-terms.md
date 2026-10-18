# LMRの減深量は対数表と3段階のhistory補正にStockfishの項を1/100手単位で加えて求め、上限を係数として残す

## ステータス

承認済み。決定日は2026年10月18日。

## コンテキスト

LMR（late move reductions）は、順序付けの後方にある静かな手を浅く読み、αを上回ったときだけ本来の深さで読み直す手法である。
中将棋は1局面の合法手が100手を超えるので、後方の手をより大きく減深する余地がある。
[旧ADR](archive/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)は、2026年9月7日に、減深量を残り深さと手番号の対数の積の表から求め、history値で±1する3段階の補正を加え、0以上`min(3, d − 2)`以下に切り詰めると決めた。
この式は、[棋力向上段階5](../plans/archived/strength-stage5.md)の[診断bench](../measurements/strength-stage5/strength-stage5-lmr-bench.md)で係数を選び、[STC](../measurements/strength-stage5/strength-stage5-lmr-stc.md)と[LTC](../measurements/strength-stage5/strength-stage5-lmr-ltc.md)でともに`H1`となった。
旧ADRは、上限を3より大きくする案を、標準時間制御の到達深さで減深後の深さが1になる頻度が増えるとして棄却し、この判断をSTCの到達深さが5〜6だった時期のものとしていた。

2026年10月17日に、[探索の枝刈りとLMRの関数形の見直し](../plans/archived/search-pruning-forms.md)で、前向き枝刈りと減深の式をチェスのStockfishの形へ寄せることにした。
Stockfishの減深量は、ノードの種別、置換表の情報、および子のβ打ち切りの数に依存するが（`src/search.cpp:1183-1192, 1345-1388`）、minaseの減深量はそれらに依存しなかった。
同じ日のエンジンの開発者の決定により、Stockfishにある項目は入れて形と値をStockfishに合わせ、新しい係数はStockfishの値を換算した値から始める。
減深量の上限のように、minaseにあってStockfishにない仕組みのうち中将棋の測定で決めたものは残す。

## 決定

LMRは、残り深さ2以上のノードで、置換表の手、捕獲手、およびkiller手を除く非捕獲手に適用し、根では行わない。
成る非捕獲手も対象に含む。
減深量は1/100手単位の整数で求める。
基になるのは、残り深さ`d`と手番号`m`から`floor(100 × ln d × ln m/(LmrDivisor/100))`を前計算した表の値と、history値が閾値以上なら1手減らし、閾値の符号を反転した値以下なら1手増やす3段階の補正である。
対象の手、根で行わないこと、表、および3段階の補正は、旧ADRから引き継ぐ。

これに、Stockfishの減深量の項のうち、statScoreの項と補正量の項を除くすべてを、1/100手または百分率に換算した係数で加える。
加える項は、improvingでないときの表の値の増分、PVノード、ttPv、手番号、置換表の手が捕獲手であること、子のβ打ち切りの数、読む手が置換表の手であること、αと判定用の評価値の差、cutNode、およびallNodeの項である。
減深後の深さは`max(1, d − 1 − clamp(trunc(r/100), −E, LmrMaxReduction)) + [PVノード]`とし（`src/search.cpp:1402`）、`E`は根からの手数が反復の深さの2倍未満なら2、それ以外なら0とする。
`[PVノード]`は、PVノードなら1、それ以外なら0を表す。
上限`LmrMaxReduction`は調整する係数とし、正の減深量にだけ掛けて、負の減深量による延長には掛けない。

減深した探索がαを超えたら、その値と最善値の差に応じて通常深さを1手深くまたは浅くしてから再探索する（`src/search.cpp:1414-1420`）。
LMRの分岐を通らない零窓の探索、つまり零窓のノードの先頭の手と、残り深さ1のノードの2手目以降の手でも同じ式で減深量を求め、置換表の手がなければ増分を足し、減深量が大きければ通常深さから1手または2手減らして読む（同1428-1436行）。
上記のうち強さを決める値は係数としてSPSAで調整し、閾値と分母の定数はStockfishの値を換算して固定する。

## 検討した代案

Stockfishの項を加えない旧ADRの形は、Stockfishにある項目は入れるというエンジンの開発者の決定に反するので採らなかった。
StockfishやYaneuraOuと同じく減深量の上限をなくす案は、中将棋の測定で決めた既存の仕組みを残すというエンジンの開発者の決定により採らなかった。
上限は定数3から係数に改め、範囲を1〜8として、cutNodeの増分などとの釣り合いをSPSAに選ばせた。
表に手単位の整数を持ち続ける案は、定数項や倍率を足すと表の段階で切り捨てた端数が失われるので採らなかった。
表に`floor(100x)`を持てば、整数`h`について`floor((floor(100x) + 100h)/100) = floor(x) + h`が成り立つので、単位を改めるだけのコミットは探索を変えず、探索ノード数の一致で検証できた。
Stockfishと同じ1/1024手の単位で求める案は、minaseの係数の表が百分率で揃っているので採らなかった。
置換表の手の減深を減らす項（`src/search.cpp:1368-1369`）を、minaseが置換表の手をLMRの対象にしないので省く案は採らなかった。
この項はLMRの分岐を通らない探索で先頭に読む置換表の手に働き、省くとcutNodeで先頭に読む置換表の手が、cutNodeの増分によって1手か2手浅く読まれる。
LMRの分岐を通らない減深に捕獲手とkiller手も含める案は、Stockfishではそれらが減深の分岐で読まれる手なので採らず、LMRを捕獲手、killer手、および根へ広げる変更とともに[historyの形の設計書](../plans/history-forms.md)に委ねた。

旧ADRで棄却した案は、この決定でも棄却したままとする。
history値に比例して減深量を連続的に増減する案は、比例係数と切り詰めの2つの自由度を持ち込み、それぞれを1つに絞る診断が要るので採らない。
Stockfishのこの項（statScoreの項）を加えるかは、[historyの形の設計書](../plans/history-forms.md)が扱う。
失う良い結果の割合に10%以下という絶対基準を課す案は、[試行診断](../measurements/strength-stage5/strength-stage5-lmr-trial-bench.md)で当時の規則自体が良い結果の31.5%を失っていて基準を満たさなかったので採らない。

## 結果

式の詳細は`crates/minase/src/search/alphabeta/pruning.rs`の`LmrCoefficients::lmr_reduction`、`lmr_depth`、`lmr_research_depth`、`non_lmr_depth`、および`lmr_table`のdocコメントにあり、係数の現行値は`params.rs`にある。
cutNodeとallNodeの区別は[置換表の打ち切りのADR](archive/allow-null-move-and-tt-cutoff-at-pv-nodes-and-use-node-types-at-zero-window-nodes.md)が、ttPvは[reverse futility pruningのADR](archive/skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)が、判定用の評価値は[静的評価のADR](key-correction-history-by-material-and-apply-it-to-all-static-eval-checks.md)が定める。
補正量の項は[correction historyの表の設計書](../plans/correction-history-tables.md)が扱い、本決定には含めない（`src/search.cpp:1353`）。
詰みを探す局面で王手になる手とならない手の減深量を変える項も、[浅い枝刈りの拡張の設計書](../plans/shallow-pruning-extensions.md)が扱い、本決定には含めない（同1391-1392行）。

減深量は負にもなり得るので、PVノードの1手と合わせて、旧来の式より深く読むノードが増える。
係数を開始値に置いた時点では、benchの探索ノード数はこの変更のコミットで直前より106.6%増えた（[benchの記録](../measurements/search-pruning-forms/search-pruning-forms-bench.md)）。
2026年10月18日の30,000ペアのSPSAでは、`LmrMaxReduction`は3のまま範囲の上限8に達せず、`LmrTtCapture`と`LmrMoveCount`は下限の0になった（[SPSAの記録](../measurements/search-pruning-forms/search-pruning-forms.md)）。
上限はこのSPSAで選び直したが、3段階のhistory補正は、評価関数がPSTでSTCの到達深さが5〜6だった時期の判断のまま残っている。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、設計書を始めた時点のmaster`99ccc6bb`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、776ペアで`H0`となった（[STCの記録](../measurements/search-pruning-forms/search-pruning-forms-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
