# 浅いノードの前向き枝刈りは王駒に利きが届くノードでは行わず、reverse futility pruningはStockfishの条件で残り深さ18まで行う

## ステータス

置き換え済み。決定日は2026年10月18日。[浅い前向き枝刈りは深さと窓の条件をStockfishに合わせて王駒に利きが届くノードでは行わず、reverse futility pruningはβと判定用の評価値の加重平均を返す](../extend-shallow-pruning-to-stockfish-ranges-and-skip-it-when-a-royal-is-attacked.md)によって置き換えられた。

## コンテキスト

通常探索は、浅いノードで枝を読まずに済ませる前向き枝刈りを持つ。
reverse futility pruning、futility pruning、およびrazoringは静的評価を根拠にし、late move pruningは順序付けの後方にある手の数を根拠にする。
中将棋では王駒を相手の利きへ動かす着手も、王手を放置する着手も合法である（RULES.md第8条）。
王駒が取られ得る局面の静的評価は、最後の王駒の捕獲を詰みの値とする探索値から大きく外れる。
王駒の捕獲を禁じる規則はないので、王駒の升では、相手駒の疑似利きと実際に捕獲できる駒の集合が一致する。

[旧ADR](skip-static-eval-pruning-when-a-royal-is-attacked.md)は、2026年9月6日に、これらの枝刈りを残り深さ3以下の零窓のノードに限り、王駒に利きが届くノードを除くと決めた。
[棋力向上段階4](../../plans/archived/strength-stage4.md)の[診断bench](../../measurements/strength-stage4/strength-stage4-pruning-bench.md)では、非PVかつ詰み帯でないノードのうち、いずれかの王駒に利きが届くノードは残り深さ1、2、3で1.56%、1.87%、2.99%であり、最後の王駒だけに限ると1.52%、1.80%、2.99%だった。
同じ適用条件は、その後SEEによる捕獲手の枝刈りに広がった（[ADR](prune-losing-captures-by-see-at-shallow-nodes.md)）。
2026年10月4日には、reverse futility pruning、razoring、および定義を改めたlate move pruningも同じ条件のまま採用された（[採用のADR](../adopt-revived-search-items-as-a-bundle.md)）。
このときのreverse futility pruningは、深さ1〜3に共通の1つの余裕値を補正前の静的評価から引いて判定する形だったので、深さ1と深さ3の余裕値の比を1以外にできなかった。

2026年10月17日に、現行の形のまま探索と時間管理の係数をSPSAで調整し直したところ、余裕値の係数`ReverseFutilityMargin`が開始値428から下限0まで動いた。
この調整は、後続の設計書で全係数を調整し直すので、反映せずに中止した。
Stockfishの余裕値は深さに比例して増え、improvingのときは補正量の項を除いて深さ2以下で0以下になる（`src/search.cpp:1022-1033`）。
現行の形では全深さの余裕値を一律に0へ近づけることでしかこの挙動に近づけないので、SPSAが係数を下限まで動かしたのはそのためである可能性がある。
同じ日に、[探索の枝刈りとLMRの関数形の見直し](../../plans/archived/search-pruning-forms.md)で、前向き枝刈りと減深の式をStockfishの形へ寄せることにした。
エンジンの開発者の決定により、Stockfishにある項目は入れて形と値をStockfishに合わせ、深さの上限のような閾値はStockfishの値で固定し、中将棋の規則から来る王駒への利きの除外は残す。

## 決定

futility pruning、late move pruning、razoring、およびSEEによる捕獲手の枝刈りは、残り深さ3以下の零窓のノードで、αとβがいずれも詰み帯と専用の帯の外にあり、手番側のいずれの王駒にも相手の疑似利きが届かない場合に限って行う。
着手の展開を省くfutility pruning、late move pruning、およびSEEによる捕獲手の枝刈りは、さらにそのノードで探索済みの最善値が負の詰み帯の外にあることを要求し、razoringは残り深さ2以下に限る。
これらの条件は旧ADRから引き継ぐ。
null move pruningには、王駒への利きの判定を加えない。

reverse futility pruningの条件に使う2つの語を、先に定める。
ttPvは、PVノードであるか、置換表のエントリがPVの印を持つことをいう。
印はエントリの予約領域の1ビットに置き、付け方はStockfishに従う（`src/search.cpp:842, 1651-1657, 1741, 1908`）。
詰みを探す局面は、前の反復で確定した根の評価値の絶対値が、反復の深さ`D`について歩兵価値の`361 + 105769/D²`%以上である反復をいう。
2つの定数はStockfishの`seekMate`の750と220000を歩兵価値208で換算した値であり、Stockfishが棋力のための値ではないとして調整しない値なので固定する（`src/search.cpp:394-395`）。
詰みを探す局面では、razoringを行わない（同1017行）。

reverse futility pruningは、次の条件をすべて満たすノードで判定用の評価値を返し、その値を置換表に保存しない。

- 残り深さが18以下であり、詰みを探す局面では5以下である。
- ttPvでない。PVノードはttPvなので、零窓のノードに限られる。
- 置換表の手がないか、合法手であることを確かめた置換表の手が捕獲手である。
- αとβが詰み帯と専用の帯の外にあり、手番側のいずれの王駒にも相手の疑似利きが届かない。
- 判定用の評価値がβ以上であり、余裕値を引いてもβ以上である。

余裕値はStockfishの式（`src/search.cpp:1025-1030`）を歩兵価値の百分率の係数で表したものであり、深さに比例する項からimprovingとopponentWorseningの項を引き、補正量の絶対値に比例する項を足す。
深さに掛ける倍率の上限（歩兵価値の41%）と、深さの上限18および5は、Stockfishの値で固定する。
判定用の評価値とopponentWorseningの定義は、[静的評価のADR](../key-correction-history-by-material-and-apply-it-to-all-static-eval-checks.md)と[improvingのADR](judge-improving-on-the-corrected-static-eval-and-use-it-in-pruning-and-lmr.md)にある。

## 検討した代案

深さ1〜3に共通の1つの余裕値を補正前の静的評価から引いて判定する旧ADRの形は、浅い深さと深い深さで別の余裕値を選べないので廃した。
深さの上限や余裕値の係数の上限をSPSAで調整する案は、調整するのは各仕組みの強さを決める値だけとし、上下限や深さの条件はStockfishの値で固定するというエンジンの開発者の決定により採らなかった。
Stockfishは深さの上限を、詰みの発見のために調整しない値としている（`src/search.cpp:1021`）。
王駒への利きの除外を外してStockfishの条件だけにする案は、除外が中将棋の規則から来るので、エンジンの開発者の決定により採らなかった。

旧ADRで棄却した案は、この決定でも棄却したままとする。
判定を最後の王駒に限る案は、除外率がほとんど変わらないので採らない。
reverse futility pruningの返り値をβへ丸める案は、境界としては常に正しいが、上位ノードが得る情報を減らすので採らない。
返り値をβと判定用の評価値の重み付き平均にするStockfishの形（`src/search.cpp:1033`）は、[浅い枝刈りの拡張の設計書](../../plans/archived/shallow-pruning-extensions.md)が扱うので、本決定には含めない。

## 結果

診断で確かめたのはβ以上かどうかの判定が正しいことだけであり、返す値の正しさは確かめていないので、reverse futility pruningが返す値は、null move pruningやLMRの返り値と同じくfail-softの近似として扱う。
探索済みの最善値の条件により、各ノードで少なくとも1手は探索されるので、全手を枝刈りしたノードを合法手のない負けと取り違えない。
王駒に利きが届くノードの割合は残り深さ3以下でしか測っていないので、reverse futility pruningを残り深さ18まで広げた後の除外率は分からない。
王駒への利きを調べる費用は係数で消せないので、枝刈りの効果が小さい構成では費用だけが残る。

条件の判定は`crates/minase/src/search/alphabeta/negamax.rs`の`negamax`にあり、王駒への利きは他の条件が成立したときだけ`royal_under_attack`で調べてノード内で再利用する。
余裕値の式と固定の定数は`pruning.rs`の`reverse_futility_margin`と`REVERSE_FUTILITY_MAX_DEPTH`の周辺に、詰みを探す局面の判定は`is_seek_mate`にあり、ttPvの印は`tt.rs`の`Hit`の`is_pv`である。
`ReverseFutilityMargin`は廃し、余裕値の6つの係数を新たに調整の対象にした。
係数を開始値に置いた時点では、余裕値が深さ1で歩兵価値の約24%と、旧来の余裕値の428%よりはるかに小さく、benchの探索ノード数はこのコミットで直前より77.2%減った（[benchの記録](../../measurements/search-pruning-forms/search-pruning-forms-bench.md)）。
2026年10月18日の30,000ペアのSPSAでは、`mult`の定数`ReverseFutilityMultBase`が開始値22から235へ動いた（[SPSAの記録](../../measurements/search-pruning-forms/search-pruning-forms.md)）。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、設計書を始めた時点のmaster`99ccc6bb`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、776ペアで`H0`となった（[STCの記録](../../measurements/search-pruning-forms/search-pruning-forms-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](../adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
