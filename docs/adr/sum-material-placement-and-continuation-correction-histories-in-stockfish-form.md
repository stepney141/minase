# correction historyは材料表、駒配置の4表、およびcontinuation correction historyの重み付きの和で求めてStockfishの形で更新し、各ノードで1回求めた補正後の静的評価を枝刈りと減深のすべての判定に使う

## ステータス

承認済み。決定日は2026年10月20日。

## コンテキスト

correction historyは、探索値と静的評価のずれを局面の特徴ごとに学習し、枝刈りの判断に使う静的評価を補正する手法であり、StockfishとYaneuraOuが実装している（[既存実装の調査](../research/forward-pruning-prior-art.md)）。
[旧ADR](archive/key-correction-history-by-material-and-apply-it-to-all-static-eval-checks.md)は、2026年10月18日に、双方の駒種別の枚数の組を鍵にした1つの補正表（以下、材料表）を各ノードで1回読み、補正後の静的評価を枝刈りと減深のすべての判定に使うと決めた。
材料表は探索ごとに初期化し、探索値と補正前の評価の差の指数移動平均を値に持ち、その絶対値を`CorrectionCap`で切り詰めていた。

鍵の選び方は、[棋力向上段階8](../plans/archived/strength-stage8.md)の[再診断](../measurements/strength-stage8/strength-stage8-correction-bench.md)と、[棋力向上段階12](../plans/archived/strength-stage12.md)の[診断](../measurements/strength-stage12/strength-stage12-activation-diag.md)に基づく。
段階12の診断は、歩兵と仲人の配置などを鍵にした表を加えると予測誤差が悪化したとして見送ったが、比べた予測は材料表と追加の表の単純平均であり、材料表の補正を半分に薄める形だった。
歩兵と仲人の配置を鍵にした表は、平均しても誤差の悪化が0.20%にとどまり、表を引いた参照の89.50%で0でない値を返した。
これらの診断は、既定の評価関数がPSTだった時期のものである。

Stockfishは、歩兵の配置、小駒の配置、および先手と後手それぞれの歩兵以外の駒の配置を鍵にした4つの表と、手番側自身の2手前、4手前、6手前の手を文脈として直前の相手の手で引く表（continuation correction history）の値を、表ごとの重みで足し合わせて補正量にする（`src/search.cpp:95-118`）。
各表は、合成した補正後の評価と探索値の差から作った1つの更新量を表ごとの倍率で掛け、重力式（要素`v`へ更新量`b`を`v ← v + b − v|b|/1024`で取り込み、値が上限に近いほど動きにくくする式）で更新する（`src/search.cpp:120-143, 1664-1673`）。
補正量の大きさは、LMR（後方の手を浅く読む減深）の減深量とreverse futility pruningの余裕値にも使う。

2026年10月17日のエンジンの開発者の決定により、[探索の関数形の見直しの統括計画](../plans/search-forms.md)の各サブタスクでは、形や値がStockfishとYaneuraOuで違えばStockfishに合わせ、Stockfishにだけある項目は入れ、minaseの既存の仕組みのうち中将棋の診断や測定で決めたものは残して調整の対象にする。
同じ日の決定により、[correction historyの表の設計書](../plans/archived/correction-history-tables.md)については、表の大きさと共有の仕方をStockfishと同じにし、表を合成した補正後の評価との残差で更新し、表を対局の間減衰させずに持ち越し、更新の倍率と初期値はStockfishの値で固定する。

## 決定

補正量は、手番側の行から引いた次の表の要素`e`の重み付きの和`S`から、`c = trunc(P × S / 102400)`で求める。`P`は評価関数の重みファイルが持つ探索用の歩兵価値である。

- 材料表（双方の駒種別の枚数の組を鍵にする）。重みは`CorrectionMaterialWeight`。
- pawnの表（双方の歩兵と仲人の配置を鍵にする）。重みは`CorrectionPawnWeight`。
- minorの表（双方の、現在の駒種が猛豹、銅将、銀将、金将、盲虎、醉象、麒麟のいずれかである駒の配置を鍵にする）。成った歩兵は金将、成った仲人は醉象として数え、太子や獅子に成った駒は数えない。重みは`CorrectionMinorWeight`。
- 先手と後手それぞれのnon-pawnの表（その側の、現在の駒種が歩兵でも仲人でもない全駒の配置を鍵にする）。王駒と成った駒を含む。重みは2表に共通の`CorrectionNonPawnWeight`。
- continuation correction history（手番側自身の2手前、4手前、6手前の手を文脈とし、1手前の相手の手で引く）。2手前と4手前の文脈の要素には`CorrectionContinuationWeight`、6手前の文脈には`CorrectionContinuation6Weight`を掛ける。1手前の手がない場合（根とnull moveの直後）は、表を引かずに定数303（Stockfishの80695の換算値、`src/search.cpp:104-109`）を加える。

重みの単位は、要素が上限1024のときの補正量の歩兵価値に対する百分率である。
補正後の静的評価は、NNUEの出力に`c`を足して評価関数の出力の範囲に収めた値とし、合成した補正量そのものには上限を置かない。
補正後の静的評価の使い方は、旧ADRから引き継ぐ。
通常探索の各ノードでは、置換表の照合とIIRの後に補正量を1回だけ求め、補正後の静的評価を手数ごとに保持する。根でも同じく求めて保持する。
reverse futility pruning、razoring、null move pruning、futility pruning、late move pruning、およびLMRは、すべてこの静的評価に基づいて判定し、reverse futility pruningの余裕値の補正量の項にも同じ補正量を使う。
reverse futility pruning、razoring、およびLMRの、評価値とαの差を使う項は、静的評価の代わりに判定用の評価値を使う。
判定用の評価値は、置換表の照合に成功し、その値が詰み帯と、反復による勝敗に使う専用の値の帯の外にある場合に限って、置換表の値で静的評価を置き換えたものである。
置き換えるのは、置換表の値が静的評価より大きくてその種別が下界か正確な値の場合と、静的評価以下でその種別が上界か正確な値の場合であり、それ以外は静的評価のままとする（Stockfish `src/search.cpp:862-865`）。
置換表の保存値と静止探索のstand-patには、補正を加えない。

配置の鍵は、所有者、成りの状態を区別する駒の状態、および升の組ごとの64ビットの乱数の排他的論理和であり、着手の移動元、到達升、および捕獲した駒の升から差分で更新する。
continuation correction historyは、文脈の手の着手前の駒種（29種と「文脈なし」）と到達升、および1手前の手の着手後の駒種と到達升で引き、升は現在のノードの手番側が後手なら180度回転する。
文脈の手がない場合（根より前とnull move）は、Stockfishの番兵と同じく「文脈なし」の区画の固定の1升を引く。

表の要素は−1024以上1024以下の16ビット整数とし、局面の5表の初期値は−5、continuation correction historyの初期値は5とする（`src/search.cpp:719, 731-733`）。
更新は、探索値が詰み帯と専用の帯の外にあり、手番側の王駒に相手の利きが届かないノードで行う。
さらに、上界のノードでは探索値が補正後の静的評価以下であること、下界と正確な値のノードでは探索値が補正後の静的評価より高く、最善手が捕獲手でないことを条件とする。
差`D`は、探索値からノードの入口で求めた補正後の静的評価を引いた値であり、更新量は`b0 = clamp(trunc(D × d × K / P), −256, 256)`、`b = trunc(1061 × b0 / 1024)`で求める（`src/search.cpp:1669-1672`）。
`d`は残り深さ、`K`は上界のノードでは`CorrectionFailLowBonus`、それ以外では`CorrectionBonus`である。
各表の要素は、表ごとの倍率`f`（材料表とpawnの表が128、minorの表が150、non-pawnの表が186、continuation correction historyの2手前、4手前、6手前が130、70、35）で`trunc(b × f / 128)`とした量を、上限1024の重力式で取り込む。
continuation correction historyは2手前、4手前、6手前の順に更新し、同じ要素を指す文脈があれば、前の更新の結果に続けて更新する。
根の探索も、完了したときに同じ条件で局面の表を更新する。

7つの重みと2つの更新量はSPSAで調整し、表の上限、初期値、更新の倍率、更新量の切り詰め、および1061/1024はStockfishの値で固定する。
表の寿命と共有の範囲は[historyの持ち越しのADR](carry-histories-and-correction-histories-within-a-game.md)が、補正量のLMRでの使い方は[LMRのADR](reduce-all-later-moves-and-root-moves-by-lmr-with-stat-score-and-correction.md)が定める。

## 検討した代案

材料表を除いてStockfishの表だけにする案は、材料表が中将棋の診断で鍵の候補のうち最も予測を改善した表なので、エンジンの開発者の決定により採らなかった。
材料表の重みも調整の対象にしたので、SPSAが重みを下げて材料表を弱めることもできる。

minorの集合をYaneuraOuのように将棋向けに定め直した区分（`source/position.cpp:27-36`）にする案と、continuation correction historyを2手前と4手前の2つに限るYaneuraOuの形は、Stockfishに合わせるという決定により採らなかった。
小駒の範囲は、日本中将棋連盟の駒の格付けが「小駒」とする駒から王駒の太子を除いた7種とした。成った歩兵と仲人を含めることは格付けに明文がなく、設計書の判断である。

更新量に1061/1024を掛けてから±256で切り詰める案は、Stockfishが切り詰めの後に掛けるので採らなかった。
Stockfishが同じ値の手を選び直して上界のノードでも最善手を持つ場合（`src/search.cpp:1541`）に合わせる案は、minaseの最善手の選び方を変えることになるので採らず、Stockfishの更新の条件をminaseの境界の種類へ対応させた。
更新の条件から王駒への利きの条件を外す案は、中将棋には王手を受けた側に解消する義務がなく（RULES.md第8条第4項）、王駒が取られ得る局面の静的評価は探索値から大きく外れるので採らなかった。この条件はStockfishの`!ss->inCheck`に近い役割を持つ。ただし、RULES.md第8条の王手は最後の王駒が取られ得る状態に限るのに対し、この条件はいずれかの王駒に相手の利きが届けば成り立つので、王手より広い。

LMRの補正項を、切り捨てた後の補正量`c`から求める案は、Stockfishが静的評価には割った値を、LMRには割る前の和を使うので採らなかった。

判定用の評価値に、Stockfishと同じく詰み帯と専用の帯の置換表の値も使う案は、旧ADRと同じく採らない。専用の帯の値は経路に依存するので打ち切りに使わないという既存の規則（[ADR](reserve-a-path-dependent-band-for-repetition-outcomes.md)）に揃え、詰み帯の値を枝刈りの返り値に持ち込まないためである。

旧ADRが棄却した案のうち、最善手が捕獲手のノードを含む全観測点で更新する案（段階8の変種A）と、王駒の升の組を鍵にする表を加える案は、棄却したままとする。
前者は段階8の再診断で予測を改善せず、StockfishとYaneuraOuも最善手が捕獲手のノードを更新しない。後者はStockfishに対応する表がなく、王駒の升はnon-pawnの表の鍵に含まれる。
旧ADRが見送った、駒配置の鍵の表を加える案と補正表を持ち越す案は、この決定で採った。

## 結果

鍵、表、合成、および更新の詳細は`crates/minase/src/search/alphabeta/correction.rs`のdocコメントにあり、補正後の静的評価の求め方は`negamax.rs`の`negamax`と`root.rs`の`search_root`のdocコメントにある。係数の現行値は`params.rs`にある。
形の変更をすべて入れて係数を開始値に置いた版は、設計書を始めた時点のmasterに比べて、benchの深さ3、5、7の探索ノード数の合計が1.5%多く、NPSが約6.7%低く、深さ9を読み終える時間が約12.8%長かった（[benchの記録](../measurements/correction-history-tables/correction-history-tables-bench.md)）。
鍵の選び方を決めた段階8と段階12の診断は既定の評価関数がPSTだった時期のものであり、新しい形の各表の寄与は測っていない。
