# 秒読みのない時計では、相手より残り時間が少ないときに目安時間を縮め、そのために相手の残り時間を探索の必須の入力にする

## ステータス

承認済み。決定日は2026年10月22日。

## コンテキスト

minaseの時間管理は、探索の開始時に、自分の残り時間、加算、および秒読みから、hardで抑える前の目安時間`soft_raw`を求め、そこから1手の目安時間softと打ち切りの上限hardを決める（[残り手数の見積りのADR](estimate-moves-to-go-from-ply.md)）。
hardは、`soft_raw`の`HardSoftRatio`%、残り時間の`HardRemainingShare`%に秒読みの8割を足した値、および残り時間と秒読みの和から安全余裕を引いた値のうち最も小さい値（1 ms以上）である。
これまでの探索の制限`ClockLimits`は自分の側の時計だけを持ち、USIの`go`は手番でない側の`btime`または`wtime`を使わず、CECPの`otim`は値を検査するだけで捨てていた。

Stockfishは、自分の残り時間が相手より少ないときだけ、目安時間の倍率に`1 + 0.9 × min(timeAdvantage, 0)`を掛ける（`src/timeman.cpp:151-156`）。
`timeAdvantage`は、自分と相手の残り時間の差を両者の和に1を足した値で割った量である。
Stockfishは、ノード数を時間として使う設定と、周期的な時間制御で区切りの最後の1手（`movestogo == 1`）では、この補正を使わない。
上限時間は目安時間の倍数からも決まるので、補正は上限時間にも及ぶ（同`159-162`）。
YaneuraOuの時間管理（`source/timeman.cpp`）は、相手の残り時間を参照しない。

[持ち時間の伸縮の設計書](../plans/archived/time-instability-scaling.md)は、2026年10月17日のエンジンの開発者の決定に従い、Stockfishの時間管理の項目をminaseへ入れた。
同じ決定により、統括計画の各サブタスクでは、Stockfishにだけある項目は入れ、秒読みのようにStockfishに相当する仕組みがない事項はYaneuraOuに合わせる。
エンジンの開発者は、同じ日に、相手との残り時間の差による補正を同書に含めると決めた。

[持ち時間を温存しないADR](spend-main-time-from-the-opening.md)は、秒読みつきの時計で持ち時間を後半の手のために温存せず、序盤から分担して使うと決め、温存を再提案するなら秒読みつきの時計で棋力を測る必要があるとした。
秒読みが正の時計では、持ち時間を使い切っても毎手の秒読みで指せるので、持ち時間の差は時間が足りないことを表さない。
そこで不利なときに消費を減らせば、持ち時間を後の手に残す温存になる。

対局ハーネスの`crates/minase/src/harness/clock.rs`、開発用のGUIの`minase-gui`、およびlishogiのBotは、時計を使う`go`に両側の残り時間を送っている。

## 決定

有限の探索に時計があり、その秒読みが0で、`movetime`の指定がない探索では、自分の残り時間が相手より少ないときに、`soft_raw`に千分率の因子を掛けてから、hardとsoftを従来の式で求める。
深さやノード数の制限を時計と併用した探索も、この対象に含める。
因子は、自分の残り時間を`R`、相手の残り時間を`R_o`として`1000 − ⌊TimeAdvantage × (R_o − R) / (R + R_o + 1)⌋`とし、係数`TimeAdvantage`はStockfishの0.9を換算した900‰から始めて、範囲0〜1000‰でSPSAで調整する。
秒読みが正の時計と、`movetime`を併用した時計には、この因子を掛けない。

`ClockLimits::new`は、相手の残り時間を省略可能なミリ秒の値として構築時の引数に受け取る。
`SearchLimits::new`は、時計があり、秒読みが0で、`movetime`がなく、相手の残り時間がない制限を`SearchError::MissingOpponentClock`で拒否し、深さやノード数の制限を併用しても拒否する。
相手の残り時間が0であることは正当な入力であり、相手の残り時間がないこととは区別する。
USIの`go`は、手番でない側の`btime`または`wtime`がなければ値を未指定のまま渡して0で代用せず、`go ponder`も同じ検査を通す。
CECPは`otim`の値を保持し、`time`と`otim`がどちらの順で届いても受け入れ、新規対局と時間制御の設定`level`で保持した値を消す。
CECPで`time`だけが届いた時点ではエラーにせず、探索を始めるときに`SearchLimits::new`が拒否する。

## 検討した代案

秒読みつきの時計にも因子を掛ける案は採らなかった。
秒読みはStockfishにない仕組みなので、エンジンの開発者の決定により秒読みつきの時計ではYaneuraOuに合わせ、YaneuraOuは相手の残り時間を使わない。
秒読みつきの時計で不利なときに消費を減らすと、持ち時間を後の手に残す温存になり、持ち時間を温存しないADRの決定と食い違う。

相手の残り時間がないときに、0か自分の残り時間で補って探索を続ける案は採らなかった。
どちらで補っても因子は1000‰になり、補正が働かないまま探索が進むので、入力の欠落に気づけない。
欠けた入力は既定値で補わず、エラーとして拒否する。

CECPで`otim`が届いていないときに、`level`の初期持ち時間を相手の残り時間として使う案は採らなかった。
初期持ち時間は、対局が進んだ後の相手の残り時間を表さない。

相手の残り時間を、秒読みの安全余裕と同じく、構築した後に呼ぶ設定関数で与える案は採らなかった。
相手の残り時間は時計の入力であり、秒読みの安全余裕のような任意の調整値とは性質が違う。

相手の残り時間がないことを`ClockLimits::new`で拒否する案は採らなかった。
因子を使うかは`movetime`の有無でも決まり、`ClockLimits`だけでは決まらないので、相手の残り時間がない状態は`ClockLimits`に持たせ、`SearchLimits::new`で検査する。

Stockfishの2つの適用除外（ノード数を時間として使う設定と、周期的な時間制御の最後の1手）は、minaseにはその入力がないので設けなかった。

## 結果

これまで受理していた入力を拒否し、公開の`ClockLimits::new`の引数も変わるので、互換性を壊す変更である。
秒読みが0で`movetime`のないUSIの`go`に手番でない側の残り時間がない場合と、CECPで`otim`を送らずに`time`だけで探索を始める場合は、探索を始めずにエラーを返す。
片側の時計だけを持つ入力でも、秒読みが正か`movetime`を併用していれば受理する。
対局ハーネス、開発用のGUI、およびlishogiのBotは両側の残り時間を送っているので、拒否の対象にならない。

hardの候補のうち`soft_raw`の倍数の項は、因子を掛けた`soft_raw`から求めるので同じ割合で縮む。
残り時間の割合の項と、残り時間から安全余裕を引いた上限は変わらないので、最終のhardが縮むとは限らない。
`R_o − R`は`R + R_o + 1`より小さく、`TimeAdvantage`は1000以下なので、因子は正である。
この式は、1000から引く補正項を切り捨てた整数の近似であり、Stockfishの浮動小数の倍率全体を千分率に丸める方法とは結果が異なり得る。

因子は`crates/minase/src/search/alphabeta/time.rs`の`time_advantage_factor`と`ClockCoefficients::budget`に、適用の条件は同じファイルの`time_budget`にある。
入力の検査は`crates/minase/src/search/limits.rs`の`SearchLimits::new`に、USIとCECPの扱いは`crates/minase/src/protocol/usi.rs`の`parse_go_config`と`cecp.rs`の`TimeControl`のdocコメントにある。
固定深さの`minase dev bench`は時計を使わないので、この変更で探索ノード数、評価値、および最善手は変わらなかった（[benchの記録](../measurements/time-instability-scaling/time-instability-scaling-bench.md)）。
`TimeAdvantage`は、持ち時間の伸縮の設計書のほかの係数とまとめて、秒読みのない時計の自己対局で30,000ペアのSPSAにより調整し、最終値は開始値900‰とほぼ同じ897‰だった（[SPSAの記録](../measurements/time-instability-scaling/time-instability-scaling.md)）。
持ち時間の伸縮のほかの項目とあわせた候補は、STCで起点に対して303ペア（有効297）でLLR +2.945の`H1`となり、平均得点から求めた差は約+51 Eloだった（[STCの記録](../measurements/time-instability-scaling/time-instability-scaling-stc.md)）。この差には探索の係数の調整し直しも含まれ、この因子だけの寄与は測っていない。
ほかの項目は、秒読みつきの短い条件で`H1`とならなかったので秒読みが0の時計に限ったが（[伸縮のADR](scale-the-soft-limit-by-search-instability-and-keep-the-stable-best-move-rule.md)）、この因子は初めから秒読みが0の時計だけに掛けているので、適用の条件は変わらない。
