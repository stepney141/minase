# 静止探索の補正と枝刈りの形の見直しの設計書

## 先に読む要約

本書は、静止探索（quiescence search、読みの末端で捕獲手だけを読み進め、駒の取り合いの途中で評価しないための探索）の評価と枝刈りの式を、チェスのStockfishと将棋のYaneuraOu（やねうら王）が共通に持つ形へ寄せ、現行のmasterより強くなるかを確かめる。
[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)によれば、minaseの静止探索は、stand-pat（何も指さずに現局面の静的評価を探索値の下限として採る選択肢）とdelta pruning（取る駒の価値と余裕値を足してもαに届かない捕獲手を読まない枝刈り）に補正前の静的評価を使い、1ノードで読む捕獲手の数を両エンジンと違う数え方で制限している。
本書が変えるのは4つである。
1つ目に、delta pruningの余裕値を、SEE（static exchange evaluation、到達升での駒の取り合いの損得の見積り）による枝刈りの閾値にも使う。
2つ目に、correction history（探索値と静的評価のずれを学習して静的評価を補正する手法）の補正を、stand-patとdelta pruningの基準に加える。
3つ目に、手数制限（move count pruning）を、実際に読んだ捕獲手の数ではなく、候補の列から取り出した順番で数える。
4つ目に、手数制限の例外である取り返しを、直前の手が捕獲でなくても、その手の到達升の駒を取る手に広げる。
2つ目は補正に掛ける倍率を新しい係数にし、補正を使わない現行の値0から始める。
1つ目、3つ目、4つ目は係数を持たない形の置き換えであり、両エンジンの形をそのまま開始時の形とする。
全項目を1つの候補にまとめ、既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整し、短時間条件STCと長時間条件LTCの2段階の逐次検定（段階ゲート）で採否を決める。
完了条件は、候補の採否が測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案した。
連鎖の1つ前の[correction historyの表の見直し](correction-history-tables.md)の完了を待って着手し、着手は利用者の指示による。
次の一手は、前計画の完了後に起点を確定し、フェーズ1（形の変更の実装）を始めることである。

## 目的

本書が確かめるのは、「静止探索の評価と枝刈りの式を両エンジンの形へ寄せ、新しい形の係数を調整すれば、現行の形で調整した探索より強くなる」という仮説である。
minaseの探索では、静止探索のノードが全ノードの大半を占める（[手数制限のADR](../adr/limit-searched-captures-in-qsearch.md)）。
静止探索は深さの上限なく再帰するので、1ノードあたりの枝刈りの差が探索全体へ積み重なる（[教訓](../lessons/recall-loss-does-not-bound-recursive-pruning.md)）。
現行の手数制限は、delta pruningとSEEの枝刈りを通過して実際に読んだ捕獲手を数えるので、枝刈りされた候補が多いノードほど後方の候補まで読む。
両エンジンは枝刈りの前に候補の順番を数え、3番目以降の候補を、取り返しなどの例外を除いて読まない（Stockfish `src/search.cpp:1816-1826`）。
[手数制限のADR](../adr/limit-searched-captures-in-qsearch.md)は、この数え方を未測定の候補として残している。
また、通常探索の補正の適用先を広げる[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)は、静止探索への適用を「すべての静止探索ノードの値を変え、通常探索の項目と効果を切り分けられない」として別の候補に分けた。
本書はその候補を、静止探索の他の形の差と合わせて1つの候補として測る。

## 適用範囲

本書の用語を次のように定める。

- **前計画**は、連鎖の1つ前の[correction historyの表の見直し](correction-history-tables.md)である。
- **起点**は、前計画が完了した後、フェーズ1を始める時点のmasterの先頭のコミットである。前計画の候補が採用されていれば、その調整値と補正の形を持つ。
- **形の変更**は、本書の4項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で表（`crates/minase/src/search/alphabeta/params.rs`）の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **補正値**は、起点の通常探索のfutility pruningが静的評価に足している補正の値である。現行では`CorrectionTable::read`の値であり、前計画が鍵を加えた場合は、起点でfutility pruningが使う合成値を指す。
- **候補の列**は、静止探索のノードで`QsearchBuffers::next`が返す捕獲手の列である。置換表の手を先頭に置き、残りを取る駒の価値の群の降順に並べる。入口のdelta pruningで除いた捕獲手は生成されないので、この列に含まれない（[ADR](../adr/generate-qsearch-captures-by-value-group-per-piece.md)）。
- **詰み帯と専用の帯**は、詰みまでの手数を表す評価値の範囲と、千日手などの反復の結果を表す評価値の範囲（`crates/minase/src/search/mod.rs`の`MATE_THRESHOLD`と`REPETITION_BAND_START`）である。通常の評価値はこれらの外にある。
- **例外の手**は、相手の最後の王駒を取る手、取り返し、および獅子が1手で2枚を取る手である。取り返しの定義は本書のQ4で変える。

現行の静止探索は、次の順に判定する（`crates/minase/src/search/alphabeta/quiesce.rs`の`quiesce`）。
最大plyでは補正前の静的評価を返す。
補正前の静的評価をstand-patとし、β以上なら置換表を使わずに返す。
αをstand-patまで引き上げ、`α − stand-pat − 余裕値`以下の価値しか取れない捕獲手を入口のdelta pruningで生成から除き、候補がなければstand-patを返す。
置換表を照会した後、候補の列を順に調べる。
相手の最後の王駒を取る手は枝刈りせず、詰みの値を使う。
それ以外の候補は、delta pruning、SEEの値が0未満なら読まない枝刈り、手数制限の順に判定する。
手数制限は、実際に読んだ捕獲手が`QsearchMoveLimit`手に達した後の候補を、直前の相手の捕獲手の到達升の駒を取る手と獅子の2枚取りを除いて読まない。

対象は次の4項目である。

- SEEによる枝刈りの閾値へのdelta pruningの余裕値の流用（Q1）。
- correction historyの補正のstand-patとdelta pruningへの適用（Q2）。
- 手数制限（move count pruning）の数え方の変更（Q3）。
- 取り返しの例外の定義の拡大（Q4）。

次の事項は対象外とする。

- delta pruningとSEEで枝刈りした手の見積りで最善値を引き上げること（Stockfish `src/search.cpp:1834, 1841`）。minaseは入口のdelta pruningで対象の捕獲手を生成しないので、引き上げに要る値を持たない。両エンジンの形を再現するには除いた捕獲手を生成する必要があり、段階生成の速度上の利点を失う（[ADR](../adr/generate-qsearch-captures-by-value-group-per-piece.md)）。ループ内で枝刈りする手は置換表の手とαが上がった後の手だけなので、ループ内だけで引き上げても両エンジンの形にはならない。
- SEEの下限を負にして小さな損の捕獲を読む形（Stockfishは歩兵価値の約36%、YaneuraOuは約81%の損を許す）。現行の下限0と両エンジンの下限の違いは形ではなく値の違いである。Q1の閾値は現行の下限0と組み合わせて定めているので、下限を変えるとQ1の閾値の式も組み直す必要があり、別の候補とする。
- 王手になる捕獲手と成る捕獲手を例外に加えること。設計判断の表を参照。
- 取り返しと獅子の2枚取りをdelta pruningとSEEの枝刈りの例外にすること。設計判断の表を参照。
- stand-patがβ以上のときに返り値をβへ寄せること（Stockfish `src/search.cpp:1783, 1904`、YaneuraOuは`(bestValue + β)/2`、`yaneuraou-search.cpp:4817, 5094`）。αβの窓の外の値をそのまま返す方式（fail-soft）の返り値の形であり、[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)がreverse futility pruningの返り値を対象外としたのと同じく、別に扱う。
- 置換表の値をstand-patの代わりに使うこと（Stockfish `src/search.cpp:1768-1770`）。minaseは置換表の照会をstand-patの判定の後に置いている（[ADR](../adr/check-tt-after-stand-pat-and-trim-lion-captures-in-qsearch.md)）。
- 王駒が狙われた局面でstand-patを使わず全合法手を読むこと。約120 Elo弱かった（[ADR](../adr/use-stand-pat-when-royals-are-attacked.md)）。
- YaneuraOuの静止探索の1手詰め判定（`yaneuraou-search.cpp:4766`）、静止探索の深さの上限（[ADR](../adr/omit-quiescence-depth-limit.md)）、および静止探索での補正表の更新。両エンジンとも補正表は通常探索でだけ更新する（Stockfish `src/search.cpp:1315, 1672`、YaneuraOu `yaneuraou-search.cpp:4417`）。

## 依存関係

起点は、[correction historyの表の見直し](correction-history-tables.md)が完了した後のmasterの先頭である。
前計画の候補が採用されたかどうかにかかわらず、起点の表の既定値をSPSAの開始値とする。
前計画は補正値の鍵と大きさを変え得るので、Q2が読む補正値も、本書の検証（起点との探索ノード数の一致）の基準も、前計画の結果で変わる。
このため、実装を含めて前計画の完了後に始める。
設計書の順序は、[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)から始まり、浅い枝刈り、history、correction historyの表、本書、aspiration windows、時間管理と続く連鎖として提案したものであり、最終的な順序は利用者が決める。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、同時対局数は16を上限とする。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は次のADRの決定の一部を変える。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確認し、新しいADRで置き換える。
決定の一部だけを変えるので、変えない部分の決定も新しいADRへ引き継ぐ。

- [手数制限のADR](../adr/limit-searched-captures-in-qsearch.md)は、置換表の手を含めて実際に読んだ捕獲手を数え、取り返しを直前の相手の捕獲手の到達升で判定すると決めた。Q3は数え方を候補の列の順番に、Q4は取り返しの判定を直前の手の到達升に変える。上限の値は2026年10月4日にSPSAで決めたものであり、既定の評価関数がNNUEへ置き換わる前の測定である。同じADRは、Stockfishのように生成した全捕獲手を数えて上限を2とする方式を未測定の候補として残している。
- [correction historyの鍵と適用先のADR](../adr/key-correction-history-by-material.md)は、補正を置換表の保存値と静止探索のstand-patに使わないと決めた。Q2はstand-patとdelta pruningの基準に補正を加える。この決定の根拠となった予測誤差の診断と採否の測定は、既定の評価関数がPSTだった時期のものであり、静止探索への適用は測っていない。

次のADRは本書で変えない。
[delta pruningに成り益を加えないADR](../adr/keep-delta-pruning-without-promotion-gain.md)は、取る駒の価値に成り益を足す修正を棄却した。成る手をdelta pruningの例外にするStockfishの形は別の仕組みだが、本書は採らない。
[SEEの判定不能の範囲のADR](../adr/mark-only-lion-rule-dependent-exchanges-unknown-in-see.md)が定める、獅子の規則に依存して判定不能となる捕獲手を枝刈りしない規則は、Q1の閾値でも保つ。

## 設計判断

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 4項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとにSPSAと段階ゲートを繰り返す案は、1項目あたりSPSAの約75時間に事前確認、事前対局、STC、およびLTCを加えた約82〜114時間を要するので採らない。このため、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| SEEの閾値（Q1） | 両エンジンは、delta pruningの基準`futilityBase = 静的評価 + 余裕値`を使い、SEEの値が`α − futilityBase`未満の捕獲手を読まない（Stockfish `src/search.cpp:1839-1843`、YaneuraOu `yaneuraou-search.cpp:4965-4967`）。minaseの静止探索はSEEの値が0未満の捕獲手を読まないので、閾値を`α − futilityBase`と0の大きい方にする。これは両エンジンの判定と現行の下限0を合わせた形である。係数は加えず、余裕値は既存の`DeltaMargin`を共有する。両エンジンも同じ`futilityBase`を2つの判定に使うからである。閾値に倍率の係数を掛ければ倍率0で現行と同じ探索から始められるが、その係数は開始時の一致のためだけの装置になるので置かない。開始時から探索が変わるので、benchのノード数の変化を記録する。 |
| SEEの判定の拡張 | 現行の`see_prunes`は非負の余裕値`m`だけを受け取り、交換の値が`−m`未満なら真を返す。Q1は正の閾値を要するので、符号付きの閾値`T`を受け取り、交換の値が`T`未満なら真を返す契約へ改める。既存の呼び出しはすべて`T = −m`へ移し、挙動を変えない。通常探索の浅いノードのSEE pruning（`negamax.rs`、深さ別の余裕値`m`を渡す）は`T = −m`、静止探索（`pruning.rs`の`capture_is_pruned_by_see`）と手選択器のkiller手の前置き（`ordering.rs`）は余裕値0なので`T = 0`になる。先行の[浅い枝刈りの拡張](shallow-pruning-extensions.md)が採用されていれば、そのS2（capture historyの項を足した余裕値）とS4（静かな手のSEE）の呼び出しも`T = −余裕値`へ移す。等号で枝刈りしないこと、獅子の規則に依存して判定不能なら枝刈りしないこと、および2段階移動の手を枝刈りしないことは変えない。最初の取り返しで判定を打ち切る早期判定（`see.rs`の`see_prunes`の冒頭）は非負の余裕値を前提に導いたので、正の閾値でも交換列を最後まで逆算した値と同じ判定になるように導き直し、既存の参照照合テストを負、0、正の閾値の例へ広げる。 |
| 静止探索での補正（Q2） | stand-pat、αの引き上げ、入口と候補ごとのdelta pruning、およびQ1の`futilityBase`に、補正前の静的評価に補正値の`QsearchCorrectionScale`%を足した値を使う。両エンジンは通常探索と同じ補正後の評価を静止探索でも使う（Stockfish `src/search.cpp:1754-1776`、YaneuraOu `yaneuraou-search.cpp:4730, 4791`）。倍率の係数を置くのは、開始値0で現行と同じ探索にするためだけではない。補正表は最善手が捕獲手でなく王駒に利きが届かないノードだけで学習するので（[ADR](../adr/key-correction-history-by-material.md)）、捕獲が続く静止探索の局面に同じ大きさの補正が合うかは分からず、倍率をSPSAに選ばせる。[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF2は、通常探索の枝刈りに係数なしで補正を使う。F2の適用先は補正表が学習するのと同じ種類の通常探索のノードなので、等倍で使う根拠がある。静止探索のノードにはその根拠がないので、本書は扱いを分ける。範囲の下端0を開始値にするのは、0が補正を使わない現行の挙動であり、負の倍率には意味がないからである。補正後の値は評価関数の出力の範囲（`crates/minase/src/eval/nnue/`の`EVALUATION_LIMIT`）に収め、詰み帯と専用の帯へ入れない。王駒に利きが届くかは調べず、全ノードで同じ倍率を使う。stand-patは王駒が狙われた局面でも使うと決めており（[ADR](../adr/use-stand-pat-when-royals-are-attacked.md)）、利きの判定を全静止探索ノードに加えると費用が大きいからである。 |
| 補正を変えない箇所 | 最大plyに達したノードの返り値は補正前の静的評価のままとする。Stockfishも同じ箇所で`evaluate(pos)`をそのまま返す（`src/search.cpp:1729-1730`）。補正表の更新、更新に使う差（探索値と補正前の評価の差）、および置換表の保存の規則も変えない。 |
| 補正の鍵の保守 | 現行の静止探索は着手後のplyの鍵（`material_keys`）を更新しないので、補正値を読めない。静止探索の着手の後にも、起点の`search_move`が更新するすべての補正の鍵を、同じ関数で更新する。前計画が鍵を加えた場合も、その全部を対象にする。 |
| 手数制限の数え方（Q3） | 候補の列で現在の候補より前に取り出した候補の数を数え、それが上限`QsearchMoveLimit`以上なら、例外の手を除いて読まない。置換表の手、例外の手、および枝刈りした候補も数える。両エンジンは置換表の手を含む全候補を枝刈りの前に数え、3番目以降を読まない（Stockfish `src/search.cpp:1816, 1825-1826`、YaneuraOu `yaneuraou-search.cpp:4897, 4928`）。前に取り出した候補の数は0から数えるので、上限2は3番目以降の候補を読まないことに当たる。数える数が現行以上になるので、上限が大きく手数制限が働かない場合を除き、どの上限の値でも現行と同じ探索にはならない。 |
| 手数制限の開始値と範囲 | 開始値は現行の値4とする。上限4の新しい数え方は、両エンジンの値2より多くの手を読み、数える候補が枝刈りした候補にも広がるので、現行の上限4と比べて同じか少ない数の手を読む。現行の値を当てはめるのは、枝刈りを通過した候補だけを数える現行の形で調整した値が、入口のdelta pruningで低価値の捕獲を除いた後の候補の列に対して、最も近い既知の値だからである。範囲は現行の1〜7を1〜16へ広げ、両エンジンの2と、現行の値の数倍を含める。 |
| 判定の順序 | 最後の王駒を取る手を先に判定して詰みの値を使い、それ以外は手数制限、delta pruning、SEEの枝刈りの順に判定する。Q3の後は手数が枝刈りの結果に依存しないので、順序は読む手の集合を変えず、計算の費用が最も小さい判定を先に置く。Stockfishも手数制限を先に判定する（`src/search.cpp:1825`）。 |
| 取り返しの定義（Q4） | 取り返しを、直前の手の到達升にある駒を取る手とし、直前の手が捕獲でなくてもよいことにする。両エンジンは直前の手の到達升`prevSq`を、手の種類によらず使う（Stockfish `src/search.cpp:1799, 1822`、YaneuraOu `yaneuraou-search.cpp:4851, 4915`）。静止探索の中の手はすべて捕獲手なので、差が出るのは通常探索の非捕獲手の直後に静止探索へ入ったノードと、razoringが呼ぶ静止探索だけである。根とnull moveの直後に取り返しの対象を持たないこと、居喰いと獅子の2段階移動でも取った駒の升の集合に直前の到達升が含まれるかで判定することは変えない。 |
| 例外に残す手 | 最後の王駒を取る手と獅子の2枚取りは例外に残す。最後の王駒を取る手は終局の値を持ち、チェスには対応する手がない。獅子の2枚取りは中将棋に固有の手であり、[手数制限のADR](../adr/limit-searched-captures-in-qsearch.md)のbenchで実際に働いた例外である。フェーズ1で、Q3とQ4の後に各例外が働く回数を数え、働かない例外があれば測定記録に残す（[教訓](../lessons/measure-conservative-guard-coverage.md)）。 |
| 例外に加えない手 | 成る捕獲手は例外に加えない。Stockfishは成る手をdelta pruningと手数制限の例外にするが、YaneuraOuはこの条件を外したほうが強かったとして、通常のビルドでは外している（`yaneuraou-search.cpp:4916-4921`、`config.h:337`）。成り益をdelta pruningに足す修正は、minaseでもわずかに弱い方向の結果だった（[ADR](../adr/keep-delta-pruning-without-promotion-gain.md)）。王手になる捕獲手も例外に加えない。中将棋は王手放置が合法であり（RULES.md第8条）、王手が応手を強いる性質を持たない。minaseは静止探索で王手の判定を行っておらず、全候補で利きを調べる費用も加わる。 |
| 例外の適用範囲 | 取り返しと獅子の2枚取りは、現行どおり手数制限だけの例外とし、delta pruningとSEEの枝刈りは適用する。両エンジンは取り返しをdelta pruningとSEEの閾値の判定からも除くが、minaseで同じことをするには、入口のdelta pruningで直前の到達升の駒を価値によらず生成対象に残す変更が要る。これは本書の4項目の外にある挙動の変更なので、別の候補とする。Q1の閾値はdelta pruningを交換の見積りへ広げたものなので、delta pruningと同じく最後の王駒を取る手だけを除く。 |
| SPSAの対象 | 起点の表の調整対象の全係数（`FmScale`を除く）に新しい1係数を加える。`QsearchMoveLimit`は意味と範囲を変えた既存の係数として含める。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の係数を含むので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。3,750反復、1反復8ペア（計30,000ペア）、持ち時間10秒＋0.1秒、同時対局数16、`c_end`（最後の反復の摂動幅）は範囲の1/6、`r_end`（最後の反復の学習率）は0.002である（[SPSAの手引き](../guides/spsa.md)）。30,000ペアは候補を1つ作るための予算であり、最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。先行の設計書をすべて採用した場合、本書の時点の係数の数は85前後になる（起点の表で数え直す）。30,000ペアでこの数の係数を十分に調整できるかは未検証であり、不採用はこの予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。 |

## 変更の内容

以下の式で、`P`は探索用の歩兵価値（評価関数の重みファイルが持つ値）、`v`は補正前の静的評価、`c`は補正値、`α`はその時点のαを表す。
`Δ`はdelta pruningの余裕値`P × DeltaMargin/100`である。
整数除算は、`params.rs`の規約どおり0方向へ丸める。式の`\mathrm{trunc}`はこの丸めを表す。

### Q1　SEEの閾値（futilityBaseによるSEE pruning）

delta pruningの基準を`F = s + Δ`とし（`s`はQ2のstand-pat）、SEEの閾値`T`を次の式にする。

```math
T = \max(0, \alpha - F)
```

最後の王駒を取る手を除く候補は、SEEの値が`T`未満なら読まない。
獅子の規則に依存して判定不能な手と2段階移動の手は、現行どおり読む。
αはstand-pat以上なので`α − F`は`−Δ`以上であり、`T`が正になって現行より多く枝刈りするのは、αがstand-patを余裕値より大きく上回るノードだけである。

### Q2　静止探索のstand-patへの補正（correction history）

stand-patを次の値にする。

```math
s = \mathrm{clamp}\left(v + \mathrm{trunc}\left(\frac{c \times \mathrm{QsearchCorrectionScale}}{100}\right), -\mathrm{EVALUATION\_LIMIT}, \mathrm{EVALUATION\_LIMIT}\right)
```

`s`を、β以上で返す判定、αの引き上げ、入口のdelta pruningの閾値`α − s − Δ`、候補ごとのdelta pruningの判定`s + 取る駒の価値 + Δ ≤ α`、およびQ1の`F`に使う。
最善値の初期値と、候補がないときの返り値も`s`である。
静止探索の着手の後に、起点の`search_move`と同じ方法で補正の鍵を更新する。
`QsearchCorrectionScale`が0のとき、現行と同じになる。

### Q3　手数制限の数え方（move count pruning）

候補の列で現在の候補より前に取り出した候補の数を`n`とする。
最後の王駒を取る手でも例外の手でもない候補は、`n`が`QsearchMoveLimit`以上なら読まない。
`n`には、置換表の手、例外の手、およびdelta pruningまたはSEEで枝刈りした候補を含める。
入口のdelta pruningで生成しなかった捕獲手は候補の列に現れないので数えない。
これらは取る駒の価値が閾値以下の捕獲であり、群の降順の列では生成された候補より後に来るので、数えても前の候補の順番は変わらない。

### Q4　取り返しの例外（recapture）

通常探索の`search_move`と静止探索は、子のノードへ渡す取り返しの対象を、捕獲の有無によらず指した手の到達升にする。
根と、null moveの直後のノードは対象を持たない。
`Searcher::previous_capture`は直前の手の到達升を表すことになるので、名前とdocコメントを内容に合わせる。

### 新しい係数と範囲を変える係数

| 係数 | 開始値 | 範囲 | 範囲の根拠 |
|---|---|---|---|
| `QsearchCorrectionScale`（静止探索で補正値に掛ける倍率、百分率） | 0 | 0〜150 | 両エンジンは通常探索と同じ補正を使うので100に当たる。開始値0は範囲の下端であり、補正を使わない現行の挙動である。負の倍率には意味がないので範囲に含めない |
| `QsearchMoveLimit`（候補の列の順番の上限、手） | 4 | 1〜16 | 現行の範囲は1〜7で、意味は読んだ捕獲手の数だった。両エンジンは2である（Stockfish `src/search.cpp:1825`、YaneuraOu `yaneuraou-search.cpp:4928`）。設計判断の表を参照 |

`DeltaMargin`（現行355、歩兵価値に対する百分率）はQ1の閾値にも使われるようになるが、範囲は変えない。
両エンジンの`futilityBase`の余裕値は、Stockfishが306で歩兵価値208の約147%、YaneuraOuが328で歩兵価値90の約364%であり（Stockfish `src/search.cpp:1794`、YaneuraOu `yaneuraou-search.cpp:4837`）、現行の範囲50〜500に含まれる。

## 所要時間の見積り

時間は、同時対局数16の測定機での見積りである。
SPSAは、同時対局数16で3,000ペアを完走した実績（[SPSAの手引き](../guides/spsa.md)の27,138秒、1ペア約9秒）に基づき、30,000ペアを約75時間（約3日）とする。2026年10月17日の実行では1反復あたり約1分で進んだが、これは暫定の観測である。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 形の変更の実装と検証（codexへ委任） | 半日から1日程度 |
| 2 | 時間切れの事前確認（13反復、104ペア） | 約15分 |
| 3 | SPSA（30,000ペア） | 約75時間 |
| 4 | 反映、テスト、および事前対局（32ペア） | 約1時間 |
| 5 | 候補のSTC | 約1〜8時間 |
| 6 | 通過した場合のLTC | 約5〜30時間 |

測定機の占有は段階2から段階6までの和であり、STCで棄却される場合に約77〜84時間（約3.2〜3.5日）、LTCまで進む場合に約82〜114時間（約3.4〜4.8日）となる。

## 実装フェーズ

基本シードは、時間切れの事前確認を4400000000、セッションを4450000000、事前対局を4500000000、STCを4550000000、LTCを4600000000と仮に定める。
これらは起案時点で、保存済みの測定とSPSAの基本シード（`data/matches/`と`data/spsa/`の`manifest.json`）、および他の設計書が予約した値から、各測定のペア数以上離れた未使用の値である（[教訓](../lessons/derive-seed-adjacent-collision.md)）。
着手時に同じ方法で未使用であることを確かめ直し、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　形の変更の実装

起点からブランチ`qsearch-forms`を切り、worktree `data/worktrees/qsearch-forms`で作業する。
`see_prunes`の契約の変更、Q2、Q1、Q3、Q4の順に、1つを1コミットにする。
挙動を変えない`see_prunes`の契約の変更と、開始値で探索を変えないQ2を先に入れ、探索を変えるQ1、Q3、Q4を後に置くのは、それぞれによる変化を分けて記録するためである。
`see_prunes`の契約の変更とQ2のコミットでは、`minase dev bench`の全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
Q1、Q3、Q4の各コミットでは同じbenchのノード数を記録し、直前のコミットとの差を残す。
Q2は全静止探索ノードで補正値を読み、鍵を更新するので、`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点と形の変更で交互に3回ずつ測り、探索速度の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
Q2の鍵の更新は、係数0では補正値に0を掛けるのでbenchの一致では検証できない。
そこで、`invariants`フィーチャの検査（`quiesce`の累算値の検査と同じ型）とテストで、静止探索の各着手の後の鍵が、局面から計算し直した`material_key`の値と一致することを、居喰い、獅子の2枚取り、および成る捕獲を含めて確かめる。
起点で補正の鍵が増えていれば、そのすべてを同じ方法で確かめる。
`--features search-stats`のbenchで、Q1の閾値が新たに枝刈りした候補の数、Q3の手数制限が読まなかった候補の数、例外ごとに手数制限を免れた候補の数、およびQ4で新たに取り返しとなった候補の数を数え、測定記録に残す。これらの計数は`crates/minase/src/search/stats.rs`の項目として加える。
Q2は開始値で働かないので、倍率を100にしたときに、stand-patがβ以上で打ち切ったノードの数がどれだけ変わるかも、コミットしない一時的な変更で数えて記録する（[教訓](../lessons/measure-feature-activation-before-sprt.md)）。
新しい係数は`params.rs`の表へ追加し、`QsearchMoveLimit`の範囲とdocコメントを改め、調整用ビルド（`--features tuning`つきのビルド）が`Tune_<係数名>`のUSIオプションとして宣言することを確かめる。
`crates/minase/src/search/alphabeta/tests/qsearch_move_limit.rs`のテストは、削除せず新しい数え方と取り返しの定義に合わせて書き直す。
新しい式の単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
`see_prunes`は、負、0、正の閾値のそれぞれで、交換列を最後まで逆算した参照値との照合テストで確かめる。
`quiesce`、`see_prunes`、`Searcher::previous_capture`、および`correction.rs`のモジュールのdocコメントを、新しい契約に合わせて書き直す。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過、ノード数の一致と記録、静止探索の鍵の一致、探索速度の記録、および発動回数の記録である。

### フェーズ2　パラメーターファイルと時間切れの事前確認

形の変更の最後のコミット（Q4まで入ったコミット）でビルドした`minase`を`data/spsa/qsearch-forms-runner/`へ複製してrunner（測定を実行するプログラム）として固定し、以後のフェーズはこのrunnerで実行する。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除したファイルを`data/spsa/qsearch-forms.params`とする。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、使い捨てのスクリプトで機械的に突き合わせる。

続いて、SPSAの手引きの第3節に従い、時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
写しの開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150へ書き換えた`data/spsa/qsearch-forms-safety.params`で、13反復（104ペア）を実行する。
起点で時間管理の係数の名前や範囲が変わっていれば、起点のSPSAの手引きの第3節に従う。

```console
<固定したminase> spsa \
  --run-dir data/spsa/qsearch-forms-safety --seed 4400000000 \
  --engine commit:<形の変更> --params data/spsa/qsearch-forms-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`qsearch-forms-safety`に残す。
完了条件は、時間切れとエンジン異常が0件であることと、測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<固定したminase> spsa \
  --run-dir data/spsa/qsearch-forms --seed 4450000000 \
  --engine commit:<形の変更> --params data/spsa/qsearch-forms.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 3750 --pairs-per-iteration 8
```

最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`qsearch-forms`に、SPSAの手引きの第6節の項目とともに残す。
最終値を丸めた整数が範囲の上限または下限になった係数を示す。
`QsearchCorrectionScale`については、最終値が開始値（現行と同じ挙動）からどれだけ離れたかを、`c_end`に対する比で示す。
この比は、どの形の変更が使われたかを読むための目安であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

SPSAの手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、テストの前提が新しい既定値でも成り立つようにテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で握りつぶさない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

採否測定の前に、候補が時間切れやエンジン異常を起こさないことと、時間の使い方の変化を確かめるため、STCの条件で候補と起点の32ペアの事前対局を行う。

```console
<固定したminase> match run \
  --run-dir data/matches/qsearch-forms-smoke --seed 4500000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 elo --pairs 32
```

候補と起点の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ（反復深化で読み終えた深さ）、および探索を止めた理由の分布を集計して、測定記録`qsearch-forms-smoke`に残す。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、および測定記録の保存である。

### フェーズ5　採否測定

```console
<固定したminase> match run \
  --run-dir data/matches/qsearch-forms-stc --seed 4550000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

STCの判定は[SPRTの手引き](../guides/sprt.md)の振分け規則に従う。
候補が有意に強い`H1`ならLTCへ進み、改善を示せない`H0`なら不採用とし、3,000ペアで判定保留ならその時点の対数尤度比の符号でLTCへ進むかを決める。
LTCは測定名の接尾辞を`-ltc`、基本シードを4600000000、持ち時間を`time=60000+200`とし、`--max-pairs`を付けない。
LTCで`H1`となり、エンジン異常、時間切れ、および拒否着手が0件なら採用し、`H0`なら不採用とする。
LTCの判定保留は、手引きのとおり`--resume`と`--target-pairs`で延長する。
結果は測定名ごとの測定記録に残す。
完了条件は、採否の確定と測定記録の保存である。

## 検証

- `see_prunes`の契約の変更とQ2の各コミットで、`minase dev bench`の全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。
- 静止探索の各着手の後の補正の鍵が、局面から計算し直した値と一致すること。
- Q1、Q3、Q4の各コミットで、benchのノード数の差、Q1の閾値で枝刈りした候補の数、例外ごとの発動回数、および手数制限で読まなかった候補の数が記録されていること。
- `see_prunes`が、負、0、正の閾値のそれぞれで、交換列を最後まで逆算した参照値と同じ判定を返すこと。既存の呼び出しがすべて`T = −余裕値`へ移っていること。
- 新しい係数が調整用ビルドの`Tune_`オプションとして宣言され、`QsearchMoveLimit`の範囲が1〜16に改まり、パラメーターファイルの開始値と範囲が表と一致すること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`が通過すること。
- 時間切れの事前確認、SPSA、事前対局、および採否の測定で、エンジン異常、時間切れ、および拒否着手が0件であること。最初の測定で1件でも出た場合は、測定を止めて原因を調べる。

## 完了条件

- 形の変更の検証結果、時間切れの事前確認、SPSAの結果、事前対局、および採否が、測定記録に残っている。
- 採用した場合は、候補のブランチがmasterへ統合され、依存関係の節に挙げたADR（起点で有効な後継を含む）が、変えない部分の決定を引き継いだ新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- 不採用の場合は、結論と測定記録だけをmasterへ残し、候補のブランチはmasterへ統合しない。
- ROADMAPの状態表が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)。
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。補正値は95〜118行、静止探索の補正後の評価は1754〜1776行、stand-patの返り値は1780〜1789行と1903〜1904行、`futilityBase`は1794行、`prevSq`は1799行、手数の数え方と枝刈りの例外は1816〜1853行。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。静止探索の補正後の評価は4730行と4791行、stand-patの返り値は4817行、`futilityBase`は4837行、`prevSq`は4851行、手数の数え方と枝刈りの例外は4897〜4989行。成る手の例外は`#if STOCKFISH`の中にあり（4916〜4921行）、通常のビルドでは`source/config.h`の337行のとおり無効である。
- [手数制限のADR](../adr/limit-searched-captures-in-qsearch.md)、[correction historyの鍵と適用先のADR](../adr/key-correction-history-by-material.md)、および[段階生成のADR](../adr/generate-qsearch-captures-by-value-group-per-piece.md)。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)と[SPRTによる棋力測定の手引き](../guides/sprt.md)。
