# aspiration windowsの幅と窓の移し方の見直しの設計書

## 先に読む要約

minaseの探索は反復深化、つまり深さ1から1手ずつ深くして探索を繰り返す方式で進み、各反復は探索木の根である手番側の現局面から読む。
本書は、各反復で根を狭い窓から読み始めるaspiration windows（前の反復の値の周りに窓を置き、窓を外れたら広げて同じ深さを読み直す手法）を、チェスのStockfishの形にそろえ、現行のmasterより強くなるかを確かめる。
窓とは、根の探索で正確な値を求める評価値の範囲αからβであり、狭いほど探索は速いが、値が範囲を外れると読み直しになる。
現行のminaseは、深さ5以上で、直前の反復の値を中心に歩兵価値（探索が余裕値の単位に使う歩兵1枚の駒価値）の55%を半幅として窓を置き、外れた側の端だけを元の端から広げる（`crates/minase/src/search/alphabeta/root.rs`）。
本書は、2026年10月17日の利用者の決定に従い、窓の幅、中心、拡大、外れたときの窓の移し方、およびfail high（値が窓の上端以上になること）のときの減深を、Stockfishの形と値にそろえる。
具体的には、初期の半幅を根の値の大きさとともに広げ、中心を、直前の反復の最善手を根で読むたびに得た値を、探索ノード数で重み付けして平均した値に置き、窓を全深さで使い、外れたときは返った値を基準に窓を移して反対側の端も寄せ、fail highが続くときは読み直しを浅くする。
新しい係数と、形を変える既存の係数は、Stockfishの換算値を開始値とし、既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整する。
調整した版を、起点のmasterと短時間条件STCおよび長時間条件LTCの2段階の逐次検定（段階ゲート）で対局させ、1つの候補として採否を決める。
完了条件は、候補の採否が測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案した。
[連鎖の1つ前の設計書（静止探索の関数形の見直し）](qsearch-forms.md)（以下、前計画）の完了を待って着手し、着手は利用者の指示による。
次の一手は、前計画の完了後に起点を確定し、フェーズ1（形の変更の実装）を始めることである。

## 目的

本書が確かめるのは、「aspiration windowsをStockfishの形にそろえて係数を調整し直せば、現行の形で調整した探索より強くなる」という仮説である。
現行の形は、[aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)が2026年9月に決めたものであり、当時の既定の評価関数はPST（駒の種類と升ごとの値の表による評価）だった。
2026年10月12日に既定の評価関数がNNUE（差分更新できる小型のニューラルネットワーク）へ置き換わった後、窓の形は測り直していない。
窓は`AspirationDelta`と`AspirationGrowth`の2係数でSPSAの対象になっているが、現行の形では、評価値の大きさによる幅の違い、反復ごとの値の揺れを均した中心、大きく外れたときの窓の移し方、およびfail highのときの減深を、係数の値で表せない。

2026年10月17日に、利用者は、探索の関数形を見直す一連の設計書の判断事項を、基本的にStockfishとYaneuraOu（やねうら王）に合わせると決めた。
両エンジンで形や値が違う場合はStockfishに合わせ、Stockfishにだけある項目は入れる。
本書の範囲では、幅、中心、拡大、窓の移し方をStockfishの形にそろえ、fail lowとfail highで寄せ方を分け、fail highのときの減深と、ワーカー番号で幅をずらす項も含めると決めた。
新しい係数の開始値は両エンジンの換算値（両者が違えばStockfishの値）とし、現行と同じ探索になる開始値の原則はとらない。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、前計画が完了した後、フェーズ1を始める時点のmasterの先頭のコミットである。
- **形の変更**は、本書の項目を実装したコミット列であり、係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で表（`crates/minase/src/search/alphabeta/params.rs`）の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **ワーカー**は、Lazy SMP（複数のスレッドが置換表を共有して同じ局面を探索する並列化）の各探索スレッドであり、主ワーカーの番号は0、補助ワーカーは1から数える。採否の測定は`Threads=1`で行うので、測定で動くのは主ワーカーだけである。
- **根の手**は、根の局面の合法手である。各ワーカーは、USIの`go`コマンドで探索を始めるたびに、根の手ごとの記録を空にする。
- **半幅**`δ`は、窓の中心から各端までの距離であり、窓外れのたびに端を動かす量にも使う（現行の`AspirationWindow`の`delta`）。**窓の幅**は`β − α`をいう。
- **窓外れ**は、根の探索の返り値がα以下になること（fail low）か、β以上になること（fail high）をいう。等号も窓外れとする現行の扱いは変えない。
- **返った値**は、窓外れのときに根の探索が返した値である。minaseの探索はfail-soft（窓の外の値を窓の端に丸めずに返す方式）であり、根の`search_root`は窓の端とは別に最大値を追い、その値を返す。fail lowの返った値はα以下の上界、fail highの返った値はβ以上の下界である。
- **詰み帯と専用の帯**は、詰みまでの手数を表す評価値の範囲と、千日手などの反復の結果を表す評価値の範囲（`crates/minase/src/search/mod.rs`の`MATE_THRESHOLD`と`REPETITION_BAND_START`）である。通常の評価値はこれらの外にある。

対象は次の項目である。

- 半幅の下限と拡大の丸め（M1）。挙動を変えない移行である。
- 根の手ごとの探索ノード数、平均、2乗平均の記録（M2）。記録の追加であり、挙動を変えない移行である。
- 窓の中心を、直前の反復の最善手の平均にすること（A1、Stockfishの`averageScore`）。
- 初期の半幅を、Stockfishの形の定数項、評価値の大きさの項、およびワーカー番号の項の和にすること（A2）。
- 窓を全深さで使うこと（A3）。
- 窓外れのときの窓の移し方と拡大率（A4）。
- fail highの回数による減深（A5、Stockfishの`failedHighCnt`と`failHighRecovery`）。

次の事項は対象外とする。

- 深さを増やさずに同じ深さを読み直す回数による減深（Stockfish `src/search.cpp:335, 369-370, 411`の`searchAgainCounter`）。この回数は、目安時間の50%を過ぎたら深さを増やさない時間管理の仕組み（`src/search.cpp:634`の`increaseDepth`）が増やすものであり、[連鎖の次の設計書](time-instability-scaling.md)はこの仕組みを対象外としている。minaseでは回数が常に0になるので、項を置かない。
- 評価値の平均から評価関数の楽観度（optimism）を調整する項（Stockfish `src/search.cpp:397-399`）。評価関数の側の変更であり、minaseのNNUEは楽観度の入力を持たない。
- fail lowのときに先読みの的中で止める指示を取り消す処理（Stockfish `src/search.cpp:443-444`の`stopOnPonderhit`）。先読みの扱いは時間管理の側の仕組みであり、本書の範囲外である。
- 窓外れの途中経過をUSIの`lowerbound`または`upperbound`の行で出力すること。[aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)が、対局ハーネスに保存される手の深さと経過時間が未完了の反復の値になるとして棄却しており、本書も出力しない。

## 依存関係

起点は、[連鎖の1つ前の設計書](qsearch-forms.md)が完了した後のmasterの先頭である。
前計画の候補が採用されたかどうかにかかわらず、起点の表の既定値をSPSAの開始値とし、本書が形を変える係数だけを本書の開始値に置き換える。
前計画の候補がmasterへ統合されると`params.rs`の既定値が変わり、本書の検証の基準も変わるので、実装を含めて前計画の完了後に始める。
探索の関数形をStockfishとYaneuraOuの形へ寄せる設計書は、[枝刈りとLMRの関数形](search-pruning-forms.md)、浅い枝刈り、history、correction history、静止探索、本書、および時間管理の順の連鎖として起案している。
この順序は提案であり、最終的な順序は利用者が決める。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、同時対局数は16を上限とする。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は次のADRの決定を変える。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確認し、新しいADRで置き換える。
本書が変えない決定、つまり窓外れの途中経過を出力しないこと、および補助ワーカーが自分の探索の記録だけで窓を置くことも、新しいADRへ引き継ぐ。

- [aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)は2026年9月12日に、深さ5以上で直前の完了値を中心に窓を置き、外れた側の端だけを元の端から広げ、fail highのときに浅く読み直さないと決めた。窓の実装を採用した[STC](../measurements/strength-stage6-aspiration-stc.md)と[LTC](../measurements/strength-stage6-aspiration-ltc.md)は、PSTを既定の評価関数としていた時期の測定である。窓を使い始める深さ5は、当時のSTCの完了深さの最頻値が6だったことから選んだ値であり、測って選んだ値ではない。外れなかった側の端を中点へ寄せる案は、窓の意味を複雑にするという設計上の判断で棄却され、測られていない。fail highの回数だけ読み直しを浅くする方式は、[棋力向上段階12](archived/strength-stage12.md)のSTCで改善を示せない判定`H0`となって棄却された（[測定記録](../measurements/strength-stage12-aspiration-stc.md)）。この測定もPSTの時期のもので、2026年9月27日に行われ、当時のminaseはStockfishの`failHighRecovery`に当たる持ち越しを持たなかった。ADRは、この方式を再び提案するなら新しい構成で別の主張として段階ゲートにかけることを求めているが、2026年10月17日の利用者の決定により、本書はこの方式を他の項目と同じ候補に含める。本書は、窓を使い始める深さ、中心、窓の移し方、およびfail highのときの減深の決定をすべて置き換える。

[深さを飛ばすADR](../adr/diversify-lazy-smp-workers-by-skipping-depths.md)による補助ワーカーの多様化は変えず、A2のワーカー番号の項はそれに加わる。
[反復の結果の帯のADR](../adr/reserve-a-path-dependent-band-for-repetition-outcomes.md)が定めた窓の端の保護、つまり詰み帯または専用の帯に達した端を無限へ置き換える扱いは変えない。

## 設計判断

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 全項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。代償として、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| 開始値 | 2026年10月17日の利用者の決定により、係数の開始値はStockfishの値の換算値とする。同じ日の利用者の決定により、Stockfishに同じ形の値がある既存の係数も換算値に置き直すので、`AspirationDelta`と`AspirationGrowth`も起点の表の値ではなくStockfishの換算値から始める。挙動を変えない移行（M1、M2）のコミットだけをノード数の一致で検証し、それ以外のコミットでは`minase dev bench`のノード数の変化を記録する。 |
| 換算の単位 | minaseの係数は歩兵価値に対する百分率で表す。Stockfishの評価値は歩兵価値208を単位とし（`src/types.h:199`）、現行の埋め込みネットの歩兵価値は100である。Stockfishの値`x`は歩兵価値の`100x/208`%に換算する。 |
| SPSAで調整する値と固定する定数 | 2026年10月17日の利用者の決定により、SPSAで調整するのは各仕組みの強さを決める値だけとし、上下限、平均の重み、および更新の内部の定数はStockfishの値で固定する。本書で調整するのは初期の半幅の定数項、評価値の大きさの項の重み、および拡大率の3係数である。平均の重みの定数と減深の回数の規則は、変更の内容の節の「固定する定数」の表に行番号つきでまとめる。ワーカー番号の項も換算値で固定する。採否の測定とSPSAは`Threads=1`で行い（[Threadsの既定値のADR](../adr/keep-default-threads-at-one.md)）、動くのは番号0の主ワーカーだけなので、この項は測定で働かない。 |
| 評価値の単位を持つ係数の範囲 | 2026年10月17日の利用者の決定により、式の形と開始値はStockfishに合わせ、値はminaseのSPSAが決める。評価値の単位を持つ係数の範囲は、Stockfishの換算値（歩兵価値208で割る）、YaneuraOuの換算値（歩兵価値90で割る）、およびminaseの現行の値の3つをすべて含むように取る。中将棋は将棋ともチェスとも異なるゲームであり、YaneuraOuは同じ形の定数を将棋向けに調整し直しているからである。3つの値は係数の表に並べる。開始値が他の2つから遠いのは`AspirationDelta`であり、開始値2%は現行の55%の約1/27、YaneuraOuの約5.6%の約1/3である。`AspirationScoreWidth`の開始値204%も、現行の0（評価値の項がない形）とYaneuraOuの約100%から離れている。 |
| SPSAの対象 | 起点の表の調整対象の係数（`FmScale`を除く。数は起点で確定する）に、新しい1係数`AspirationScoreWidth`を加える。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の係数を含めるので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。3,750反復、1反復8ペア（計30,000ペア）、持ち時間10秒＋0.1秒、同時対局数16、`c_end`（最後の反復の摂動幅）は範囲の1/6、`r_end`（最後の反復の学習率）は0.002である（[SPSAの手引き](../guides/spsa.md)）。30,000ペアは候補を1つ作るための予算であり、最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。先行の設計書の候補をすべて採用した場合、調整する係数の数は本書の時点で90前後になる（起点の表で数え直す）。30,000ペアでこの数を十分に調整できるかは未検証であり、不採用は、この予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。 |
| 窓の中心（A1、averageScore） | 根の手ごとに、その手を読むたびの値の指数移動平均を持ち、次の反復の窓の中心には、直前に完了した反復の最善手の平均を使う。Stockfishと同じく、平均には窓外れのときの値や、2手目以降を零窓（幅1の窓）で読んだときの上界も入れ、重みはその手の今回の探索ノード数とそれまでの累計の比で決める（`src/search.cpp:1474-1495`）。重みの式は変更の内容の節に書く。探索が中断された手の値は入れない（`src/search.cpp:1466-1467`）。 |
| 帯の値と中心 | Stockfishと同じく、詰み帯と専用の帯の値も平均に入れる。ただし、中心が詰み帯または専用の帯にある反復は、現行と同じく全窓で読む。[反復の結果の帯のADR](../adr/reserve-a-path-dependent-band-for-repetition-outcomes.md)が窓の端に施した保護を、中心にも適用するためである。この保護は中将棋の反復の扱いから来るminaseの仕組みであり、2026年10月17日の利用者の決定により、Stockfishにない仕組みでも残す。 |
| 初期の半幅（A2） | Stockfishの`delta = 5 + threadIdx % 8 + |meanSquaredScore|/10193`（`src/search.cpp:389`）を換算する。定数項5は歩兵価値の約2.4%なので、`AspirationDelta`の開始値を2とする。2乗平均の項は、根の値が歩兵価値の10倍のときに加わる半幅を歩兵価値に対する百分率で表す係数`AspirationScoreWidth`とし、開始値は約204%とする。ワーカー番号の項は、番号`k`について`(k mod 8)`をStockfishの単位で加える形を、歩兵価値で換算して固定する。現行の埋め込みネットでは0〜3の値になる。 |
| 半幅の下限と拡大の丸め（M1） | 初期の半幅は1以上とし、拡大は`max(δ + 1, ⌊δ × AspirationGrowth/100⌋)`として、丸めた後も必ず1以上広がるようにする。現行の拡大`⌊δ × AspirationGrowth/100⌋`は整数除算なので、半幅が3以下で倍率が125%前後なら半幅が広がらないことがある。A2の開始値では、根の値が小さいと初期の半幅は2になり得る。Stockfishの拡大`delta += 47 × delta/128`（`src/search.cpp:455`）も、Stockfishの初期の半幅が5以上なので丸めで止まることはない。現行の埋め込みネットの歩兵価値は100で、`AspirationDelta`の現行の値55%では初期の半幅は55なので、M1は起点の探索を変えない。 |
| 全深さで窓を使う（A3） | Stockfishは全深さで窓の式を使う（`src/search.cpp:389-392`）。深さ1では平均がまだないので、Stockfishでは窓が下端に寄り、最初の探索がfail highした後に返った値の周りへ移る。minaseは深さ1では最善手の平均がなく窓を置けないので、全窓で読む。深さ2以上では、直前の反復の最善手に平均があれば窓を使う。 |
| 窓の移し方（A4） | Stockfishの形にそろえ、fail lowとfail highで寄せ方を分ける（`src/search.cpp:437-450`）。fail lowでは、βを元のαへ下げ、αを返った値から半幅だけ下げる。fail highでは、αを「元のβ − 半幅」と元のαの大きい方へ上げ、βを返った値から半幅だけ上げる。半幅は、拡大する前の値を使う。拡大率は既存の係数`AspirationGrowth`とし、開始値をStockfishの`1 + 47/128`に当たる137%とする。 |
| fail highの減深（A5） | Stockfishの形にそろえる（`src/search.cpp:403-411, 461-462`）。反復ごとにfail highの回数を0から数え、fail highのたびに1増やし、fail lowで0に戻す。各読み直しの深さは`max(1, d − 回数 − 持ち越し)`とする。持ち越しは`go`ごとに0から始め、各反復の開始時に2減らして0で止め、窓の内側で反復を完了した時点で回数が1以上なら、反復の終わりに持ち越しを`(回数 + 1)/2 + 2`とする。浅く読んだ窓で完了した反復も、深さ`d`の反復の完了として扱う。Stockfishも、完了した反復を`rootDepth`の深さとして記録する（`src/search.cpp:534`）。根の置換表への保存は、実際に読んだ深さで行う。 |
| 読み直しの停止の保証 | 半幅は1以上で始まり、M1により窓外れのたびに少なくとも1増えるので、有限回の窓外れで専用の帯の下端（`REPETITION_BAND_START`）の2倍を越える。その後のfail lowでは新しいαが、fail highでは新しいβが、必ず無限へ置き換わる。このときfail highの新しいαは、「元のβ − 半幅」が負の側の帯の端`−REPETITION_BAND_START`を下回り、元のαより小さいか無限へ置き換わるので、`max(β − δ, α)`により元のαのままになり、fail lowの新しいβは元のαになる。したがって、半幅が十分大きくなった後は、fail high（βが無限になる）、fail low（αが無限になり、βが元のαになる）、fail high（両端が無限になる）と続く場合でも、高々3回の窓外れで両端が無限になり、読み直しは止まる。無限のβでfail highすることも、無限のαでfail lowすることもないからである。 |
| 途中結果の採用 | 秒読みつき時計で中断した窓の途中結果は、現行どおり、その窓の開始時のαを上回る値だけを採る（`root.rs`の`RootResults`）。A4は読み直す窓のαを上げることがあり、A5は窓を浅く読むことがあるが、採る値の基準が読み直す窓のαであることは変わらない。 |

## 変更の内容

以下の式で、`P`は探索用の歩兵価値（評価関数の重みファイルが持つ値）、`d`は反復の深さ、`s`は窓外れのときの返った値、`δ`は現在の半幅、`k`はワーカー番号を表す。
整数の除算は、現行の`params.rs`と同じく0方向へ丸める。
平均、2乗平均、および探索ノード数の計算は64ビット整数で行う。

### M1　半幅の下限と拡大の丸め

初期の半幅を1以上にし、拡大を次の式にする。

```math
\delta' = \max\left(\delta + 1,\ \left\lfloor \frac{\delta \times \mathrm{AspirationGrowth}}{100} \right\rfloor\right)
```

計算は現行と同じく64ビット整数で行い、32ビット整数の上限で飽和させる。
半幅が4以上なら、倍率125%以上の第2項は第1項以上になるので、第1項が効くのは半幅が3以下のときだけである。
起点の係数と埋め込みネットでは半幅は55以上なので、探索は変わらない。

### M2　根の手ごとの記録

各ワーカーは、根の手`m`ごとに、累計の探索ノード数`E(m)`、値の平均`a(m)`、および2乗平均`q(m)`を持つ。
`go`の開始時には、すべての手の`E(m)`を0、`a(m)`と`q(m)`を空にする。
根で手`m`を読み終え、探索が中断されていなければ、その手を読むのに使ったノード数`N`と値`v`で次のように更新する。
読み直しを含め、根で手を読むたびに更新する。

```math
E_{\mathrm{prev}} = \max(1,\ E(m)), \qquad E(m) \leftarrow E(m) + N, \qquad
w = \min\left(24,\ \max\left(12,\ \left\lfloor \frac{32 \times 2N}{2N + 3E_{\mathrm{prev}}} \right\rfloor\right)\right)
```

`a(m)`が空なら`a(m) = v`、それ以外なら`a(m) ← (w × v + (32 − w) × a(m))/32`とする。
`q(m)`が空なら`q(m) = v × |v|`、それ以外なら`w_q = min(w, 16)`として`q(m) ← (w_q × v × |v| + (32 − w_q) × q(m))/32`とする。
これはStockfishの`src/search.cpp:1474-1501`と同じ式である。
M2の時点では記録を窓に使わないので、探索は変わらない。

### A1とA3　窓の中心と窓を使う深さ

深さ`d ≥ 2`の反復は、直前に完了した反復の最善手`b`の平均`a(b)`が空でなく、`a(b)`が詰み帯と専用の帯の外にあるとき、A2で定める初期の半幅`δ₀`を使って`[a(b) − δ₀, a(b) + δ₀]`を初期の窓とする。
それ以外は全窓で読む。
詰み帯または専用の帯に達した端を無限へ置き換える現行の扱いは変えない。
補助ワーカーは、自分の直前に完了した反復の最善手と、自分の記録を使う。

### A2　初期の半幅

```math
\delta_0 = \max\left( \left\lfloor \frac{P \times \mathrm{AspirationDelta}}{100} \right\rfloor + \left\lfloor \frac{\mathrm{AspirationScoreWidth} \times |q(b)|}{10000 \times P} \right\rfloor + \left\lfloor \frac{(k \bmod 8) \times P}{208} \right\rfloor,\ 1 \right)
```

根の値が`10P`で一定なら`q(b) = 100P²`となり、第2項は歩兵価値の`AspirationScoreWidth`%になる。
Stockfishでは`(10 × 208)²/10193`が約424であり、歩兵価値208の約204%に当たる。
第3項は、Stockfishでワーカー番号ごとに1単位ずつ加わる量を、歩兵価値の1/208として換算した値である。

### A4　窓外れのときの窓の移し方

fail low（`s ≤ α`）のとき、元の窓の端`α`と`β`から、新しい端を次のように求める。

```math
\beta' = \alpha, \qquad \alpha' = s - \delta
```

fail high（`s ≥ β`）のとき、次のように求める。

```math
\alpha' = \max(\beta - \delta,\ \alpha), \qquad \beta' = s + \delta
```

αが負の無限ならfail lowは起きず、βが正の無限ならfail highは起きないので、上の式の元の端のうち外れた側は常に有限である。
fail highで元のαが負の無限でも、`β − δ`が`−REPETITION_BAND_START`より上なら新しいαは有限になる。Stockfishの`max(beta − delta, alpha)`も同じである。
新しい端を求めた後、詰み帯または専用の帯に達した端を無限へ置き換え、その後でM1の式で半幅を広げる。

### A5　fail highの減深

各ワーカーは、`go`ごとに0から始める持ち越し`r`を持つ。
深さ`d`の反復を始めるとき、`r ← max(0, r − 2)`とし、fail highの回数`h`を0にする。
窓で読むたびに、読む深さを`max(1, d − h − r)`とする。
fail highのたびに`h`を1増やし、fail lowのたびに`h`を0に戻す。
窓外れの読み直しを経た場合も含め、窓の内側で反復を完了したとき、`h > 0`なら`r ← (h + 1)/2 + 2`とする。
完了した反復は、読んだ深さによらず深さ`d`の反復として記録する。

### 係数

SPSAで調整する3係数を次に示す。
評価値の単位を持つ係数は、歩兵価値に対する百分率で表す。

| 係数 | 開始値 | 範囲 | Stockfish | YaneuraOu | 現行 |
|---|---|---|---|---|---|
| `AspirationScoreWidth`（新規。根の値が歩兵価値の10倍のときに加わる半幅） | 204 | 0〜400 | 約204（`src/search.cpp:389`の`/10193`） | 約100（`yaneuraou-search.cpp:1670`の`/9000`） | 0（項がない） |
| `AspirationDelta`（既存。初期の半幅の定数項） | 2 | 1〜200 | 約2.4（定数項5、`src/search.cpp:389`） | 約5.6（定数項5、`yaneuraou-search.cpp:1670`） | 55 |
| `AspirationGrowth`（既存。拡大率、評価値の単位を持たない百分率） | 137 | 125〜400 | 約137（`delta += 47 * delta / 128`、`src/search.cpp:455`） | 約133（`delta += delta / 3`、`yaneuraou-search.cpp:1795`） | 190 |

`AspirationDelta`の範囲は、起点の10〜200の下限を1へ広げ、3つの値を含める。
`AspirationScoreWidth`の上限400は、Stockfishの換算値の約2倍である。
`AspirationGrowth`の範囲は起点のままで、3つの値を含む。

### 固定する定数

次の定数はStockfishの値で固定し、SPSAの対象にしない。

| 定数 | 値 | Stockfishの箇所 |
|---|---|---|
| 平均の重みの尺度`Scale` | 32 | `src/search.cpp:1480` |
| 重みの式で`E_prev`に掛ける比3/2の分子`ChiNumerator`と分母`ChiDenominator` | 3と2 | `src/search.cpp:1481-1482` |
| 平均の重みの下限`MinWeight`と上限`MaxWeight` | 12と24 | `src/search.cpp:1483-1484` |
| 2乗平均の重みの上限 | 16 | `src/search.cpp:1489` |
| fail high 1回あたりの減深 | 1手 | `src/search.cpp:410, 450` |
| 反復の開始時に持ち越しから引く量 | 2 | `src/search.cpp:405` |
| 完了時のfail highの回数が1以上だった反復の後の持ち越し | `(回数 + 1)/2 + 2` | `src/search.cpp:462` |
| 読み直しの深さの下限 | 1 | `src/search.cpp:410` |
| ワーカー番号の項の周期と1番号あたりの量 | 8、Stockfishの1単位（歩兵価値の1/208） | `src/search.cpp:389` |

## 所要時間の見積り

時間は、同時対局数16の測定機での見積りである。
SPSAは、同時対局数16で3,000ペアを完走した実績（[SPSAの手引き](../guides/spsa.md)の27,138秒、1ペア約9秒）に基づき、30,000ペアを約75時間（約3日）と見込む。
2026年10月17日の実行では1反復（8ペア）あたり約1分で進んだが、これは途中までの暫定の観測である。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので、[枝刈りとLMRの関数形の設計書](search-pruning-forms.md)と同じ幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 形の変更の実装と検証（codexへ委任） | 半日程度 |
| 2 | 時間切れの事前確認（13反復、104ペア） | 約15分 |
| 3 | SPSA（30,000ペア） | 約75時間 |
| 4 | 反映、テスト、および事前対局（32ペア） | 約1時間 |
| 5 | 候補のSTC | 約1〜8時間 |
| 6 | 通過した場合のLTC | 約5〜30時間 |

測定機の占有は段階2から6の和であり、STCで棄却される場合の約77時間（0.25＋75＋1＋1）から、LTCまで進む場合の約114時間（0.25＋75＋1＋8＋30）までとなる。

## 実装フェーズ

基本シードは、時間切れの事前確認を4700000000、SPSAのセッションを4750000000、事前対局を4800000000、STCを4850000000、LTCを4900000000と仮に定める。
これらは起案時点で、連鎖の他の設計書が予約した値と重ならないように割り当てた値である。
着手時に、保存済みの測定とSPSAの基本シード（`data/matches/`と`data/spsa/`の`manifest.json`）から各測定のペア数以上離れていることを確かめ（[教訓](../lessons/derive-seed-adjacent-collision.md)）、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　形の変更の実装

起点からブランチ`aspiration-window-forms`を切り、worktree `data/worktrees/aspiration-window-forms`で作業する。
M1、M2、A1とA3、A2、A4、A5の順に、1項目を1コミットにする。
挙動を変えない移行のM1とM2を先に置き、その後の各項目による探索の変化を分けて記録する。
M1は、A2で初期の半幅が小さくなる前に入れ、A4で外れなかった側の端を寄せる時点で停止の保証が成り立っているようにする。
A1とA3は、中心を平均に変えると深さ5で窓を使い始める理由がなくなるので、1つのコミットにする。
M1とM2のコミットでは、`minase dev bench`の全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
A1からA5の各コミットでは、同じbenchの探索ノード数、評価値、および最善手を記録し、直前のコミットとの差を残す。
M2は根で手を読むたびに記録を更新するので、`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点とM2のコミットで交互に3回ずつ測り、探索速度の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
新しい係数は`params.rs`の表へ追加し、既存の2係数の既定値と範囲を表の値に書き換え、調整用ビルド（`--features tuning`つきのビルド）が`Tune_<係数名>`のUSIオプションとして宣言することを確かめる。
新しい式の単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
M2の重み、窓の移し方、減深と持ち越しの推移、初期の半幅の各項、および帯の中心で全窓にすることを、数値の例で確かめる。
停止の例には、半幅1、倍率125%で、返った値0が窓の端に等しいためにfail lowとfail highが交互に起きる場合を含め、半幅が1のまま止まらずに広がり、有限回で窓の内側の結果が得られることを確かめる。
`crates/minase/src/search/alphabeta/tests/root.rs`の既存のテストは、深さ5以上で窓を使う挙動と外れた側だけを広げる挙動を前提にしているので、新しい形の数値の例に書き直す。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過、M1とM2のノード数の一致、A1からA5のノード数の記録、および探索速度の記録である。

### フェーズ2　パラメーターファイルと時間切れの事前確認

形の変更の最後のコミットでビルドした`minase`を`data/spsa/aspiration-window-forms-runner/`へ複製してrunner（測定を実行するプログラム）として固定し、以後のフェーズはこのrunnerで実行する。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除したファイルを`data/spsa/aspiration-window-forms.params`とする。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、[枝刈りとLMRの関数形の設計書](search-pruning-forms.md)と同じく使い捨てのスクリプトで機械的に突き合わせる。

続いて、手引きの第3節に従い、時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
パラメーターファイルの写しの開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150へ書き換えた`data/spsa/aspiration-window-forms-safety.params`で、13反復（104ペア）を実行する。

```console
<固定したminase> spsa \
  --run-dir data/spsa/aspiration-window-forms-safety --seed 4700000000 \
  --engine commit:<形の変更> --params data/spsa/aspiration-window-forms-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`aspiration-window-forms-safety`に残す。
完了条件は、時間切れとエンジン異常が0件であることと、測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<固定したminase> spsa \
  --run-dir data/spsa/aspiration-window-forms --seed 4750000000 \
  --engine commit:<形の変更> --params data/spsa/aspiration-window-forms.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 3750 --pairs-per-iteration 8
```

最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`aspiration-window-forms`に、手引きの第6節の項目とともに残す。
最終値を丸めた整数が範囲の上限または下限になった係数を、測定記録に示す。
本書の3係数については、最終値が開始値からどれだけ離れたかを、`c_end`に対する比で示す。
この比は、Stockfishの換算値からどれだけ動いたかを読むための目安であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、テストの前提が新しい既定値でも成り立つようにテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で握りつぶさない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

採否測定の前に、候補が時間切れやエンジン異常を起こさないことと、時間の使い方の変化を確かめるため、STCの条件で候補と起点の32ペアの事前対局を行う。

```console
<固定したminase> match run \
  --run-dir data/matches/aspiration-window-forms-smoke --seed 4800000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 elo --pairs 32
```

候補と起点の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ（反復深化で読み終えた深さ）、および探索を止めた理由の分布を、[枝刈りとLMRの関数形の設計書](search-pruning-forms.md)の事前対局と同じ方法で集計して、測定記録`aspiration-window-forms-smoke`に残す。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、および測定記録の保存である。

### フェーズ5　採否測定

```console
<固定したminase> match run \
  --run-dir data/matches/aspiration-window-forms-stc --seed 4850000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

STCの判定は[SPRTの手引き](../guides/sprt.md)の振分け規則に従う。
候補が有意に強い`H1`ならLTCへ進み、改善を示せない`H0`なら不採用とし、3,000ペアで判定保留ならその時点の対数尤度比の符号でLTCへ進むかを決める。
LTCは測定名の接尾辞を`-ltc`、基本シードを4900000000、持ち時間を`time=60000+200`とし、`--max-pairs`を付けない。
LTCで`H1`となり、エンジン異常、時間切れ、および拒否着手が0件なら採用し、`H0`なら不採用とする。
LTCの判定保留は、手引きのとおり`--resume`と`--target-pairs`で延長する。
結果は測定名ごとの測定記録に残す。
完了条件は、採否の確定と測定記録の保存である。

## 検証

- M1とM2の各コミットで、`minase dev bench`の全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。
- A1からA5の各コミットで、同じbenchの探索ノード数、評価値、および最善手が記録されていること。
- M2の重み、窓の移し方、減深と持ち越し、初期の半幅、帯の中心の扱い、および読み直しの停止を、数値の例の単体テストで確かめていること。
- 秒読みつき時計の途中結果の採用（`RootResults::partial_result`）と、先読み中の読み直しが反復の開始時刻と安定性を変えないこと（`tests/ponder.rs`の`ponder_aspiration_research_preserves_iteration_start_and_stability`）の既存のテストが、変更なしで通ること。
- 新しい係数と書き換えた2係数が調整用ビルドの`Tune_`オプションとして宣言され、パラメーターファイルの開始値と範囲が表と一致すること。
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
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。減深の回数と持ち越しの宣言は335〜336行、初期の窓は389〜392行、減深は403〜411行と461〜462行、窓外れの処理は437〜455行、完了した反復の深さの記録は534行、`increaseDepth`は634行、根の手の平均の更新は1466〜1501行。`src/search.h`の154〜158行（根の手の記録）、`src/thread.cpp`の306〜318行（根の手の表の作成）、`src/types.h`の199行（歩兵価値）。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。初期の窓は1670〜1673行。
- [aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)と、その根拠の[段階6のSTC](../measurements/strength-stage6-aspiration-stc.md)および[段階12のSTC](../measurements/strength-stage12-aspiration-stc.md)。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)と[SPRTによる棋力測定の手引き](../guides/sprt.md)。
