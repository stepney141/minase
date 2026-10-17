# aspiration windowsの幅と窓の移し方の見直しの設計書

## 先に読む要約

minaseの探索は反復深化、つまり深さ1から1手ずつ深くして探索を繰り返す方式で進み、各反復は探索木の根である手番側の現局面から読む。
本書は、各反復で根を狭い窓から読み始めるaspiration windows（前の反復の値の周りに窓を置き、窓を外れたら広げて同じ深さを読み直す手法）の形を、チェスのStockfishと将棋のYaneuraOu（やねうら王）が共通に持つ形へ寄せ、現行のmasterより強くなるかを確かめる。
窓とは、根の探索で正確な値を求める評価値の範囲αからβであり、狭いほど探索は速いが、値が範囲を外れると読み直しになる。
現行のminaseは、深さ5以上で、直前の反復の値を中心に歩兵価値（探索が余裕値の単位に使う歩兵1枚の駒価値）の一定の割合の幅で窓を置き、外れた側の端だけを元の端から広げる（`crates/minase/src/search/alphabeta/root.rs`）。
両エンジンは、窓の幅を評価値の大きさとともに広げ、中心を根の値の移動平均に置き、窓を全深さで使い、外れたときは返った値を基準に窓を移して反対側の端も寄せる（[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)）。
本書が変えるのは5つである。
1つ目に、初期の窓の半幅（中心から端までの距離）に、根の値の符号つき2乗の平均の大きさに比例する項を加える。
2つ目に、窓の中心を、直前の反復の値から、完了した反復の値の指数移動平均へ寄せる。
3つ目に、窓を使い始める深さを係数にする。
4つ目に、窓を外れたとき、外れた側の新しい端を、元の端だけでなく返った値の側へも移す。
5つ目に、窓を外れたとき、外れなかった側の端を外れた側へ寄せる。
5つの係数はすべて現行と同じ探索になる値を開始値とし、既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整する。
調整した版を、起点のmasterと短時間条件STCおよび長時間条件LTCの2段階の逐次検定（段階ゲート）で対局させ、1つの候補として採否を決める。
完了条件は、候補の採否が測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案した。
[連鎖の1つ前の設計書（静止探索の関数形の見直し）](qsearch-forms.md)（以下、前計画）の完了を待って着手し、着手は利用者の指示による。
次の一手は、前計画の完了後に起点を確定し、フェーズ1（形の変更の実装）を始めることである。

## 目的

本書が確かめるのは、「aspiration windowsの幅、中心、使い始める深さ、および外れたときの窓の移し方を両エンジンの形へ寄せ、新しい形の係数を調整すれば、現行の形で調整した探索より強くなる」という仮説である。
現行の形は、[aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)が2026年9月に決めたものであり、当時の既定の評価関数はPST（駒の種類と升ごとの値の表による評価）だった。
2026年10月12日に既定の評価関数がNNUE（差分更新できる小型のニューラルネットワーク）へ置き換わった後、窓の形は測り直していない。
窓の幅は`AspirationDelta`と`AspirationGrowth`の2係数でSPSAの対象になっているが、現行の形では、評価値の大きさによる幅の違い、反復ごとの値の揺れを均した中心、および大きく外れたときの窓の移し方を、係数の値で表せない。
形を変えて係数を調整し直せば、これらをSPSAが選べるようになる。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、前計画が完了した後、フェーズ1を始める時点のmasterの先頭のコミットである。
- **形の変更**は、本書の5項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で表（`crates/minase/src/search/alphabeta/params.rs`）の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **ワーカー**は、Lazy SMP（複数のスレッドが置換表を共有して同じ局面を探索する並列化）の各探索スレッドである。採否の測定は`Threads=1`で行うので、測定で動くのは主ワーカーだけである。
- **半幅**`δ`は、窓の中心から各端までの距離であり、窓外れのたびに端を動かす量にも使う（現行の`AspirationWindow`の`delta`）。**窓の幅**は`β − α`をいう。
- **窓外れ**は、根の探索の返り値がα以下になること（fail low）か、β以上になること（fail high）をいう。等号も窓外れとする現行の扱いは変えない。
- **返った値**は、窓外れのときに根の探索が返した値である。minaseの探索はfail-soft（窓の外の値を窓の端に丸めずに返す方式）であり、根の`search_root`は窓の端とは別に最大値を追い、その値を返す。fail lowの返った値はα以下の上界、fail highの返った値はβ以上の下界である。
- **詰み帯と専用の帯**は、詰みまでの手数を表す評価値の範囲と、千日手などの反復の結果を表す評価値の範囲（`crates/minase/src/search/mod.rs`の`MATE_THRESHOLD`と`REPETITION_BAND_START`）である。通常の評価値はこれらの外にある。

対象は次の5項目である。

- 初期の半幅への評価値の大きさの項（A1）。
- 窓の中心の指数移動平均（A2）。
- 窓を使い始める深さの係数化（A3）。
- 窓外れのときに返った値の側へ窓を移す項（A4）。
- 窓外れのときに外れなかった側の端を寄せる項（A5）と、読み直しが必ず止まるようにする半幅の下限と拡大の丸めの変更。

次の事項は対象外とする。

- fail highの回数に応じて読み直しを浅くする方式（Stockfish `src/search.cpp:410-411, 462`）。[棋力向上段階12](archived/strength-stage12.md)がPSTの時期にこの方式を測り、STCで改善を示せない判定`H0`だった（[測定記録](../measurements/strength-stage12-aspiration-stc.md)）。[aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)は、再び提案するなら新しい構成で別の主張として段階ゲートにかけることを求めている。本書は複数の項目を1つの候補に束ねるので、この方式を加えると、不採用のときにこの方式が負の寄与をしたかを切り分けられず、ADRの求める別の主張にもならない。
- ワーカー番号で初期の半幅をずらす項（両エンジンの`threadIdx % 8`）。採否の測定とSPSAは`Threads=1`で行い（[Threadsの既定値のADR](../adr/keep-default-threads-at-one.md)）、主ワーカーの番号は0なので、この項は測定で一度も働かず、係数を調整できない。補助ワーカーの多様化は、[深さを飛ばすADR](../adr/diversify-lazy-smp-workers-by-skipping-depths.md)の方式が受け持つ。
- 根の手ごとに平均を持つこと。両エンジンは根の各手に平均を持ち、窓の中心には直前の反復の最善手の平均を使う（Stockfish `src/search.cpp:1480-1501`、YaneuraOu `yaneuraou-search.cpp:4105-4110`）。本書は、ワーカーごとに完了した反復の値の平均を1つだけ持つ。理由は設計判断の表に書く。
- 評価値の平均から評価関数の楽観度（optimism）を調整する項（Stockfish `src/search.cpp:397-399`）。評価関数の側の変更であり、YaneuraOuも無効にしている（`yaneuraou-search.cpp:1675-1681`の`#if 0`）。
- 窓外れや反復の読み直しの回数による時間管理（Stockfish `src/search.cpp:369-370`の`searchAgainCounter`、fail lowによる時間の延長）。fail lowによる延長は[時間管理のADR](../adr/stop-early-on-stable-best-move-without-extensions.md)がSTCの`H0`で棄却しており、時間配分の形は[連鎖の次の設計書](time-instability-scaling.md)が扱う。
- 窓外れの途中経過をUSIの`lowerbound`または`upperbound`の行で出力すること。[aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)が、対局ハーネスの記録を未完了の反復の値にするとして棄却している。
- 窓の拡大率の形。外れるたびに半幅に定数を掛ける形は両エンジンと同じであり、係数`AspirationGrowth`の範囲125〜400%は、Stockfishの約137%（`delta += 47 * delta / 128`、`src/search.cpp:455`）とYaneuraOuの約133%（`delta += delta / 3`、`yaneuraou-search.cpp:1795`）を含む。既存の係数としてSPSAで調整する。

## 依存関係

起点は、[連鎖の1つ前の設計書](qsearch-forms.md)が完了した後のmasterの先頭である。
前計画の候補が採用されたかどうかにかかわらず、起点の表の既定値をSPSAの開始値とする。
前計画の候補がmasterへ統合されると`params.rs`の既定値が変わり、本書の検証（起点との探索ノード数の一致）の基準も変わるので、実装を含めて前計画の完了後に始める。
探索の関数形を両エンジンの形へ寄せる設計書は、[枝刈りとLMRの関数形](search-pruning-forms.md)、浅い枝刈り、history、correction history、静止探索、本書、および時間管理の順の連鎖として起案している。
この順序は提案であり、最終的な順序は利用者が決める。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、同時対局数は16を上限とする。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は次のADRの決定の一部を変える。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確認し、新しいADRで置き換える。
本書が変えない決定（窓外れの途中経過を出力しないこと、fail highのときに浅く読み直さないこと、補助ワーカーが自分の直前の完了値を使うこと）も、新しいADRへ引き継ぐ。

- [aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)は2026年9月12日に、深さ5以上で直前の完了値を中心に窓を置き、外れた側の端だけを元の端から広げると決めた。窓の実装を採用した[STC](../measurements/strength-stage6-aspiration-stc.md)と[LTC](../measurements/strength-stage6-aspiration-ltc.md)は、PSTを既定の評価関数としていた時期の測定である。窓を使い始める深さ5は、当時のSTCの完了深さの最頻値が6だったことから選んだ値であり、測って選んだ値ではない。外れなかった側の端を中点へ寄せる案は、窓の意味を複雑にするという設計上の判断で棄却され、測られていない。返った値を基準に窓を移す形は、代案に含まれていない。fail highのときの減深だけはSTCの`H0`で棄却され、ADRはこれについてだけ再測定の条件を留保している。本書は、窓を使い始める深さを係数A3にし、中心をA2の平均へ寄せ、A4とA5で窓の移し方を係数にする。減深は本書でも扱わない。

[反復の結果の帯のADR](../adr/reserve-a-path-dependent-band-for-repetition-outcomes.md)が定めた窓の端の保護、つまり詰み帯または専用の帯に達した端を無限へ置き換える扱いは変えない。

## 設計判断

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 5項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。代償として、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| 新しい係数の開始値 | 5項目はいずれも、両エンジンの形が現行の形を係数の特別な値として含むので、その値を開始値とする（半幅の項0、平均の重み0、窓を使い始める深さ5、移す割合0、寄せる割合0）。現行と同じ挙動から始めれば、形の変更のコミットが探索を変えていないことを探索ノード数の一致で検証でき、最終値はSPSAが決める。同値にするためだけの係数や条件は加えていない。 |
| 新しい係数の範囲 | 両エンジンの対応する値を範囲に含める。`AspirationMinDepth`以外の4係数は、開始値0を範囲の下限に置く。負の値は、根の値とともに半幅を縮める形、平均の外挿、外れなかった側の端を広げる形、および外れた側の端を元の端より内側へ戻す形になり、どれも両エンジンにない。特に移す割合（A4）が負だと、返った値が窓から大きく離れたときに外れた側の端が広がらず、読み直しが進まない。開始値を内側に置くためだけにこれらの向きを範囲へ加えると、自由度と、それを守る下限などの装置が増えるので加えない。下限での摂動の切り詰めは[利得のADR](../adr/decay-spsa-gains-from-one-sixth-of-the-range.md)が許容しており、SPSAは正の側へ動かすことで効果を調べられる。 |
| SPSAの対象 | 起点の表の調整対象の係数（`FmScale`を除く。数は起点で確定する）に、新しい5係数を加える。新しい係数だけを調整する案は、既存の係数、特に`AspirationDelta`と`AspirationGrowth`との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の係数を含めるので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。375反復、1反復8ペア（計3,000ペア）、持ち時間10秒＋0.1秒、同時対局数16、`c_end`（最後の反復の摂動幅）は範囲の1/6、`r_end`（最後の反復の学習率）は0.002である（[SPSAの手引き](../guides/spsa.md)）。係数が増えても総ペア数は変えない。3,000ペアは候補を1つ作るための予算であり、最終値を最適値とは呼ばない。先行の設計書の候補をすべて採用した場合、調整する係数の数は本書の時点で90前後になる（起点の表で数え直す）。3,000ペアでこの数を十分に調整できるかは未検証であり、不採用は、この予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。 |
| 半幅の評価値の項（A1） | 初期の半幅に、根の値`v`について`v × |v|`の指数移動平均`q`の絶対値に比例する項を加える。両エンジンの`5 + |meanSquaredScore|/c`と同じ2乗の形である（Stockfish `src/search.cpp:389`は`c = 10193`、YaneuraOu `yaneuraou-search.cpp:1670`は`c = 9000`）。係数は、根の値の絶対値が歩兵価値の10倍のときに加わる半幅を、歩兵価値に対する百分率で表す。この単位では、Stockfishは約204%（歩兵価値208）、YaneuraOuは約100%（歩兵価値90）になる。中将棋の評価値は歩兵価値で測ると大きく、探索用駒価値では獅子が歩兵の約25倍なので、両エンジンの値をそのまま範囲の中心にはしない。 |
| 半幅の下限と拡大の丸め | 初期の半幅は1以上とし、拡大は`max(δ + 1, ⌊δ × AspirationGrowth/100⌋)`として、丸めた後も必ず1以上広がるようにする。現行の拡大`⌊δ × AspirationGrowth/100⌋`は整数除算なので、半幅が1で倍率が125〜199%なら半幅が広がらない。重みファイルの検査は歩兵価値に正であることしか求めないので、歩兵価値が10未満なら初期の半幅は0にもなり得る。A5で外れなかった側の端を寄せると、半幅が広がらない窓は2つの位置の間を往復し得るので、この丸めが停止の保証に要る。現行の埋め込みネットの歩兵価値は100であり、`AspirationDelta`の下限10%でも初期の半幅は10以上なので、倍率125%以上の拡大は常に2以上広げる。したがってこの変更は、現行の重みファイルとSPSAの範囲では探索を変えない。 |
| 窓の中心（A2、averageScore） | ワーカーごとに、完了した反復の値の指数移動平均`c`を持ち、窓の中心にする。前回までの平均に掛ける重みを係数`AspirationAverageWeight`とし、0のとき`c`は直前の完了値になって現行と同じになる。YaneuraOuは重み50%の平均を使う（`yaneuraou-search.cpp:4105-4106`）。Stockfishの重みは定数でなく、その手の今回の探索ノード数とそれまでの累計の比で決まり、前回までの平均に掛ける重みは25〜62.5%の範囲を動く（`src/search.cpp:1480-1495`）。A1の`q`も同じ重みで更新する。Stockfishは`q`の重みを別に上限で抑えるが（`src/search.cpp:1489`）、係数を分けると片方の係数が0の間はもう片方の勾配が測れないので、1つにまとめる。 |
| 平均に入れる値 | 平均に入れるのは、窓の内側で完了した反復の値だけとする。両エンジンは根の手を読むたびに、窓外れの値も含めて平均を更新する。窓外れの値は上界か下界にすぎず、[根の手の並べ替えのADR](../adr/order-root-moves-by-previous-subtree-nodes.md)も同じ理由で零窓の値を並べ替えに使っていない。平均を根の手ごとに持つ形は、最善手が交替したとき、新しい最善手の平均が主に零窓の上界から作られるので、同じ理由で採らない。平均はUSIの`go`コマンドごとに捨て、ワーカーごとに持つ。Stockfishの根の手の表も`go`ごとに作り直される（`src/thread.cpp:306-318`）。 |
| 帯の値の扱い | 完了した反復の値が詰み帯または専用の帯にあれば、平均`c`と`q`を捨て、次の反復は現行と同じく全窓で読む。次に帯の外の値で完了した反復から、平均を作り直す。重みは0以上100未満なので、平均は帯の外の値の重み付き平均であり、帯へ入らない。 |
| 窓を使い始める深さ（A3） | 係数`AspirationMinDepth`とし、開始値は現行の5とする。両エンジンは全深さで窓の式を使うが、深さ1では平均がまだないので、Stockfishでは窓が下端に寄り、最初の探索がfail highした後に返った値の周りへ移る（`src/search.cpp:389-392, 446-450`）。minaseは深さ1では直前の完了値がなく窓を使えないので、両エンジンに当たる値は2である。 |
| 返った値の側へ移す（A4、re-centering） | fail lowのとき、新しいαを「元のα − 半幅」から、返った値`s`の側へ`AspirationFailShift`%だけ移す。100%でStockfishとYaneuraOuの`alpha = s − delta`になる（`src/search.cpp:440`、`yaneuraou-search.cpp:1780`）。fail highも対称に扱う。fail-softの返った値はα以下（fail highならβ以上）なので、0以上の値では外れた側の新しい端は現行より外側に来る。返った値が窓から大きく離れたとき、現行の形では半幅が倍率で広がるまで何度も読み直すが、この項はその回数を減らす。 |
| 外れなかった側の端を寄せる（A5） | fail lowのとき、βを元の窓の幅の`AspirationOppositePull`%だけαの側へ寄せる。fail highも対称に扱う。100%でStockfishとYaneuraOuのfail lowの`beta = alpha`になる（`src/search.cpp:439`、`yaneuraou-search.cpp:1779`）。50%では、最初のfail highの`alpha = max(beta − delta, alpha)`と同じ位置になる。初期の窓の幅は半幅の2倍なので、`beta − delta`は窓の中点だからである。両エンジンはfail lowとfail highで寄せ方が違うが、係数を2つに分けると片側の窓外れが少ない局面で勾配が測りにくくなるので、1つの係数にする。 |
| 読み直しの停止の保証 | 外れなかった側の端が無限であれば寄せない。外れた側の端は毎回少なくとも現在の半幅だけ外へ動き、半幅は1以上で始まって窓外れのたびに少なくとも1増えるので、有限回の窓外れで専用の帯の下端（`REPETITION_BAND_START`）の2倍を越える。その後の窓外れでは、外れた側の端が必ず無限へ置き換わる。無限の端は寄せないので戻らず、無限のβでfail highすることも、無限のαでfail lowすることもないから、その後の窓外れは高々2回で両端が無限になり、読み直しは止まる。 |
| 途中結果の採用 | 秒読みつき時計で中断した窓の途中結果は、現行どおり、その窓の開始時のαを上回る値だけを採る（`root.rs`の`RootResults`）。A5は読み直す窓のαを上げることがあるが、採る値の基準が読み直す窓のαであることは変わらない。 |

## 変更の内容

以下の式で、`P`は探索用の歩兵価値（評価関数の重みファイルが持つ値）、`d`は反復の深さ、`v`は完了した反復の値、`s`は窓外れのときの返った値、`δ`は現在の半幅を表す。
整数の除算は、現行の`params.rs`と同じく0方向へ丸める。
`q`の計算は64ビット整数で行う。

### A1　初期の半幅（評価値の大きさの項）

初期の半幅を次の式にする。

```math
\delta_0 = \max\left( \frac{P \times \mathrm{AspirationDelta}}{100} + \frac{\mathrm{AspirationScoreWidth} \times |q|}{10000 \times P},\ 1 \right)
```

`q`は、A2の重みで更新した`v × |v|`の指数移動平均である。
根の値が`10P`で一定なら`q = 100P²`となり、第2項は`P × AspirationScoreWidth/100`、つまり歩兵価値の`AspirationScoreWidth`%になる。
`AspirationScoreWidth`が0のとき、初期の半幅が1以上になる現行の重みファイルでは現行と同じになる。

### A2　窓の中心（指数移動平均）

ワーカーは、USIの`go`コマンドごとに空の平均`c`と`q`を持って反復を始める。
窓の内側で反復を完了するたびに、その値`v`で次のように更新する。
`v`が詰み帯または専用の帯にあれば、`c`と`q`を空にする。
`c`が空なら`c = v`、`q = v × |v|`とする。
それ以外なら、`w = AspirationAverageWeight`として、`c ← (w × c + (100 − w) × v)/100`、`q ← (w × q + (100 − w) × v × |v|)/100`とする。
現行の`search_iteration`も窓の内側で完了した値だけを返し、それが次の反復の`prev`になるので、`AspirationAverageWeight`が0のとき、`c`は現行の`prev`と同じ値になる。

### A3　窓を使い始める深さ

深さ`d`の反復は、`d ≥ AspirationMinDepth`で`c`が空でないとき、`[c − δ₀, c + δ₀]`を初期の窓とする。
それ以外は全窓で読む。
詰み帯または専用の帯に達した端を無限へ置き換える現行の扱いは変えない。
`AspirationMinDepth`が5のとき、現行と同じになる。

### A4とA5　窓外れのときの窓の移し方

fail low（`s ≤ α`）のとき、元の窓の端`α`と`β`から、新しい端を次のように求める。

```math
\alpha' = \alpha - \delta - \frac{\mathrm{AspirationFailShift} \times (\alpha - s)}{100}, \qquad
\beta' = \beta - \frac{\mathrm{AspirationOppositePull} \times (\beta - \alpha)}{100}
```

fail high（`s ≥ β`）のとき、次のように求める。

```math
\beta' = \beta + \delta + \frac{\mathrm{AspirationFailShift} \times (s - \beta)}{100}, \qquad
\alpha' = \alpha + \frac{\mathrm{AspirationOppositePull} \times (\beta - \alpha)}{100}
```

αが無限ならfail lowは起きず、βが無限ならfail highは起きないので、外れた側の端は常に有限である。
式の`δ`は、この窓外れで拡大する前の半幅である。
外れなかった側の端が無限のときは、その端を動かさない。
新しい端を求めた後、詰み帯または専用の帯に達した端を無限へ置き換え、その後で半幅を次の式で広げる。
この順序は現行の`widen_low`と`widen_high`と同じである。

```math
\delta' = \max\left(\delta + 1,\ \left\lfloor \frac{\delta \times \mathrm{AspirationGrowth}}{100} \right\rfloor\right)
```

計算は現行と同じく64ビット整数で行い、32ビット整数の上限で飽和させる。
半幅が4以上なら、倍率125%以上の第2項は第1項以上になるので、第1項が効くのは半幅が3以下のときだけである。
`AspirationFailShift`と`AspirationOppositePull`が0のとき、現行と同じになる。

### 新しい係数

| 係数 | 開始値 | 範囲 | 範囲の根拠 |
|---|---|---|---|
| `AspirationScoreWidth`（根の値が歩兵価値の10倍のときに加わる半幅、歩兵価値に対する百分率） | 0 | 0〜400 | Stockfishは約204%、YaneuraOuは約100%。上限400は、根の値が歩兵価値の10倍のとき、現行の`AspirationDelta`55%の約7倍の半幅を加える |
| `AspirationAverageWeight`（前回までの平均に掛ける重み、百分率） | 0 | 0〜75 | YaneuraOuは50%、Stockfishは25〜62.5%。100%に近い重みは中心がほとんど動かなくなるので、上限を75とする |
| `AspirationMinDepth`（手） | 5 | 2〜10 | 両エンジンに当たる値は2（設計判断の表を参照）。上限10は暫定の値であり、最終値が上限に達した場合は測定記録に残す |
| `AspirationFailShift`（返った値の側へ移す割合、百分率） | 0 | 0〜150 | 両エンジンは100%。150%は返った値よりさらに外へ移す形である |
| `AspirationOppositePull`（外れなかった側の端を寄せる割合、百分率） | 0 | 0〜100 | 両エンジンはfail lowで100%、最初のfail highで50%に当たる。50%は[aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)が棄却した中点へ寄せる案である |

## 所要時間の見積り

時間は、同時対局数16の測定機での見積りである。
SPSAは、同時対局数16の完走実績（[SPSAの手引き](../guides/spsa.md)の27,138秒）に基づき約7.5時間と見込む。
2026年10月17日の実行では1反復（8ペア）あたり約1分で進んだが、これは途中までの暫定の観測である。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので、[枝刈りとLMRの関数形の設計書](search-pruning-forms.md)と同じ幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 形の変更の実装と検証（codexへ委任） | 数時間程度 |
| 2 | 時間切れの事前確認（13反復、104ペア） | 約15分 |
| 3 | SPSA（3,000ペア） | 約7.5時間 |
| 4 | 反映、テスト、および事前対局（32ペア） | 約1時間 |
| 5 | 候補のSTC | 約1〜8時間 |
| 6 | 通過した場合のLTC | 約5〜30時間 |

測定機の占有は段階2から6の和であり、STCで棄却される場合の約10時間（0.25＋7.5＋1＋1）から、LTCまで進む場合の約47時間（0.25＋7.5＋1＋8＋30）までとなる。

## 実装フェーズ

基本シードは、時間切れの事前確認を4700000000、SPSAのセッションを4750000000、事前対局を4800000000、STCを4850000000、LTCを4900000000と仮に定める。
これらは起案時点で、連鎖の他の設計書が予約した値と重ならないように割り当てた値である。
着手時に、保存済みの測定とSPSAの基本シード（`data/matches/`と`data/spsa/`の`manifest.json`）から各測定のペア数以上離れていることを確かめ（[教訓](../lessons/derive-seed-adjacent-collision.md)）、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　形の変更の実装

起点からブランチ`aspiration-window-forms`を切り、worktree `data/worktrees/aspiration-window-forms`で作業する。
A3、A2、A1、半幅の下限と拡大の丸め、A5、A4の順に、1項目を1コミットにする。
丸めの変更をA5より前に置くのは、外れなかった側の端を寄せる処理が入った時点で停止の保証が成り立っているようにするためである。
A2の平均の状態は、`search_iteration`が受け取る直前の完了値（現行の`prev`）に代えて、ワーカーの反復深化のループが持つ。
6つのコミットはすべて、開始値と現行の重みファイルで現行と同じ探索になるので、各コミットで`minase dev bench`の全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
窓は深さ5以上の反復で使われるので、深さ5と7のbenchが窓の経路を通る。
新しい処理は根で反復ごとに数回しか実行されないので、探索速度の測定は行わない。
新しい係数は`params.rs`の表へ追加し、調整用ビルド（`--features tuning`つきのビルド）が`Tune_<係数名>`のUSIオプションとして宣言することを確かめる。
新しい式の単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
係数が0以外のときの窓の端、半幅の下限、帯の値で平均を捨てること、外れなかった側の端が無限のときに寄せないこと、および読み直しが有限回で止まることを、数値の例で確かめる。
停止の例には、半幅1、倍率125%、`AspirationOppositePull`＝100%、返った値0の場合を含め、窓が`[0, 1]`と`[−1, 0]`の間を往復せずに広がることを確かめる。
`crates/minase/src/search/alphabeta/tests/root.rs`の既存のテストは、深さ5以上で窓を使う挙動と外れた側だけを広げる挙動を前提にしているので、係数の既定値で同じ挙動になることを確かめる形に直す。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過と、ノード数の一致である。

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
  --iterations 375 --pairs-per-iteration 8
```

最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`aspiration-window-forms`に、手引きの第6節の項目とともに残す。
最終値を丸めた整数が範囲の上限または下限になった係数を、測定記録に示す。
新しい5係数については、最終値が開始値（現行と同じ挙動）からどれだけ離れたかを、`c_end`に対する比で示す。
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

- 6つの各コミットで、`minase dev bench`の全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。
- 係数が0以外のときの窓の端、半幅の下限、帯の値の扱い、および読み直しの停止を、数値の例の単体テストで確かめていること。
- 秒読みつき時計の途中結果の採用（`RootResults::partial_result`）と、先読み中の読み直しが反復の開始時刻と安定性を変えないこと（`tests/ponder.rs`の`ponder_aspiration_research_preserves_iteration_start_and_stability`）の既存のテストが、変更なしで通ること。
- 新しい5係数が調整用ビルドの`Tune_`オプションとして宣言され、パラメーターファイルの開始値と範囲が表と一致すること。
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
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。初期の窓は389〜392行、窓外れの処理は436〜455行、fail highの減深は404〜411行と461〜462行、根の手の平均の更新は1474〜1501行。`src/thread.cpp`の306〜318行（根の手の表の作成）。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。初期の窓は1670〜1673行、窓外れの処理は1775〜1795行、根の手の平均の更新は4105〜4110行。
- [aspiration windowsのADR](../adr/widen-only-the-failed-side-of-aspiration-windows.md)と、その根拠の[段階6のSTC](../measurements/strength-stage6-aspiration-stc.md)および[段階12のSTC](../measurements/strength-stage12-aspiration-stc.md)。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)と[SPRTによる棋力測定の手引き](../guides/sprt.md)。
