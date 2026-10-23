# aspiration windowsは、fail highの回数、反復をまたぐ持ち越し、および目安の時間の半分を過ぎた後に始めた反復の回数で窓の深さを減らし、完了した反復は反復の深さの結果として扱う

## ステータス

承認済み。決定日は2026年10月23日。

## コンテキスト

aspiration windowsは、反復深化の各反復で前の反復の値の近くに狭い窓を置いて根を読み始め、根の値が窓を外れたら窓を移して同じ反復を読み直す手法である。
根の値が窓の上端β以上になることをfail high、下端α以下になることをfail lowという。
fail highの後の読み直しを浅くする方式は、fail highで最善手が既に良いと分かっているので、窓を確定させる読み直しを浅く済ませて次の反復へ早く進むことを狙う。
窓の置き方と移し方は[窓の形のADR](center-aspiration-windows-on-the-best-move-average-and-move-both-edges-on-failure.md)が定める。

2026年9月27日に、[棋力向上段階12](../plans/archived/strength-stage12.md)は、同じ反復の中で連続したfail highの回数`n`を数え、読み直しの深さを`max(1, 深さ − n)`とする変更を測った。
段階12は、Stockfishの導入版（コミット081af908）にならって、fail lowで回数を0へ戻し、完了した反復の深さには減らした後の深さを使い、主ワーカーだけに適用した。
STCは1,360有効ペア、得点率49.1%の`H0`だった（[測定記録](../measurements/strength-stage12/strength-stage12-aspiration-stc.md)）。
この測定は既定の評価関数がPSTだった時期のものであり、当時のminaseは、減らした深さを次の反復へ持ち越す仕組みを持たなかった。
[2026年9月12日のADR](archive/widen-only-the-failed-side-of-aspiration-windows.md)は、fail highの後に浅く読み直さないことを決定に含め、この段階12の結果を根拠として記録した。

現行のStockfishは、反復ごとにfail highの回数`failedHighCnt`を0から数え、fail highのたびに1増やし、fail lowで0に戻す（`src/search.cpp:403-450`）。
反復をまたいで持ち越す減深`failHighRecovery`は、反復を始めるたびに0を下限として2減らし、fail highが1回以上あった反復を完了したら`(failedHighCnt + 1)/2 + 2`とする（同`335-336, 405, 461-462`）。
各窓を`max(1, rootDepth − failedHighCnt − failHighRecovery − 3 × (searchAgainCounter + 1)/4)`の深さで読み、反復の深さ`rootDepth`はそのまま保って、完了した反復の出力に使う（同`410-412, 516-518`）。
この処理は主スレッドに限らず、全スレッドで行う。
`searchAgainCounter`は、全スレッドが共有する状態`increaseDepth`が偽の間に始めた反復の数であり、各スレッドが反復を始めるたびに数える（同`369-370`）。
`increaseDepth`は探索ごとに真から始まる（`src/thread.cpp:304`）。
主スレッドは、時間で止めなかった反復の境界で、先読み中なら真、そうでなければ経過時間が目安時間に4因子の積を掛けた時間`totalTime`の50%以下なら真とする（`src/search.cpp:634`）。

2026年10月17日のエンジンの開発者の決定により、[aspiration windowsの形の見直し](../plans/archived/aspiration-window-forms.md)は、fail highの減深をStockfishの形にそろえた。
[旧ADR](archive/reduce-aspiration-re-search-depth-after-fail-highs-and-keep-the-iteration-depth.md)はその決定を記録したが、`searchAgainCounter`の項は、当時のminaseにその回数を増やす時間管理の仕組みがなかったので置かなかった。
続く[持ち時間の伸縮の設計書](../plans/time-instability-scaling.md)は、同じ日のエンジンの開発者の決定により、Stockfishにだけある`increaseDepth`を入れた。
その時間の基準は、[伸縮のADR](scale-the-soft-limit-by-search-instability-and-keep-the-stable-best-move-rule.md)が定める、基礎予算の目安時間softに4因子の積を掛けて根の候補が1手なら500 msで抑えた時間`t`（hardで抑える前の時間）である。
2026年10月18日のエンジンの開発者の決定により、両設計書の候補はSTCの結果によらず採用する（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
2026年10月17日の決定は、SPSAでは各仕組みの強さを決める値だけを調整し、上下限と更新の内部の定数はStockfishの値で固定することも定めており、両設計書は減深の規則と50%の閾値をこの固定する定数に分類した。

似た規則は、[持ち時間の効率的な使用](../plans/archived/time-management-efficiency.md)の第2段階で一度実装された。
第2段階は、持ち時間の分担そのものにした目安時間の半分を過ぎたら回数を増やし、同じ`3 × (回数 + 1)/4`の減深で読む方式だった。
[事前判定](../measurements/time-management-efficiency/time-management-efficiency-stage2-bench.md)では、置換表を局面ごとに空にした探索で、浅くした反復の所要時間を直前の反復の所要時間で割った比の中央値が0.0であり、置換表の照会だけで終わる空費と判定して見送った（[予算の再定義を採らないADR](keep-discarding-hard-interrupted-iterations-and-reject-fair-share-budgets.md)）。
この判定は既定の評価関数がNNUEへ置き換わる前に深さ8〜12の探索で行い、fail highの減深との組合せは調べていない。

## 決定

各ワーカーは、USIの`go`コマンドで探索を始めるたびに0から始める2つの値として、fail highの減深の持ち越し`h_carry`と、後述の「深さを増やさない」状態の間に始めた反復の回数`k`を持つ。
深さ`d`の反復を始めるたびに`h_carry`を0を下限として2減らし、fail highの回数`h`を0にする。
各窓は`D = max(1, d − h − h_carry − ⌊3 × (k + 1) / 4⌋)`の深さで読み、fail highのたびに`h`を1増やし、fail lowのたびに`h`を0に戻す。
窓の内側で反復を完了したとき、`h`が1以上なら`h_carry`を`(h + 1)/2 + 2`とし、`h`が0なら反復の開始時に減らした`h_carry`を保つ。
主ワーカーと補助ワーカーは同じ規則に従い、補助ワーカーが深さを飛ばして反復する場合（[ADR](diversify-lazy-smp-workers-by-skipping-depths.md)）も、実際に行う反復ごとに1回だけ`h_carry`を減らす。

探索チームの全ワーカーは「深さを増やしてよい」かを表す1つの状態を共有し、状態は探索ごとに「増やしてよい」から始めて前の探索から持ち越さない。
4因子を使う探索（[伸縮のADR](scale-the-soft-limit-by-search-instability-and-keep-the-stable-best-move-rule.md)が定める、時計の秒読みが0で`movetime`がない探索）では、主ワーカーが、完了反復の境界で次の反復へ進むと決めたときにだけ状態を書き換える。
的中前の先読みなら「増やしてよい」とし、そうでなければ、的中からの経過時間が`t`の50%以下なら「増やしてよい」、超えていれば「増やさない」とする。
各ワーカーは、実際に反復を始めるときに状態が「増やさない」なら`k`を1増やし、`k`はその探索の間は減らさず、窓の読み直しでは増やさない。
補助ワーカーは深さを飛ばしたまま、実際に始めた反復ごとに1回だけ`k`を判定する。

根の探索には、反復の深さ`d`と窓の深さ`D`を別々に渡す。
探索木の中でLMRの延長を許す範囲の判定に使う`Searcher::root_depth`と、詰みを探すかの印には`d`を使う。
根のLMRの適用と減深量、子の探索の深さ、根のhistoryとcorrection historyの更新、および置換表への保存の要求には`D`を使う。
`D`を減らして完了した反復も深さ`d`の反復として扱い、進捗の通知、探索結果の深さ、最善手の安定性の判定、目安時間の伸縮で使う最善手が最後に変わってからの距離、次の反復の根の手の並べ替え、およびワーカーの結果の選択には`d`を使う。

根の置換表への保存は窓の深さ`D`で要求し、同じ局面には既存以上の深さの結果だけを書き込む既存の置換規則に従う。
このため、fail highの後に浅く読み直した窓の結果は、先に深く読んだ窓の下界を上書きしないことがある。
aspiration windowsの設計書の判断により、この挙動を受け入れる。

## 検討した代案

fail highの後も元の深さで読み直す2026年9月12日のADRの方式は、Stockfishにある項目を入れるというエンジンの開発者の決定により採らなかった。
そのADRが根拠にした段階12の`H0`はPSTの時期の結果であり、測った形は、持ち越しを持たず、完了深さを減らした後の深さとし、主ワーカーだけに適用するものだった。
本決定はその結果を新しい測定で覆したものではなく、この項目だけの棋力への寄与は測っていない。

`searchAgainCounter`の項を置かない旧ADRの形は、同じ決定により採らなかった。
旧ADRが項を置かなかった理由は回数を増やす時間管理の仕組みがないことであり、持ち時間の伸縮の設計書がその仕組みを入れた。
第2段階の事前判定の空費は、Stockfishの方針により項目を入れると決めた後も測り直しておらず、その効果は同書のSTCの記録に残す。

段階12とStockfishの導入版のように、窓の深さを減らして完了した反復を減らした深さの反復として記録する方式は採らなかった。
現行のStockfishは反復の深さ`rootDepth`を保ったまま別の深さを根の探索へ渡し、完了した反復の出力に`rootDepth`を使う。
`rootDepth`は、根より下でLMRの減深量が負のときに延長を許す範囲`ss->ply < 2 * rootDepth`、singular extensionの余裕値、同じ値の手を最善手へ繰り上げる条件、および詰みを探すかの印にも使われる（Stockfish `src/search.cpp`の順に1402行、1290〜1292行、1544行、395行）。
minaseの`Searcher::root_depth`はこのうちLMRの延長の範囲に使っており、詰みを探すかの印とともに`d`で決める。

fail highの減深を主ワーカーだけに限る段階12の方式は、現行のStockfishにその限定がないので採らなかった。

経過時間を、hardで抑えた後の目安時間`min(t, hard)`の50%と比べる案は、持ち時間の伸縮の設計書の判断により採らなかった。
Stockfishは、上限時間で抑える前の`totalTime`と比べる（`src/search.cpp:634`）。
hardで抑えた後の時間と比べると、hardが効いている局面で、Stockfishより早く窓を浅くし始める。

「深さを増やさない」状態を次の探索へ持ち越す案は、Stockfishが探索ごとに状態を初期化するので採らなかった。
`k`を窓の読み直しごとに増やす案は、Stockfishが反復の開始時に1回だけ数えるので採らなかった。

根に限って、正確な値を常に上書きする別の置換規則を設ける案は、aspiration windowsの設計書の判断により採らなかった。
Stockfishは根の手の表の先頭を置換表の手として使い、正確な値は常に上書きするので（`src/search.cpp:839-840`、`src/tt.cpp:109`）、この差はminaseの置換規則から来る。
根だけに別の規則を設けると置換規則が2つになるので、この差を受け入れた。

減深の回数、持ち越しの減り方と再設定の式、`k`による減深の式、50%の閾値、および窓の深さの下限をSPSAで調整する案は、設計書がこれらを固定する内部の定数に分類したので採らなかった。
50%は、目安の時間のさらに半分という境界の位置であり、強さを決める倍率ではない。

## 結果

減深と持ち越しの規則は`crates/minase/src/search/alphabeta/root.rs`の`FailHighReduction`と`increase_depth_reduction`に、反復の深さと窓の深さの使い分けは同じファイルの`Searcher::search_iteration`と`Searcher::search_root`のdocコメントにあり、持ち越しは`Searcher`の`fail_high_recovery`が持つ。
`k`は`searcher.rs`の`Searcher::search_again_counter`と`Searcher::begin_iteration`に、共有する状態は`team.rs`の`SharedSearch::increase_depth`に、状態を決める判定は`time.rs`の`should_increase_depth`にある。
固定した定数は`root.rs`と`time.rs`の名前つきの定数である。

4因子を使わない探索（秒読みが正の時計の探索、`movetime`を指定した探索、時計のない深さやノード数の制限の探索、および無期限の探索）では、状態は「増やしてよい」のままで`k`は0なので、`k`による減深は働かない。
固定深さの`minase dev bench`もこれに当たり、持ち時間の伸縮の設計書の全13コミットで、全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致した（[benchの記録](../measurements/time-instability-scaling/time-instability-scaling-bench.md)）。
fail highの減深は、[aspiration windowsのbenchの記録](../measurements/aspiration-window-forms/aspiration-window-forms-bench.md)では、深さ3、5、7の全15局面の探索ノード数の合計を直前のコミットより12.1%減らした。
benchの深さは反復の深さであり、benchが報告する深さ`d`の結果は、深さ`d`まで読んだ窓の結果とは限らない。

時間で止めない境界の判定には次の反復の予測完了時刻が使われるので、予測の係数`IterationRatio`が`k`の働きに関わる。
`IterationRatio`の起点の値は203%なので、最善手が安定したときの早期終了が働く境界で次の反復を始めるのは、経過時間が`min(t, hard)`の約49%以下のときだけである。
そのため、早期終了が働き、`t`がhard以下の境界では、状態は新たに「増やさない」にならない。
これは次の反復の予測を変えずに残したことによる既知の相互作用であり、`k`の働きの強さはSPSAが調整する係数を通して決まる。
SPSAの後の`IterationRatio`は152%なので、この境界は約66%に上がり、早期終了が働く境界でも、経過時間が`t`の50%を超えて約66%以下なら、次の反復を始めると同時に状態が「増やさない」になる。

反復の深さと窓の深さが違うことは、テストの深さ上限の選び方にも影響した。
[RULES.md](../rules/RULES.md)第31条の反復規則R1の下で、2つのワーカーが探索経路の周回を評価する探索を10回繰り返すテスト`article_31_r1_two_workers_score_the_path_cycle_ten_times`は、深さ上限3では、10回のうち1回だけ周回を読めずに失敗した。
fail highの後に窓の深さが減るので、深さ上限3では最後の窓の深さが1になる回が生じる。
どの回でそうなるかは、2つのワーカーが置換表を共有する順序によって変わる。
深さ上限を4にすると、深さ3でfail highがもう1回増えても深さ4の反復の窓の深さは2以上になり、2手先で完成する周回を読めるので、テストの深さ上限を4に直した。
固定の深さで読める手を期待するテストは、窓の深さが反復の深さより浅くなり得ることを見込んで深さ上限を選ぶ必要がある。

浅い窓の結果が深い記録を上書きしないとき、次の反復の根の先頭手は、置換表に残った、先に深く読んだ窓の手になり、直前の完了反復の最善手と異なることがある。
この差が残るのは、後の反復がその深さ以上の窓を内側で完了するまでの間である。

完了した反復を深さ`d`として扱うので、窓の深さを減らして完了したワーカーの結果が、より深い窓で1つ浅い反復を完了したワーカーの結果より優先されることがある。
段階12の実装が減らした深さを完了深さとした理由の1つはこの優先を避けることだったが、SPSAとSTCは`Threads=1`で行うので、補助ワーカーへの適用とあわせてこの影響は測っていない。
報告する完了深さも反復の深さなので、`k`やfail highで窓を浅く読んだ手の完了深さは、実際に読んだ窓の深さより深く数えられる。
開始値で行った持ち時間の伸縮の[事前対局](../measurements/time-instability-scaling/time-instability-scaling-start.md)でも、手数帯ごとの完了深さを起点と比べるときにこの差は分けていない。

fail highの減深は窓の形の変更とあわせて1つの候補とし、ほかの係数とまとめてSPSAで調整した。
その版は、STCで設計書を始めた時点のmasterに対して上限の3,000ペアまでに判定できず、LLRは+2.177、平均得点から求めた差は約+8.6 Eloだった（[STCの記録](../measurements/aspiration-window-forms/aspiration-window-forms-stc.md)）。
[事前対局](../measurements/aspiration-window-forms/aspiration-window-forms-smoke.md)では、その版が報告した完了深さは、400手目より前のすべての手数帯で起点より約0.7〜1.1手浅かった。
`k`による減深は、持ち時間の伸縮の項目とあわせて1つの候補とし、ほかの係数とまとめてSPSAで調整した（[SPSAの記録](../measurements/time-instability-scaling/time-instability-scaling.md)）。
その版は、STCで持ち時間の伸縮の設計書を始めた時点のmasterに対して303ペア（有効297）でLLR +2.945の`H1`となり、平均得点から求めた差は約+51 Eloだった（[STCの記録](../measurements/time-instability-scaling/time-instability-scaling-stc.md)）。
この差は持ち時間の伸縮のほかの項目と探索の係数の調整し直しを含むので、`k`による減深だけの寄与ではない。
秒読みつきの短い条件で`H1`とならなかったので、`k`による減深も、4因子とともに秒読みが0の時計に限った（[測定記録](../measurements/time-instability-scaling/time-instability-scaling-byoyomi-short.md)）。
