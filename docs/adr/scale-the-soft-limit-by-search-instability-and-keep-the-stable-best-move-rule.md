# 秒読みのない時計では、探索の開始時に求めた基礎予算の目安時間softに、探索の不安定さの4因子の積を完了反復ごとに掛けて伸縮させ、最善手が安定したときの早期終了と短い詰みでの停止を併用する

## ステータス

承認済み。決定日は2026年10月23日。

## コンテキスト

minaseの時間管理は、探索の開始時に、1手の目安時間softと打ち切りの上限hardの組（以下、基礎予算）を求める。
以下、Lazy SMPの0番のワーカーを主ワーカー、反復深化の反復のうちaspiration windowsの窓の内側で読み終えたものを完了反復と呼ぶ。
主ワーカーは、完了反復のたびに、softと次の反復の予測完了時刻から次の反復を始めるかを決める。
hardは、すべてのワーカーが探索中の時計の検査で照合する。
次の反復の予測完了時刻は、探索の開始からの累積経過時間に固定比`IterationRatio`を掛けて求める（[予測のADR](predict-next-iteration-by-fixed-ratio.md)）。
秒読みが正の時計で持ち時間を使い切った後の手（以下、秒読み期）では、主ワーカーは予測と最善手の安定性を使わず、hardに当たる締切まで次の反復を始める。

[旧ADR](archive/stop-early-on-stable-best-move-without-extensions.md)は、2026年9月12日に、[棋力向上段階6](../plans/archived/strength-stage6.md)の測定に基づいて、次の反復を始めるかの判定に延長の信号を使わず、最善手が安定したときだけ早く止めると決めた。
窓が評価値の低下で外れることをfail-lowという。
fail-lowを記録してsoftの条件を外す延長は、STCが292有効ペアでLLR −2.945の`H0`だった（[測定記録](../measurements/strength-stage6/strength-stage6-faillow-stc.md)）。
完了反復の最善手が直前の反復と異なるときに、直後の判定1回だけsoftの条件を外す延長は、STCが700有効ペアでLLR −2.960の`H0`だった（[測定記録](../measurements/strength-stage6/strength-stage6-bmchange-stc.md)）。
直近4反復の最善手が同じときに、softの条件を「経過時間がsoft未満」から「予測完了時刻がsoft以下」へ置き換える早期終了は、STCが275有効ペアでLLR +2.988、LTCが574有効ペアでLLR +2.957の`H1`だった（[STC](../measurements/strength-stage6/strength-stage6-stable-stc.md)、[LTC](../measurements/strength-stage6/strength-stage6-stable-ltc.md)）。
以下、この早期終了を二値の短縮と呼ぶ。
反復数4は、中将棋での[反復深化の診断](../measurements/strength-stage6/strength-stage6-iteration-diag.md)で比べた3〜5から選んだ値である。
これらの測定は、2026年10月12日に既定の評価関数がNNUEへ置き換わる前に行った。
旧ADRは、延長を再び提案するなら別の信号として段階ゲートにかける必要があるとしていた。

Stockfishは、完了反復のたびに目安時間に4つの因子の積を掛け、上限時間で抑えた結果を経過時間が超えていれば止める（`src/search.cpp:592-613, 621-624`）。
falling evalは、評価値が前の探索の平均値と4反復前の値のそれぞれから下がった量による因子である。
best move stabilityは、最善手が最後に変わった深さからの距離による因子である。
best move instabilityは、最善手の変化回数の減衰つきの累計による因子である。
best move effortは、最善手の部分木が全ノード数に占める割合による因子である。
積は1より大きくも小さくもなるので、局面が不安定なら長く、安定なら短く読む。
Stockfishは、根の候補が1手なら積を掛けた時間を500 msで抑える（同`615-617`）。
また、最善手の値が3手以内に詰ます値以上か、2手で詰まされる値に等しければ、時間によらず止める（同`621-624`）。
falling evalとbest move stabilityは、前の探索の評価値、その手の平均値、および前の探索の最後の短縮率`timeReduction`を次の探索へ持ち越して使う（同`252-260, 644`）。

同じ4因子は、[持ち時間の効率的な使用](../plans/archived/time-management-efficiency.md)の第3段階で、定数をStockfishのまま固定し、積に正規化の定数0.48を掛けた形で一度実装された。
第3段階は単独では測られず、予算の再定義と途中結果の採用を積んだ最終候補としてSTCで`H0`となった（[予算の再定義を採らないADR](keep-discarding-hard-interrupted-iterations-and-reject-fair-share-budgets.md)）。
その事前の[ベンチ](../measurements/time-management-efficiency/time-management-efficiency-stage3-bench.md)では、Stockfishの定数のままではminaseの深さ6〜8の探索で積が系統的に大きくなり、best move stabilityの因子は240局面中180局面以上で上限に張り付いた。

[持ち時間の伸縮の設計書](../plans/time-instability-scaling.md)は、[探索の関数形の見直しの統括計画](../plans/search-forms.md)の最後のサブタスクとして、Stockfishの時間管理の項目をminaseへ入れた。
2026年10月17日のエンジンの開発者の決定により、統括計画の各サブタスクでは、StockfishとYaneuraOuで形や値が違えばStockfishに合わせ、Stockfishにだけある項目は入れ、秒読みのようにStockfishに相当する仕組みがない事項はYaneuraOuに合わせる。
同じ決定により、新しい係数の開始値はStockfishの値の換算値とし、SPSAでは各仕組みの強さを決める値だけを調整して、上下限と内部の定数はStockfishの値で固定する。
minaseにあってStockfishにない仕組みのうち、中将棋の診断や測定で決めたものは残してSPSAで調整し、二値の短縮はこれに当たる。
同じ日に、エンジンの開発者は、4因子を秒読みつきの時計でも使うこと、best move effortをStockfishの連続の形にすること、根の候補が1手の局面に上限を置くこと、前の探索の値を持ち越すこと、および詰みでの早期停止を入れることを決めた。
YaneuraOuも、秒読みつきの時計で4因子の積を目安時間に掛ける（`yaneuraou-search.cpp:1989-1996`）。
2026年10月18日のエンジンの開発者の決定により、統括計画のサブタスクはSTCの結果によらず採用する（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）ので、旧ADRが延長の再提案に求めた単独の段階ゲートは行わない。
一方、秒読みつきの時計はlishogiのBotと人間との対局で使う時計なので、2026年10月22日に、エンジンの開発者は設計書の案1を選んだ。案1は、SPSAで調整した候補を秒読みつきの短い条件と長い条件で起点と比べ、両方が`H1`なら秒読みつきの時計でも4因子などを使い、どちらかが`H1`でなければ秒読みが0の時計に限るというものである。
候補は秒読みつきの短い条件で`H1`とならなかった（[測定記録](../measurements/time-instability-scaling/time-instability-scaling-byoyomi-short.md)）。

4因子の積は1を超えてsoftを伸ばせるので、延長を使わないという旧ADRの決定は本決定で変わる。
段階6の2つの延長は、信号が立った境界でsoftの条件を外してhardの予測だけで次の反復を始めるかを決める二値の規則であり、伸ばす量は信号の大きさによらなかった。
4因子の積は、信号の大きさに応じてsoftを連続的に伸縮させ、縮める方向も持ち、その強さはSPSAで決まる。

## 決定

有限の探索に時計があり、その秒読みが0で、`movetime`の指定がない探索では、主ワーカーが完了反復ごとに4因子を千分率で求め、基礎予算のsoftに4因子の積を掛けた時間`t`を求め直す。
この対象には、深さやノード数の制限を時計と併用した探索を含め、秒読みが正の時計の探索は、持ち時間を使い切る前の手（以下、持ち時間期）も含めて対象にしない。
根の候補が1手なら`t`を500 msで抑え、`soft' = min(t, hard)`を、次の反復の開始判定と、二値の短縮で予測完了時刻と比べるsoftの値に使う。
hardは変えず、最初の反復を完了するまでは基礎予算のsoftを使う。
先読みから続く探索での`soft'`の使い方は、[先読みのADR](measure-budget-from-ponderhit-and-add-a-quarter-after-pondering.md)が定める。
4因子の式の形はStockfishに合わせ、切片、傾き、および倍率の11係数をSPSAで調整し、上下限と内部の定数はStockfishの値で固定する。
設計書の判断により、負になり得る除算は0の方向へ丸め、積は桁あふれしたら飽和させてから500 msの上限とhardを順に適用する。

二値の短縮は残し、比べる直近の完了反復の数を定数から係数`StableIterations`（開始値4）に改めて、4因子の係数とともにSPSAで調整する。
この係数は、時計と`movetime`を併用した探索を含め、二値の短縮を使うすべての判定に働く。

4因子を使う探索で、時間で止めるかを判定する完了反復の境界では、最善手の値が`MATE − 3`以上か`−(MATE − 2)`に等しければ、経過時間によらず止める。
手数は双方の着手をそれぞれ1手と数え、停止の理由は目安時間による停止と同じ`StopReason::SoftLimit`とする。

前の探索の評価値とその手の平均値、および主ワーカーの最後の短縮率`r`（best move stabilityの`timeReduction`に当たる値）は、対局内で次の探索へ持ち越す表をまとめた構造体`HistoryTables`に置く。
評価値と平均値は採用したワーカーの返した手から取り、どのワーカーも深さ1を完了しなかった探索の後は「なし」にする。
前の探索の値が「なし」なら、その探索の間はfalling evalの因子を上限の1728‰とする。
`r`は主ワーカーから取り、短縮率を一度も求めなかった探索の後は1000‰、`HistoryTables`の初期化の後はStockfishと同じ850‰とする。
秒読みつきの時計で途中結果を採った探索では、途中結果の評価値とその手の平均値を記録する。
先読みの探索、深さやノード数の制限の探索、`movetime`の探索、および予想が外れて`stop`で止めた先読みの探索も、同じ規則で記録する。

## 検討した代案

延長を使わず二値の短縮だけで止める旧ADRの形は、Stockfishにある項目を入れるというエンジンの開発者の決定により採らなかった。
旧ADRが根拠にした段階6の`H0`は、NNUEより前の評価関数で、信号の大きさによらない二値の延長を測った結果である。
本決定はその結果を新しい測定で覆したものではなく、4因子それぞれの棋力への寄与は測っていない。

二値の短縮を外し、最善手の安定をStockfishのbest move stabilityの因子だけで扱う案は採らなかった。
持ち時間の効率的な使用の第3段階は、二値の短縮を4因子の積で置き換えていた。
二値の短縮は中将棋の診断から反復数を選んでSTCとLTCで`H1`となった規則であり、エンジンの開発者の決定によりStockfishの仕組みと併用する。
二値の短縮は、softとの比べ方を経過時間から予測完了時刻へ切り替える規則なので、softに掛ける倍率では表せない。

前の反復の伸縮後のsoftに、さらに次の反復の因子を掛ける案は採らなかった。
Stockfishは完了反復のたびに、探索の開始時に決めた目安時間に改めて積を掛ける（`src/search.cpp:612-613`）。
伸縮後のsoftに重ねて掛けると、前の反復の因子が後の反復のsoftに残り続ける。

第3段階と同じく、Stockfishの定数を固定して積に正規化の定数を掛ける案は採らなかった。
積の水準は、best move stabilityの分母の倍率`TimeReductionScale`や各因子の切片をSPSAが動かして決められるので、正規化の定数は加えていない。

Stockfishの`interpolate`（2点を通る直線、`src/misc.h:486-489`）の2点の座標4つを係数にする案は採らなかった。
同じ直線を表す座標の組は無数にあるので、SPSAが直線を変えない方向にも係数を動かしてしまう。
直線は切片と傾きの2係数で表し、呼び出し側の上下限は固定した。

best move effortをYaneuraOuの階段状の形にし、深さ10以上の条件（`yaneuraou-search.cpp:1987`）を付ける案は、Stockfishの連続の形を使うというエンジンの開発者の決定により採らなかった。

Stockfishと同じく、前の探索の値の初期値を`VALUE_INFINITE`（有限値32001、`src/types.h:155`）とする案は、設計書の判断により採らなかった。
Stockfishでは、この初期値のために対局の最初の探索のfalling evalがほぼ常に上限に張り付くが、評価値が32001に近い局面では4反復前の値との差の項が負に大きくなり、下限になり得る。
minaseでは、前の探索の値が「なし」なら因子を上限とすることを、欠測時の規則として明示した。

4因子、根の候補が1手の局面の上限、詰みでの停止、[深さを増やさない規則](reduce-aspiration-depth-after-fail-highs-and-late-iterations.md)、および[先読みの後の増分](measure-budget-from-ponderhit-and-add-a-quarter-after-pondering.md)を、秒読みつきの時計の持ち時間期にも使う案は、測ったうえで採らなかった。
2026年10月17日のエンジンの開発者の決定はこの案であり、設計書はSPSAまでをこの形で進めた。
SPSAの最終値を反映した候補は、秒読みつきの短い条件（持ち時間6秒、秒読み0.3秒）のGSPRTで、起点に対して1,634有効ペアでLLR −2.991の`H0`となり、平均得点から求めた差は約−4 Elo（95%信頼区間はおよそ−15〜+7）だった。
この測定では候補に時間切れが1件出ており、そのペアを除いたLLRは−2.922で`H0`の境界に届かないが、`H1`の境界からは大きく離れている。
案1の規則では、短い条件か長い条件のどちらかで`H1`とならなければ適用を限定するので、長い条件は測らずに、適用を秒読みが0の時計に限った。限定した版は測り直していない。

前の探索の値を持ち越すために公開APIかプロトコル層に新しい状態を加える案は、設計書の判断により採らなかった。
`HistoryTables`は、探索を始める関数`start_search`へ所有権ごと渡され、`SearchHandle::join`で返る。
プロトコル層は、先読みの探索の後も含めて返った値を保持し、新規対局と、探索の表を作り直す設定の変更で初期化している。
値を`HistoryTables`に置けば、先読みの探索の結果で更新し、対局の開始で消すという要件を満たし、公開APIもプロトコル層も変えずに済む。
`HistoryTables`の表の寿命と共有範囲は[対局内で表を持ち越すADR](carry-histories-and-correction-histories-within-a-game.md)が定めるが、承認済みのADRは本文を書き換えないので、同じ構造体に値を加える決定は本ADRに書いた。
Stockfishも、新規対局のほか、スレッドの構成を作り直す処理で同じ値を初期化する（`src/thread.cpp:251, 270-275`）。

評価値と平均値を常に主ワーカーから取る案は、設計書の判断により採らなかった。
Stockfishは、採用したスレッドの最善手の値と平均値を記録する（`src/search.cpp:252-260`）。
短縮率を求めるのは主ワーカーだけなので、`r`は主ワーカーから取る。

最善手の変化の回数をStockfishと同じくワーカーごとのカウンタで数える案（`src/search.cpp:583-587`）は採らなかった。
因子が使うのは全ワーカーの合計だけなので、探索チームが共有する原子的なカウンタ1個に各ワーカーが加え、主ワーカーが完了反復のたびに1回の不可分な交換で回収する。

積が桁あふれしないように因子に上限を加える案は、設計書の判断により採らなかった。
best move instabilityの因子にはStockfishと同じく上限がないので、積は桁あふれし得る。
積を符号なし128ビット整数で飽和させれば、飽和した値は`soft'`ではhardで抑えられ、[深さを増やさない規則](reduce-aspiration-depth-after-fail-highs-and-late-iterations.md)の判定では経過時間の2倍より大きいので、結果は桁あふれしない計算と同じになる。
0の方向への丸めはRustの整数除算のままであり、falling evalの2つの項は足してから1回だけ歩兵価値で割る。

## 結果

4因子と`t`の計算は`crates/minase/src/search/alphabeta/time.rs`の`InstabilityScaling`、因子ごとの関数、`scaled_soft_millis`、および`scaled_time_budget`に、詰みでの停止の条件は同じファイルの`mate_early_stop`に、二値の短縮は`stable_signal`にある。
4因子などを使う探索かの判定は、`team.rs`の`uses_instability_scaling`の1か所にまとめ、根の候補が1手の局面の上限、詰みでの停止、深さを増やさない規則、および先読みの後の増分も同じ判定に従う。
反復の境界での判定の順序は`deepening.rs`の`run_main_worker`に、前の探索の値の記録は`team.rs`の`record_previous_search`と`history.rs`の`HistoryTables`のdocコメントにある。
新しい12係数（4因子の11係数と`StableIterations`）の開始値、範囲、およびStockfishとYaneuraOuの換算値は`params.rs`にあり、固定した上下限は`time.rs`の名前つきの定数である。

前の探索の値は`HistoryTables`の初期化で消えるので、新規対局だけでなく、規則の変更、置換表容量の指定、引き分けの評価値の変更、およびワーカー数の変更でも消える。
`movetime`の探索のように短縮率を求めない探索の後は`r`が1000‰になり、Stockfishが探索の開始時に`timeReduction`を1にして時間管理を使わない探索の後もその値を持ち越すこと（`src/search.cpp:295, 644`）に合わせた。
Stockfishがbest move stabilityの距離と組にしている、読み直しで失った詰みの最善手を戻す処理（`src/search.cpp:526-560`）はminaseにないので、距離は完了反復の最善手だけで数える。
根の候補は、反復規則による勝ちの手への限定や負けの手の除外をした後の手の列なので、その数は盤面の合法手の数と一致するとは限らない。
中将棋では王手を放置する着手も王駒を相手の利きへ移す着手も合法なので（[RULES.md](../rules/RULES.md)第8条第3項から第5項）、根の候補が1手になるのは動ける駒が少ない局面か反復規則で候補が絞られた局面に限られ、500 msの上限の発動の頻度は測っていない。
500 msの上限は完了反復の境界でだけ働くので、着手が500 ms以内に返ることは保証しない。
結果を選ぶ規則は完了深さを優先するので、主ワーカーが詰みの条件で止まっても、より深い補助ワーカーの結果が採られることがあり、返す手が詰みであるとは限らない。

固定深さの`minase dev bench`は時間予算を使わないので、形の変更の全13コミットで全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致した（[benchの記録](../measurements/time-instability-scaling/time-instability-scaling-bench.md)）。
1つのコアに固定したNPSは0.15%下がったが、この差は測定の揺れと区別できなかった。
開始値で行った16ペアの[事前対局](../measurements/time-instability-scaling/time-instability-scaling-start.md)では、時間切れとエンジン異常は0件であり、1局の総思考時間は起点の0.992倍だった。
同じ事前対局では、1手の思考時間の中央値はすべての手数帯で起点より短く、完了深さは7つの手数帯のうち5つで起点を下回り、探索途中のhardによる中断は起点の10手に対して23手だった。
起点の時計の再構成（[記録](../measurements/time-instability-scaling/time-instability-scaling-baseline.md)）では、起点のSTCの99.58%の手が反復の境界の判定で止まっており、softを伸縮させる因子はほぼすべての手の思考時間を変え得る。

4因子、二値の短縮の反復数、詰みでの停止、および設計書のほかの項目は1つの候補とし、既存の係数とまとめて103係数を30,000ペアのSPSAで調整した（[SPSAの記録](../measurements/time-instability-scaling/time-instability-scaling.md)）。
評価値の下落に反応する`TimeFallingPrevious`は4,784から2,141へ、`TimeFallingIteration`は2,288から1,351へ下がった。最善手の変化回数に反応する`TimeInstabilitySlope`は2,229から803へ下がり、切片の`TimeInstabilityBase`は1,077から1,255へ上がった。二値の短縮の反復数`StableIterations`は4から5へ上がった。
SPSAの最終値を反映した候補の32ペアの[事前対局](../measurements/time-instability-scaling/time-instability-scaling-smoke.md)では、時間切れとエンジン異常は0件、1局の総思考時間は起点の0.965倍であり、完了深さは7つの手数帯のすべてで起点より深かった。
同じ候補は、[STC](../measurements/time-instability-scaling/time-instability-scaling-stc.md)で起点に対して303ペア（有効297）でLLR +2.945の`H1`となり、平均得点から求めた差は約+51 Elo（95%信頼区間はおよそ+27〜+75）だった。
候補は時間管理の形に加えて探索の係数もSPSAで調整し直しているので、この差は4因子などの効果だけを表すものではない。
秒読みが正の時計では4因子などを使わないので、秒読みつきの時計の時間管理は、二値の短縮の反復数とSPSAで調整し直した既存の係数を除いて起点と同じである。
