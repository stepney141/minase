# correction historyは駒種別の枚数を鍵にし、各ノードで1回求めた補正後の静的評価を枝刈りと減深のすべての判定に使う

## ステータス

置き換え済み。決定日は2026年10月18日。[correction historyは材料表、駒配置の4表、およびcontinuation correction historyの重み付きの和で求めてStockfishの形で更新し、各ノードで1回求めた補正後の静的評価を枝刈りと減深のすべての判定に使う](../sum-material-placement-and-continuation-correction-histories-in-stockfish-form.md)によって置き換えられた。

## コンテキスト

correction historyは、探索値と静的評価のずれを局面の特徴ごとに学習し、枝刈りの判断に使う静的評価を補正する手法であり、StockfishとYaneuraOuが実装している（[既存実装の調査](../../research/forward-pruning-prior-art.md)）。
[旧ADR](key-correction-history-by-material.md)は、2026年9月21日に、補正表の鍵と更新の条件を定め、補正を浅いノードのfutility pruningの判定にだけ使うと決めた。

鍵と更新の条件は、[棋力向上段階8](../../plans/archived/strength-stage8.md)が、探索の進行に沿って表を更新しながら次の観測の予測誤差で候補を比べた[再診断](../../measurements/strength-stage8/strength-stage8-correction-bench.md)に基づく。
観測点の78.3%は最善手が捕獲手のノードであり、全観測点で更新する変種Aの改善率は−4.84%から1.99%にとどまった。
最善手が捕獲手でなく、手番側の王駒に相手の利きが届かないノードでだけ更新する変種Bでは、駒種別の枚数の組を鍵にした改善率が15.41%、0でない補正を参照した割合が66.9%だった。
捕獲手による駒得は評価の偏りではないので、StockfishとYaneuraOuも最善手が捕獲手のノードでは補正を更新しない。
この形はSTCとLTCでともに`H1`となった（[STC](../../measurements/strength-stage8/strength-stage8-correction-stc.md)、[LTC](../../measurements/strength-stage8/strength-stage8-correction-ltc.md)）。
その後、[棋力向上段階12](../../plans/archived/strength-stage12.md)の[診断](../../measurements/strength-stage12/strength-stage12-activation-diag.md)では、歩兵と仲人の配置などを鍵にした表を加えると平均絶対誤差が0.20〜7.77%悪化した。
表を対局内で持ち越しても、futilityの判定で枝刈りの可否が変わる静かな手は、STC相当で0.69%、LTC相当で0.80%だった。

2026年10月17日に、[探索の枝刈りとLMRの関数形の見直し](../../plans/archived/search-pruning-forms.md)で、前向き枝刈りと減深の式をチェスのStockfishの形へ寄せることにした。
同じ日のエンジンの開発者の決定により、Stockfishにある項目は入れて形と値をStockfishに合わせ、中将棋の診断や測定で決めたminaseの既存の仕組みは残す。
Stockfishは、補正をノードごとに1回だけ読み、補正後の静的評価を各ノードの最初に求めて保持し、静的評価を使う判定のすべてに使う（`src/search.cpp:832, 846-870`）。
それまでのminaseは、浅い枝刈りの対象になるノードなどに限って静的評価を求め、futility pruningの判定の時点で補正を読み直していた。
旧ADRの持ち越しの診断は、補正をfutility pruningだけに使う前提で測ったものであり、適用先を広げた場合は測っていない。

## 決定

補正表は、双方の駒種別の枚数の組と手番側を鍵とし、探索ごとに初期化して反復深化の間だけ持ち越す。
探索値が詰み帯と専用の帯の外にあり、置換表へ保存する値の境界の種類から補正の向きが定まり、最善手が捕獲手でなく、手番側の王駒に相手の利きが届かないノードでだけ、探索値と補正前の評価の差で表を更新する。
この鍵と更新の条件は旧ADRから引き継ぐ。

通常探索の各ノードでは、置換表の照合とIIRの後に補正を1回だけ読み、NNUEの出力に足して評価関数の出力の範囲に収めた値を静的評価とし、手数ごとに保持する。
根でも同じく静的評価を求めて保持し、子の比較に使う。
reverse futility pruning、razoring、null move pruning、futility pruning、late move pruning、およびLMRは、すべてこの静的評価に基づいて判定し、reverse futility pruningの余裕値の補正量の項にも同じ補正の値を使う。

reverse futility pruning、razoring、およびLMRのαとの差の項は、静的評価の代わりに判定用の評価値を使う。
判定用の評価値は、置換表の照合に成功し、その値が詰み帯と専用の帯の外にある場合に限って、置換表の値で静的評価を置き換えたものである。
置き換えるのは、置換表の値が静的評価より大きくてその種別が下界か正確な値の場合と、静的評価以下でその種別が上界か正確な値の場合であり、それ以外は静的評価のままとする（Stockfish `src/search.cpp:862-865`）。
置換表の保存値と静止探索のstand-patには、補正を加えない。

## 検討した代案

補正をfutility pruningの判定にだけ使い続ける旧ADRの方式は、Stockfishに合わせるというエンジンの開発者の決定に反し、その根拠の診断も適用先を広げた場合を測っていないので採らなかった。
判定用の評価値に、Stockfishと同じく詰み帯と専用の帯の置換表の値も使う案は、設計書の判断で採らなかった。
専用の帯の値は経路に依存するので打ち切りに使わないという既存の規則（[ADR](../reserve-a-path-dependent-band-for-repetition-outcomes.md)）に揃え、詰み帯の値を枝刈りの返り値に持ち込まないためである。

旧ADRで棄却した案は、この決定でも棄却したままとする。
全観測点で更新する案（変種A）は、上記のとおり予測を改善しなかったので採らない。
王駒の升の組を鍵にする案と、王駒の升の組と枚数の組を組み合わせて鍵にする案は、変種Bでの改善率が15.27%と14.98%で枚数の組を下回ったので採らない。
駒配置の鍵の表を加える案は予測誤差を悪化させ、補正表を対局内で持ち越す案は発動率が基準に届かなかったので、どちらも見送ったままとする。

## 結果

静的評価は、浅い枝刈りの対象に限らず、通常探索のすべてのノードで求める。
設計書の変更をすべて入れて係数を開始値に置いた版は、この計算と置換表の手の合法性の確認を加えたうえで、探索速度が設計書を始めた時点のmasterの1.26倍だった（[benchの記録](../../measurements/search-pruning-forms/search-pruning-forms-bench.md)）。
補正の上限と更新の重みは`crates/minase/src/search/alphabeta/params.rs`の`CorrectionCap`と`CorrectionWeight`であり、2026年10月18日に、NNUEを既定の評価関数とする探索で、ほかの係数とまとめた30,000ペアのSPSAで調整し直した（[SPSAの記録](../../measurements/search-pruning-forms/search-pruning-forms.md)）。
鍵と更新の詳細は`correction.rs`の`CorrectionTable`と`material_key`のdocコメントに、静的評価と判定用の評価値の求め方は`negamax.rs`の`negamax`と、`pruning.rs`の`corrected_static_eval`および`pruning_eval`のdocコメントにある。
鍵と更新の条件、および持ち越しを見送った根拠の診断は、既定の評価関数がPSTだった時期のものであり、適用先を広げた後には測り直していない。
駒配置の鍵の診断では、各表の補正を平均せずに足し合わせる形を評価していないので、その形は未測定の候補として残る。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、設計書を始めた時点のmaster`99ccc6bb`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、776ペアで`H0`となった（[STCの記録](../../measurements/search-pruning-forms/search-pruning-forms-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](../adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
