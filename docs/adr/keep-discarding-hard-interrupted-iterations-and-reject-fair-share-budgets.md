# 秒読みのない時計では、予算を分担そのものにして途中結果で指す時間管理を採らず、hardで中断した反復を捨てる現行の機構を保つ

## ステータス

承認済み。決定日は2026年10月22日。

## コンテキスト

[持ち時間の効率的な使用](../plans/archived/time-management-efficiency.md)は、issue #7への対策として、StockfishとYaneuraOuに倣った時間管理の再構成を試みた（両エンジンの該当箇所は[設計書の参考資料](../plans/archived/time-management-efficiency.md#参考資料)にある）。
再構成は4段階に分け、1段階を1コミットとして順に積み重ねた。
第0段階は、根の各手の探索結果を保持する表の導入である。
第1段階は、打ち切りの上限hardで中断した反復でも、その反復で探索を終えた手の中の最善手を指す途中結果の採用と、目安時間softとhardを持ち時間の1手あたりの分担そのものにする予算の再定義である。
第2段階はsoftの半分を過ぎた後の反復の深さを抑える方式、第3段階は局面の信号の積でsoftを伸縮させる方式である。
第2段階は、[事前判定](../measurements/time-management-efficiency/time-management-efficiency-stage2-bench.md)で、抑えた反復の所要時間を直前の反復の所要時間で割った比の中央値が0.0であり、置換表の照会だけで終わる空費と分かったので見送った。
第2段階を除く残りの段階を加えた最終候補は、STCで有効665ペアでLLR −2.958の`H0`となり、候補側に時間切れが2件出た（[記録](../measurements/time-management-efficiency/time-management-efficiency-stc.md)）。
構成要素を分けた分解測定では、第0段階と第1段階を加えた構成D1が有効260ペアでLLR −2.954、D1から序盤の係数を外したD2が有効259ペアでLLR −2.962であり、どちらも`H0`だった（[D1](../measurements/time-management-efficiency/time-management-decomposition-d1-stc.md)、[D2](../measurements/time-management-efficiency/time-management-decomposition-d2-stc.md)）。
D1の1局の総思考時間の中央値は基準の27.6秒に対して23.7秒であり、基準の方式が最後の反復をsoftを超えて完了させるために使っていた時間を、D1は使い残していた。

[旧ADR](archive/reject-fair-share-budget-with-partial-results.md)は、2026年9月18日に、これらの結果から、秒読みのない時計では現行の機構を保つと決めた。
旧ADRは、第2段階の方式と第3段階の方式も代案として棄却した。
第3段階を棄却した理由は、候補が200手目までに持ち時間を使い切って中終盤の到達深さが浅くなり、STCで`H0`だったことである。
測定は評価関数がPSTとFMだった時期の構成で行ったので、旧ADRは、同じ方式を再提案するには現行の構成での再測定が要るとしていた。

[持ち時間の伸縮の設計書](../plans/time-instability-scaling.md)は、2026年10月17日のエンジンの開発者の決定に従い、Stockfishの時間管理の項目をminaseへ入れた。
その中には、第3段階と同じ4因子の積でsoftを伸縮させる項目と、Stockfishの`increaseDepth`に当たる、目安の時間の半分を過ぎたら深さを増やさない項目がある。
第3段階の`H0`は、予算の再定義と途中結果の採用を積んだ最終候補で測ったものであり、信号による伸縮だけの効果を示すものではない。
事後分析は、`H0`の主因を、係数つきの予算が持ち時間を200手目までに使い切らせて中終盤で浅くなったことと結論した。
時間切れは、hardが残り時間の割合で抑えられずに持ち時間を使い切った後に起きた。
第3段階は4因子の定数をStockfishの値のまま固定し、積に正規化の定数0.48を掛け、最善手が4反復続けて同じときの早期終了を置き換えた。
第2段階の事前判定は、第1段階で再定義した予算の下で、置換表を局面ごとに空にした深さ8〜12の探索で行った。
設計書は、予算の式（相手との残り時間の差による補正と先読みの後の25%増しを除く）、次の反復の予測、および中断した反復を捨てる機構を変えず、hardを残り時間の割合で抑える項を残したうえで、この2項目を入れた。
2026年10月18日のエンジンの開発者の決定により、同書の候補はSTCの結果によらず採用し、STCは記録として行う（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
旧ADRが再提案に求めた現行の構成での再測定は、このSTCの記録で代える。

## 決定

秒読みのない時計では、反復の完了時にsoftを照合して次の反復を始めるかを決め、始めた反復はhardまで続ける現行の機構を保つ。
hardで中断した反復の結果は捨て、hardは残り時間の割合の項でも抑える。

旧ADRが代案として棄却した2つの方式は、予算を再定義しない形で採った。
局面の信号の積でsoftを伸縮させる方式は[伸縮のADR](scale-the-soft-limit-by-search-instability-and-keep-the-stable-best-move-rule.md)が、目安の時間の半分を過ぎた後の反復を浅く読む方式は[窓の深さのADR](reduce-aspiration-depth-after-fail-highs-and-late-iterations.md)が定める。
基礎予算の式は、[相手との残り時間の差による補正](shrink-the-soft-limit-when-behind-on-clocks-without-byoyomi.md)と[先読みの後の25%増し](measure-budget-from-ponderhit-and-add-a-quarter-after-pondering.md)を除いて変えない。

## 検討した代案

予算を分担そのものにして探索中の中断で止め、途中結果を採用する案は、上記の使い残しが生じてD1とD2がともに`H0`だったので採らなかった。

hardから残り時間の割合の項を外す案は、加算のある時計で残り時間が少ない局面の上限を引き上げ、第3段階との組合せで時間切れの原因になったので採らなかった（[打ち切り上限は残り時間の割合でも抑える](../lessons/cap-hard-limit-by-remaining-fraction.md)）。

hardを残り時間の割合で抑える係数`HardRemainingShare`の開始値を、Stockfishの上限時間の係数0.8097（`src/timeman.cpp:161`）に置き直す案は、設計書の判断により採らなかった。
2026年10月17日の決定は、Stockfishに同じ形の値がある既存の係数の開始値を換算値に置き直すとしているが、この係数は式の形が似ていても意味が違うので、同じ形の値には当たらない。
Stockfishの上限時間は、中断した反復でも探索を終えた手の結果を使う前提の安全弁であり、上限まで使っても計算は着手に反映される。
minaseは中断した反復の結果を捨てるので、この項は使い捨てになり得る長考を抑え、残り時間の少ない局面での時間切れを防ぐ。
開始値と範囲は、起点の47%（範囲10〜50%）のままとした。

## 結果

旧ADRの時点では、途中結果を採用する方式を採らないと決めたので、根の探索結果を保持する表は残していなかった。
その後、aspiration windowsの窓の中心を置くために、根の手ごとの評価値の平均と累計の探索ノード数を記録する仕組みが入った（[窓の形のADR](center-aspiration-windows-on-the-best-move-average-and-move-both-edges-on-failure.md)）。
伸縮の4因子のうち、評価値の下落と最善手のノード占有率はこの記録を使うが、秒読みのない時計で途中結果を採るためには使わない。

秒読みが正の時計の配分と途中結果の扱いは、[秒読みつき時計での持ち時間と秒読みの活用](../plans/byoyomi-time-usage.md)が別に定めており、この決定は秒読みのない時計に適用する。
時間管理の機構を変える提案では、採否測定の前に総思考時間と使い残しを現行と比べる（[予算の機構を変えたら総思考時間と使い残しを現行と比べる](../lessons/compare-total-time-usage-before-sprt.md)）。
設計書は、統括計画の決定により、この比較を採否の条件ではなく記録にした。
開始値の[事前対局](../measurements/time-instability-scaling/time-instability-scaling-start.md)では、1局の総思考時間は起点の0.992倍であり、完了深さは7つの手数帯のうち5つで起点を下回った。
予算を分担そのものにする方式の測定は評価関数がPSTとFMだった時期の構成で行ったので、この方式を再提案するには現行の構成での再測定が要る。
