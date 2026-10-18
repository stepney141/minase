# 探索の不安定さに応じた持ち時間の伸縮の設計書

## 先に読む要約

本書は、1手に使う目安の時間を探索の不安定さに応じて連続的に伸び縮みさせる時間管理（Stockfishの`totalTime`の計算に当たる、time management by search instability）を入れ、現行のmasterより強くなるかを確かめる。
minaseは、反復深化（深さ1から1ずつ読みを深める探索の外殻）の各反復を終えるたびに、目安の時間softと打ち切りの上限hardを見て次の反復を始めるかを決める。
現行でsoftを変える信号は、最善手が4反復続けて同じときに判定を厳しくする二値の短縮だけである。
Stockfishは、評価値の下落（falling eval）、最善手が最後に変わった深さからの距離（best move stability）、最善手の変化回数（best move instability）、および最善手の部分木が全ノードに占める割合（best move effort）の4因子をsoftに掛け、両方向に連続で伸縮させる。
本書はこの4因子をStockfishと同じ形で入れ、あわせて時間管理にあるStockfishの5つの項目（相手との残り時間の差による補正、合法手が1つの局面の上限、softの半分を過ぎたら深さを増やさない規則、詰みを見つけたときの早期停止、および先読み中の目安時間の25%増し）を加える。
二値の短縮は残し、その反復数も調整の対象にする。
新しい係数はStockfishの値を換算して始め、既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整する。
調整した版を、起点のmasterと短時間条件STCおよび長時間条件LTCの2段階の逐次検定（段階ゲート）で対局させ、1つの候補として採否を決める。
相手との残り時間の差の補正を除く項目は秒読みつきの時計（lishogiのBotが主に使う時計）の持ち時間期にも働かせるので、段階ゲートを通った候補を秒読みつきの2条件でも測る。
完了条件は、候補の採否と秒読みつきの時計での扱いが、測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案した。
連鎖の1つ前の[aspiration windowsの関数形の見直し](aspiration-window-forms.md)の完了と、利用者の着手の指示を待つ。
次の一手は、前の設計書の完了後に起点を確定し、フェーズ0（起点の時計の再構成）を始めることである。

## 目的

本書が確かめるのは、「探索の不安定さに応じてsoftをStockfishの形で連続的に伸縮させ、その強さをSPSAで調整すれば、現行の時間管理より強くなる」という仮説である。
発端は、[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)が、探索の不安定さによる伸縮を、minaseの時間管理が参照していない変数として挙げたことである。
StockfishとYaneuraOuは、反復を終えるたびに4因子の積をsoftに掛け、局面が不安定なら長く、安定なら短く読む（Stockfish `src/search.cpp:592-613`、YaneuraOu `yaneuraou-search.cpp:1969-1990`）。
minaseの二値の短縮は安定な手を短くするだけであり、短縮の程度も安定の度合いによらない。

時間管理の改善について、利用者は2026年10月7日に目的を棋力とし、「新しい時間配分の計画は立てない」として検討を終えた。
その後、係数の関数形の比較を受けて、利用者は2026年10月17日に本書の起案を明示的に指示した。
本書は、持ち時間の消費の速さや人間との対局での使い方を目的とする計画ではなく、探索の関数形を両エンジンの形へ寄せる系列（[枝刈りとLMRの関数形の見直し](search-pruning-forms.md)から始まる連鎖）の最後の1本である。

同じ日に、利用者は系列の全設計書に共通する方針を決めた（2026年10月17日の利用者の決定）。

- 両エンジンで形や値が違う場合はStockfishに合わせ、YaneuraOuにだけある項目は入れず、Stockfishにだけある項目は入れる。秒読みのようにStockfishに相当する仕組みがない事項は、YaneuraOuに合わせる。
- 新しい係数の開始値は、両エンジンの値を換算した値（両者が違えばStockfishの値）とする。既存の係数も、Stockfishに同じ形の値があれば換算値を開始値に置き直す。
- SPSAで調整するのは、各仕組みの強さを決める値（切片、傾き、重み、倍率など）だけとし、上下限や更新の内部の定数はStockfishの値で固定する。
- minaseにあってStockfishにない既存の仕組みのうち、中将棋の診断や測定で決めたものは残し、Stockfishの仕組みを追加して併用し、残したものもSPSAで調整する。最善手が4反復続いたら早く止める規則はこれに当たる。
- 評価値の単位を持つ定数は、開始値をStockfishの換算値としたうえで、範囲にStockfishとYaneuraOuの換算値とminaseの現行の対応する値をすべて含める。中将棋は将棋ともチェスとも異なるゲームなので、値はminaseのSPSAが決める。

本書については、秒読みつきの時計でも因子を使うこと、ノード占有率をStockfishの連続の形にすること、相手との残り時間の差による補正と合法手が1つの局面の上限を含めること、前の手の探索の値を持ち越す項を含めること、およびStockfishにだけある`increaseDepth`、詰みでの早期停止、先読み中の25%増しを含めることを、同じ日に利用者が決めた。

似た変更を含む候補は、過去に3つの設計書で扱われた。
測定まで進んだ候補はいずれも、改善を示せない判定`H0`か、判定前の中止に終わった。
本書はそれぞれと次の点で異なる。

[棋力向上段階6](archived/strength-stage6.md)は、2つの二値の延長をSTCで測り、いずれも`H0`だった（[fail-lowによる延長](../measurements/strength-stage6/strength-stage6-faillow-stc.md)、[最善手交替時の延長](../measurements/strength-stage6/strength-stage6-bmchange-stc.md)）。
fail-lowとは、aspiration windows（直前の反復の評価値の周りに狭い窓を置いて根を読む手法）で、評価値が窓の下限を割り込むことであり、最善手が崩れた兆候とされる。
2つの延長はどちらも、信号が立った反復の境界ではsoftの条件を外し、hardの予測だけで次の反復を始めるかを決める形であり、伸ばす量は信号の大きさによらなかった。
本書の因子は信号の大きさに応じてsoftを伸縮させ、縮める方向も持ち、強さはSPSAで調整する。

[持ち時間の効率的な使用](archived/time-management-efficiency.md)の第3段階は、本書と同じ4因子の積を、各因子の定数をStockfishのまま実装し、積全体に正規化の定数0.48を掛けた。
同じ設計書の第2段階は`increaseDepth`に当たる深さの抑制を実装したが、[事前判定](../measurements/time-management-efficiency/time-management-efficiency-stage2-bench.md)で抑えた反復が置換表の照会だけで終わる空費と判定して見送った。
第3段階は単独では測られず、予算の再定義と途中結果の採用（第1段階）を積んだ最終候補としてSTCで`H0`となった（[測定記録](../measurements/time-management-efficiency/time-management-efficiency-stc.md)）。
事前の[ベンチ](../measurements/time-management-efficiency/time-management-efficiency-stage3-bench.md)では、Stockfishの定数はminaseの深さ6〜8の探索で積を系統的に大きくし、正規化前の積の中央値は2.08であり、最善手の安定による因子（Stockfishの`reduction`、本書のT2）は240局面中180局面以上で上限に張り付いた。
事後分析は、`H0`となった主因を、係数つきの予算が持ち時間を200手目までに使い切らせ、中終盤で浅くなったことと結論した。
同じ候補は、hardから残り時間の割合の項を外していたので、時間切れも2件出した。
本書の開始値は第3段階の正規化前の定数とほぼ同じであり、開始時点の積が大きくなる見込みは同じである。
違いは3つある。
第1に、本書は強さを決める定数を固定せず、積の水準を決める係数（T2の分母など）を含めてSPSAで調整する。
第2に、現行のsoftとhardの式、次の反復の予測、および中断した反復の破棄を変えず、hardを残り時間の割合で抑える項を残す。
第3に、第2段階と第3段階の測定はNNUEへの置き換え（2026年10月12日）より前の評価関数で行われ、[ADR](../adr/reject-fair-share-budget-with-partial-results.md)は同じ方式の再提案に現行の構成での再測定を求めている。
開始時点の時間の使い方は、SPSAの前の事前対局（フェーズ2）で記録する。

[秒読みつき時計での持ち時間と秒読みの活用](byoyomi-time-usage.md)は、持ち時間期（持ち時間を使い切る前の手）の配分を局面の難しさで変える段階Bとして、持ち時間の消費の速さが異なる3候補を用意した。
B0は消費の水準を現行に保つ候補、B1は持ち時間を早く使い切る候補、B2は持ち時間を200手目まで残す候補である。
3候補はいずれも、先に採用した段階A（秒読み期の締切と途中結果の採用）の構成を秒読みつきの条件で上回らなかった。
3候補は共通の二値の「難しさの信号」を使った。
この信号は、深さ6以上の反復を終えるたびに、その反復か直前の反復で最善手が変わったか、評価値が深さd − 2の反復より歩兵1枚以上下がったときに真となる。
softは、信号が真なら基準時間の`r/v̄`倍、偽なら`1/v̄`倍とした（rとv̄は実戦のログから事前に求めた定数）。
B1は短い条件で`H0`となった（[測定記録](../measurements/byoyomi-time-usage/byoyomi-b1-short-r2.md)）。
B0は判定前に利用者の判断で止め（[記録](../measurements/byoyomi-time-usage/byoyomi-b0-short-r2.md)）、B2は[煙試験](../measurements/byoyomi-time-usage/byoyomi-b-smoke.md)だけで選別の測定をせずに不採用とした。
本書の因子は、2段階ではなく連続の量であり、深さ6以上という発動の条件を持たずに毎反復で働き、強さを実戦のログからの定数ではなくSPSAで決める。
また、段階Bは基準時間の式そのものを候補ごとに変えたが、本書は`soft_raw`（残り時間、加算、および秒読みから求める目安時間）の式を変えない。
B1の`H0`は基準時間の変更と信号を併せた結果なので、信号による伸縮だけの効果を示すものではない。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、連鎖の1つ前の設計書が完了した後、フェーズ0を始める時点のmasterの先頭のコミットである。
- **形の変更**は、本書の項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で表（`crates/minase/src/search/alphabeta/params.rs`）の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。採否はGSPRT（ペア単位の結果で対数尤度比を逐次計算し、境界に達した時点で止める検定）で判定し、`H1`は候補が有意に強い判定、`H0`は改善を示せない判定である。
- **soft**と**hard**は、`crates/minase/src/search/alphabeta/time.rs`の`TimeBudget`の目安時間と上限時間である。softは反復を終えたときに次の反復を始めるかの判定に、hardは探索の途中で打ち切る判定に使う。`soft_raw`は、hardで抑える前の目安時間である。
- **持ち時間期**と**秒読み期**は、秒読みが正の時計で持ち時間を使い切る前と後の手である。秒読み期は`TimeBudget::byoyomi_period`が真になる探索であり、予測と安定性を使わず、締切まで反復を始める。
- **二値の短縮**は、直近`StableIterations`反復（現行は4）の最善手が同じとき（`stable_signal`）、次の反復を始める条件を「経過時間がsoft未満」から「予測完了時刻がsoft以下」へ置き換える現行の規則である（[ADR](../adr/stop-early-on-stable-best-move-without-extensions.md)）。
- **主ワーカー**は、Lazy SMP（複数のスレッドが置換表を共有してそれぞれ反復深化を行う並列化）の0番のワーカーであり、時間の判定はこのワーカーだけが行う。
- **完了反復**は、主ワーカーがaspiration windowsの窓の内側で読み終えた反復であり、`d`はその反復の番号（Stockfishの`rootDepth`）を表す。
- **詰み帯と専用の帯**は、詰みまでの手数を表す評価値の範囲と、千日手などの同一局面の反復の結果を表す評価値の範囲（`crates/minase/src/search/mod.rs`の`MATE_THRESHOLD`と`REPETITION_BAND_START`）である。
- **前の探索**は、同じ対局の中で直前に終わった探索である。先読みの探索も含み、対局の開始（USIの`usinewgame`、CECPの`new`）で消える。
- **先読み**は、USIの`go ponder`で始まる探索（相手の予想手を指したと仮定した局面を相手の手番中に読むこと）であり、予想が当たると`ponderhit`、外れると`stop`が届く。**先読みの的中**は`ponderhit`を受けることである。
- **千分率**は1/1000を単位とする整数であり、因子と係数の多くをこの単位で表す。1000‰が倍率1に当たる。
- **手数帯**は、`scripts/clock_profile.py`が集計に使う通算の手数の区間（0〜49、50〜99、100〜149、150〜199、200〜299、300〜399、400以上）である。

対象は次の10項目である。

- 評価値の下落による因子（T1、falling eval）。前の探索の平均値との差の項を含む。
- 最善手が最後に変わった深さからの距離による因子（T2、best move stability）。前の探索の値を持ち越す項を含む。
- 最善手の変化回数による因子（T3、best move instability）。
- 最善手の部分木のノード占有率による因子（T4、best move effort）。
- 相手との残り時間の差による補正（T5、time advantage）。
- 合法手が1つの局面での目安時間の上限（T6）。
- softの半分を過ぎたら次の反復の深さを増やさない規則（T7、Stockfishの`increaseDepth`）。
- 詰みを見つけたときの早期停止（T8）。
- 先読みを使う対局での目安時間の25%増し（T9）。
- 二値の短縮の反復数の係数化（T10）。

次の事項は対象外とする。

- 秒読みに関わる係数と`ByoyomiMargin`（秒読みの締切に残す余裕を与えるUSIオプション）。秒読みのない時計で調整するSPSAでは勝敗に影響しないからである（[ADR](../adr/exclude-byoyomi-terms-and-safety-margin-from-spsa.md)）。
- 秒読み期の判定。秒読み期は締切まで反復を始める規則であり、softを使わないので、本書の因子は働かない。YaneuraOuも秒読み期は締切まで読む。
- `movetime`による探索。Stockfishは`movetime`では時間管理を使わない。
- fail highの回数による減深（Stockfishの`failedHighCnt`と`failHighRecovery`）。[aspiration windowsの設計書](aspiration-window-forms.md)のA5が扱う。T7は、A5が定める窓の読む深さの式に減深の項を1つ加える。

## 依存関係

起点は、[aspiration windowsの関数形の見直し](aspiration-window-forms.md)が完了した後のmasterの先頭である。
この順序は、[枝刈りとLMRの関数形の見直し](search-pruning-forms.md)から始まる連鎖の提案であり、最終的な順序は利用者が決める。
前の設計書の採否にかかわらず、起点の表の既定値をSPSAの開始値とする。
前の設計書の候補がmasterへ統合されると`params.rs`の既定値と係数の数が変わるので、実装を含めて前の設計書の完了後に始める。
T1は根の各手の平均値（Stockfishの`averageScore`）を、T4は根の各手の累計ノード数（Stockfishの`effort`）を使う。
どちらも、aspiration windowsの設計書のM2が各ワーカーに持たせる`a(m)`と`E(m)`を再利用し、本書では二重に導入しない。
T7は、同じ設計書のA5が定める窓の読む深さの式に、減深の項を1つ加える。
その設計書が採用されずに起点に入っていなければ、本書がM2の記録を同じ定義で導入し、T7の式からA5の項を除く。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、測定機での対局の同時対局数は16を上限とする。
時間切れの事前確認とSPSAのセッションは、[探索の関数形の見直しの統括計画](search-forms.md)に従い、vast.aiで借りる調整機で行う。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は次の3つのADRの決定の一部を変える。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確認し、新しいADRで置き換える。
変えずに残る決定も、新しいADRへ引き継ぐ。

- [延長の信号を使わないADR](../adr/stop-early-on-stable-best-move-without-extensions.md)は、fail-lowと最善手交替による延長を使わず、最善手が4反復続けて同じときだけ早く止めると決めた。本書の因子は1を超えてsoftを伸ばせるので、延長を使わないという決定を変える。二値の短縮は残し、反復数を係数にする。測定はNNUEへの置き換えより前の評価関数で行われた。このADRは評価関数の時期による再測定の留保を書いていないが、「延長を再び提案する場合は、別の信号として段階ゲートにかける必要がある」としており、本書は段階ゲートで判定する。
- [予算を分担そのものにする時間管理を採らないADR](../adr/reject-fair-share-budget-with-partial-results.md)は、代案として「局面の信号の積でsoftを伸縮させる案」と「softの半分を過ぎた後の反復の深さを抑える案」を棄却した。本書はこの2つを、予算の再定義を伴わない形で再び提案する。秒読みのない時計でhardまで続けて中断した反復を捨てる機構と、hardを残り時間の割合でも抑える決定は変えない。測定はNNUEへの置き換えより前の評価関数で行われ、ADR自身が「同じ方式を再提案するには現行の構成での再測定が要る」と留保している。
- [先読みの的中から予算を測るADR](../adr/continue-search-and-measure-budget-from-ponderhit.md)は、目減りを補う別の機構を持たないと決めた。本書のT9は、先読みを使う対局の先読みでない探索にだけ25%増しを加える（設計判断の表を参照）。的中から予算を測る機構は変えず、的中時の再判定にも因子を掛けたsoftを渡す。測定はNNUEへの置き換えより前で、起点や再判定の方式を変える提案には再測定が要ると留保している。

次のADRの決定は変えないが、本書の変更と関係するので、それぞれの留保とあわせて記す。

- [残り手数の見積りのADR](../adr/estimate-moves-to-go-from-ply.md)の見積りと、softをhard以下に保つ契約は変えない。T5は見積りから求めた`soft_raw`に因子を掛けるだけである。測定はNNUEへの置き換えより前で、見積りの方式を変える提案には再測定が要ると留保している。
- [次の反復の完了時刻を固定倍で予測するADR](../adr/predict-next-iteration-by-fixed-ratio.md)の予測式は変えない。二値の短縮が予測完了時刻と比べるsoftが、本書の因子を掛けたsoftになる。測定はNNUEへの置き換えより前で、予測の方式を変える提案には再測定が要ると留保している。
- [持ち時間を温存しないADR](../adr/spend-main-time-from-the-opening.md)は、持ち時間を後半のために温存しないと決めた。本書の因子は各手の判定でsoftを伸縮させるだけで、後半のために時間を残す配分ではない。T5は不利なときに消費を減らすので、秒読みつきの時計で働かせると持ち時間を残す方向に作用する。秒読みつきの時計を前提とするYaneuraOuはこの補正を持たないので、T5は秒読みが0の時計に限る。このADRは温存を再提案する場合に秒読みつきの時計での測定を求めている。
- [序盤の係数を秒読みの項にだけ掛けるADR](../adr/scale-only-the-byoyomi-term-in-the-opening.md)の係数と式は変えない。
- [秒読みの項の係数をSPSAで調整しないADR](../adr/exclude-byoyomi-terms-and-safety-margin-from-spsa.md)は、秒読みのない対局で勝敗に影響しない係数を調整の対象から外すと決めた。新しい係数は秒読みのない対局の予算を変えるので、調整の対象にできる。
- [根の手を前回のノード数で並べるADR](../adr/order-root-moves-by-previous-subtree-nodes.md)の並べ方は変えない。T4は並べ替え用の反復ごとのノード数ではなく、aspiration windowsの設計書のM2が持つ探索全体の累計を使う。

## 設計判断

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 10項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとの寄与は測らず、不採用のときに一部の項目だけを選び直して再測定しない。 |
| 伸縮の形（time management by search instability） | Stockfishと同じく、完了反復ごとにT1〜T4の4因子の積を求めて目安時間に掛け、hardで抑える（`src/search.cpp:612-613, 622-623`）。minaseでは、次の反復を始めるかの判定に使うsoftを`min(soft × 積, hard)`に置き換え、合法手が1つならさらにT6で抑える。T1〜T4とT6はhardを変えない。T5は予算を求める時点で`soft_raw`に掛ける。各因子の式の形はStockfishに合わせる（2026年10月17日の利用者の決定）。 |
| 調整する値と固定する値 | 各因子の強さを決める切片、傾き、および倍率の11個と、二値の短縮の反復数の計12個を係数にする。上下限、平均の重み、前の探索の値の初期値、深さを減らす式、詰みの手数、および先読みの増分はStockfishの値で固定する（2026年10月17日の利用者の決定）。固定する定数は「固定する定数」の表にまとめる。Stockfishの`interpolate(x, x0, x1, y0, y1)`（2点を通る直線）は、直線の切片と傾きの2つの係数で表し、`clamp`の上下限は固定する。2点の座標の4つを係数にすると、同じ直線を表す組が無数にあり、SPSAが直線を変えない方向にも係数を動かしてしまうからである。 |
| 開始値 | 新しい係数は、Stockfishの値を千分率と歩兵価値で換算した値とする（2026年10月17日の利用者の決定）。評価値の差に掛かる係数は、Stockfishの歩兵価値208（`src/types.h:199`）で歩兵1枚あたりの値に換算する。[第3段階のベンチ](../measurements/time-management-efficiency/time-management-efficiency-stage3-bench.md)によれば、この開始値ではminaseの深さで積が系統的に大きくなる。積の水準は、T2の分母（`TimeReductionScale`）や各因子の切片の係数を通してSPSAが動かせるので、正規化の定数は加えない。開始時点の時間の使い方は、フェーズ2の事前対局で記録する。 |
| 評価値の単位を持つ係数の3つの値 | 評価値の差に掛かる`TimeFallingPrevious`と`TimeFallingIteration`は、範囲にStockfishの換算値（÷208）、YaneuraOuの換算値（÷90）、およびminaseの現行の値（項がないので0）をすべて含める（2026年10月17日の利用者の決定）。歩兵1枚あたり千分率で、`TimeFallingPrevious`はStockfishが4784、YaneuraOuが約1904、現行が0、`TimeFallingIteration`はStockfishが2288、YaneuraOuが約888、現行が0である。どちらも開始値がYaneuraOuの約2.5倍であり、現行の0から最も遠い。YaneuraOuの歩兵価値は小さいので、歩兵1枚で見ると評価値の下落への反応はStockfishより弱い。中将棋の歩兵は他の駒に比べてさらに価値が小さく、歩兵単位の評価値の振れが大きいので、開始時点ではわずかな下落でT1が上限に張り付く見込みがある。 |
| 既存の係数の開始値 | 時間管理の既存の6係数は、起点の表の値のまま始める。`HardRemainingShare`（hardを残り時間の割合で抑える項）は、Stockfishの0.8097（`src/timeman.cpp:161`）と式の形は似ているが、意味が違うので、2026年10月17日の利用者の決定にいう「同じ形の値」には当たらないと判断した。Stockfishの上限時間は、中断した反復でも探索を終えた手の結果を使う前提の安全弁であり、上限まで使っても計算は着手に反映される。minaseは秒読みのない時計でhardで中断した反復の結果を捨てる（[ADR](../adr/reject-fair-share-budget-with-partial-results.md)）ので、この項は使い捨てになり得る長考を抑え、残り時間の少ない局面での時間切れを防ぐ働きを持つ（[打ち切り上限の教訓](../lessons/cap-hard-limit-by-remaining-fraction.md)）。このため開始値は現行の27%、範囲も現行の10〜50%のままとする。`ExpectedPlies`、`MinMoves`、`IncrementShare`、`HardSoftRatio`、および`IterationRatio`は、Stockfishの式の形が違うか対応する項がないので、起点の表の値のままとする。 |
| 前の探索の値の持ち越し | Stockfishと同じく、前の探索が選んだ手の評価値（`bestPreviousScore`）、その手の平均値（`bestPreviousAverageScore`）、および前の探索の最後の`timeReduction`（`previousTimeReduction`）を次の探索へ持ち越す（`src/search.cpp:259-260, 644`）。値は対局の開始で消し、先読みの探索の結果でも更新する。Stockfishも`ucinewgame`でだけ初期化し（`src/thread.cpp:270-275`）、局面が対局の続きかどうかを確かめない。minaseが現在探索をまたいで持ち越すのは置換表とhistory表だけなので、探索の開始と終了の関数に、持ち越す値の組を受け取って返す引数を加える。この変更は公開API（`start_search`と`SearchHandle::join`）の互換性を壊すので、該当のコミットは`feat!`と`BREAKING CHANGE:`のフッターで書く（[CONTRIBUTING.md](../../CONTRIBUTING.md)）。 |
| 対局の最初の探索 | Stockfishは前の値を`VALUE_INFINITE`で初期化するので、対局の最初の探索ではT1の前の探索の項が非常に大きくなり、因子が上限に張り付く。反復の値の環状配列は0で埋める（`src/search.cpp:319-322`）。本書はこの挙動を写し、前の探索がないときはT1の因子を上限の1728‰とし、環状配列を0で埋める。lishogiのBotは各対局の最初の1手を`movetime`で探索させるので、Botではこの規則が働かない。 |
| 二値の短縮との関係（T10） | 二値の短縮を残し、T2の因子と併用する（2026年10月17日の利用者の決定）。二値の短縮は中将棋での[反復深化の診断](../measurements/strength-stage6/strength-stage6-iteration-diag.md)から反復数4を選び、STCとLTCで`H1`となった規則である（[STC](../measurements/strength-stage6/strength-stage6-stable-stc.md)、[LTC](../measurements/strength-stage6/strength-stage6-stable-ltc.md)）。softとの比べ方を経過時間から予測完了時刻へ切り替える規則なので、softに掛ける倍率では表せない。反復数は定数`STABLE_ITERATIONS`から係数`StableIterations`に改め、現行の4から始めて他の係数と一緒に調整する。 |
| 評価値の比較 | Stockfishと同じく、T1は完了反復の値を、前の探索の平均値と、4反復前の完了反復の値（`iterValue`の4要素の環状配列）の両方と比べる（`src/search.cpp:594-596, 637-638`）。評価値が詰み帯と専用の帯にある場合を特別に扱わない点もStockfishと同じであり、大きな差は因子の上下限で抑えられる。 |
| 最善手の変化の数え方 | 根の探索で、2手目以降の手の値がその時点のαを上回って最善手が入れ替わった回数を、各ワーカーが数える（`src/search.cpp:1531-1532`）。aspiration windowsの読み直しの中の入れ替わりも数える。主ワーカーは反復を始めるたびに累計を半分にし、完了反復のたびに全ワーカーの回数を足してワーカーの回数を0に戻す（`src/search.cpp:354, 583-587`）。因子では累計をワーカー数で割る（`src/search.cpp:607`）。採否測定は`Threads=1`なので主ワーカーの回数だけになり、Botの4スレッドでは補助ワーカーの回数も加わる。 |
| ノード占有率 | Stockfishと同じく、主ワーカーの探索の開始からの累計で、最善手の部分木に使ったノード数が主ワーカーの全ノード数に占める割合を求め、直線と上下限で因子にする（`src/search.cpp:592, 609-610, 1474`）。YaneuraOuの階段状の形と深さ10以上の条件（`yaneuraou-search.cpp:1987`）は採らない。根の手ごとの累計は、aspiration windowsの設計書のM2が主ワーカーに持たせる`E(m)`を再利用し、二重に持たない。 |
| 相手との残り時間の差（T5） | Stockfishと同じく、自分の残り時間が相手より少ないときだけ`soft_raw`に因子を掛ける（`src/timeman.cpp:151-156`）。係数が正なら縮める。`soft_raw`に掛けるので、hardのうち`soft_raw`の倍数で決まる項（`soft_raw × HardSoftRatio / 100`）も同じ割合で変わる。残り時間の割合の項と、残り時間から余裕を引いた安全上限は変わらない。Stockfishも上限時間を目安時間の倍数で決める（`src/timeman.cpp:159-162`）。 |
| T5を秒読みが0の時計に限る | 秒読みはStockfishにない仕組みなので、秒読みつきの時計ではYaneuraOuに合わせる（2026年10月17日の利用者の決定）。YaneuraOuの`source/timeman.cpp`は相手の残り時間を使わない。秒読みが正の時計では、持ち時間を使い切っても毎手の秒読みで指せるので、持ち時間の差は時間が足りないことを表さず、不利なときに消費を減らせば持ち時間を後の手に残す温存にもなる。 |
| T5の入力 | `ClockLimits`に相手の残り時間を持たせる。T5を使う時計（秒読みが0で`movetime`を併用しない時計）では相手の残り時間を必須とし、届いていなければ0などで代えずに明確なエラーにする。T5を使わない時計（秒読みつきの時計、`movetime`を併用する時計）では相手の残り時間を要求しない。USIでは、`go`の`btime`と`wtime`のうち手番でない側の値を渡し、T5を使う時計でその値がない`go`は、現行の不正な`go`と同じくエラーの文を返して拒否する。CECP（XBoardの通信規約）では、現行で検査だけして捨てている`otim`を保持し、T5を使う時計で`otim`が届いていなければエラーを返す。探索のAPIでは、相手の残り時間がないT5を使う時計を`SearchLimits::new`が`SearchError`で拒否する。これまで受理していた入力を拒否するので、互換性を壊す変更として書く。対局ハーネスは両側の時計を毎手送っているので（`crates/minase/src/harness/clock.rs`）、測定には影響しない。 |
| 合法手が1つの局面（T6） | Stockfishと同じく、合法手が1つなら目安時間を500 msで抑える（`src/search.cpp:615-617`）。中将棋では王手を放置する着手も、王駒を相手の利きへ移す着手も合法であり（[RULES.md](../rules/RULES.md)第8条第3項から第5項）、合法手が1つになるのは動ける駒が1枚でその行き先も1升に限られる局面だけなので、対局中にほぼ発動しない。発動しないのでSPSAで値を調整できず、500 msは固定する。 |
| 深さを増やさない規則（T7、increaseDepth） | Stockfishと同じく、時間で止めなかった反復の境界で、経過時間が伸縮後の目安時間の50%を超えていれば、全ワーカーが共有する「深さを増やさない」フラグをセットする（`src/search.cpp:634`）。各ワーカーは反復を始めるときにフラグがセットされていれば読み直しの回数kを1増やし、aspiration windowsの設計書のA5が定める窓の読む深さの式に、減深の項`3 × (k + 1) / 4`を1つ加える（`src/search.cpp:369-370, 410-411`）。先読みの間は常に深さを増やす。[第2段階の事前判定](../measurements/time-management-efficiency/time-management-efficiency-stage2-bench.md)は、PSTを既定の評価関数としていた時期の深さで、浅くした反復が置換表の照会だけで終わる空費になると判定した。本書はこの規則をStockfishの方針により含め、効果は段階ゲートで判定する。閾値50%は、softのさらに半分という境界の位置であり、強さを決める倍率ではないので固定する。 |
| 詰みでの早期停止（T8） | Stockfishと同じく、時間で止めるかを判定する反復の境界で、最善手の値が3手以内に詰ます値以上であるか、2手で詰まされる値であれば、目安時間を待たずに止める（`src/search.cpp:622-623`）。手数は、どちらの対局者の着手も1手と数える（1手は片側の1着手）。2手で詰まされる値だけを等号で判定するのもStockfishと同じである。minaseの評価値の詰み帯は詰みまでの手数を表すので、同じ判定ができる。先読みの間は止めず、的中の時点で止める。YaneuraOuの既定の経路の停止の判定（`yaneuraou-search.cpp:2041`）はこの条件を持たないが、秒読みに関わらない項目なのでStockfishに合わせる。 |
| 先読み中の25%増し（T9） | Stockfishは先読みを有効にした対局のすべての探索で、目安時間を25%増やす（`src/timeman.cpp:164-165`）。Stockfishは先読みの探索の経過時間を探索の開始から数えるので、的中した手では相手の手番中の時間まで目安時間から差し引かれ、時計に対して時間を使い残す。25%増しは、その使い残しを他の手で使う働きを持つ。minaseは的中の時点から予算を測るので、的中した手で使い残しが生じない（[ADR](../adr/continue-search-and-measure-budget-from-ponderhit.md)）。的中した手にも25%を足すと、同じ目減りを2回補うことになる。そこで、25%増しは先読みを使う対局の先読みでない探索（先読みが外れた後の探索など）にだけ掛け、的中から予算を測る探索には掛けない。minaseはUSIの`USI_Ponder`を宣言しないので、「先読みを使う対局」は、同じ対局で`go ponder`を1回以上受けたことで判定する。25%は、SPSAと段階ゲートの対局が先読みを使わないので調整できず、固定する。 |
| 秒読みつきの時計 | T1〜T4、T6、T7、およびT8は、秒読みつきの時計の持ち時間期にも働かせる（2026年10月17日の利用者の決定、案A）。秒読みはStockfishにない仕組みであり、YaneuraOuは秒読みつきの時計でも4因子の積を目安時間に掛けるからである（`yaneuraou-search.cpp:1989-1996`）。秒読み期は現行のままとする。SPSAは秒読みのない時計で係数を選ぶので、秒読みつきの持ち時間期での値の妥当性は調整の外にある。そこで、段階ゲートを通った候補を、秒読みつきの2条件（短い条件`time=6000+0,byoyomi=300`と長い条件`time=45000+0,byoyomi=300`）のGSPRTで起点と比べる。両方が`H1`なら統合する。どちらかが`H1`でなければ、本書の項目を秒読みが0の時計に限るコミットを加えてから統合する。この後始末は測定の前に定める規則であり、秒読みが0の時計の予算を変えないので、段階ゲートの判定はそのまま有効である。人間との対局で棋力が落ちる変更を許容せず、非劣性の検定（候補が基準より一定の幅以上は弱くないことを示す検定）で落ちていないとみなす扱いも採らないことは、利用者の方針として記録されているので、秒読みつきの時計を変えるには`H1`を要求する。費用は、[段階Aの短い条件](../measurements/byoyomi-time-usage/byoyomi-a-short-c10.md)が1ペア約30秒、[長い条件](../measurements/byoyomi-time-usage/byoyomi-a-long-c10.md)が約39秒（いずれも同時対局数10）なので、判定までの130〜3,000ペアに対して短い条件が約1〜25時間、長い条件が約2〜33時間、合わせて約3〜58時間である。 |
| STCを省略しない | [SPRTの手引き](../guides/sprt.md)は、短時間条件では効果が原理的に現れない時間管理の変更に限り、STCを省略してLTCへ直行することを認める。本書の因子は深さによらず毎反復で働き、SPSAもSTCの持ち時間で係数を選ぶので、効果はSTCに現れる。第3段階のベンチでも、深さ6〜8の探索で因子は大きく振れていた。このため手引きの段階ゲートを省略せずに使う。 |
| SPSAの対象 | 起点の調整対象の全係数に新しい12係数を加える。調整対象は、`FmScale`（調整用ビルドだけが参照する、因子分解機（factorization machine）による評価の補正の倍率）を除く表の全係数である。起案時のmasterの表は38係数であり、連鎖の先行の設計書がすべて採用されると、本書の12係数を含めて約102になる。数は着手時に起点の表で数え直す。時間管理の係数を含むので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。3,750反復、1反復8ペア（計30,000ペア）、調整機での実行、持ち時間は測定機の10秒＋0.1秒を[統括計画](search-forms.md#持ち時間の換算)の方式で換算した値、同時対局数は[統括計画](search-forms.md#同時対局数とメモリ)が調整機ごとに決める数、`c_end`（最後の反復の摂動幅）は範囲の1/6、`r_end`（最後の反復の学習率）は0.002である（[SPSAの手引き](../guides/spsa.md)）。30,000ペアは候補を1つ作るための予算であり、最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。約100の係数を30,000ペアで十分に調整できるかは未検証であり、不採用は「この予算と設定で改善を示さなかった」ことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。 |
| 事前対局での時間の使い方の確認 | [SPSAの手引き](../guides/spsa.md)の第7節は、時間の使い方の変化を合否の条件にせず記録すると定める。この規則は、既存の機構のまま係数の値だけを調整し直した候補を想定しており、時間の使い方が変わること自体が調整の目的に含まれるからである。本書は予算の機構を変えるので、より具体的な規則である[教訓](../lessons/compare-total-time-usage-before-sprt.md)を優先して適用する。この教訓は、1手ごとの指標を満たした予算の機構の変更が、1局の時間の使い残しや手数帯の間の偏りのために段階ゲートで`H0`となった経験から定めたものである。採否測定の前の事前対局で、1局の総思考時間が起点の90%以上110%以下に収まり、手数帯ごとの完了深さがどの帯でも起点を下回らないことを確かめる。満たさない場合は採否測定へ進まず、集計を利用者に示して判断を仰ぐ。第3段階が`H0`となった主因（序盤の使いすぎで中終盤が浅くなること）は、手数帯ごとの完了深さに表れる。 |

## 変更の内容

以下の式で、`P`は探索用の歩兵価値（評価関数の`pawn_value`）、`v`は主ワーカーの完了反復の評価値を表す。
因子と、因子に足し引きする係数は千分率で表し、`clamp(x, lo, hi)`は`lo`以上`hi`以下への切り詰めを表す。
整数の除算は0の方向へ丸め、積の途中は桁あふれしない広い整数型で計算する。

### 伸縮の適用

本書の項目を使う探索は、時計による探索のうち、`movetime`を併用せず、秒読み期でないものとする。
主ワーカーは完了反復ごとにT1〜T4の因子を求め、次の式で伸縮後の目安時間`soft'`を求める。

```math
\mathrm{soft'} = \min\left(\left\lfloor \frac{\mathrm{soft} \times f_1 \times f_2 \times f_3 \times f_4}{1000^4} \right\rfloor,\ \mathrm{hard}\right)
```

合法手が1つなら、さらに`soft' = min(soft', 500 ms)`とする（T6）。
`soft'`は、反復の完了時の`should_start_next_iteration`、先読みの探索で反復を始める前の判定、および的中時の`iteration_prediction_fits`による再判定（`searcher.rs`の`check_time`）で、softの代わりに使う。
二値の短縮が予測完了時刻と比べる値も`soft'`である。
最初の反復を終えるまでは、因子を掛けずにsoftを使う。
本書の項目を使わない探索、秒読み期、および探索中のhardの検査は現行のままである。

### T1　評価値の下落（falling eval）

```math
f_1 = \mathrm{clamp}\left(\mathrm{TimeFallingBase} + \frac{\mathrm{TimeFallingPrevious} \times (A_{\mathrm{prev}} - v) + \mathrm{TimeFallingIteration} \times (I - v)}{P},\ 576,\ 1728\right)
```

`A_prev`は前の探索が選んだ手の平均値、`I`は4反復前の完了反復の値である。
`I`は、主ワーカーが持つ4要素の環状配列から読み、読んだ後に`v`で上書きする。
環状配列は探索の開始時に前の探索が選んだ手の評価値で埋め、前の探索がなければ0で埋める。
前の探索がない場合は、その探索のすべての完了反復で`f_1`を1728‰とする。
この場合も環状配列は同じ規則で更新するが、その探索の`f_1`には影響しない。

### T2　最善手が最後に変わった深さからの距離（best move stability）

```math
r = \mathrm{clamp}(\mathrm{TimeReductionIntercept} + \mathrm{TimeReductionSlope} \times x,\ 629,\ 1544), \qquad f_2 = \left\lfloor \frac{(1468 + r_{\mathrm{prev}}) \times 10^6}{\mathrm{TimeReductionScale} \times r} \right\rfloor
```

`x`は、反復の番号dから、最善手が最後に変わった完了反復の番号を引いた値である（`src/search.cpp:533-534, 600-602`）。
深さdで最善手が変わった直後は`x = 0`である。
`r_prev`は前の探索の最後の`r`であり、対局の最初の探索では850‰、前の探索が時間管理を使わなかった場合は1000‰とする。
後者は、Stockfishが探索の開始時に`timeReduction`を1で初期化し、時間管理を使わない探索ではその値のまま持ち越すことに合わせた（`src/search.cpp:295, 644`）。

### T3　最善手の変化回数（best move instability）

```math
c_d = \left\lfloor \frac{c_{d-1}}{2} \right\rfloor + 1024 \times n_d, \qquad f_3 = \mathrm{TimeInstabilityBase} + \left\lfloor \frac{\mathrm{TimeInstabilitySlope} \times c_d}{1024 \times W} \right\rfloor
```

`n_d`は、主ワーカーの反復d − 1の完了から反復dの完了までに（`d = 1`では探索の開始から反復1の完了までに）、全ワーカーの根の探索で、2手目以降の手の値がその時点のαを上回って最善手が入れ替わった回数の合計である。
`c_d`は変化回数の減衰つきの累計を1/1024回の単位で持ち、`c_0 = 0`とする。
`W`はワーカー数である。
Stockfishと同じく、`f_3`に上下限は置かない。

### T4　最善手のノード占有率（best move effort）

```math
e = \left\lfloor \frac{100000 \times E_{\mathrm{best}}}{\max(1, N)} \right\rfloor, \qquad f_4 = \mathrm{clamp}\left(\mathrm{TimeEffortIntercept} + \left\lfloor \frac{\mathrm{TimeEffortSlope} \times e}{100000} \right\rfloor,\ 693,\ 838\right)
```

`E_best`は、主ワーカーが探索の開始から反復dの最善手の部分木に使ったノード数の累計であり、aspiration windowsの設計書のM2が主ワーカーに持たせる`E(best)`を再利用する（起点に無い場合だけ同じ定義で導入する）。
`N`は主ワーカーの全ノード数である。
`e`はStockfishの`nodesEffort`と同じく、割合を100000倍した値である。

### T5　相手との残り時間の差（time advantage）

秒読みが0で`movetime`を併用しない時計に限り、`clock_budget_with_margin`の`soft_raw`に次の因子を掛けてから、hardとsoftを現行の式で求める。

```math
f_5 = 1000 + \left\lfloor \frac{\mathrm{TimeAdvantage} \times \min(0,\ R - R_o)}{R + R_o + 1} \right\rfloor, \qquad \mathrm{soft\_raw'} = \left\lfloor \frac{\mathrm{soft\_raw} \times f_5}{1000} \right\rfloor
```

`R`は自分の残り時間、`R_o`は相手の残り時間（いずれもミリ秒）である。
自分の残り時間が相手以上なら因子は1000‰である。
hardのうち`soft_raw`の倍数の項は`soft_raw'`から求めるので、自分が不利なときはhardの候補も縮む。

### T7　深さを増やさない規則（increaseDepth）

主ワーカーは、時間で止めなかった完了反復の境界で、先読みの間でなく、経過時間が`soft'`の50%を超えていれば、全ワーカーが共有する「深さを増やさない」フラグをセットし、そうでなければクリアする。
各ワーカーは自分の回数`k`を探索の開始時に0とし、反復を始めるときにフラグがセットされていれば`k`を1増やす。
`k`はその探索の間は減らさず、反復の番号dの探索を次の深さで行う。

```math
\mathrm{depth} = \max\left(1,\ d - h - h_{\mathrm{carry}} - \left\lfloor \frac{3 \times (k + 1)}{4} \right\rfloor\right)
```

`h`と`h_carry`は、[aspiration windowsの設計書](aspiration-window-forms.md)のA5のfail highの回数と持ち越し（A5の式の`h`と`r`、Stockfishの`failedHighCnt`と`failHighRecovery`）である。
A5の窓の読む深さ`max(1, d − h − r)`に、T7の項を1つだけ加えた式であり、窓を読み直すたびにこの深さを使う。
反復の番号dはフラグにかかわらず1ずつ増え、T2の距離と二値の短縮は反復の番号で数える。

### T8　詰みでの早期停止

伸縮後の目安時間で止めるかを判定する反復の境界で、最善手の値が「3手以内に詰ます値」以上か、「2手で詰まされる値」に等しければ、経過時間にかかわらず止める。
先読みの間は止めず、的中の時点で止める。

### T9　先読みを使う対局での25%増し

同じ対局で`go ponder`を1回以上受けた後の、先読みでない探索では、`soft'`の基になるsoftを`soft + ⌊soft / 4⌋`とし、hardを超えないように抑える。
先読みとして始めた探索には、的中してそのまま続いた場合も含めて掛けない。

### T10　二値の短縮の反復数

`stable_signal`が比べる直近の完了反復の数を、定数`STABLE_ITERATIONS`から係数`StableIterations`に改める。

### 新しい係数

開始値はStockfish（コミット5c4cef0b）の値の換算であり、Stockfishに値がない`StableIterations`だけは現行の値である。
YaneuraOuの値は、形が同じ係数について記す。

| 係数 | 開始値 | 範囲 | Stockfish | YaneuraOu | minaseの現行 | 換算の根拠 |
|---|---|---|---|---|---|---|
| `TimeFallingBase`（‰） | 115 | 0〜1500 | 115 | 113 | 項なし | 11.48/100（`src/search.cpp:594`）。YaneuraOuは11.325/100 |
| `TimeFallingPrevious`（歩兵1枚あたり‰） | 4784 | 0〜8000 | 4784 | 約1904 | 項なし（0） | 内部単位あたり2.30/100を歩兵価値208で換算（2.30 × 208 × 10）。YaneuraOuは2.115/100を歩兵価値90で換算 |
| `TimeFallingIteration`（歩兵1枚あたり‰） | 2288 | 0〜5000 | 2288 | 約888 | 項なし（0） | 1.1/100を同じく換算（1.1 × 208 × 10、`src/search.cpp:595`）。YaneuraOuは0.987/100 |
| `TimeReductionIntercept`（‰） | 254 | 0〜1500 | 254 | 形が違う | 項なし | `interpolate(x, 4.96, 18.79, 0.639, 1.712)`（`src/search.cpp:600-602`）の直線の`x = 0`での値。0.639 − 4.96 × 1.073 / 13.83 |
| `TimeReductionSlope`（距離1あたり‰） | 78 | 0〜300 | 78 | 形が違う | 項なし（0） | 同じ直線の傾き1.073 / 13.83 ≈ 0.0776。YaneuraOuはロジスティック関数を使う |
| `TimeReductionScale`（‰） | 2284 | 1000〜4000 | 2284 | 2238 | 項なし | `src/search.cpp:605`。YaneuraOuは2.2375 |
| `TimeInstabilityBase`（‰） | 1077 | 500〜1500 | 1077 | 1040 | 項なし（1000） | `src/search.cpp:607` |
| `TimeInstabilitySlope`（変化1回あたり‰） | 2229 | 0〜4000 | 2229 | 1896 | 項なし（0） | `src/search.cpp:607` |
| `TimeEffortIntercept`（‰） | 1642 | 500〜2500 | 1642 | 形が違う | 項なし | `interpolate(e, 75800, 104510, 0.969, 0.714)`（`src/search.cpp:609-610`）の直線の`e = 0`での値。0.969 + 75800 × 0.255 / 28710 |
| `TimeEffortSlope`（占有率100%あたり‰） | −888 | −2000〜500 | −888 | 形が違う | 項なし（0） | 同じ直線の傾き −0.255 / 28710 × 100000。YaneuraOuは階段状 |
| `TimeAdvantage`（‰） | 900 | 0〜1000 | 900 | 項なし | 項なし（0） | `src/timeman.cpp:155`の0.9。上限1000は、差の割合が−1に近づいても因子が負にならない値である |
| `StableIterations`（反復） | 4 | 2〜8 | 項なし | 項なし | 4 | 現行の`STABLE_ITERATIONS`。範囲は、4反復を選んだ[診断](../measurements/strength-stage6/strength-stage6-iteration-diag.md)が比べた3〜5を含む |

SPSAの摂動幅は反復とともに`(3750 / k)^0.101`に比例して縮み（kは反復番号）、最初の反復では最後の反復の`3750^0.101 ≈ 2.30`倍になる（`crates/minase/src/bin/minase/spsa_runner/model.rs`）。
`c_end`は範囲の幅の1/6なので、最初の反復の摂動幅は範囲の幅の約0.38倍（2.30 ÷ 6）である。
開始値から範囲の端までの距離がこれより短い係数は、最初の反復の摂動が切り詰められる。
該当するのは、下側の`TimeFallingBase`、`TimeReductionIntercept`、および`TimeReductionSlope`と、上側の`TimeAdvantage`である。
[利得のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)はこの切り詰めを許容しており、係数ごとの摂動幅の設定は加えない。

### 固定する定数

| 定数 | 値 | Stockfishの出典 |
|---|---|---|
| T1の上下限 | 576‰と1728‰ | `src/search.cpp:597` |
| T1の環状配列の長さと、前の探索がないときの初期値 | 4要素、0 | `src/search.h:302`、`src/search.cpp:319-322, 637-638` |
| 前の探索がないときのT1 | 上限の1728‰ | `src/thread.cpp:270`、`src/search.cpp:594-597` |
| 根の各手の平均値の重み | 32を分母とし、重みを12〜24に制限、比の定数3/2 | `src/search.cpp:1480-1495` |
| T2の`r`の上下限 | 629‰と1544‰ | `src/search.cpp:601-602` |
| T2の分子の定数 | 1468‰ | `src/search.cpp:604-605` |
| 対局の最初の探索の`r_prev` | 850‰ | `src/thread.cpp:271` |
| 時間管理を使わなかった探索の後の`r_prev` | 1000‰ | `src/search.cpp:295, 644` |
| T3の累計の減衰 | 反復ごとに1/2 | `src/search.cpp:354` |
| T4の上下限 | 693‰と838‰ | `src/search.cpp:610` |
| T6の上限 | 500 ms | `src/search.cpp:615-617` |
| T7の閾値と減深 | `soft'`の50%、`3 × (k + 1) / 4` | `src/search.cpp:369-370, 410-411, 634` |
| T8の詰みの手数 | 3手以内に詰ます、2手で詰まされる | `src/search.cpp:622-623` |
| T9の増分 | 25% | `src/timeman.cpp:164-165` |

## 所要時間の見積り

時間は、時間切れの事前確認とSPSAが調整機、それ以外が同時対局数16の測定機での見積りである。
SPSAは、[統括計画](search-forms.md#所要時間と費用)の見積りにより、調整機で約13〜18時間と見込み、時間切れの事前確認を含む1セッションの費用は約25〜35米ドルである。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので幅を持たせた。

| フェーズ | 内容 | 見積り |
|---|---|---|
| 0 | 起点の時計の再構成（保存記録の集計だけ） | 約10分 |
| 1 | 形の変更の実装と検証（codexへ委任） | 1〜2日 |
| 2 | 時間切れの事前確認（調整機）と開始値の事前対局（測定機、16ペア） | 約35分と約30分 |
| 3 | SPSA（調整機、30,000ペア） | 約13〜18時間 |
| 4 | 反映、テスト、および事前対局（32ペア） | 約1時間 |
| 5（STC） | 候補のSTC | 約1〜8時間 |
| 5（LTC） | 通過した場合のLTC | 約5〜30時間 |
| 6 | LTCで採用となった場合の秒読みつきの2条件 | 約3〜58時間 |

測定機の占有（フェーズ2の開始値の事前対局とフェーズ4から6の和）は、STCで棄却される場合に約2.5〜9.5時間、LTCで棄却される場合に約7.5〜39.5時間、秒読みつきの2条件まで進む場合に約10.5〜97.5時間となる。
調整機は、フェーズ2の時間切れの事前確認とフェーズ3で約13〜18時間使う。

## 実装フェーズ

基本シードは、時間切れの事前確認を5000000000、開始値の事前対局を5025000000、SPSAのセッションを5050000000、候補の事前対局を5100000000、STCを5150000000、LTCを5200000000、秒読みつきの短い条件を5250000000、長い条件を5350000000と仮に定める。
起案時点で`data/matches/`と`data/spsa/`の全226件の`manifest.json`の基本シード（`seed`または`settings.seed`）を調べた範囲では、4,900,000,000から5,500,000,000の間に既存の基本シードはなく、各測定のペア数の上限（LTCと長い条件は既定の100,000ペア）を足しても既存の測定と重ならない（[教訓](../lessons/derive-seed-adjacent-collision.md)）。
着手時に同じ方法で未使用であることを確かめ直し、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ0　起点の時計の再構成

[教訓](../lessons/reconstruct-clock-from-match-records.md)に従い、起点と同じ探索と時間管理のコードを一方の側に持つ直近のSTCの実行ディレクトリ（連鎖の前の設計書のSTC）から、`scripts/clock_profile.py`でその側の時計を再構成する。
手数帯ごとの完了深さ、思考時間と残り時間の中央値、および停止理由の分布を、測定記録`time-instability-scaling-baseline`に残す。
本書の因子は、softによる停止（`StopReason::SoftLimit`）で終わる手と、hardによる中断（`StopReason::HardLimit`）で終わる手の両方に働く。
hardで中断された手も、最後の反復の境界ではsoftの判定を通っており、因子が1より小さければその境界で止まり得るからである。
一方、hardによる打ち切りで止まる手では`soft' ≤ hard`なので、伸ばす方向の因子は働かない。
この記録は、フェーズ2とフェーズ4の事前対局と比べる基準であり、採否の条件にはしない。
起点を一方の側に持つSTCの実行ディレクトリがない場合（利用者が連鎖の順序を変えた場合など）は、フェーズ2の開始値の事前対局の起点側の集計で代える。
完了条件は、測定記録の保存である。

### フェーズ1　形の変更の実装

起点からブランチ`time-instability-scaling`を切り、worktree `data/worktrees/time-instability-scaling`で作業する。
挙動を変えない移行を先に、時間予算や探索の深さを変える項目を後に、次の順で1項目を1コミットにする。

1. 前の探索の値を持ち越す引数の追加（`start_search`と`SearchHandle::join`、対局の開始での消去）。互換性を壊す変更として書く。
2. 全ワーカーの最善手の変化回数の計数。根の各手の平均値と累計ノード数は、aspiration windowsの設計書のM2の記録を再利用し、起点に無い場合だけ同じ定義でこのコミットに含める。
3. T5の入力の受け取り（`ClockLimits`の相手の残り時間、`SearchLimits::new`の検査、USIの`go`の解析、CECPの`otim`の保持）。互換性を壊す変更として書く。
4. T10（`StableIterations`の係数化。開始値4では挙動を変えない）。
5. 伸縮の適用の枠組みとT1。
6. T2、T3、T4、T5の因子の計算、T6、T7、T8、およびT9を、それぞれ1コミット。

`minase dev bench`（固定の15局面を固定の深さで読み、探索ノード数、評価値、および最善手を出力するベンチ）は時間管理を通らない。
手順1から4は挙動を変えない移行なので、benchの全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致することで検証する。
手順5と6の各コミットでも同じbenchを実行してノード数を記録する。
固定深さの探索では時間予算を使わず、T7のフラグもセットされないので、ノード数は変わらない見込みであり、変わった場合は根の探索の計数などが探索そのものを変えていないかを調べる。
時間予算の変化はbenchに表れないので、因子ごとの数値の例による単体テストで、因子の向き、上下限、`soft' ≤ hard`、T6の上限、T7の深さ、T8の停止、T9の対象、および前の探索がない場合の扱いを検査し、時間の使い方の変化はフェーズ2の事前対局で記録する。
単体テストの期待値は、式を写して計算せず、手で求めた数値で書く（[テストの手引き](../guides/testing.md)）。
T5を使う時計で相手の残り時間がない入力が、USI、CECP、および`SearchLimits::new`のそれぞれでエラーになることも検査する。
新しい係数は`params.rs`の表へ追加し、調整用ビルド（`--features tuning`つきのビルド）が`Tune_<係数名>`のUSIオプションとして宣言することを確かめる。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過、手順1から4のbenchの一致、および手順5と6のbenchの記録である。

### フェーズ2　パラメーターファイル、時間切れの事前確認、および開始値の事前対局

形の変更の最後のコミットでビルドした`minase`を`data/spsa/time-instability-scaling-runner/`へ複製してrunner（測定を実行するプログラム）として固定し、以後のフェーズはこのrunnerで実行する。
ただし、時間切れの事前確認とSPSAのセッションは[SPSAの手引き](../guides/spsa.md)の調整機の手順で実行し、調整機では同じコミットからビルドしたrunnerを使う。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除したファイルを`data/spsa/time-instability-scaling.params`とする。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、使い捨てのスクリプトで機械的に突き合わせる。

続いて、手引きの第3節に従い、時間管理の係数を思考時間が長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
パラメーターファイルの写し`data/spsa/time-instability-scaling-safety.params`を作り、手引きの4係数の開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150に置き換える。
新しい係数は、積が最も大きくなる側の端に置き換える。
値は、`TimeFallingBase`＝1500、`TimeFallingPrevious`＝8000、`TimeFallingIteration`＝5000、`TimeReductionIntercept`＝0、`TimeReductionSlope`＝0、`TimeReductionScale`＝1000、`TimeInstabilityBase`＝1500、`TimeInstabilitySlope`＝4000、`TimeEffortIntercept`＝2500、`TimeEffortSlope`＝500、`TimeAdvantage`＝0、`StableIterations`＝8とする。
このファイルで、[統括計画](search-forms.md#実行の構成)が定める反復数（同時対局数191なら156反復、1,248ペア。同時対局数は調整機ごとに決まる）を調整機で実行する。
この組ではsoft'がほぼ常にhardに達し、序盤からhardまで読む手が続いて、残り時間が少ない局面へ早く入る。
T1〜T4とT6はhardを伸ばさず、T5は`TimeAdvantage`が0以上なのでhardを縮める側にしか働かない。
hardには残り時間の割合の項と、残り時間から余裕を引いた安全上限も残るので時間切れは起きにくいはずだが、実際に起きないことを測定で確かめる。

```console
<調整機のminase> spsa \
  --run-dir data/spsa/time-instability-scaling-safety --seed 5000000000 \
  --engine commit:<形の変更> --params data/spsa/time-instability-scaling-safety.params \
  --rules engine-default --each <換算した持ち時間> --concurrency <調整機の同時対局数> \
  --iterations <事前確認の反復数> --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`time-instability-scaling-safety`に残す。

次に、開始値の形の変更と起点を、STCの条件で16ペア対局させる。

```console
<固定したminase> match run \
  --run-dir data/matches/time-instability-scaling-start --seed 5025000000 \
  --candidate commit:<形の変更> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 elo --pairs 16
```

1局の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ、および停止理由の分布を`scripts/clock_profile.py`で集計し、測定記録`time-instability-scaling-start`に残す。
この対局は開始値での時間の使い方の変化を記録するものであり、採否や係数の選び直しには使わない。
時間切れまたはエンジン異常が1件でもあれば、原因を調べてからSPSAへ進む。
完了条件は、時間切れとエンジン異常が0件であることと、2つの測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<調整機のminase> spsa \
  --run-dir data/spsa/time-instability-scaling --seed 5050000000 \
  --engine commit:<形の変更> --params data/spsa/time-instability-scaling.params \
  --rules engine-default --each <換算した持ち時間> --concurrency <調整機の同時対局数> \
  --iterations 3750 --pairs-per-iteration 8
```

このコマンドは調整機で実行し、実行ディレクトリは測定機の`data/spsa/time-instability-scaling`へ取得してから反映に使う。
最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`time-instability-scaling`に、手引きの第6節の項目とともに残す。
最終値を丸めた整数が範囲の上限または下限になった係数を示す。
新しい12係数については、最終値が開始値からどれだけ離れたかを`c_end`に対する比で示す。
この比は、どの因子が使われたかを読むための目安であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、テストの前提が新しい既定値でも成り立つようにテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で握りつぶさない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

採否測定の前に、STCの条件で候補と起点の32ペアの事前対局を行う。

```console
<固定したminase> match run \
  --run-dir data/matches/time-instability-scaling-smoke --seed 5100000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 elo --pairs 32
```

候補と起点の1局の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ、および停止理由の分布を`scripts/clock_profile.py`の再構成で集計し、測定記録`time-instability-scaling-smoke`に残す。
設計判断の表の「事前対局での時間の使い方の確認」の条件を満たさない場合は、採否測定へ進まずに集計を利用者に示す。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、時間の使い方の条件の確認、および測定記録の保存である。

### フェーズ5　採否測定

2026年10月18日の利用者の決定により、本書の変更はSTCの判定によらず採用する（[統括計画](search-forms.md)の設計判断「各サブタスクの採否」）。この節のSTCは採否に使わない記録として行い、LTCは行わない。以下の判定の規則は、この決定より前に書いたものであり、統括計画の決定が優先する。秒読みつきの確認の扱いは、本書に着手するときに決める。

```console
<固定したminase> match run \
  --run-dir data/matches/time-instability-scaling-stc --seed 5150000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

STCの判定は[SPRTの手引き](../guides/sprt.md)の振分け規則に従う。
候補が有意に強い`H1`ならLTCへ進み、改善を示せない`H0`なら不採用とし、3,000ペアで判定保留ならその時点の対数尤度比の符号でLTCへ進むかを決める。
LTCは測定名の接尾辞を`-ltc`、基本シードを5200000000、持ち時間を`time=60000+200`とし、`--max-pairs`を付けない。
LTCで`H1`となり、エンジン異常、時間切れ、および拒否着手が0件なら採用とし、フェーズ6へ進む。
`H0`なら不採用とする。
LTCの判定保留は、手引きのとおり`--resume`と`--target-pairs`で延長する。
結果は測定名ごとの測定記録に残す。
完了条件は、採否の確定と測定記録の保存である。

### フェーズ6　秒読みつきの時計での確認

LTCで採用が決まった候補を、秒読みつきの短い条件で起点と比べる。
同時対局数は、16で行った秒読みつきの測定が過負荷による時間切れで無効になった先例（[段階Aの短い条件](../measurements/byoyomi-time-usage/byoyomi-a-short-c10.md)）に従い10とする。

```console
<固定したminase> match run \
  --run-dir data/matches/time-instability-scaling-byoyomi-short --seed 5250000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=6000+0,byoyomi=300 --concurrency 10 gsprt --max-pairs 3000
```

短い条件が`H1`なら、測定名の接尾辞を`-byoyomi-long`、基本シードを5350000000、持ち時間を`time=45000+0,byoyomi=300`とし、`--max-pairs`を付けずに長い条件で比べる。
両方が`H1`で、エンジン異常、時間切れ、および拒否着手が0件なら、候補をそのまま統合する。
どちらかが`H1`でなければ、本書の項目（T1〜T4とT6〜T10の判定への適用）を秒読みが0の時計に限るコミットを加えて統合する。
このコミットでは、秒読みが0の時計の予算と判定が変わらないことを単体テストで確かめる。
短い条件で上限の3,000ペアに達した判定保留は、対数尤度比が0以上でも`H1`とはみなさず、本書の項目を秒読みが0の時計に限るコミットを加える。
完了条件は、秒読みつきの時計の扱いの確定と測定記録の保存である。

## 検証

- 挙動を変えない移行のコミットで、`minase dev bench`の全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。時間予算や深さを変える項目のコミットでは、同じbenchのノード数を記録すること。
- 項目ごとの単体テストが、因子の向き、上下限、`soft' ≤ hard`、T6の上限、T7の深さ、T8の停止、T9の対象、および前の探索がない場合の扱いを検査していること。
- T5を使う時計で相手の残り時間がない入力が、USI、CECP、および`SearchLimits::new`でエラーになること。
- 新しい12係数が調整用ビルドの`Tune_`オプションとして宣言され、パラメーターファイルの開始値と範囲が表と一致すること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`が通過すること。
- 時間切れの事前確認、開始値の事前対局、SPSA、候補の事前対局、および採否の測定で、エンジン異常、時間切れ、および拒否着手が0件であること。1件でも出た場合は、その測定を止めて原因を調べる。
- 候補の事前対局で、総思考時間が起点の90%以上110%以下であり、手数帯ごとの完了深さがどの帯でも起点を下回らないこと。

## 完了条件

- 起点の時計の再構成、形の変更の検証結果、時間切れの事前確認、開始値の事前対局、SPSAの結果、候補の事前対局、採否、および秒読みつきの2条件の結果が、測定記録に残っている。
- 採用した場合は、候補のブランチ（秒読みつきの確認で後始末のコミットを加えた場合はそれを含む）がmasterへ統合され、依存関係の節に挙げた3つのADR（起点の時点で後継があればその後継）が新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- 不採用の場合は、結論と測定記録だけをmasterへ残し、候補のブランチはmasterへ統合しない。
- ROADMAPの状態表が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)。
- Stockfish（コミット5c4cef0b）の`src/search.cpp`、`src/search.h`、`src/thread.cpp`、`src/timeman.cpp`、および`src/types.h`。前の探索の値の記録は`search.cpp`の259〜260行と644行、反復の値の初期化は319〜322行、変化回数の累計の減衰は354行、深さを増やさない回数は369〜370行、反復の深さの式は410〜411行、最善手が変わった深さの記録は533〜534行、4因子と積は592〜613行、合法手が1つの上限は615〜617行、停止と詰みの判定は622〜634行、根の手の平均値とノード数の累計は1474〜1495行、変化回数の計数は1531〜1532行。前の探索の値の初期化は`thread.cpp`の270〜275行。相手との残り時間の差は`timeman.cpp`の151〜156行、上限時間は159〜162行、先読みの25%増しは164〜165行。歩兵価値は`types.h`の199行。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`と`source/timeman.cpp`。4因子と積は1969〜1990行、合法手が1つの上限は1995〜1996行、停止の判定は2041行。`timeman.cpp`は相手の残り時間を参照しない。
- [持ち時間の効率的な使用](archived/time-management-efficiency.md)の「棄却した再構成の各段階」、[第2段階の事前判定](../measurements/time-management-efficiency/time-management-efficiency-stage2-bench.md)、および[第3段階のベンチ](../measurements/time-management-efficiency/time-management-efficiency-stage3-bench.md)。
- [棋力向上段階6](archived/strength-stage6.md)と[秒読みつき時計での持ち時間と秒読みの活用](byoyomi-time-usage.md)。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)、[SPRTによる棋力測定の手引き](../guides/sprt.md)、および[反復予算の調査](../research/spsa-iteration-budget.md)。
