# 浅い前向き枝刈りは深さと窓の条件をStockfishに合わせて王駒に利きが届くノードでは行わず、reverse futility pruningはβと判定用の評価値の加重平均を返す

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

通常探索は、浅いノードで枝を読まずに済ませる前向き枝刈りを持つ。
reverse futility pruning、futility pruning、およびrazoringは静的評価を、late move pruningは順序付けの後方にある手の数を、SEEによる枝刈りは到達升での駒の取り合いの見積りを、それぞれ根拠にする。
中将棋では王駒を相手の利きへ動かす着手も、王手を放置する着手も合法である（RULES.md第8条）。
王駒が取られ得る局面の静的評価は、最後の王駒の捕獲を詰みの値とする探索値から大きく外れる。
王駒の捕獲を禁じる規則はないので、王駒の升では、その升へ疑似利きを持つ相手駒の集合と、実際に王駒を捕獲できる相手駒の集合が一致する。
[棋力向上段階4](../plans/archived/strength-stage4.md)の[診断bench](../measurements/strength-stage4/strength-stage4-pruning-bench.md)では、非PVかつ詰み帯でないノードのうち、いずれかの王駒に利きが届くノードは残り深さ1、2、3で1.56%、1.87%、2.99%であり、最後の王駒だけに限ると1.52%、1.80%、2.99%だった。

[旧ADR](archive/skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)は、2026年10月18日に、futility pruning、late move pruning、SEEによる捕獲手の枝刈り、およびrazoringを残り深さ3以下の零窓のノードに限り、王駒に利きが届くノードを除くと決めた。
razoringはさらに残り深さ2以下に限り、静止探索の値がα以下のときだけその値を返していた。
futility pruningは、深さ別の余裕値とimprovingでないときの縮小率から境界値をノードごとに1回求め、静かな手に一律に当てていた。
同じADRは、reverse futility pruningをStockfishの条件で残り深さ18まで行い、判定用の評価値をそのまま返すと決めていた。
深さの上限3は、評価関数がPSTとFMだった時期に段階4で静的評価に基づく枝刈りを入れたときの値であり、その後に加えたSEEによる捕獲手の枝刈りとlate move pruningも同じ上限を引き継いでいた。

Stockfishでは、late move pruningとSEEによる枝刈りに深さの上限がなく、静かな手のfutility pruningはLMRの減深量を引いた深さであるlmrDepthが12未満のときに働き、いずれも根以外のPVノードでも働く（`src/search.cpp:1196-1259`）。
ただし、静かな手のfutility pruningと静かな手のSEEによる枝刈りは、根から前の反復の主変化を辿るfollowPVのPVノードでは行わない（同1227行）。
razoringは、allNodeで判定用の評価値が深さに比例する余裕値を引いたαより小さいときに、静止探索の値をそのまま返す（同1017-1018行）。
reverse futility pruningは、βと判定用の評価値の加重平均`(661β + 363e)/1024`を返す（同1033行）。
2026年10月17日に中止した現行の形でのSPSAでは、razoringの余裕値`RazoringMargin2`が範囲の上限に達し、`RazoringMargin1`も上限に近づいた。
余裕値が大きいほどrazoringは働かないので、これはNNUEの下で現行の形のrazoringが役に立っていない兆候と読める。

2026年10月17日のエンジンの開発者の決定により、Stockfishにある項目は入れてYaneuraOuにだけある項目は入れず、深さの上限や平均の重みのような値はStockfishの値で固定することにした。
同じ決定により、中将棋の規則から来る王駒への利きの除外と、成る手を枝刈りの対象から外す保護は残し、razoringのコードは外さずにStockfishの形に改め、調整し直す。
[浅い前向き枝刈りの深さと対象の拡張](../plans/archived/shallow-pruning-extensions.md)は、最初に旧ADRの「残り深さ3以下の零窓」という1つの判定を手法ごとの条件へ分け、その後で各手法の条件と式をStockfishの形に改めた。

## 決定

手を展開せずに飛ばす枝刈りは、late move pruning、静かな手のfutility pruning、捕獲手のfutility pruning、SEEによる捕獲手と王手になる静かな手の枝刈り、および静かな手のSEEによる枝刈りの5つである。
これらは、残り深さの上限を持たず、零窓のノードとPVノードの両方で、置換表の手以外の手に行う。
ただし、静かな手のfutility pruningと静かな手のSEEによる枝刈りは、followPVのPVノードでは行わない。
どの前向き枝刈りも、αとβがいずれも詰み帯と専用の帯の外にあり、手番側のいずれの王駒にも相手の疑似利きが届かないことを要求する。
専用の帯は、反復による勝敗を表すために詰み帯の内側に置いた評価値の範囲であり、[専用の帯のADR](reserve-a-path-dependent-band-for-repetition-outcomes.md)が定める。
手を飛ばす5つの枝刈りは、さらにそのノードで探索済みの最善値が負の詰み帯と専用の帯の外にあることも要求する。
これらの条件は旧ADRから引き継ぎ、成る手を対象にしない保護も起点から変えない。

lmrDepthは、手を読む深さ`d − 1`から、[LMRのADR](reduce-lmr-by-log-table-history-steps-and-stockfish-terms.md)の減深量のうち枝刈りより前に決まる部分`r_pre`を手単位に直して引いた符号付きの値`d − 1 − r_pre/100`である（`src/search.cpp:1203`）。
`d`は内部反復減深（IIR）を適用した後の残り深さ、`P`は歩兵価値、除算は0方向へ丸める整数除算である。
静かな手のfutility pruningは、捕獲手でも成る手でも王手になる手でもない手について、lmrDepthが12未満で、見積り`e + P × (FutilityBase + FutilityLmrDepth × lmrDepth + FutilityAboveAlpha × [e > α])/100`がα以下なら、その手を読まない。
`e`は補正後の静的評価であり、`[e > α]`は`e`がαより大きければ1、そうでなければ0を表す。
読まなかった手の見積りが最善値より大きく、最善値と見積りがともに詰み帯と専用の帯の外にあれば、最善値だけをその見積りに上げる（`src/search.cpp:1249-1251`）。
捕獲手と静かな手のSEEによる枝刈りと捕獲手のfutility pruningの式は[SEEと駒得の見積りによる枝刈りのADR](prune-captures-and-quiet-moves-by-see-and-capture-futility-with-depth-scaled-margins.md)が、late move pruningの上限と数え方は[late move pruningのADR](apply-late-move-pruning-at-all-depths-and-pv-nodes-counting-only-quiet-moves.md)が、王手になる手の判定は[王手の判定のADR](judge-gives-check-on-the-post-move-occupancy-without-precomputation.md)が定める。

razoringは、allNodeで、詰みを探す局面でなく、判定用の評価値が`α − P × RazoringDepthMargin × d/100`より小さいとき、静止探索を窓`(α, β)`で呼んでその値をそのまま返す。
深さの上限は持たない。
allNodeはPVノードでもcutNodeでもない零窓のノードであり、旧ADRの零窓の条件はこの条件に含まれる。
cutNodeとallNodeの定義は[null moveのADR](try-null-move-only-at-cut-nodes-under-stockfish-conditions-and-allow-tt-cutoffs-at-pv-nodes.md)にある。

reverse futility pruningの条件に使う2つの語を、先に定める。
ttPvは、PVノードであるか、置換表のエントリがPVの印を持つことをいう。
印はエントリの予約領域の1ビットに置き、付け方はStockfishに従う（`src/search.cpp:842, 1651-1657, 1741, 1908`）。
詰みを探す局面は、前の反復で確定した根の評価値の絶対値が、反復の深さ`D`について歩兵価値の`(361 + 105769/D²)`%以上である反復をいう。
2つの定数はStockfishの`seekMate`の750と220000を歩兵価値208で換算したものであり、Stockfishが棋力のための値ではないとして調整しないので固定する（`src/search.cpp:394-395`）。
詰みを探す局面では、razoringを行わない（同1017行）。

reverse futility pruningは、次の条件をすべて満たすノードで`(661β + 363e)/1024`を返し、その値を置換表に保存しない。
ここでの`e`は判定用の評価値である。

- 残り深さが18以下であり、詰みを探す局面では5以下である。
- ttPvでない。PVノードはttPvなので、零窓のノードに限られる。
- 置換表の手がないか、合法手であることを確かめた置換表の手が捕獲手である。
- αとβが詰み帯と専用の帯の外にあり、手番側のいずれの王駒にも相手の疑似利きが届かない。
- 判定用の評価値がβ以上であり、余裕値を引いてもβ以上である。

判定用の評価値がβ以上なので、返り値はβ以上`e`以下に収まる。
余裕値はStockfishの式（`src/search.cpp:1025-1030`）を歩兵価値の百分率の係数で表したものであり、深さに比例する項からimprovingとopponentWorseningの項を引き、補正量の絶対値に比例する項を足す。
深さに掛ける倍率の上限（歩兵価値の41%）、深さの上限18および5、および返り値の重み661と363は、Stockfishの値で固定する。
返り値の式以外は旧ADRから引き継ぐ。
判定用の評価値の定義は[静的評価のADR](key-correction-history-by-material-and-apply-it-to-all-static-eval-checks.md)に、improvingとopponentWorseningの定義は[improvingのADR](judge-improving-on-the-corrected-static-eval-and-use-it-in-null-move-pruning-but-not-futility-margins.md)にある。
null move pruningを試す条件は、[null moveのADR](try-null-move-only-at-cut-nodes-under-stockfish-conditions-and-allow-tt-cutoffs-at-pv-nodes.md)が定める。
旧ADRがnull move pruningには王駒への利きの判定を加えないとした決定は、そのADRが改め、手番側の王駒に相手の疑似利きが届くノードではnull moveを試さない。

## 検討した代案

浅い前向き枝刈りを残り深さ3以下の零窓のノードに限り続ける旧ADRの形は、Stockfishに合わせるというエンジンの開発者の決定により改めた。
上限3は評価関数がPSTとFMだった時期の値であり、NNUEの下では選び直していなかった。
静かな手のfutility pruningで深さ別の余裕値`FutilityMargin1`〜`3`とimprovingでないときの縮小率`NonImprovingFutility1`〜`3`を使い続ける案は、Stockfishの式が手ごとのlmrDepthを使い、improvingを使わないので採らなかった。
起点の深さ別の余裕値はSPSAの結果で歩兵価値の317%、264%、315%であり、深さについて単調でなかった。
razoringを残り深さ2以下に限り、深さ別の余裕値で判定して静止探索の値がα以下のときだけ返す旧来の形は、Stockfishの形に改めた。
razoringを外す案は、エンジンの開発者の決定によりコードを残して調整し直すので採らなかった。

YaneuraOuにだけある項は、エンジンの開発者の決定により入れなかった。
静かな手のfutility pruningの`151 × !bestMove`の項と、razoringの深さの2乗に比例する余裕値がこれに当たる。
Stockfishの浅い枝刈りの前提条件`pos.non_pawn_material(us)`（`src/search.cpp:1196`）は、ポーンだけの終盤のための条件であり、中将棋には相当する仕組みがない。
Stockfishに相当する仕組みがない事項はYaneuraOuに合わせるというエンジンの開発者の決定により、YaneuraOuと同じくこの条件を持たない（`yaneuraou-search.cpp:3585-3588`）。
reverse futility pruningの返り値をYaneuraOuの`(2β + e)/3`にする案（`yaneuraou-search.cpp:3227`）は、両エンジンで形や値が違う場合はStockfishに合わせるというエンジンの開発者の決定により採らなかった。
返り値の重みをSPSAで調整する案は、平均の重みはStockfishの値で固定するというエンジンの開発者の決定により採らなかった。

旧ADRで棄却した案は、この決定でも棄却したままとする。
reverse futility pruningで深さ1〜3に共通の1つの余裕値を補正前の静的評価から引く形は、残り深さの浅いノードと深いノードで別の余裕値を選べないので採らない。
深さの上限や余裕値の係数の上限をSPSAで調整する案は、上下限や深さの条件はStockfishの値で固定するというエンジンの開発者の決定により採らない。
王駒への利きの除外を外してStockfishの条件だけにする案は、除外が中将棋の規則から来るので採らない。
判定を最後の王駒に限る案は、除外率がほとんど変わらないので採らない。
reverse futility pruningの返り値をβへ丸める案は、境界としては常に正しいが、上位ノードが得る情報を減らすので採らない。

## 結果

診断で確かめたのはβ以上かどうかの判定が正しいことだけであり、返す値の正しさは確かめていないので、reverse futility pruningが返す値は、null move pruningやLMRの返り値と同じくfail-softの近似として扱う。
razoringが返す静止探索の値も、αを上回る場合を含めてそのまま上位へ渡る。
探索済みの最善値の条件により、各ノードで少なくとも1手は探索されるので、全手を枝刈りしたノードを合法手のない負けと取り違えない。
静かな手のfutility pruningで上げるのは最善値だけであり、最善手、α、最善応手列、および探索済みの手の記録は変えない。
見積りはα以下なので、上げた最善値もα以下に留まり、他の手がαを超えない限り置換表に保存する種別は上界のままである。

王駒に利きが届くノードの割合は残り深さ3以下の非PVノードでしか測っていないので、深さとPVノードへ適用を広げた後の除外率は分からない。
王駒への利きを調べる費用は係数で消せないので、枝刈りの効果が小さい構成では費用だけが残る。
`minase dev bench`の深さ3、5、7の全15局面の探索ノード数の合計は、条件を手法ごとに分けたコミットで起点と一致し、静かな手のfutility pruningを手ごとの判定に改めたコミットで直前より21.4%減った（[benchの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-bench.md)）。
razoringをallNodeの形にしたコミットでは6.9%増え、深さ7では12.4%増えたが、増えた理由は確かめていない。
reverse futility pruningの返り値を変えたコミットでは0.7%増えた。
設計書の変更全体では、1秒あたりの探索ノード数が起点より約42%下がった。
利きの逆引きと計算に関わる関数の自己時間の割合は、深さ12の探索で起点の約13%から約27%へ増えた。
このうち王手の判定は探索時間の約4%に当たり、残りは、枝刈りを判定するノードと手が増えたことによる王駒への利きの判定とSEEだと考えられるが、自己時間を呼び出し元ごとには分けていない。

条件の判定は`crates/minase/src/search/alphabeta/negamax.rs`の`negamax`にあり、王駒への利きは他の条件が成立したときだけ`royal_under_attack`で調べてノード内で再利用する。
静かな手のfutility pruningの式は`pruning.rs`の`FutilityCoefficients::futility_pruning_value`、最善値の更新の条件は`should_raise_futility_best_score`、lmrDepthの上限12は`QUIET_FUTILITY_LMR_DEPTH_LIMIT`、lmrDepthは`pruning_lmr_depth`にある。
razoringの余裕値は`razoring_margin`に、reverse futility pruningの余裕値と返り値は`reverse_futility_margin`と`reverse_futility_value`に、深さの上限は`REVERSE_FUTILITY_MAX_DEPTH`と`SEEK_MATE_REVERSE_FUTILITY_MAX_DEPTH`にある。
詰みを探す局面の判定は`is_seek_mate`にあり、ttPvの印は`tt.rs`の`Hit`の`is_pv`である。
係数は`params.rs`の`FutilityBase`、`FutilityLmrDepth`、`FutilityAboveAlpha`、および`RazoringDepthMargin`であり、Stockfishの値を換算した79、57、43、および164から調整する。
`FutilityMargin1`〜`3`、`NonImprovingFutility1`〜`3`、および`RazoringMargin1`〜`2`は廃した。
`RazoringDepthMargin`の開始値は、YaneuraOuの換算値の約1/5、起点の深さ1あたりの余裕値の約1/9〜1/16である。

2026年10月18日から19日にかけての30,000ペアのSPSAでは、razoringの余裕値`RazoringDepthMargin`が開始値164から2,197へ、静かな手のfutility pruningのlmrDepthあたりの増分`FutilityLmrDepth`が57から219へ上がり、浅い前向き枝刈りを控える側へ動いた（[SPSAの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions.md)）。
razoringの余裕値は、旧来の深さ1の余裕値2,622に近く、razoringはほとんど働かない値に戻った。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、[前の設計書](../plans/archived/search-pruning-forms.md)を採用した版`a892a5e6`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、1,381ペアで`H1`となった（[STCの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
