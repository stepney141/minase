# correction historyの表の追加と持ち越しの設計書

## 先に読む要約

本書は、静的評価の補正（correction history）の仕組みをチェスのStockfishの形へそろえ、現行のmasterより強くなるかを確かめる。
correction historyは、探索値と静的評価のずれを局面の特徴ごとに表へ学習し、枝刈りの判断に使う静的評価を補正する手法である。
minaseの補正表は駒種別の枚数を鍵にした1つの表だけで、探索ごとに0から学び直し、補正前の誤差の指数移動平均を学ぶ。
Stockfishは、駒の配置を鍵にした複数の表と、手番側自身の少し前の手を文脈として直前の相手の手で引く表（continuation correction history）の補正を重み付きで足し合わせる。
各表は、合成した補正後の評価に残る誤差で更新され、対局の間持ち越される。
補正量の大きさは、枝刈りと減深の式にも使われる。
本書が変えるのは5つである。
1つ目に、補正表を対局の間、減衰させずに持ち越す。
2つ目に、歩兵と仲人の配置、小駒（日本中将棋連盟の規則が「小駒」とする駒）の配置、および先手と後手それぞれの歩兵と仲人以外の駒の配置を鍵にした4つの表（Stockfishのpawn、minor、non-pawnの表に相当）を加え、現行の表とともに重み付きで足し合わせる。局面を鍵にした表は、Stockfishと同じく探索の全ワーカーで共有する。
3つ目に、2手前、4手前、6手前の手を文脈として直前の手で引く表（continuation correction history）を加える。
4つ目に、表の値の形と更新をStockfishの形にする。値は上限つきの整数とし、合成した補正後の評価との差から更新量を作る。
5つ目に、補正量の絶対値をlate move reductions（LMR、後方の手を浅く読む減深）の減深量に使う。
補正量の絶対値をreverse futility pruningの余裕値に使う項は、[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF1が扱う。
SPSAで調整するのは各仕組みの強さを決める重みと更新量だけとし、上限、表ごとの更新の倍率、初期値などはStockfishの値で固定する。
新しい係数の開始値はStockfishの値を中将棋の歩兵価値へ換算した値とし、既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整する。
本書は、探索の関数形をStockfishとYaneuraOuの形へ寄せる設計書の連鎖の4本目であり、1つ前の設計書が完了した後のmasterを起点とする。
調整した版を起点のmasterと短時間条件STCおよび長時間条件LTCの2段階の逐次検定（段階ゲート）で対局させ、1つの候補として採否を決める。
完了条件は、候補の採否が測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案し、同日の2回の利用者の決定に合わせて改訂した。
[historyの更新量の形とcontinuation historyの設計書](history-forms.md)の完了を待って着手し、着手は利用者の指示による。
次の一手は、連鎖の1つ前の設計書の完了後に起点を確定し、依存関係の節に従って起点の式を確かめてから、フェーズ1（形の変更の実装）を始めることである。

## 目的

本書が確かめるのは、「補正表の鍵、値の形、更新、寿命、および補正量の使い道をStockfishの形へそろえ、新しい形の係数を調整すれば、現行の形で調整した探索より強くなる」という仮説である。
[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)は、correction historyの鍵、寿命、および適用先を、係数の値の調整では埋まらない差に分類した。
このうち適用先の拡大は[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF2が、reverse futility pruningの余裕値の補正量の項は同書のF1が扱う。
本書は残りの鍵、値の形、更新、寿命、および補正量の大きさのLMRへの利用を扱う。

2026年10月17日の利用者の決定（2回）により、本書の判断は次の方針に従う。
第1に、StockfishとYaneuraOu（やねうら王）で形や値が違う場合はStockfishに合わせ、YaneuraOuにだけある項目は入れず、Stockfishにだけある項目は入れる。
第2に、新しい係数の開始値は両エンジンの換算値とし、両者が違えばStockfishの換算値とする。
第3に、本書については、continuation correction historyを含め、表の大きさをStockfishと同じにし、表の更新を合成した補正後の評価との残差で行い、表を対局の間減衰させずに持ち越す。
continuation correction historyの表の引き方は、[historyの更新量の形とcontinuation historyの設計書](history-forms.md)の共有の表と同じ考え方（Stockfishの形）にそろえる。
第4に、minaseにあってStockfishにない既存の仕組みのうち、中将棋の診断や測定で決めたもの（本書では駒種別の枚数を鍵にした表）は残し、Stockfishの仕組みを並べて足し、残したものもSPSAで調整する。中将棋の規則から来る条件（王駒への利きの除外など）も残す。
第5に、SPSAで調整するのは各仕組みの強さを決める値（重みと更新量）だけとし、上下限や更新の内部の定数はStockfishの値で固定する。
第6に、評価値の単位を持つ係数の範囲は、Stockfishの換算値、YaneuraOuの換算値、およびminaseの現行の対応する値の3つをすべて含むように取る。中将棋は将棋ともチェスとも異なるので、値はminaseのSPSAが決める。
第7に、表はStockfishと同じくワーカー間で共有し、表ごとの更新の倍率と初期値はStockfishの値で固定し、小駒の範囲は本書の定義とし、駒種別の枚数の表は残す。
この方針により、新しい係数の開始値は現行と同じ探索にはならない。
各項目のコミットでは探索ノード数の変化を記録し、挙動を変えない移行のコミットだけをノード数の一致で検証する。

現行の形は、[ADR](../adr/key-correction-history-by-material.md)が診断に基づいて選んだものである。
その診断は、駒配置の表を加えると予測誤差が悪化したとして見送ったが、比べた予測は現行の表と追加の表の等分の平均であり、現行の補正を半分に薄める形だった（[診断の記録](../measurements/strength-stage12-activation-diag.md#項目6の補正表の鍵)）。
同じ記録で、歩兵と仲人の配置を鍵にした表は、平均しても誤差を0.20%しか悪化させず、0でない値を返す参照が89.50%あった。
表の持ち越しを見送った根拠の発動率（STC相当で0.69%）も、補正をfutility pruningにだけ使う前提の値である。
加えて、これらの診断は既定の評価関数がPSTだった時期のものであり、評価関数をNNUEに替えた後には測られていない。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、連鎖の1つ前の設計書が完了した後、フェーズ1を始める時点のmasterの先頭のコミットである。
- **形の変更**は、本書の5項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で表（`crates/minase/src/search/alphabeta/params.rs`）の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **補正表**は、局面または手の組を鍵にして、静的評価の補正のもとになる値を持つ表である。本書の後の補正表は、Stockfishと同じく、要素を−1024以上1024以下の整数で持つ。
- **材料表**は、現行の補正表（双方の駒種別の枚数の組を鍵にした表）をいう。
- **補正量**は、全表の値を重み付きで足し合わせた値であり、静的評価と同じ単位を持つ。補正後の静的評価は、補正前の静的評価に補正量を足し、評価関数の出力の範囲に収めた値である。
- **k手前の手**は、現局面へ至る着手を1手前として、探索経路を遡って数えた着手である。2手前、4手前、6手前の手は手番側自身の手、1手前の手は相手の手である。
- **上界のノード**は、探索値がα以下に終わり、置換表へ上界として保存するノードである。Stockfishでは最善手を持たないノードに当たる。
- **改良の判定（improving）**は、`negamax.rs`の`improving`が定める判定であり、同じ手番の2手前の局面より静的評価が上がったことをいう。
- 減深量と深さの単位は、探索の深さ1を1手と数える。

対象は次の5項目である。

- 補正表の持ち越し（C1）。
- 局面の駒配置を鍵にした4つの表と、表の重み付きの和（C2）。
- continuation correction historyの表（C3）。
- 表の値の形と更新（C4）。
- LMRの減深量の補正量の項（C5）。

次の事項は対象外とする。

- reverse futility pruningの余裕値の補正量の項。[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF1が、Stockfishの余裕値の形（`src/search.cpp:1027-1030`）の一部として扱い、本書の補正量をそのまま使う。
- 静止探索のstand-patへの補正の適用。連鎖の次の`qsearch-forms.md`で扱い、本書の補正量をそのまま使う。
- 王駒の升の組を鍵にした表。[ADR](../adr/key-correction-history-by-material.md)はこの鍵を材料表の鍵の代わりとして比べただけであり、Stockfishにも対応する表がない。王駒の升は、C2の先手と後手の配置の鍵に含まれる。
- 表ごとの更新量の倍率と、表の初期値の調整。Stockfishの値を固定の定数として使い、SPSAの対象にしない。固定する定数の節を参照。
- 置換表の値で補正後の評価を置き換えること（Stockfish `src/search.cpp:862-865`）。search-pruning-formsが扱う。
- singular extensionの余裕値への補正量の利用（Stockfish `src/search.cpp:1288`）と、singular extensionの探索の結果による表の更新（Stockfish `src/search.cpp:1306-1316`）。minaseはsingular extension（置換表の手だけが良い手かを浅い探索で確かめて延長する手法）を持たない。
- null move pruningを試す条件に補正後の評価を使う変更を、本書で新たに加えること。この条件は[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)のS6が扱う。S6とF2がともに採用されていれば、条件には既に補正後の評価が使われており、本書の補正量もその条件に入る（依存関係の節を参照）。

## 依存関係

本書は、[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)から始まる設計書の連鎖の4本目である。
起点は、連鎖の1つ前の[historyの更新量の形とcontinuation historyの設計書](history-forms.md)が完了した後のmasterの先頭であり、前の設計書の候補が採用されたかどうかは問わない。
この順序は提案であり、最終的な順序は利用者が決める。
前の設計書の候補がmasterへ統合されると`params.rs`の既定値と探索の式が変わり、本書の検証の基準も変わるので、実装を含めて前の設計書の完了後に始める。
起点の表の既定値を、既存の係数のSPSAの開始値とする。

本書の項目は、起点に連鎖の前の設計書の変更が入っているかどうかで、次のように扱う。
以下のF1、F2、F4は、[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)の項目であり、それぞれreverse futility pruningの余裕値の形、補正後の評価の適用先の拡大、およびLMRの減深量の形の変更である。
フェーズ1の最初に起点のコードを読み、どれに当たるかを確かめる。

- [探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)が採用されていれば、補正後の静的評価はreverse futility pruning、razoring、null move pruningの減深量、改良の判定、およびfutility pruningに使われている。本書の補正量は、それらすべての判定に入る。
- さらに[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)のS6（null move pruningを試す条件に、減深量の計算と同じ静的評価を使う変更）も採用されていれば、本書の補正量はnull move pruningを試すかどうかの判定にも入る。
- [探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)が採用されていなければ、補正後の静的評価はfutility pruningにだけ使われている。本書は適用先を広げない。一括で不採用となった項目群から一部だけを選び直して再測定することになるからである。この場合、新しい表と持ち越しが探索に及ぶ範囲はfutility pruningとC5だけになり、効果は小さいと見込まれる。この場合に着手するかは、着手時に利用者が決める。
- C5が補正項を引く位置と単位は、先行案の採否ではなく起点の実装で決める。起点のLMRが手の端数を持つ単位（F4またはhistory-formsの1/100手）で減深量を計算していれば、その単位を使う。手単位の整数で計算していれば、F4の設計判断「LMRの減深量の単位」と同じ方法で表を1/100手単位に改める。この変更は、整数`h`について`floor((floor(100x) + 100h)/100) = floor(x) + h`が成り立つので、探索を変えない。起点の単位が1/100手以外なら、`LmrCorrection`はその単位で定め、新しい係数の表の範囲と換算値もその単位へ直す。
- [historyの更新量の形とcontinuation historyの設計書](history-forms.md)は、ply `q`のノードで指した手について、その`Move`と着手前の駒種をplyごとに記録する（null moveと根より前は記録を持たない）。この記録が起点にあれば、C3はそれを再利用する。なければ、C3で同じ定義の記録を加える。同書のcontinuation historyの表と同じく、C3も升を手番側に合わせて180度回転して引く。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、同時対局数は16を上限とする。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は次のADRの決定の一部を変える。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確かめ、新しいADRで置き換える。
一部だけを変えるADRでは、残る決定も新しいADRへ引き継ぐ。

- [correction historyの鍵と適用先のADR](../adr/key-correction-history-by-material.md)は、2026年9月21日に、材料表だけを鍵とし、探索ごとに初期化し、補正前の誤差を学び、補正をfutility pruningにだけ使うと決めた。本書は、材料表を残したうえで鍵の表を加え、表を対局の間持ち越してワーカー間で共有し、値の形と更新をStockfishの形にし、補正量の大きさを減深量に使う。更新の条件のうち、手番側の王駒に相手の利きが届くノードを除くことは残し、捕獲手と境界の種類の条件はStockfishの形にする。決定と根拠の診断は既定の評価関数がPSTだった時期のものであり、ADR自身が、各表の補正を平均せずに足し合わせる形を未測定の候補として残している。
- [butterfly historyと捕獲履歴の持ち越しのADR](../adr/carry-history-tables-within-a-game.md)は、2026年10月10日に、補正表を持ち越さないと決めた。本書のC1は、補正表を対局の間持ち越す。この決定は既定の評価関数がPSTだった時期のもので、根拠はfutility pruningにだけ補正を使う前提の発動率0.69%である。ADRは補正表の持ち越しの再測定を留保していない。
- 起点でLMRの式を定めているADRは、C5で補正量への依存が加わる。現行の[late move reductionsの式のADR](../adr/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)は2026年9月7日のPST時期の決定であり、上限と段差のhistory補正をSTCの到達深さが5〜6だった時期の判断として見直しの余地を留保している。同じADRが棄却した連続量の項はhistoryの項であり、補正量の項ではない。

## 設計判断

「利用者の決定」と書いた判断は、2026年10月17日の2回の利用者の決定による。
それ以外は、本書の起案者がその方針の下で判断したものである。

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 5項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとにSPSAと段階ゲートを繰り返す案は、1項目あたりSPSAの約75時間にSTCとLTCを加えた約81〜113時間を要するので採らない。代償として、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| 準拠する形 | 利用者の決定により、Stockfishの形に合わせる。YaneuraOuは将棋向けに小駒の区分を定め直し（`source/position.cpp:27-36`）、continuation correction historyを2手前と4手前の2つに限るが（`yaneuraou-search.cpp:732-737`）、これらのYaneuraOuだけの形は採らない。 |
| 新しい係数の開始値 | 利用者の決定により、Stockfishの値を換算した値とする。換算は、評価値の差を歩兵価値の比で直す（Stockfishの歩兵価値は208、`src/types.h:199`）。材料表の重みはStockfishに対応する値がないので、現行の補正の上限に当たる値から始める（材料表の行を参照）。新しい開始値では探索が現行と一致しなくなるので、各項目のコミットで探索ノード数の変化を記録する。 |
| 新しい係数の範囲 | 利用者の決定により、Stockfishの換算値、YaneuraOuの換算値（歩兵価値90、`source/evaluate.h:78`）、およびminaseの現行の対応する値の3つをすべて含める。現行に表や項がない係数は、現行の値を0とする。3つの値は次の行に示す。 |
| 3つの値と開始値の隔たり | 重みは、pawnがStockfish 52、YaneuraOu 105、現行0、minorが36、75、0、non-pawnが44、107、0、2手前と4手前の文脈が30、69、0、6手前の文脈が24、なし、0である。更新量は、上界でないノードが20、9、なし、上界のノードが30、12、なしである。LMRの項は101、45、0である。材料表の重みは、StockfishにもYaneuraOuにもなく、起点の`CorrectionCap`の値（現行の既定値なら220）である。重みの開始値はYaneuraOuの換算値の約半分であり、更新量とLMRの項の開始値はYaneuraOuの換算値の2倍を超える。補正の表と項を持たない現行からは、いずれも離れた値から始まる。 |
| SPSAの対象 | 利用者の決定により、SPSAで調整するのは各仕組みの強さを決める値（表の重み、更新量、LMRの項）だけとし、残した材料表の重みも調整する。上下限、表ごとの更新の倍率、初期値などはStockfishの値で固定し、固定する定数の節に行番号とともにまとめる。起点の表の調整対象の全係数（`FmScale`を除く）から、新しい形では使わなくなる`CorrectionCap`と`CorrectionWeight`を除き、新しい9係数を加える。係数の数は起点の時点で数え直す。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の係数を含めるので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。3,750反復、1反復8ペア（計30,000ペア）、持ち時間10秒＋0.1秒、同時対局数16、`c_end`（最後の反復の摂動幅）は範囲の1/6、`r_end`（最後の反復の学習率）は0.002である（[SPSAの手引き](../guides/spsa.md)）。30,000ペアは候補を1つ作るための予算であり、最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。連鎖の先行案をすべて採用した場合、本書の時点の調整対象は88係数前後になる（起点の表で数え直す）。30,000ペアでこの数を十分に調整できるかは未検証であり、不採用はこの予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。 |
| 局面の鍵（C2） | Stockfishの4つの鍵（pawn、minor、先手と後手のnon-pawn）に当たる鍵を使う（Stockfish `src/position.cpp:505-526`）。pawnの鍵は、[historyの更新量の形とcontinuation historyの設計書](history-forms.md)がpawn historyのために導入する双方の歩兵と仲人の配置の鍵を再利用する。[日本中将棋連盟の競技規則](https://www.chushogi-renmei.com/kouza/rule2007.htm)は駒の格付けで歩兵と仲人を「歩」の格にまとめ、RULES.md第3条第11項も2種を「価値ある駒」から除く。history-formsが採用されていなければ、同じ定義の鍵を本書で導入する。minorの鍵は、同じ格付けが「小駒」とする猛豹、銅将、銀将、金将、盲虎、醉象、麒麟、および太子のうち、王駒の太子を除く7種の双方の配置とする。Stockfishのminorの鍵もキングを除く（`src/position.cpp:518-524`）。成った歩兵は金将と、成った仲人は醉象と同じ動きなので小駒に含める。この扱いは格付けに明文がなく、本書の判断であり、利用者はこの定義を了承した。先手と後手のnon-pawnの鍵は、それぞれの歩兵と仲人以外の全駒の配置とし、Stockfishと同じく王駒と成った駒を含める（`src/position.cpp:516, 947-957`）。pawnとnon-pawnの鍵の定義は[段階12の診断](../measurements/strength-stage12-activation-diag.md)のパッチ`correction-keys.patch`と同じであり、その診断は差分更新した鍵を局面からの再計算と440,293回照合して、不一致0件を確かめている。 |
| 材料表を残すこと | 利用者の決定により残す。Stockfishに材料表はないが、中将棋の[診断](../adr/key-correction-history-by-material.md)で鍵の候補のうち最も予測を改善した表である。他の表と同じ形に改め、Stockfishの表と並べて足す。重み`CorrectionMaterialWeight`の開始値は、起点の`CorrectionCap`の値とする（現行の既定値なら220）。新しい形では、要素が上限1024に達したときの補正量が重みの百分率×歩兵価値になるので、起点の補正の上限と同じ大きさになる値である。更新量の倍率は、Stockfishで基準になるpawnの表と同じ128/128とする。 |
| 表の合成 | Stockfishと同じく、各表の要素に表ごとの重みを掛けて足し合わせ、補正後の評価を評価関数の出力の範囲に収める（Stockfish `src/search.cpp:95-118`）。補正量の全体の上限`CorrectionCap`は、Stockfishにないので除く。先手と後手のnon-pawnの表には同じ重みを使う。2手前と4手前の文脈にも同じ重みを使い、6手前には別の重みを使う。1手前の手がnull moveのときは、Stockfishと同じくcontinuation correction historyの代わりに定数を加える。この定数は換算すると歩兵価値の約0.3%である。 |
| 表の大きさと共有 | 利用者の決定により、大きさと共有の仕方をStockfishと同じにする。局面を鍵にした5つの表（材料、pawn、minor、2つのnon-pawn）は、探索チームの全ワーカーで共有し、手番側ごとに「65,536×ワーカー数以上の最小の2のべき」要素の`i16`とする（Stockfish `src/history.h:40, 158-171, 189-191, 206-212`、`src/thread.cpp:213`）。要素は不可分の読み書き（`Relaxed`順序）で扱い、Stockfishも同じく競合を許す（`src/history.h:60-63`）。1ワーカーあたりの大きさは5表で約1.3 MBである。continuation correction historyは、Stockfishと同じくワーカーごとに持つ（`src/search.h:335`）。手の組を直接の添字で引く1つの表であり（`src/history.h:177-185`）、1ワーカーあたり約36 MBである。 |
| continuation correction historyの引き方（C3） | 利用者の決定により、[historyの更新量の形とcontinuation historyの設計書](history-forms.md)の共有の表と同じ考え方にそろえる。1つの表を「文脈の手の駒種、文脈の手の到達升、1手前の手の駒種、1手前の手の到達升」で引き、2手前、4手前、6手前の文脈は同じ表の中の区画を引く。Stockfishも1つの表を3つの文脈で引く（`src/search.cpp:104-109`）。升は、手番側が後手なら盤を180度回転させた升で引く。駒の手番は、手番側と何手前の手かで決まるので添字に含めない。文脈の手の駒種は着手前の駒種とし、1手前の手の駒種は着手後に到達升にある駒種とする。Stockfishも文脈には動かした駒（`src/position.cpp:816-820`、`src/search.cpp:692-693`）を、1手前の手には到達升の駒（`src/search.cpp:106`）を使う。文脈の手がない場合（根より前とnull move）は、Stockfishの番兵（`src/search.cpp:303-309, 697-702`）と同じく、表の中の「文脈なし」の区画を引いて更新する。表は文脈の駒種29種に「文脈なし」を加えた30種、升144、駒種29、升144の約1,800万要素の`i16`である。 |
| 値の形と更新（C4） | 利用者の決定により、Stockfishの形にする。要素は−1024以上1024以下の整数とし、更新量`b`に対して`v ← v + b − v|b|/1024`の重力式で更新する（Stockfish `src/history.h:70-76`）。更新量は、探索値と合成した補正後の静的評価との差、つまり全表の合成の残差から作り、各表へ表ごとの倍率を掛けて同じ観測で更新する（Stockfish `src/search.cpp:120-143, 1664-1673`）。表ごとの倍率と初期値は、利用者の決定によりStockfishの値で固定する。現行の指数移動平均の重み`CorrectionWeight`と深さの頭打ち8は、新しい形では使わないので除く。Stockfishは深さの頭打ちを持たず、更新量を±256で切り詰める。 |
| 更新の条件（C4） | Stockfishの条件に合わせる（`src/search.cpp:1666-1667`）。上界でないノードでは、最善手が捕獲手でなく、探索値が補正後の静的評価より高い場合に更新する。上界のノードでは、探索値が補正後の静的評価以下の場合に更新し、捕獲手の条件は課さない。Stockfishの`!ss->inCheck`に当たる条件として、手番側の王駒に相手の利きが届くノードを除く現行の条件は残す。中将棋の規則から来る条件であり、王手の義務がなく（RULES.md第8条）、王駒が取られ得る局面の静的評価は探索値から大きく外れるからである。探索値が詰み帯と専用の帯にあるノードを除く現行の条件も残す。正確な値のノードで探索値が静的評価より低い場合は、現行は更新するが、Stockfishに合わせて更新しない。更新量は上界のノードで1.5倍になる（Stockfish 18/12）。 |
| 持ち越し（C1） | 利用者の決定により、全表を対局の間、減衰させずに持ち越す。Stockfishは表を新規対局でだけ初期化する（`src/search.cpp:713-736`）。表は`HistoryTables`に置き、初期化する時点はbutterfly historyと同じく、新規対局、受理された規則変更、受理された置換表容量の変更、およびワーカー数の変更とする（[ADR](../adr/carry-history-tables-within-a-game.md)）。持ち越しの割合の係数は置かない。 |
| 補正量の大きさの利用（C5） | 補正量の絶対値が大きいノードでは静的評価が当てにならないとみなし、LMRの減深量を減らす。Stockfishと同じ項である（`src/search.cpp:1353`）。減深量の項は、歩兵価値1枚分の補正量あたりの1/100手とする。reverse futility pruningの余裕値の項（同1030行）は、search-pruning-formsのF1が持つ。 |
| 補正を読む時点 | Stockfishと同じく、補正量はノードごとに1回だけ読む（`src/search.cpp:832`）。補正後の評価を使う判定（F2）、更新の残差、およびC5は、すべてこの値を使う。search-pruning-formsのF2も、同じくノードごとに1回読む形をとる。 |

## 変更の内容

以下の式で、`P`は評価関数の重みファイルが持つ探索用の歩兵価値、`d`はIIR（内部反復減深）を適用した後の残り深さ、`trunc`は0方向への切り捨て、`clamp(x, a, b)`は`x`を`a`以上`b`以下に切り詰めた値を表す。

### C1　補正表の持ち越し

補正表を`Searcher`の生成時に作る現行の形をやめ、`HistoryTables`に置いて探索をまたいで受け渡す。
局面を鍵にした表は全ワーカーで共有する1組とし、continuation correction historyはワーカーごとに持つ。
最初のコミットでは、表を新しい根の探索の開始時に初期化して現行と同じ挙動を保つ。
C4からC3までを入れた後のC1のコミットで、根の探索ごとの初期化をやめ、表を`HistoryTables`と同じ時点でだけ初期化し、局面の表を全ワーカーで共有する。
`minase dev bench`は局面ごとに`HistoryTables`を初期化するので、持ち越しの挙動は単体テストで確かめる。

### C2　局面の駒配置を鍵にした表と重み付きの和

材料表に加えて、pawnの表`e_P`、minorの表`e_m`、先手と後手のnon-pawnの表`e_N先`と`e_N後`を持つ。
pawnの鍵はhistory-formsが導入する歩兵と仲人の配置の鍵を再利用する。
minorとnon-pawnの鍵も同じく、駒と升の組ごとの64ビットの乱数を排他的論理和で重ねたものとし、着手の移動元、到達升、および捕獲した駒の升から差分で更新し、null moveでは前の局面の鍵をそのまま引き継ぐ。
居喰いのように移動元と到達升が同じ着手でも、同じ式で正しく更新される。
補正量`c`を次の式で求める。

```math
c = \mathrm{trunc}\left(\frac{P \times (W_M e_M + W_P e_P + W_m e_m + W_N (e_{N先} + e_{N後}) + Q)}{102400}\right)
```

ここで`e_M`は材料表の要素であり、各要素は手番側の行から引く。
`W_M`、`W_P`、`W_m`、`W_N`、`W_C`、`W_{C6}`は、それぞれ`CorrectionMaterialWeight`、`CorrectionPawnWeight`、`CorrectionMinorWeight`、`CorrectionNonPawnWeight`、`CorrectionContinuationWeight`、`CorrectionContinuation6Weight`である。
重みの単位は、要素が上限1024のときの補正量を歩兵価値に対する百分率で表したものである。
`Q`はC3の項であり、1手前の手がある場合は`W_C (e_{C2} + e_{C4}) + W_{C6} e_{C6}`、1手前の手がnull moveの場合は定数303である。
定数303は、Stockfishの定数80695（`src/search.cpp:109`）を同じ単位へ換算した値である。
補正後の静的評価は、補正前の静的評価に`c`を足し、評価関数の出力の範囲に収めた値とする。

### C3　continuation correction history

ply `p`のノードの`k`手前の手は、history-formsの記録のうちply `p − k`のノードで指した手である。
1手前の手はply `p − 1`、2手前、4手前、6手前はそれぞれply `p − 2`、`p − 4`、`p − 6`の記録にあたる。
1手前の手は、記録の`Move`と着手前の駒種から、着手後の駒種（成る手なら成った駒種）と到達升`Move::to`の組で表す。
文脈の手は、記録の着手前の駒種と到達升の組で表す。
居喰いとじっとの到達升は移動元と同じ升である。
`k`が2、4、6のそれぞれについて、文脈の手と1手前の手の升を手番側に合わせて回転し、表`T`を引いて`e_{Ck}`とする。
文脈の手がない場合は、表の「文脈なし」の区画を引く。
1手前の手がnull moveの場合は表を引かず、C2の定数`Q`を使い、更新もしない。

### C4　表の値の形と更新

全表の要素を−1024以上1024以下の`i16`とする。
起点の`update_correction`が更新する時点で、次の条件を満たすノードについて更新量`b`を求める。
条件は、探索値が詰み帯と専用の帯の外にあること、手番側の王駒に相手の利きが届かないこと、および上界でないノードでは最善手が捕獲手でなく`D > 0`、上界のノードでは`D ≤ 0`であることである。
ここで`D`は、探索値から、ノードごとに1回読んだ補正量による補正後の静的評価を引いた差である。

```math
b = \mathrm{clamp}\left(\mathrm{trunc}\left(\frac{D \times d \times K}{P}\right), -256, 256\right)
```

`K`は、上界でないノードでは`CorrectionBonus`、上界のノードでは`CorrectionFailLowBonus`である。
表`i`の要素`v`は、`b_i = clamp(trunc(b × f_i/128), −1024, 1024)`として、`v ← v + b_i − trunc(v × |b_i|/1024)`で更新する。
`f_i`は、材料表とpawnの表が128、minorの表が150、non-pawnの2表が186、continuation correction historyの2手前、4手前、6手前の区画がそれぞれ130、70、35である。
表を初期化するときは、局面の表の全要素を−5、continuation correction historyの全要素を5にする。
`CorrectionCap`と`CorrectionWeight`は`params.rs`の係数の表から除く。
更新の倍率、初期値、および上下限は、固定する定数の節の値を使う。

### C5　LMRの減深量の補正量の項

補正項`q_corr = trunc(LmrCorrection × |c| / P)`を求める。
`c`はノードごとに1回読んだ補正量である。
`q_corr`は、[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF4が定める1/100手単位の減深量の式から、allNodeによる拡大の前に1回だけ引く。
history-formsのhistoryの項も同じ位置で引く。
以後の拡大、手単位への丸め、上下限、および延長とPVノードの扱いは、F4の式に従う。
全表の要素が上限1024に達したときの補正量は、開始値の重みの和（起点の`CorrectionCap`の値＋260%）から歩兵価値の同じ倍率になる。
開始値101で減らす量は最大でその倍率と同じ手数であり、起点の`CorrectionCap`が現行の220なら約4.8手である。
実際の要素は上限より小さいので、通常はこれより少なく減らす。

### 新しい係数

換算値の列は、Stockfishの値を歩兵価値208、YaneuraOuの値を歩兵価値90で、minaseの単位へ直した値である。

| 係数 | 開始値 | 範囲 | 換算値と根拠 |
|---|---|---|---|
| `CorrectionMaterialWeight`（材料表の重み、要素が1024のときの補正量の歩兵価値に対する百分率） | 起点の`CorrectionCap`の値（例: 220） | 0〜500 | StockfishにもYaneuraOuにもない。開始値は起点の`CorrectionCap`の値である（設計判断の表を参照）。`CorrectionCap`の範囲は50〜400なので、範囲は起点の値と0を含む |
| `CorrectionPawnWeight`（pawnの表の重み、同） | 52 | 0〜200 | Stockfishは13806/131072×1024/208で約52（`src/search.cpp:110, 116-118`）、YaneuraOuは12153から約105（`yaneuraou-search.cpp:737`）。現行は表がないので0 |
| `CorrectionMinorWeight`（minorの表の重み、同） | 36 | 0〜200 | Stockfishは9512から約36、YaneuraOuは8620から約75。現行は0 |
| `CorrectionNonPawnWeight`（先手と後手のnon-pawnの表に共通の重み、同） | 44 | 0〜200 | Stockfishは11615から約44、YaneuraOuは12355から約107。現行は0 |
| `CorrectionContinuationWeight`（continuation correction historyの2手前と4手前の文脈に共通の重み、同） | 30 | 0〜200 | Stockfishは7885から約30（`src/search.cpp:105-107`）、YaneuraOuは7982から約69。現行は0 |
| `CorrectionContinuation6Weight`（同じ表の6手前の文脈の重み、同） | 24 | 0〜200 | Stockfishは6307から約24（`src/search.cpp:108`）。YaneuraOuにはない。現行は0 |
| `CorrectionBonus`（上界でないノードの更新量、歩兵価値1枚分の差と深さ1手あたりの要素の点数） | 20 | 4〜60 | Stockfishは208×12/128×1061/1024で約20（`src/search.cpp:1670, 1672`）、YaneuraOuは90×12/128×1069/1024で約9（`yaneuraou-search.cpp:4414, 4417`）。現行の指数移動平均とは形が違い、対応する値はない |
| `CorrectionFailLowBonus`（上界のノードの更新量、同） | 30 | 6〜90 | Stockfishは208×18/128×1061/1024で約30、YaneuraOuは90×17/128×1069/1024で約12 |
| `LmrCorrection`（歩兵価値1枚分の補正量あたりに減らす減深量、1/100手） | 101 | 0〜200 | Stockfishは131072×208/(26310×1024)で約101（`src/search.cpp:1353`）、YaneuraOuは131072×90/(25600×1024)で約45（`yaneuraou-search.cpp:3890`）。現行は0 |

`params.rs`の係数の表から除く係数は、`CorrectionCap`（補正値の絶対値の上限）と`CorrectionWeight`（指数移動平均の重み）の2つである。

### 固定する定数

利用者の決定により、次の定数はStockfishの値で固定し、SPSAで調整しない。
行番号はStockfish（コミット5c4cef0b）のものである。

| 定数 | 値 | Stockfishの箇所 |
|---|---|---|
| 表の要素の上限 | ±1024 | `src/history.h:41` |
| 更新の重力式 | `v ← v + b_i − v|b_i|/1024`、表ごとの更新量`b_i`は±1024で切り詰める | `src/history.h:70-76` |
| 合成の残差から作る更新量`b`の切り詰め | ±256（上限の1/4） | `src/search.cpp:1670-1671` |
| 更新量に掛ける倍率 | 1061/1024。`CorrectionBonus`と`CorrectionFailLowBonus`の換算に含める | `src/search.cpp:1672` |
| 表ごとの更新の倍率（pawn、minor、non-pawn） | 128、150、186（/128） | `src/search.cpp:127, 130-133` |
| continuation correction historyの文脈ごとの更新の倍率（2手前、4手前、6手前） | 130、70、35（/128） | `src/search.cpp:139-141` |
| 材料表の更新の倍率 | 128（/128）。Stockfishに材料表はなく、基準のpawnの表と同じ値とする | なし |
| 局面の表の初期値 | −5 | `src/search.cpp:719` |
| continuation correction historyの初期値 | 5 | `src/search.cpp:731-733` |
| 1手前の手がnull moveのときの定数 | 80695。本書の単位で303 | `src/search.cpp:109` |
| 局面の表の大きさ | 手番側ごとに65,536×ワーカー数以上の最小の2のべき | `src/history.h:40`、`src/thread.cpp:213` |
| 補正後の評価の範囲 | 評価関数の出力の範囲に収める。Stockfishは終局表の値の帯の外に収める | `src/search.cpp:116-118` |

## 所要時間の見積り

時間は、同時対局数16の測定機での見積りである。
SPSAは、3,000ペア（同時対局数16）の[セッション](../measurements/spsa-stage9-20260925-c5.md)が27,138秒（1ペア約9秒）を要したことから、30,000ペアを約75時間（約3日）と見込む。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 形の変更の実装と検証（codexへ委任） | 1日程度 |
| 2 | 時間切れの事前確認（13反復、104ペア） | 約15分 |
| 3 | SPSA（30,000ペア） | 約75時間 |
| 4 | 反映、テスト、および事前対局（32ペア） | 約1時間 |
| 5 | 候補のSTC | 約1〜8時間 |
| 6 | 通過した場合のLTC | 約5〜30時間 |

実装を除く測定機の占有は、STCで棄却される場合は段階2から5までの和の約77〜84時間（約3.2〜3.5日）、LTCまで進む場合は段階2から6までの和の約82〜114時間（約3.4〜4.8日）となる。

## 実装フェーズ

基本シードは、時間切れの事前確認を4100000000、セッションを4150000000、事前対局を4200000000、STCを4250000000、LTCを4300000000と仮に定める。
これらは起案時点で、保存済みの測定とSPSAの基本シード（`data/matches/`と`data/spsa/`の`manifest.json`）、および他の設計書が予約した値から、各測定のペア数以上離れた未使用の値である（[教訓](../lessons/derive-seed-adjacent-collision.md)）。
着手時に同じ方法で未使用であることを確かめ直し、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　形の変更の実装

起点からブランチ`correction-history-tables`を切り、worktree `data/worktrees/correction-history-tables`で作業する。
最初に起点のコードを読み、依存関係の節のどの場合に当たるかを確かめて、LMRの減深量の単位を改める必要があるか、着手の記録と歩兵と仲人の配置の鍵を加える必要があるかを決める。
コミットは、挙動を変えない移行と、挙動を変える項目に分ける。
挙動を変えない移行は、補正表を`HistoryTables`へ移して根の探索ごとに初期化するコミット、必要な場合の着手の記録と歩兵と仲人の配置の鍵の追加、およびLMRの減深量の単位の変更であり、これらを先に置く。
各移行のコミットでは、`minase dev bench`の全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
挙動を変える項目は、C4、C2、C3、C1、C5の順に1項目を1コミットにする。
C4を先に置くのは、材料表だけの状態で値の形と更新の変化を分けて記録するためである。
局面の表の共有はC1のコミットで入れる。`Threads=1`の`minase dev bench`は共有の有無で結果が変わらないので、共有の正しさは複数ワーカーの単体テストで確かめる。
各項目のコミットでは、同じbenchの深さ3、5、7の全15局面の探索ノード数を記録し、直前のコミットとの差を残す。
C1はbenchのノード数を変えない（benchは局面ごとに表を初期化する）ので、持ち越しは単体テストで確かめる。
`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点と形の変更の最後のコミットで交互に3回ずつ測り、探索速度（NPS、1秒あたりの探索ノード数）の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
同じ実行で最大常駐メモリも記録し、増分が、1ワーカーのbenchで約37 MB（ワーカーごとのcontinuation correction historyの約36 MBと、共有の表の約1.3 MB）であることを確かめる。
新しい係数は`params.rs`の表へ追加し、`CorrectionCap`と`CorrectionWeight`を表から除き、調整用ビルド（`--features tuning`つきのビルド）が新しい係数を`Tune_<係数名>`のUSIオプションとして宣言することを確かめる。
単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
少なくとも、補正量の和と定数`Q`の数値例、重力式の更新と表ごとの倍率の数値例、更新の条件（上界と上界でないノード、捕獲手、王駒への利き）、`HistoryTables`の初期化の時点と根の探索をまたぐ持ち越し、複数ワーカーでの局面の表の共有と表の大きさ、continuation correction historyが後手番で升を回転して引くこと、文脈なしの区画を引くこと、1手前がnull moveのとき表を引かずに更新もしないこと、および固定シードの着手列とその全候補手について差分更新した配置の鍵が局面からの再計算と一致することを確かめる。
最後の照合は、現行の`correction_incremental_keys_match_seeded_playouts_and_undo`と同じく、捕獲、成り、獅子の2枚取り、居喰い、およびnull moveを含める。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過、移行のコミットでのノード数の一致、各項目のノード数の変化の記録、および探索速度と最大常駐メモリの記録である。

### フェーズ2　パラメーターファイルと時間切れの事前確認

形の変更の最後のコミットでビルドした`minase`を`data/spsa/correction-history-tables-runner/`へ複製してrunner（測定を実行するプログラム）として固定し、以後のフェーズはこのrunnerで実行する。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除したファイルを`data/spsa/correction-history-tables.params`とする。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、使い捨てのスクリプトで機械的に突き合わせる。

続いて、手引きの第3節に従い、時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
写しの開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150へ書き換えた`data/spsa/correction-history-tables-safety.params`で、13反復（104ペア）を実行する。
起点で時間管理の係数の名前や範囲が変わっていれば、手引きの第3節の値を起点の表に合わせて読み替える。

```console
<固定したminase> spsa \
  --run-dir data/spsa/correction-history-tables-safety --seed 4100000000 \
  --engine commit:<形の変更> --params data/spsa/correction-history-tables-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`correction-history-tables-safety`に残す。
完了条件は、時間切れとエンジン異常が0件であることと、測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<固定したminase> spsa \
  --run-dir data/spsa/correction-history-tables --seed 4150000000 \
  --engine commit:<形の変更> --params data/spsa/correction-history-tables.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 3750 --pairs-per-iteration 8
```

最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`correction-history-tables`に、手引きの第6節の項目とともに残す。
最終値を丸めた整数が範囲の上限または下限になった係数を示す。
新しい9係数については、最終値が開始値（Stockfishの換算値）からどれだけ離れたかを、`c_end`に対する比で示す。
この比は、Stockfishの値から離れた係数を読むための目安であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、テストの前提が新しい既定値でも成り立つようにテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で握りつぶさない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

採否測定の前に、候補が時間切れやエンジン異常を起こさないことと、時間の使い方の変化を確かめるため、STCの条件で候補と起点の32ペアの事前対局を行う。

```console
<固定したminase> match run \
  --run-dir data/matches/correction-history-tables-smoke --seed 4200000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 elo --pairs 32
```

手引きの第7節の手順2に従い、候補と起点の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ（反復深化で読み終えた深さ）、および探索を止めた理由の分布を集計して、測定記録`correction-history-tables-smoke`に残す。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、および測定記録の保存である。

### フェーズ5　採否測定

```console
<固定したminase> match run \
  --run-dir data/matches/correction-history-tables-stc --seed 4250000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

STCの判定は[SPRTの手引き](../guides/sprt.md)の振分け規則に従う。
候補が有意に強い`H1`ならLTCへ進み、改善を示せない`H0`なら不採用とし、3,000ペアで判定保留ならその時点の対数尤度比の符号でLTCへ進むかを決める。
LTCは測定名の接尾辞を`-ltc`、基本シードを4300000000、持ち時間を`time=60000+200`とし、`--max-pairs`を付けない。
LTCで`H1`となり、エンジン異常、時間切れ、および拒否着手が0件なら採用し、`H0`なら不採用とする。
LTCの判定保留は、手引きのとおり`--resume`と`--target-pairs`で延長する。
結果は測定名ごとの測定記録に残す。
完了条件は、採否の確定と測定記録の保存である。

## 検証

- 挙動を変えない移行の各コミットで、`minase dev bench`の全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。
- C1からC5までの各コミットで、同じbenchの探索ノード数の変化が記録されていること。
- 差分更新した配置の鍵が、捕獲、成り、獅子の2枚取り、居喰い、およびnull moveを含む着手列で、局面からの再計算と一致すること。
- 新しい9係数が調整用ビルドの`Tune_`オプションとして宣言され、パラメーターファイルの開始値と範囲が表と一致すること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`が通過すること。
- 時間切れの事前確認、SPSA、事前対局、および採否の測定で、エンジン異常、時間切れ、および拒否着手が0件であること。最初の測定で1件でも出た場合は、測定を止めて原因を調べる。

## 完了条件

- 形の変更の検証結果（ノード数の変化、探索速度、最大常駐メモリ）、時間切れの事前確認、SPSAの結果、事前対局、および採否が、測定記録に残っている。
- 採用した場合は、候補のブランチがmasterへ統合され、依存関係の節に挙げたADRが新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- 不採用の場合は、結論と測定記録だけをmasterへ残し、候補のブランチはmasterへ統合しない。
- ROADMAPの状態表が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)。
- [棋力向上段階12の発動率の診断](../measurements/strength-stage12-activation-diag.md)と、その付属の`correction-keys.patch`および`correction-results.json`。
- Stockfish（コミット5c4cef0b）。`src/history.h`の表の大きさと上限は40〜41行、重力式は70〜76行、補正表の型は152〜191行、共有の表は206〜250行、共有の要素の不可分の読み書きは60〜63行。`src/search.h`のワーカーごとのcontinuation correction historyの宣言は335行。`src/thread.cpp`の共有の表の大きさは213行。`src/search.cpp`の補正量は95〜118行、更新は120〜143行、根の番兵は303〜309行、文脈の設定は692〜702行、初期化は713〜736行、ノードでの補正量の読み出しは832行と860行、LMRの補正量の項は1353行、更新の条件と更新量は1664〜1673行。`src/position.cpp`の鍵の定義は505〜526行、成りでの鍵の更新は947〜957行、動かした駒の記録は816〜820行。`src/types.h`の歩兵価値は199行。
- YaneuraOu（コミットc1b80eaa）。`source/position.cpp`のminor pieceの定義は27〜36行。`source/engine/yaneuraou-engine/yaneuraou-search.cpp`の補正量は724〜746行、更新は748〜770行、LMRの補正量の項は3890行、更新の条件と更新量は4404〜4417行。`source/evaluate.h`の歩兵価値は78行。
- 日本中将棋連盟「中将棋連盟競技規則・総則編（2019年度版）」の駒の格付け（RULES.mdの［J1］）。
- [correction historyの鍵と適用先のADR](../adr/key-correction-history-by-material.md)と[持ち越しのADR](../adr/carry-history-tables-within-a-game.md)。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)と[SPRTによる棋力測定の手引き](../guides/sprt.md)。
